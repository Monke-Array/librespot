use transition_operator::{
    CapDecision, DurationMode, FeatureSnapshotRef, GeneratedCandidate, GenerationConfig,
    GenerationRequest, GeometryProposal, GeometryProposalCore, SourceRef, TemplateId,
    TemplateInputs, calculate_pair_output_gain, cap_decision, deduplicate_validated_plans,
    generate_candidates, geometry_id,
};

fn hash(ch: char) -> String {
    std::iter::repeat_n(ch, 64).collect()
}

fn source(track: &str, pcm: char) -> SourceRef {
    SourceRef {
        track_id: track.into(),
        pcm_profile: "pcm_s16le_stereo_44100_v1".into(),
        pcm_sha256: hash(pcm),
        pcm_frame_count: 3_000_000,
        cue_id: format!("{track}-cue"),
        cue_source_frame: 1_500_000,
    }
}

fn geometry(
    mode: DurationMode,
    frames: i64,
    bars: i64,
    quality: i64,
    tag: &str,
) -> GeometryProposal {
    GeometryProposal::finalize(GeometryProposalCore {
        outgoing_pcm_sha256: hash('a'),
        incoming_pcm_sha256: hash('b'),
        feature_snapshot_sha256: hash('c'),
        outgoing_pcm_frame_count: 3_000_000,
        incoming_pcm_frame_count: 3_000_000,
        outgoing_cue_id: format!("out-{tag}"),
        outgoing_cue_source_frame: 1_500_000,
        incoming_cue_id: format!("in-{tag}"),
        incoming_cue_source_frame: 1_500_000,
        duration_mode: mode,
        requested_dry_frames: frames,
        resolved_bar_count: bars,
        incoming_source_rate_ppm: 1_000_000,
        cue_confidence_ppm: 900_000,
        beat_confidence_ppm: 900_000,
        downbeat_confidence_ppm: 900_000,
        alignment_error_ppm_of_beat: 0,
        geometry_quality_ppm: quality,
        feature_window_ids: vec![format!("window-{tag}")],
    })
    .unwrap()
}

fn rich_inputs() -> TemplateInputs {
    TemplateInputs {
        cue_confidence_ppm: Some(900_000),
        beat_confidence_ppm: Some(900_000),
        downbeat_confidence_ppm: Some(900_000),
        alignment_error_ppm_of_beat: Some(0),
        outgoing_vocal_activity_ppm: Some(100_000),
        incoming_vocal_activity_ppm: Some(100_000),
        outgoing_vocal_sustained: Some(false),
        vocal_collision_ppm: Some(500_000),
        vocal_collision_span_frames: Some(30_000),
        vocal_collision_start_frame: Some(-50_000),
        vocal_collision_end_frame: Some(-20_000),
        outgoing_vocal_collision_strength_ppm: Some(700_000),
        incoming_vocal_collision_strength_ppm: Some(600_000),
        transient_collision_ppm: Some(100_000),
        transient_collision_span_frames: Some(20_000),
        transient_collision_start_frame: Some(-48_000),
        transient_collision_end_frame: Some(-28_000),
        outgoing_transient_collision_strength_ppm: Some(700_000),
        incoming_transient_collision_strength_ppm: Some(600_000),
        two_beats_frames: Some(44_100),
        outgoing_transient_activity_ppm: Some(700_000),
        incoming_transient_activity_ppm: Some(600_000),
        outgoing_transient_density_ppm: Some(200_000),
        outgoing_bass_occupancy_ppm: Some(500_000),
        incoming_bass_occupancy_ppm: Some(500_000),
        bass_collision_ppm: Some(600_000),
        outgoing_spectral_stability_ppm: Some(800_000),
        incoming_spectral_stability_ppm: Some(800_000),
        spectral_overlap_ppm: Some(600_000),
        outgoing_energy_variability_mdb: Some(1_000),
        incoming_energy_variability_mdb: Some(1_000),
        energy_delta_mdb: Some(4_000),
        outgoing_hard_cut_safe: Some(true),
        incoming_hard_cut_safe: Some(true),
        beat_frames: vec![
            -176_400, -154_350, -132_300, -110_250, -88_200, -66_150, -44_100, -22_050, 0,
        ],
        meter_beats: Some(4),
    }
}

fn request() -> GenerationRequest {
    let fallback = geometry(DurationMode::Seconds, 220_500, 0, 1_000_000, "fallback");
    GenerationRequest {
        outgoing: source("outgoing-track", 'a'),
        incoming: source("incoming-track", 'b'),
        feature_snapshot: FeatureSnapshotRef {
            schema_version: "transition-feature-snapshot/3".into(),
            sha256: hash('c'),
        },
        fallback_geometry: fallback.clone(),
        geometries: vec![
            geometry(DurationMode::Cut, 1, 0, 950_000, "cut"),
            geometry(DurationMode::Seconds, 132_300, 0, 940_000, "three"),
            fallback,
            geometry(DurationMode::Bars, 88_200, 1, 930_000, "one-bar"),
            geometry(DurationMode::Bars, 176_400, 2, 920_000, "two-bar"),
            geometry(DurationMode::Bars, 352_800, 4, 910_000, "four-bar"),
        ],
        features: rich_inputs(),
        outgoing_true_peak_mdbtp: Some(-1_000),
        incoming_true_peak_mdbtp: Some(-2_000),
        config: GenerationConfig {
            generator_id: "offline-transition-generator".into(),
            generator_version: "1.0.0".into(),
            generator_config_sha256: hash('d'),
            seed_hex_u64: "0000000000000000".into(),
        },
    }
}

