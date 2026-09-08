use transition_operator::{
    CandidateId, CapabilitySimplification, CurrentLoweringStatus, ExpectedSource, Operation,
    OperatorPlan, PlanValidator, RendererCapabilities, SupportResult, TemplateFeatureView,
    ValidationContext, candidate_id, classify_current_transition_plan, compare_capabilities,
    derive_capabilities, frame_to_exact_nanoseconds,
};

const SAFE_RAW: &[u8] = include_bytes!("fixtures/plans/safe-crossfade.json");

struct Features(String);
impl TemplateFeatureView for Features {
    fn snapshot_sha256(&self) -> &str {
        &self.0
    }
    fn covers_plan_window(&self, _: &transition_operator::OperatorPlanBody) -> bool {
        true
    }
    fn template_is_applicable(&self, _: &transition_operator::OperatorPlanBody) -> bool {
        true
    }
    fn recipe_matches(&self, _: &transition_operator::OperatorPlanBody) -> bool {
        true
    }
}

fn fixture() -> OperatorPlan {
    transition_operator::require_canonical_json(SAFE_RAW.strip_suffix(b"\n").unwrap_or(SAFE_RAW))
        .unwrap()
}

fn context<'a>(plan: &'a OperatorPlan, features: &'a Features) -> ValidationContext<'a> {
    ValidationContext {
        outgoing: ExpectedSource::from_ref(&plan.sources.outgoing),
        incoming: ExpectedSource::from_ref(&plan.sources.incoming),
        features,
    }
}

fn finalized(plan: &OperatorPlan) -> transition_operator::ValidatedPlan {
    let features = Features(plan.feature_snapshot.sha256.clone());
    let body = PlanValidator::validate_body(plan.body(), &context(plan, &features)).unwrap();
    transition_operator::finalize_plan(body).unwrap()
}

#[test]
fn plan_and_candidate_ids_are_domain_separated_and_self_consistent() {
    let validated = finalized(&fixture());
    assert_eq!(
        validated.plan().plan_id,
        format!("op1-{}", validated.plan_hash().hex())
    );
    let candidate = candidate_id(validated.plan_hash());
    assert!(candidate.as_str().starts_with("cand1-"));
    assert_ne!(&candidate.as_str()[6..], validated.plan_hash().hex());
    assert_eq!(
        candidate,
        CandidateId::from_plan_hash(validated.plan_hash())
    );
}

#[test]
fn audio_semantics_excludes_provenance_but_includes_audio_fields() {
    let original = fixture();
    let mut provenance_change = original.clone();
    provenance_change.provenance.generator_version = "1.0.1".into();
    let mut audio_change = original.clone();
    audio_change.template.id = "shaped_handoff".into();
    audio_change.template.recipe_id = "asym_test".into();
    let Operation::GainEnvelope(gain) = &mut audio_change.operations[0] else {
        unreachable!()
    };
    gain.points.insert(
        1,
        transition_operator::EnvelopePoint {
            frame: -110_250,
            value_ppm: 600_000,
        },
    );
    gain.interpolations
        .push(transition_operator::Interpolation::Linear);

    let a = finalized(&original);
    let b = finalized(&provenance_change);
    let c = finalized(&audio_change);
    assert_ne!(a.plan_hash(), b.plan_hash());
    assert_eq!(a.audio_semantics_hash(), b.audio_semantics_hash());
    assert_ne!(a.audio_semantics_hash(), c.audio_semantics_hash());
}

#[test]
fn canonical_input_recomputes_and_rejects_wrong_plan_id() {
    let plan = fixture();
    let features = Features(plan.feature_snapshot.sha256.clone());
    assert_eq!(
        PlanValidator::validate_bytes(
            SAFE_RAW.strip_suffix(b"\n").unwrap_or(SAFE_RAW),
            &context(&plan, &features)
        )
        .unwrap_err()
        .code(),
        "PLAN_ID_MISMATCH"
    );
    let valid = finalized(&plan);
    assert_eq!(
        PlanValidator::validate_bytes(valid.canonical_bytes(), &context(valid.plan(), &features))
            .unwrap()
            .plan_hash(),
        valid.plan_hash()
    );
}

#[test]
fn capabilities_are_derived_and_sorted_from_plan_contents() {
    let plan = finalized(&fixture());
    let capabilities = derive_capabilities(plan.plan());
    assert_eq!(
        capabilities.requirements(),
        &[
            "format:pcm_f64_stereo_44100_v1",
            "interpolation:linear",
            "operation:gain_envelope",
            "output_safety:transition_output_safety_v1",
            "source:pcm_s16le_stereo_44100_v1",
            "true_peak:bs1770_4x_v1",
        ]
    );
    assert_eq!(capabilities.max_envelope_points, 2);
    assert_eq!(capabilities.lookahead_frames, 221);

    let supporting = RendererCapabilities::reference_for_tests(&capabilities);
    assert_eq!(
        compare_capabilities(&capabilities, &supporting),
        SupportResult::Supported
    );
    let mut missing = supporting;
    missing
        .supported_requirements
        .retain(|value| value != "operation:gain_envelope");
    assert_eq!(
        compare_capabilities(&capabilities, &missing),
        SupportResult::Unsupported
    );

    let mut simplifying = RendererCapabilities::reference_for_tests(&capabilities);
    simplifying
        .supported_requirements
        .retain(|value| value != "operation:gain_envelope");
    simplifying.simplifications.push(CapabilitySimplification {
        unsupported_requirement: "operation:gain_envelope".into(),
        transform_id: "explicit_gain_transform/1".into(),
    });
    assert_eq!(
        compare_capabilities(&capabilities, &simplifying),
        SupportResult::SupportedWithSimplification {
            transform_id: "explicit_gain_transform/1".into()
        }
    );

    let mut unsorted = RendererCapabilities::reference_for_tests(&capabilities);
    unsorted.supported_requirements.reverse();
    assert_eq!(
        compare_capabilities(&capabilities, &unsorted),
        SupportResult::Unsupported
    );
}

#[test]
fn current_compatibility_never_claims_full_phase_one_lowering() {
    let plan = finalized(&fixture());
    let compatibility = classify_current_transition_plan(plan.plan());
    assert_eq!(compatibility.musical_core, CurrentLoweringStatus::Lowerable);
    assert_eq!(compatibility.full_plan, CurrentLoweringStatus::NotLowerable);

    let mut equal_power = fixture();
    equal_power.template.id = "shaped_handoff".into();
    equal_power.template.recipe_id = "eq_5s".into();
    let Operation::GainEnvelope(outgoing) = &mut equal_power.operations[0] else {
        unreachable!()
    };
    outgoing.interpolations[0] = transition_operator::Interpolation::QuarterCosine;
    let Operation::GainEnvelope(incoming) = &mut equal_power.operations[1] else {
        unreachable!()
    };
    incoming.interpolations[0] = transition_operator::Interpolation::QuarterSine;
    assert_eq!(
        classify_current_transition_plan(finalized(&equal_power).plan()).musical_core,
        CurrentLoweringStatus::NotLowerable
    );
}

#[test]
fn frame_nanosecond_compatibility_requires_an_exact_round_trip() {
    assert_eq!(frame_to_exact_nanoseconds(44_100).unwrap(), 1_000_000_000);
    assert_eq!(frame_to_exact_nanoseconds(-44_100).unwrap(), -1_000_000_000);
    assert_eq!(frame_to_exact_nanoseconds(1).unwrap(), 22_676);
}
