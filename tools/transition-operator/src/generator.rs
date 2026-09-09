use crate::candidate::{GeneratedCandidate, deduplicate_validated_plans};
use crate::canonical::canonical_json;
use crate::error::{Error, Result};
use crate::geometry::{GeometryProposal, shortlist_geometries, validate_fallback_geometry};
use crate::identity::{candidate_id, finalize_plan};
use crate::model::{
    AudioFormat, ChannelOrder, FeatureSnapshotRef, LimiterProfile, OperatorPlanBody, OutputSafety,
    OutputSafetyProfile, Provenance, SourceRef, Sources, TemplateRef, Timeline, TruePeakProfile,
};
use crate::safety::calculate_pair_output_gain;
use crate::templates::{
    FamilyApplicability, TemplateDraft, TemplateId, TemplateInputs, emit_dynamics_template,
    emit_echo_tail_template, emit_gain_template, emit_safe_fallback, emit_spectral_template,
    rich_template_families, template_registry,
};
use crate::validation::{ExpectedSource, PlanValidator, TemplateFeatureView, ValidationContext};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerationConfig {
    pub generator_id: String,
    pub generator_version: String,
    pub generator_config_sha256: String,
    pub seed_hex_u64: String,
}

#[derive(Clone, Debug)]
pub struct GenerationRequest {
    pub outgoing: SourceRef,
    pub incoming: SourceRef,
    pub feature_snapshot: FeatureSnapshotRef,
    pub fallback_geometry: GeometryProposal,
    pub geometries: Vec<GeometryProposal>,
    pub features: TemplateInputs,
    pub outgoing_true_peak_mdbtp: Option<i64>,
    pub incoming_true_peak_mdbtp: Option<i64>,
    pub config: GenerationConfig,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateSetHash(String);

impl CandidateSetHash {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct CandidateSet {
    pub candidates: Vec<GeneratedCandidate>,
    pub set_hash: CandidateSetHash,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerationDiagnostic {
    pub template_id: Option<TemplateId>,
    pub recipe_id: Option<String>,
    pub geometry_id: Option<String>,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct PairGenerationResult {
    pub candidate_set: CandidateSet,
    pub family_applicability: Vec<FamilyApplicability>,
    pub diagnostics: Vec<GenerationDiagnostic>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapDecision {
    Continue,
    FallbackOnly,
}

pub fn cap_decision(attempts: usize, accepted: usize) -> CapDecision {
    if attempts > 64 || accepted > 64 {
        CapDecision::FallbackOnly
    } else {
        CapDecision::Continue
    }
}

pub fn generate_candidates(request: GenerationRequest) -> Result<PairGenerationResult> {
    validate_request(&request)?;
    let fallback_draft = (|| {
        validate_fallback_geometry(&request.fallback_geometry)?;
        validate_geometry_context(&request, &request.fallback_geometry)?;
        let draft = emit_safe_fallback(&request.fallback_geometry)?;
        prevalidate_draft(&request, &draft)?;
        Ok(draft)
    })()
    .map_err(|error: Error| {
        Error::new(
            "PAIR_INELIGIBLE",
            format!(
                "fallback validation failed with {}: {}",
                error.code(),
                error.message()
            ),
        )
    })?;
    let geometries = shortlist_geometries(request.geometries.clone())?;
    for geometry in &geometries {
        validate_geometry_context(&request, geometry)?;
    }

    let applicability = rich_template_families(&request.features);
    let tie_order: BTreeMap<_, _> = template_registry()
        .iter()
        .enumerate()
        .map(|(index, family)| (family.id, index))
        .collect();
    let mut applicable: Vec<_> = applicability
        .iter()
        .filter(|value| value.applicable)
        .cloned()
        .collect();
    applicable.sort_by(|left, right| {
        right
            .need_score_ppm
            .cmp(&left.need_score_ppm)
            .then_with(|| tie_order[&left.id].cmp(&tie_order[&right.id]))
    });

    let mut diagnostics = Vec::new();
    for value in applicability.iter().filter(|value| !value.applicable) {
        diagnostics.push(diagnostic(
            Some(value.id),
            None,
            None,
            "TEMPLATE_INAPPLICABLE",
            "required template predicates are false",
        ));
    }
    for value in applicable.iter().skip(6) {
        diagnostics.push(diagnostic(
            Some(value.id),
            None,
            None,
            "FAMILY_QUOTA_PRUNED",
            "family was pruned by the six-family limit",
        ));
    }
    applicable.truncate(6);

    let mut attempts = 1usize;
    let mut rich_drafts = Vec::new();
    for selected in &applicable {
        let family = template_registry()
            .iter()
            .find(|family| family.id == selected.id)
            .expect("closed template registry");
        for recipe in family.recipes {
            attempts += 1;
            if cap_decision(attempts, 0) == CapDecision::FallbackOnly {
                diagnostics.push(diagnostic(
                    None,
                    None,
                    None,
                    "GENERATOR_HARD_CAP_EXCEEDED",
                    "recipe attempt hard cap exceeded",
                ));
                return fallback_only(request, fallback_draft, applicability, diagnostics);
            }
            let geometry = match crate::templates::bind_recipe(recipe, &geometries) {
                Ok(geometry) => geometry,
                Err(error) => {
                    diagnostics.push(diagnostic(
                        Some(family.id),
                        Some(recipe.id),
                        None,
                        error.code(),
                        error.message(),
                    ));
                    continue;
                }
            };
            match emit_recipe(family.id, recipe, geometry, &request.features) {
                Ok(draft) => match prevalidate_draft(&request, &draft) {
                    Ok(()) => rich_drafts.push(draft),
                    Err(error) => diagnostics.push(diagnostic(
                        Some(family.id),
                        Some(recipe.id),
                        Some(&geometry.geometry_id),
                        error.code(),
                        error.message(),
                    )),
                },
                Err(error) => diagnostics.push(diagnostic(
                    Some(family.id),
                    Some(recipe.id),
                    Some(&geometry.geometry_id),
                    error.code(),
                    error.message(),
                )),
            }
        }
    }

    let margin = rich_drafts
        .iter()
        .chain(std::iter::once(&fallback_draft))
        .map(operation_margin)
        .max()
        .unwrap_or(0);
    let pair_gain = calculate_pair_output_gain(
        request.outgoing_true_peak_mdbtp,
        request.incoming_true_peak_mdbtp,
        margin,
    )?;
    let mut finalized = Vec::new();
    finalized.push(finalize_draft(&request, &fallback_draft, pair_gain)?);
    for draft in rich_drafts {
        match finalize_draft(&request, &draft, pair_gain) {
            Ok(candidate) => finalized.push(candidate),
            Err(error) => diagnostics.push(diagnostic(
                Some(draft.template_id),
                Some(draft.recipe_id.as_str()),
                Some(&draft.geometry_id),
                error.code(),
                error.message(),
            )),
        }
    }
    if cap_decision(attempts, finalized.len()) == CapDecision::FallbackOnly {
        diagnostics.push(diagnostic(
            None,
            None,
            None,
            "GENERATOR_HARD_CAP_EXCEEDED",
            "accepted candidate hard cap exceeded",
        ));
        return fallback_only(request, fallback_draft, applicability, diagnostics);
    }

    let (mut candidates, duplicates) = deduplicate_validated_plans(finalized);
    for duplicate in duplicates {
        diagnostics.push(diagnostic(
            None,
            None,
            None,
            "DUPLICATE_AUDIO_SEMANTICS",
            &format!(
                "duplicate {} retained earlier {}",
                duplicate.duplicate_candidate_id.as_str(),
                duplicate.earlier_candidate_id.as_str()
            ),
        ));
    }
    candidates = apply_soft_cap(candidates, &applicable, &mut diagnostics);
    candidates.sort_by(|left, right| left.candidate_id.as_str().cmp(right.candidate_id.as_str()));
    let set_hash = candidate_set_hash(&request, &candidates)?;
    Ok(PairGenerationResult {
        candidate_set: CandidateSet {
            candidates,
            set_hash,
        },
        family_applicability: applicability,
        diagnostics,
    })
}

fn prevalidate_draft(request: &GenerationRequest, draft: &TemplateDraft) -> Result<()> {
    // Pair gain is injected only after every draft has passed the complete plan
    // validator. Zero is a valid non-boosting placeholder and cannot affect any
    // template, operation, source, timeline, or capability checks.
    finalize_draft(request, draft, 0).map(|_| ())
}

fn fallback_only(
    request: GenerationRequest,
    fallback: TemplateDraft,
    applicability: Vec<FamilyApplicability>,
    diagnostics: Vec<GenerationDiagnostic>,
) -> Result<PairGenerationResult> {
    let gain = calculate_pair_output_gain(
        request.outgoing_true_peak_mdbtp,
        request.incoming_true_peak_mdbtp,
        0,
    )?;
    let candidate = finalize_draft(&request, &fallback, gain)?;
    let candidates = vec![candidate];
    let set_hash = candidate_set_hash(&request, &candidates)?;
    Ok(PairGenerationResult {
        candidate_set: CandidateSet {
            candidates,
            set_hash,
        },
        family_applicability: applicability,
        diagnostics,
    })
}

fn emit_recipe(
    template_id: TemplateId,
    recipe: &crate::templates::RecipeSpec,
    geometry: &GeometryProposal,
    inputs: &TemplateInputs,
) -> Result<TemplateDraft> {
    match template_id {
        TemplateId::ShapedHandoff | TemplateId::BeatCut | TemplateId::EnergyRamp => {
            emit_gain_template(template_id, recipe, geometry, inputs)
        }
        TemplateId::BassHandoff | TemplateId::SpectralHandoff => {
            emit_spectral_template(template_id, recipe, geometry, inputs)
        }
        TemplateId::DuckedOverlap | TemplateId::RhythmicHandoff => {
            emit_dynamics_template(template_id, recipe, geometry, inputs)
        }
        TemplateId::EchoTailHandoff => emit_echo_tail_template(recipe, geometry, inputs),
        TemplateId::SafeCrossfade => Err(Error::new(
            "INVALID_GENERATOR_STATE",
            "safe fallback is generated independently",
        )),
    }
}

fn finalize_draft(
    request: &GenerationRequest,
    draft: &TemplateDraft,
    pair_gain: i64,
) -> Result<GeneratedCandidate> {
    let geometry = if draft.template_id == TemplateId::SafeCrossfade {
        &request.fallback_geometry
    } else {
        request
            .geometries
            .iter()
            .find(|geometry| geometry.geometry_id == draft.geometry_id)
            .ok_or_else(|| Error::new("GEOMETRY_NOT_FOUND", "draft geometry is unavailable"))?
    };
    let body = body_from_draft(request, draft, geometry, pair_gain);
    let expected = body.clone();
    let view = DraftFeatureView {
        snapshot_sha256: &request.feature_snapshot.sha256,
        expected: &expected,
        applicable: draft.template_id == TemplateId::SafeCrossfade
            || template_registry()
                .iter()
                .find(|family| family.id == draft.template_id)
                .is_some_and(|family| family.applicability(&request.features).applicable),
    };
    let context = ValidationContext {
        outgoing: ExpectedSource::from_ref(&request.outgoing),
        incoming: ExpectedSource::from_ref(&request.incoming),
        features: &view,
    };
    let validated_body = PlanValidator::validate_body(body, &context)?;
    let plan = finalize_plan(validated_body)?;
    let id = candidate_id(plan.plan_hash());
    Ok(GeneratedCandidate {
        candidate_id: id,
        template_id: draft.template_id,
        recipe_id: draft.recipe_id.as_str().to_owned(),
        plan,
    })
}

fn body_from_draft(
    request: &GenerationRequest,
    draft: &TemplateDraft,
    geometry: &GeometryProposal,
    pair_gain: i64,
) -> OperatorPlanBody {
    let mut outgoing = request.outgoing.clone();
    outgoing.cue_id = geometry.outgoing_cue_id.clone();
    outgoing.cue_source_frame = geometry.outgoing_cue_source_frame;
    let mut incoming = request.incoming.clone();
    incoming.cue_id = geometry.incoming_cue_id.clone();
    incoming.cue_source_frame = geometry.incoming_cue_source_frame;
    OperatorPlanBody {
        schema_version: "transition-operator-plan/1".into(),
        template: TemplateRef {
            id: draft.template_id.as_str().into(),
            version: 1,
            recipe_id: draft.recipe_id.as_str().into(),
        },
        format: AudioFormat {
            sample_rate_hz: 44_100,
            channels: 2,
            channel_order: ChannelOrder::StereoLr,
        },
        sources: Sources { outgoing, incoming },
        timeline: Timeline {
            dry_start_frame: draft.dry_start_frame,
            dry_handoff_frame: 0,
            effect_end_frame: draft.effect_end_frame,
        },
        operations: draft.operations.clone(),
        output_safety: output_safety(pair_gain),
        feature_snapshot: request.feature_snapshot.clone(),
        provenance: Provenance {
            generator_id: request.config.generator_id.clone(),
            generator_version: request.config.generator_version.clone(),
            generator_config_sha256: request.config.generator_config_sha256.clone(),
            seed_hex_u64: request.config.seed_hex_u64.clone(),
            geometry_id: draft.geometry_id.clone(),
        },
    }
}

fn output_safety(pair_gain: i64) -> OutputSafety {
    OutputSafety {
        profile: OutputSafetyProfile::TransitionOutputSafetyV1,
        pair_output_gain_mdb: pair_gain,
        sample_peak_ceiling_mdbfs: -1_200,
        true_peak_target_mdbtp: -1_000,
        limiter_profile: LimiterProfile::LookaheadPeakLimiterV1,
        lookahead_frames: 221,
        release_frames: 4_410,
        maximum_gain_reduction_mdb: 3_000,
        maximum_active_fraction_ppm: 50_000,
        activity_threshold_mdb: 100,
        true_peak_measurement_profile: TruePeakProfile::Bs1770_4xV1,
    }
}

fn operation_margin(draft: &TemplateDraft) -> i64 {
    if draft.operations.iter().any(|operation| {
        matches!(
            operation,
            crate::model::Operation::FilterEnvelope(_)
                | crate::model::Operation::CrossoverBandGain(_)
                | crate::model::Operation::FeedforwardDelayTail(_)
        )
    }) {
        3_000
    } else if draft
        .operations
        .iter()
        .any(|operation| matches!(operation, crate::model::Operation::TimeMap(_)))
    {
        1_000
    } else {
        0
    }
}

fn apply_soft_cap(
    candidates: Vec<GeneratedCandidate>,
    selected: &[FamilyApplicability],
    diagnostics: &mut Vec<GenerationDiagnostic>,
) -> Vec<GeneratedCandidate> {
    if candidates.len() <= 48 {
        return candidates;
    }
    let fallback = candidates
        .iter()
        .find(|candidate| candidate.template_id == TemplateId::SafeCrossfade)
        .expect("validated generation always has fallback")
        .clone();
    let mut by_family = BTreeMap::<TemplateId, Vec<GeneratedCandidate>>::new();
    for candidate in candidates
        .into_iter()
        .filter(|candidate| candidate.template_id != TemplateId::SafeCrossfade)
    {
        by_family
            .entry(candidate.template_id)
            .or_default()
            .push(candidate);
    }
    for values in by_family.values_mut() {
        values.sort_by(|left, right| left.candidate_id.as_str().cmp(right.candidate_id.as_str()));
    }
    let family_ids: BTreeMap<_, _> = by_family
        .iter()
        .map(|(family, values)| {
            (
                *family,
                values
                    .iter()
                    .map(|candidate| candidate.candidate_id.as_str().to_owned())
                    .collect(),
            )
        })
        .collect();
    let retained_ids =
        round_robin_candidate_ids(fallback.candidate_id.as_str(), &family_ids, selected, 48);
    let kept_ids: BTreeSet<_> = retained_ids.into_iter().collect();
    let mut kept = vec![fallback];
    for candidate in by_family.into_values().flatten() {
        if kept_ids.contains(candidate.candidate_id.as_str()) {
            kept.push(candidate);
            continue;
        }
        diagnostics.push(diagnostic(
            Some(candidate.template_id),
            Some(&candidate.recipe_id),
            Some(&candidate.plan.plan().provenance.geometry_id),
            "SOFT_CAP_PRUNED",
            "candidate was pruned by deterministic family rounds",
        ));
    }
    kept
}

fn round_robin_candidate_ids(
    fallback_id: &str,
    by_family: &BTreeMap<TemplateId, Vec<String>>,
    selected: &[FamilyApplicability],
    maximum_total: usize,
) -> Vec<String> {
    let mut positions = BTreeMap::<TemplateId, usize>::new();
    let mut kept = vec![fallback_id.to_owned()];
    while kept.len() < maximum_total {
        let mut found = false;
        for family in selected {
            let position = positions.entry(family.id).or_default();
            if let Some(candidate_id) = by_family
                .get(&family.id)
                .and_then(|values| values.get(*position))
            {
                kept.push(candidate_id.clone());
                *position += 1;
                found = true;
                if kept.len() == maximum_total {
                    break;
                }
            }
        }
        if !found {
            break;
        }
    }
    kept
}

#[derive(Serialize)]
struct CandidateSetProjection<'a> {
    generator_id: &'a str,
    generator_version: &'a str,
    generator_config_sha256: &'a str,
    seed_hex_u64: &'a str,
    outgoing_pcm_sha256: &'a str,
    incoming_pcm_sha256: &'a str,
    candidates: Vec<CandidateIdentityProjection<'a>>,
}

#[derive(Serialize)]
struct CandidateIdentityProjection<'a> {
    candidate_id: &'a str,
    plan_sha256: String,
}

fn candidate_set_hash(
    request: &GenerationRequest,
    candidates: &[GeneratedCandidate],
) -> Result<CandidateSetHash> {
    let projection = CandidateSetProjection {
        generator_id: &request.config.generator_id,
        generator_version: &request.config.generator_version,
        generator_config_sha256: &request.config.generator_config_sha256,
        seed_hex_u64: &request.config.seed_hex_u64,
        outgoing_pcm_sha256: &request.outgoing.pcm_sha256,
        incoming_pcm_sha256: &request.incoming.pcm_sha256,
        candidates: candidates
            .iter()
            .map(|candidate| CandidateIdentityProjection {
                candidate_id: candidate.candidate_id.as_str(),
                plan_sha256: candidate.plan.plan_hash().hex(),
            })
            .collect(),
    };
    Ok(CandidateSetHash(hex(&Sha256::digest(canonical_json(
        &projection,
    )?))))
}

fn validate_request(request: &GenerationRequest) -> Result<()> {
    if request.config.seed_hex_u64 != "0000000000000000" {
        return Err(Error::new(
            "INVALID_GENERATOR_SEED",
            "v1 generator seed must be zero",
        ));
    }
    if request.feature_snapshot.schema_version != "transition-feature-snapshot/2"
        || !is_hash(&request.feature_snapshot.sha256)
        || !is_hash(&request.config.generator_config_sha256)
        || !printable_ascii(&request.config.generator_id)
        || !printable_ascii(&request.config.generator_version)
    {
        return Err(Error::new(
            "INVALID_GENERATION_REQUEST",
            "generation identity fields are invalid",
        ));
    }
    Ok(())
}

fn validate_geometry_context(
    request: &GenerationRequest,
    geometry: &GeometryProposal,
) -> Result<()> {
    if geometry.outgoing_pcm_sha256 != request.outgoing.pcm_sha256
        || geometry.incoming_pcm_sha256 != request.incoming.pcm_sha256
        || geometry.feature_snapshot_sha256 != request.feature_snapshot.sha256
        || geometry.outgoing_pcm_frame_count != request.outgoing.pcm_frame_count
        || geometry.incoming_pcm_frame_count != request.incoming.pcm_frame_count
    {
        return Err(Error::new(
            "GEOMETRY_CONTEXT_MISMATCH",
            "geometry source or feature identity differs from request",
        ));
    }
    Ok(())
}

struct DraftFeatureView<'a> {
    snapshot_sha256: &'a str,
    expected: &'a OperatorPlanBody,
    applicable: bool,
}

impl TemplateFeatureView for DraftFeatureView<'_> {
    fn snapshot_sha256(&self) -> &str {
        self.snapshot_sha256
    }