fn candidate_ids(result: &transition_operator::PairGenerationResult) -> Vec<String> {
    result
        .candidate_set
        .candidates
        .iter()
        .map(|candidate| candidate.candidate_id.as_str().to_owned())
        .collect()
}

#[test]
fn shared_pair_gain_is_analytic_nonboosting_and_requires_whole_source_peaks() {
    assert_eq!(
        calculate_pair_output_gain(Some(-1_000), Some(-2_000), 3_000).unwrap(),
        -9_221
    );
    assert_eq!(
        calculate_pair_output_gain(Some(-20_000), Some(-21_000), 0).unwrap(),
        0
    );
    assert_eq!(
        calculate_pair_output_gain(None, Some(-2_000), 0)
            .unwrap_err()
            .code(),
        "MISSING_SOURCE_TRUE_PEAK"
    );
    assert_eq!(
        calculate_pair_output_gain(Some(20_000), Some(19_000), 3_000)
            .unwrap_err()
            .code(),
        "PAIR_HEADROOM_EXCEEDS_LIMIT"
    );
}

#[test]
fn repeated_and_permuted_generation_has_identical_sorted_semantic_set() {
    let first_request = request();
    let first = generate_candidates(first_request.clone()).unwrap();
    let mut permuted = first_request;
    permuted.geometries.reverse();
    permuted.features.beat_frames.reverse();
    let second = generate_candidates(permuted).unwrap();

    assert_eq!(candidate_ids(&first), candidate_ids(&second));
    assert_eq!(first.candidate_set.set_hash, second.candidate_set.set_hash);
    assert_eq!(first.diagnostics, second.diagnostics);
    assert!(
        candidate_ids(&first)
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    );
    assert!(first.candidate_set.candidates.len() <= 48);
    assert!(
        first
            .candidate_set
            .candidates
            .iter()
            .any(|candidate| candidate.template_id == TemplateId::SafeCrossfade)
    );
    let rich: std::collections::BTreeSet<_> = first
        .candidate_set
        .candidates
        .iter()
        .filter(|candidate| candidate.template_id != TemplateId::SafeCrossfade)
        .map(|candidate| candidate.template_id)
        .collect();
    assert!(
        rich.len() >= 5,
        "rich generation collapsed to too few musical strategies: {rich:?}"
    );
    let gains: std::collections::BTreeSet<_> = first
        .candidate_set
        .candidates
        .iter()
        .map(|candidate| candidate.plan.plan().output_safety.pair_output_gain_mdb)
        .collect();
    assert_eq!(gains, [-9_221].into_iter().collect());
}

#[test]
fn missing_required_feature_suppresses_family_despite_maximum_priority_value() {
    let mut request = request();
    request.features.outgoing_spectral_stability_ppm = None;
    request.features.spectral_overlap_ppm = Some(1_000_000);
    let result = generate_candidates(request).unwrap();
    assert!(
        !result
            .candidate_set
            .candidates
            .iter()
            .any(|candidate| candidate.template_id == TemplateId::SpectralHandoff)
    );
    assert!(result.diagnostics.iter().any(|record| record.template_id
        == Some(TemplateId::SpectralHandoff)
        && record.code == "TEMPLATE_INAPPLICABLE"));
}

#[test]
fn invalid_rich_recipe_cannot_break_independent_fallback() {
    let mut request = request();
    request.features.incoming_hard_cut_safe = Some(false);
    let result = generate_candidates(request).unwrap();
    assert!(
        result
            .candidate_set
            .candidates
            .iter()
            .any(|candidate| candidate.template_id == TemplateId::SafeCrossfade)
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|record| record.recipe_id.as_deref() == Some("hard_0ms")
                && record.code == "TEMPLATE_INAPPLICABLE")
    );
}

#[test]
fn no_rich_features_returns_exactly_fallback() {
    let mut request = request();
    request.features = TemplateInputs::default();
    let result = generate_candidates(request).unwrap();
    assert_eq!(result.candidate_set.candidates.len(), 1);
    assert_eq!(
        result.candidate_set.candidates[0].template_id,
        TemplateId::SafeCrossfade
    );
}

#[test]
fn hard_cap_decision_is_fallback_only_on_attempt_or_accept_sixty_five() {
    assert_eq!(cap_decision(64, 64), CapDecision::Continue);
    assert_eq!(cap_decision(65, 1), CapDecision::FallbackOnly);
    assert_eq!(cap_decision(1, 65), CapDecision::FallbackOnly);
}

