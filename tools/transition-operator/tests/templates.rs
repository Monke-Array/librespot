use transition_operator::{
    DurationMode, GeometryProposal, GeometryProposalCore, Interpolation, Operation, Target,
    TemplateId, TemplateInputs, bind_recipe, emit_safe_fallback, rich_template_families,
    template_registry,
};

fn hash(ch: char) -> String {
    std::iter::repeat_n(ch, 64).collect()
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
        outgoing_pcm_frame_count: 2_000_000,
        incoming_pcm_frame_count: 2_000_000,
        outgoing_cue_id: format!("out-{tag}"),
        outgoing_cue_source_frame: 1_000_000,
        incoming_cue_id: format!("in-{tag}"),
        incoming_cue_source_frame: 1_000_000,
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

fn applicable_inputs() -> TemplateInputs {
    TemplateInputs {
        cue_confidence_ppm: Some(900_000),
        beat_confidence_ppm: Some(900_000),
        downbeat_confidence_ppm: Some(900_000),
        alignment_error_ppm_of_beat: Some(0),
        outgoing_vocal_activity_ppm: Some(100_000),
        incoming_vocal_activity_ppm: Some(100_000),
        outgoing_vocal_sustained: Some(false),
        vocal_collision_ppm: Some(500_000),
        vocal_collision_span_frames: Some(20_000),
        transient_collision_ppm: Some(100_000),
        transient_collision_span_frames: Some(20_000),
        two_beats_frames: Some(44_100),
        outgoing_transient_activity_ppm: Some(700_000),
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
    }
}

#[test]
fn registry_is_closed_with_fixed_tie_order_quotas_and_recipe_counts() {
    let registry = template_registry();
    assert_eq!(registry.len(), 9);
    assert_eq!(
        registry
            .iter()
            .map(|family| family.id.as_str())
            .collect::<Vec<_>>(),
        [
            "safe_crossfade",
            "shaped_handoff",
            "beat_cut",
            "bass_handoff",
            "spectral_handoff",
            "ducked_overlap",
            "echo_tail_handoff",
            "energy_ramp",
            "rhythmic_handoff",
        ]
    );
    assert_eq!(
        registry
            .iter()
            .map(|family| family.quota)
            .collect::<Vec<_>>(),
        [1, 8, 4, 8, 8, 8, 4, 6, 4]
    );
    assert!(
        registry
            .iter()
            .all(|family| family.quota == family.recipes.len())
    );
}

#[test]
fn applicability_uses_exact_thresholds_and_missing_required_values_are_false() {
    let inputs = applicable_inputs();
    let applicable = rich_template_families(&inputs);
    assert!(applicable.iter().all(|family| family.applicable));

    let mut missing_bass = inputs.clone();
    missing_bass.bass_collision_ppm = None;
    let bass = rich_template_families(&missing_bass)
        .into_iter()
        .find(|family| family.id == TemplateId::BassHandoff)
        .unwrap();
    assert!(!bass.applicable);

    let mut boundary = inputs;
    boundary.cue_confidence_ppm = Some(700_000);
    boundary.beat_confidence_ppm = Some(750_000);
    boundary.downbeat_confidence_ppm = Some(700_000);
    boundary.alignment_error_ppm_of_beat = Some(31_250);
    boundary.bass_collision_ppm = Some(350_000);
    boundary.spectral_overlap_ppm = Some(450_000);
    assert!(
        rich_template_families(&boundary)
            .iter()
            .all(|family| family.applicable)
    );
}

#[test]
fn optional_priority_values_cannot_make_a_required_predicate_true() {
    let mut inputs = applicable_inputs();
    inputs.outgoing_spectral_stability_ppm = None;
    inputs.spectral_overlap_ppm = Some(1_000_000);
    let spectral = rich_template_families(&inputs)
        .into_iter()
        .find(|family| family.id == TemplateId::SpectralHandoff)
        .unwrap();
    assert!(!spectral.applicable);
    assert_eq!(spectral.need_score_ppm, 1_000_000);
}

#[test]
fn recipe_index_binds_one_sorted_compatible_geometry_modulo_count() {
    let family = template_registry()
        .iter()
        .find(|family| family.id == TemplateId::ShapedHandoff)
        .unwrap();
    let recipe = &family.recipes[1]; // eq_5s
    let low = geometry(DurationMode::Seconds, 220_500, 0, 700_000, "low");
    let high = geometry(DurationMode::Seconds, 220_500, 0, 900_000, "high");
    let wrong = geometry(DurationMode::Seconds, 132_300, 0, 1_000_000, "wrong");

    let first_order = [low.clone(), wrong.clone(), high.clone()];
    let second_order = [high, low, wrong];
    let first = bind_recipe(recipe, &first_order).unwrap();
    let second = bind_recipe(recipe, &second_order).unwrap();
    assert_eq!(first.geometry_id, second.geometry_id);
    assert_eq!(first.geometry_quality_ppm, 700_000); // recipe index 1 modulo two matches
}

#[test]
fn safe_fallback_is_exactly_two_linear_primary_gains_over_five_seconds() {
    let fallback = geometry(DurationMode::Seconds, 220_500, 0, 900_000, "fallback");
    let draft = emit_safe_fallback(&fallback).unwrap();
    assert_eq!(draft.template_id, TemplateId::SafeCrossfade);
    assert_eq!(draft.recipe_id.as_str(), "five_second_linear");
    assert_eq!(draft.dry_start_frame, -220_500);
    assert_eq!(draft.effect_end_frame, 0);
    assert_eq!(draft.operations.len(), 2);
    for (index, operation) in draft.operations.iter().enumerate() {
        let Operation::GainEnvelope(gain) = operation else {
            panic!("safe fallback emitted a non-gain operation")
        };
        assert_eq!(gain.interpolations, [Interpolation::Linear]);
        assert_eq!(gain.points[0].frame, -220_500);
        assert_eq!(gain.points[1].frame, 0);
        assert_eq!(
            gain.target,
            if index == 0 {
                Target::Outgoing
            } else {
                Target::Incoming
            }
        );
    }
}