    fn covers_plan_window(&self, _body: &OperatorPlanBody) -> bool {
        true
    }

    fn template_is_applicable(&self, _body: &OperatorPlanBody) -> bool {
        self.applicable
    }

    fn recipe_matches(&self, body: &OperatorPlanBody) -> bool {
        body == self.expected
    }
}

fn diagnostic(
    template_id: Option<TemplateId>,
    recipe_id: Option<&str>,
    geometry_id: Option<&str>,
    code: &str,
    message: &str,
) -> GenerationDiagnostic {
    GenerationDiagnostic {
        template_id,
        recipe_id: recipe_id.map(str::to_owned),
        geometry_id: geometry_id.map(str::to_owned),
        code: code.to_owned(),
        message: message.chars().take(160).collect(),
    }
}

fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn printable_ascii(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_cap_selection_is_family_round_robin_in_selection_order() {
        let selected = vec![
            FamilyApplicability {
                id: TemplateId::BeatCut,
                applicable: true,
                need_score_ppm: 900_000,
            },
            FamilyApplicability {
                id: TemplateId::ShapedHandoff,
                applicable: true,
                need_score_ppm: 800_000,
            },
        ];
        let by_family = BTreeMap::from([
            (
                TemplateId::BeatCut,
                vec![
                    "cand1-b1".to_owned(),
                    "cand1-b2".to_owned(),
                    "cand1-b3".to_owned(),
                ],
            ),
            (
                TemplateId::ShapedHandoff,
                vec![
                    "cand1-s1".to_owned(),
                    "cand1-s2".to_owned(),
                    "cand1-s3".to_owned(),
                ],
            ),
        ]);
        assert_eq!(
            round_robin_candidate_ids("cand1-f", &by_family, &selected, 4),
            ["cand1-f", "cand1-b1", "cand1-s1", "cand1-b2"]
        );
    }
}