#[test]
fn invalid_fallback_makes_the_pair_ineligible_before_rich_generation() {
    let mut request = request();
    request.fallback_geometry.requested_dry_frames = 220_499;
    request.fallback_geometry.geometry_id = geometry_id(&request.fallback_geometry.core()).unwrap();
    let error = generate_candidates(request).unwrap_err();
    assert_eq!(error.code(), "PAIR_INELIGIBLE");
}

#[test]
fn all_survivors_share_complete_safety_and_fallback_is_exact() {
    let result = generate_candidates(request()).unwrap();
    let fallback = result
        .candidate_set
        .candidates
        .iter()
        .find(|candidate| candidate.template_id == TemplateId::SafeCrossfade)
        .unwrap();
    let safety = &fallback.plan.plan().output_safety;
    assert!(
        result
            .candidate_set
            .candidates
            .iter()
            .all(|candidate| &candidate.plan.plan().output_safety == safety)
    );
    assert_eq!(fallback.recipe_id, "five_second_linear");
    assert_eq!(fallback.plan.plan().timeline.dry_start_frame, -220_500);
    assert_eq!(fallback.plan.plan().timeline.effect_end_frame, 0);
    assert_eq!(fallback.plan.plan().operations.len(), 2);
}

#[test]
fn quota_arithmetic_freezes_the_normal_bound_and_six_family_selection() {
    let quotas: Vec<_> = transition_operator::template_registry()
        .iter()
        .map(|family| family.quota)
        .collect();
    assert_eq!(quotas.iter().sum::<usize>(), 51);
    let mut rich_quotas = quotas[1..].to_vec();
    rich_quotas.sort_unstable_by(|left, right| right.cmp(left));
    assert_eq!(1 + rich_quotas.into_iter().take(6).sum::<usize>(), 43);

    let result = generate_candidates(request()).unwrap();
    assert_eq!(
        result
            .diagnostics
            .iter()
            .filter(|record| record.code == "FAMILY_QUOTA_PRUNED")
            .count(),
        2
    );
}

#[test]
fn candidate_set_hash_is_a_frozen_semantic_vector() {
    let result = generate_candidates(request()).unwrap();
    assert_eq!(result.candidate_set.candidates.len(), 37);
    assert_eq!(
        result.candidate_set.set_hash.as_str(),
        "183b2581ad882f3a5bca7229b0f54ecd915a865b844689bb6b354a3102c812cd"
    );
}

#[test]
fn semantic_deduplication_retains_first_id_and_reports_duplicate() {
    let result = generate_candidates(request()).unwrap();
    let fallback: GeneratedCandidate = result
        .candidate_set
        .candidates
        .iter()
        .find(|candidate| candidate.template_id == TemplateId::SafeCrossfade)
        .unwrap()
        .clone();
    let (retained, duplicates) = deduplicate_validated_plans(vec![fallback.clone(), fallback]);
    assert_eq!(retained.len(), 1);
    assert_eq!(duplicates.len(), 1);
    assert_eq!(duplicates[0].earlier_candidate_id, retained[0].candidate_id);
}

#[test]
fn geometry_id_change_does_not_depend_on_collection_position() {
    let mut duplicated_request = request();
    let original = duplicated_request.geometries[1].clone();
    duplicated_request.geometries.insert(0, original.clone());
    duplicated_request.geometries[0].geometry_id =
        geometry_id(&duplicated_request.geometries[0].core()).unwrap();
    let duplicated = generate_candidates(duplicated_request).unwrap();
    let normal = generate_candidates(request()).unwrap();
    assert_eq!(candidate_ids(&duplicated), candidate_ids(&normal));
}

#[test]
fn invalid_drafts_are_rejected_before_the_shared_headroom_margin_is_chosen() {
    let mut request = request();
    let mut overlong = geometry(DurationMode::Bars, 600_000, 1, 900_000, "overlong-grid");
    overlong.incoming_source_rate_ppm = 1_020_000;
    overlong.geometry_id = geometry_id(&overlong.core()).unwrap();
    request.geometries = vec![overlong];
    request.features = TemplateInputs {
        cue_confidence_ppm: Some(900_000),
        beat_confidence_ppm: Some(900_000),
        downbeat_confidence_ppm: Some(900_000),
        alignment_error_ppm_of_beat: Some(0),
        outgoing_vocal_activity_ppm: Some(100_000),
        outgoing_transient_activity_ppm: Some(700_000),
        vocal_collision_ppm: Some(900_000),
        vocal_collision_span_frames: Some(50_000),
        two_beats_frames: Some(40_000),
        beat_frames: vec![-600_000, -450_000, -300_000, -150_000, 0],
        meter_beats: Some(4),
        ..TemplateInputs::default()
    };
    let result = generate_candidates(request).unwrap();
    assert_eq!(result.candidate_set.candidates.len(), 1);
    assert_eq!(
        result.candidate_set.candidates[0]
            .plan
            .plan()
            .output_safety
            .pair_output_gain_mdb,
        -6_221
    );
    assert!(result.diagnostics.iter().any(|record| {
        record.template_id == Some(TemplateId::RhythmicHandoff)
            && record.code == "INVALID_RHYTHMIC_GATE"
    }));
}
