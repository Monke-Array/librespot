use transition_operator::{
    Band, DurationMode, GeometryProposal, GeometryProposalCore, Interpolation, Operation,
    RecipeSpec, Target, TemplateFamily, TemplateId, TemplateInputs, bind_recipe,
    emit_dynamics_template, emit_echo_tail_template, emit_gain_template, emit_safe_fallback,
    emit_spectral_template, rich_template_families, template_registry,
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
        vocal_collision_start_frame: Some(-50_000),
        vocal_collision_end_frame: Some(-20_000),
        outgoing_vocal_collision_strength_ppm: Some(800_000),
        incoming_vocal_collision_strength_ppm: Some(600_000),
        transient_collision_ppm: Some(100_000),
        transient_collision_span_frames: Some(20_000),
        transient_collision_start_frame: Some(-48_000),
        transient_collision_end_frame: Some(-28_000),
        outgoing_transient_collision_strength_ppm: Some(700_000),
        incoming_transient_collision_strength_ppm: Some(500_000),
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
        incoming_transient_activity_ppm: Some(600_000),
        beat_frames: vec![-88_200, -66_150, -44_100, -22_050, 0],
        meter_beats: Some(4),
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
fn duck_requires_both_collision_modalities_to_be_known() {
    let mut inputs = applicable_inputs();
    inputs.vocal_collision_ppm = None;
    inputs.vocal_collision_span_frames = None;
    inputs.vocal_collision_start_frame = None;
    inputs.vocal_collision_end_frame = None;
    inputs.outgoing_vocal_collision_strength_ppm = None;
    inputs.incoming_vocal_collision_strength_ppm = None;
    inputs.transient_collision_ppm = Some(500_000);
    assert!(
        !family(TemplateId::DuckedOverlap)
            .applicability(&inputs)
            .applicable
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

fn family(id: TemplateId) -> &'static TemplateFamily {
    template_registry()
        .iter()
        .find(|family| family.id == id)
        .unwrap()
}

fn recipe(id: TemplateId, recipe_id: &str) -> &'static RecipeSpec {
    family(id)
        .recipes
        .iter()
        .find(|recipe| recipe.id == recipe_id)
        .unwrap()
}

#[test]
fn gain_templates_emit_equal_power_asymmetry_and_canonical_hard_cut() {
    let inputs = applicable_inputs();
    let three = geometry(DurationMode::Seconds, 132_300, 0, 900_000, "gain-three");
    let equal = emit_gain_template(
        TemplateId::ShapedHandoff,
        recipe(TemplateId::ShapedHandoff, "eq_3s"),
        &three,
        &inputs,
    )
    .unwrap();
    let gains: Vec<_> = equal
        .operations
        .iter()
        .filter_map(|operation| match operation {
            Operation::GainEnvelope(value) => Some(value),
            _ => None,
        })
        .collect();
    assert_eq!(gains[0].interpolations, [Interpolation::QuarterCosine]);
    assert_eq!(gains[1].interpolations, [Interpolation::QuarterSine]);

    let asymmetric = emit_gain_template(
        TemplateId::ShapedHandoff,
        recipe(TemplateId::ShapedHandoff, "asym_early_3s"),
        &three,
        &inputs,
    )
    .unwrap();
    let Operation::GainEnvelope(outgoing) = &asymmetric.operations[0] else {
        panic!("expected outgoing gain")
    };
    assert_eq!(
        outgoing
            .points
            .iter()
            .map(|point| point.frame)
            .collect::<Vec<_>>(),
        [-132_300, -33_075, 0]
    );
    assert_eq!(outgoing.points[1].value_ppm, 0);

    let cut = geometry(DurationMode::Cut, 1, 0, 900_000, "hard-cut");
    let hard = emit_gain_template(
        TemplateId::BeatCut,
        recipe(TemplateId::BeatCut, "hard_0ms"),
        &cut,
        &inputs,
    )
    .unwrap();
    assert_eq!(hard.dry_start_frame, -1);
    assert!(hard.operations.iter().all(|operation| match operation {
        Operation::GainEnvelope(value) => value.interpolations == [Interpolation::Hold],
        _ => false,
    }));

    let mut unsafe_cut = inputs;
    unsafe_cut.incoming_hard_cut_safe = Some(false);
    assert_eq!(
        emit_gain_template(
            TemplateId::BeatCut,
            recipe(TemplateId::BeatCut, "hard_0ms"),
            &cut,
            &unsafe_cut,
        )
        .unwrap_err()
        .code(),
        "TEMPLATE_INAPPLICABLE"
    );
}

#[test]
fn energy_ramp_is_nonboosting_and_filter_direction_follows_signed_delta() {
    let mut inputs = applicable_inputs();
    inputs.energy_delta_mdb = Some(4_000);
    let three = geometry(DurationMode::Seconds, 132_300, 0, 900_000, "energy-three");
    let draft = emit_gain_template(
        TemplateId::EnergyRamp,
        recipe(TemplateId::EnergyRamp, "gain_3s"),
        &three,
        &inputs,
    )
    .unwrap();
    let incoming = draft
        .operations
        .iter()
        .find_map(|operation| match operation {
            Operation::GainEnvelope(value) if value.target == Target::Incoming => Some(value),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        incoming
            .points
            .iter()
            .map(|point| point.frame)
            .collect::<Vec<_>>(),
        [-132_300, -99_225, -33_075, 0]
    );
    assert_eq!(incoming.points[1].value_ppm, 630_957);
    assert!(
        incoming
            .points
            .iter()
            .all(|point| point.value_ppm <= 1_000_000)
    );

    inputs.energy_delta_mdb = Some(-4_000);
    let bars = geometry(DurationMode::Bars, 176_400, 2, 900_000, "energy-bars");
    let filtered = emit_gain_template(
        TemplateId::EnergyRamp,
        recipe(TemplateId::EnergyRamp, "filter_2bar"),
        &bars,
        &inputs,
    )
    .unwrap();
    assert!(filtered.operations.iter().any(|operation| matches!(
        operation,
        Operation::FilterEnvelope(value)
            if value.target == Target::Incoming
                && value.cutoff_points[0].cutoff_millihz == 1_600_000
                && value.cutoff_points[1].cutoff_millihz == 20_000
    )));
}

#[test]
fn spectral_templates_emit_complementary_bass_and_staggered_three_band_ownership() {
    let inputs = applicable_inputs();
    let bars = geometry(DurationMode::Bars, 176_400, 2, 900_000, "spectral-bars");
    let bass = emit_spectral_template(
        TemplateId::BassHandoff,
        recipe(TemplateId::BassHandoff, "b180_2_early"),
        &bars,
        &inputs,
    )
    .unwrap();
    let crossovers: Vec<_> = bass
        .operations
        .iter()
        .filter_map(|operation| match operation {
            Operation::CrossoverBandGain(value) => Some(value),
            _ => None,
        })
        .collect();
    assert_eq!(crossovers.len(), 2);
    assert_eq!(crossovers[0].crossover_millihz, [180_000]);
    assert_eq!(
        crossovers[0].band_gain_envelopes[0]
            .points
            .iter()
            .map(|point| point.frame)
            .collect::<Vec<_>>(),
        [-176_400, -132_300, -88_200, 0]
    );
    for index in 0..crossovers[0].band_gain_envelopes[0].points.len() {
        assert!(
            crossovers[0].band_gain_envelopes[0].points[index].value_ppm
                + crossovers[1].band_gain_envelopes[0].points[index].value_ppm
                <= 1_000_000
        );
    }

    let bands = emit_spectral_template(
        TemplateId::SpectralHandoff,
        recipe(TemplateId::SpectralHandoff, "bands_2bar_high_then_low"),
        &bars,
        &inputs,
    )
    .unwrap();
    let outgoing = bands
        .operations
        .iter()
        .find_map(|operation| match operation {
            Operation::CrossoverBandGain(value) if value.target == Target::Outgoing => Some(value),
            _ => None,
        })
        .unwrap();
    assert_eq!(outgoing.bands, [Band::Low, Band::Mid, Band::High]);
    assert_eq!(
        outgoing.band_gain_envelopes[2]
            .points
            .iter()
            .map(|point| point.frame)
            .collect::<Vec<_>>(),
        [-176_400, -117_600, 0]
    );
}

#[test]
fn duck_is_resolved_to_the_louder_source_with_exact_attack_hold_and_release() {
    let mut inputs = applicable_inputs();
    inputs.outgoing_vocal_activity_ppm = Some(100_000);
    inputs.incoming_vocal_activity_ppm = Some(900_000);
    inputs.outgoing_vocal_collision_strength_ppm = Some(800_000);
    inputs.incoming_vocal_collision_strength_ppm = Some(600_000);
    let three = geometry(DurationMode::Seconds, 132_300, 0, 900_000, "duck-three");
    let draft = emit_dynamics_template(
        TemplateId::DuckedOverlap,
        recipe(TemplateId::DuckedOverlap, "d6_fast_3s"),
        &three,
        &inputs,
    )
    .unwrap();
    let duck = draft
        .operations
        .iter()
        .find_map(|operation| match operation {
            Operation::DuckEnvelope(value) => Some(value),
            _ => None,
        })
        .unwrap();
    assert_eq!(duck.target, Target::Outgoing);
    assert_eq!(
        duck.points
            .iter()
            .map(|point| point.frame)
            .collect::<Vec<_>>(),
        [-50_882, -50_000, -20_000, -16_472]
    );
    assert_eq!(duck.points[1].value_ppm, 501_187);
}

#[test]
fn rhythmic_gate_resolves_click_safe_cells_and_rejects_cells_shorter_than_two_ramps() {
    let inputs = applicable_inputs();
    let bars = geometry(DurationMode::Bars, 88_200, 1, 900_000, "rhythm-bars");
    let draft = emit_dynamics_template(
        TemplateId::RhythmicHandoff,
        recipe(TemplateId::RhythmicHandoff, "eighth_1bar_decay"),
        &bars,
        &inputs,
    )
    .unwrap();
    let gate = draft
        .operations
        .iter()
        .find_map(|operation| match operation {
            Operation::RhythmicGate(value) => Some(value),
            _ => None,
        })
        .unwrap();
    assert_eq!(gate.points.last().unwrap().frame, 0);
    assert_eq!(gate.points.last().unwrap().value_ppm, 0);
    assert!(gate.points.windows(2).all(|pair| {
        pair[0].value_ppm == pair[1].value_ppm || pair[1].frame - pair[0].frame >= 221
    }));

    let mut too_short = inputs;
    too_short.beat_frames = vec![-800, -600, -400, -200, 0];
    let short_bars = geometry(DurationMode::Bars, 800, 1, 900_000, "short-rhythm-bars");
    assert_eq!(
        emit_dynamics_template(
            TemplateId::RhythmicHandoff,
            recipe(TemplateId::RhythmicHandoff, "eighth_1bar_even"),
            &short_bars,
            &too_short,
        )
        .unwrap_err()
        .code(),
        "RHYTHMIC_CELL_TOO_SHORT"
    );
}

#[test]
fn rhythmic_recipe_selects_its_window_from_a_longer_resolved_beat_grid() {
    let mut inputs = applicable_inputs();
    inputs.beat_frames = vec![
        -176_400, -154_350, -132_300, -110_250, -88_200, -66_150, -44_100, -22_050, 0,
    ];
    let bars = geometry(DurationMode::Bars, 88_200, 1, 900_000, "rhythm-window");
    let draft = emit_dynamics_template(
        TemplateId::RhythmicHandoff,
        recipe(TemplateId::RhythmicHandoff, "quarter_1bar_even"),
        &bars,
        &inputs,
    )
    .unwrap();
    let gate = draft
        .operations
        .iter()
        .find_map(|operation| match operation {
            Operation::RhythmicGate(value) => Some(value),
            _ => None,
        })
        .unwrap();
    assert_eq!(gate.points.first().unwrap().frame, -88_200);
    assert_eq!(gate.points.last().unwrap().frame, 0);
}

#[test]
fn echo_tail_uses_beat_fraction_round_away_capture_and_exact_effect_end() {
    let inputs = applicable_inputs();
    let cut = geometry(DurationMode::Cut, 1, 0, 900_000, "echo-cut");
    let draft = emit_echo_tail_template(
        recipe(TemplateId::EchoTailHandoff, "quarter_3tap"),
        &cut,
        &inputs,
    )
    .unwrap();
    let tail = draft
        .operations
        .iter()
        .find_map(|operation| match operation {
            Operation::FeedforwardDelayTail(value) => Some(value),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        (tail.capture_start_frame, tail.capture_end_frame),
        (-11_025, 0)
    );
    assert_eq!(
        tail.taps
            .iter()
            .map(|tap| tap.delay_frames)
            .collect::<Vec<_>>(),
        [5_513, 11_025, 16_538]
    );
    assert_eq!(
        tail.taps.iter().map(|tap| tap.gain_ppm).collect::<Vec<_>>(),
        [400_000, 240_000, 140_000]
    );
    assert_eq!(draft.dry_start_frame, -11_025);
    assert_eq!(draft.effect_end_frame, 16_538);
}

#[test]
fn incoming_time_map_is_emitted_only_for_templates_and_recipe_modes_that_allow_it() {
    let inputs = applicable_inputs();
    let mut seconds = geometry(
        DurationMode::Seconds,
        132_300,
        0,
        900_000,
        "stretch-seconds",
    );
    seconds.incoming_source_rate_ppm = 1_020_000;
    seconds.geometry_id = transition_operator::geometry_id(&seconds.core()).unwrap();
    let spectral_seconds = emit_spectral_template(
        TemplateId::SpectralHandoff,
        recipe(TemplateId::SpectralHandoff, "lp_out_3s_500"),
        &seconds,
        &inputs,
    )
    .unwrap();
    assert!(
        !spectral_seconds
            .operations
            .iter()
            .any(|operation| matches!(operation, Operation::TimeMap(_)))
    );
    let duck_seconds = emit_dynamics_template(
        TemplateId::DuckedOverlap,
        recipe(TemplateId::DuckedOverlap, "d6_fast_3s"),
        &seconds,
        &inputs,
    )
    .unwrap();
    assert!(
        !duck_seconds
            .operations
            .iter()
            .any(|operation| matches!(operation, Operation::TimeMap(_)))
    );

    let mut bars = geometry(DurationMode::Bars, 176_400, 2, 900_000, "stretch-bars");
    bars.incoming_source_rate_ppm = 1_020_000;
    bars.geometry_id = transition_operator::geometry_id(&bars.core()).unwrap();
    let spectral_bars = emit_spectral_template(
        TemplateId::SpectralHandoff,
        recipe(TemplateId::SpectralHandoff, "lp_out_2bar_500"),
        &bars,
        &inputs,
    )
    .unwrap();
    assert!(
        spectral_bars
            .operations
            .iter()
            .any(|operation| matches!(operation, Operation::TimeMap(_)))
    );

    let energy = emit_gain_template(
        TemplateId::EnergyRamp,
        recipe(TemplateId::EnergyRamp, "gain_2bar"),
        &bars,
        &inputs,
    )
    .unwrap();
    assert!(
        !energy
            .operations
            .iter()
            .any(|operation| matches!(operation, Operation::TimeMap(_)))
    );
}

#[test]
fn four_bar_equal_power_shape_respects_the_ten_second_recipe_limit() {
    let inputs = applicable_inputs();
    let too_long = geometry(DurationMode::Bars, 441_001, 4, 900_000, "long-shaped-bars");
    assert_eq!(
        emit_gain_template(
            TemplateId::ShapedHandoff,
            recipe(TemplateId::ShapedHandoff, "eq_4bar"),
            &too_long,
            &inputs,
        )
        .unwrap_err()
        .code(),
        "GEOMETRY_TOO_LONG"
    );
}
