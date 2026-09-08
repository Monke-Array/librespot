use transition_operator::{
    Band, BandGainEnvelope, CrossoverBandGain, CrossoverProfile, CutoffPoint, DelayTap,
    DuckEnvelope, DuckReason, EnvelopePoint, ExpectedSource, FeedforwardDelayTail, FilterEnvelope,
    FilterKind, Interpolation, Operation, OperatorPlan, OperatorPlanBody, PlanValidator,
    RhythmicGate, Target, TemplateFeatureView, TimeMap, TimeMapProfile, ValidationContext,
    require_canonical_json,
};

const SAFE_BYTES_RAW: &[u8] = include_bytes!("fixtures/plans/safe-crossfade.json");
const ALL_OPERATIONS_BYTES_RAW: &[u8] = include_bytes!("fixtures/plans/all-operations.json");

fn fixture_bytes(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(b"\n").unwrap_or(bytes)
}

#[test]
fn safe_fixture_has_exact_v1_shape() {
    let plan: OperatorPlan = require_canonical_json(fixture_bytes(SAFE_BYTES_RAW)).unwrap();
    assert_eq!(plan.schema_version, "transition-operator-plan/1");
    assert_eq!(plan.template.id, "safe_crossfade");
    assert_eq!(plan.operations.len(), 2);
    assert!(
        plan.operations
            .iter()
            .all(|operation| matches!(operation, Operation::GainEnvelope(_)))
    );
}

#[test]
fn all_operations_fixture_exercises_closed_union() {
    let plan: OperatorPlan =
        require_canonical_json(fixture_bytes(ALL_OPERATIONS_BYTES_RAW)).unwrap();
    let kinds: Vec<_> = plan.operations.iter().map(Operation::kind).collect();
    assert_eq!(
        kinds,
        [
            "time_map",
            "filter_envelope",
            "crossover_band_gain",
            "duck_envelope",
            "rhythmic_gate",
            "feedforward_delay_tail",
            "gain_envelope",
            "gain_envelope",
        ]
    );
}

#[test]
fn schema_is_closed_to_unknown_operations_and_backend_fields() {
    for (needle, replacement) in [
        ("\"kind\":\"gain_envelope\"", "\"kind\":\"reverb\""),
        (
            "\"op_id\":\"outgoing.primary_gain\"",
            "\"backend_command\":\"volume=2\",\"op_id\":\"outgoing.primary_gain\"",
        ),
        (
            "\"recipe_id\":\"five_second_linear\"",
            "\"preset_id\":\"spotify:opaque\",\"recipe_id\":\"five_second_linear\"",
        ),
    ] {
        let input = String::from_utf8(fixture_bytes(SAFE_BYTES_RAW).to_vec())
            .unwrap()
            .replacen(needle, replacement, 1);
        assert_eq!(
            require_canonical_json::<OperatorPlan>(input.as_bytes())
                .unwrap_err()
                .code(),
            "INVALID_JSON"
        );
    }
}

#[test]
fn plan_numbers_and_optional_values_have_no_float_or_null_escape_hatch() {
    for (needle, replacement) in [
        ("\"channels\":2", "\"channels\":2.0"),
        ("\"geometry_id\":\"geometry-safe\"", "\"geometry_id\":null"),
    ] {
        let input = String::from_utf8(fixture_bytes(SAFE_BYTES_RAW).to_vec())
            .unwrap()
            .replacen(needle, replacement, 1);
        assert_eq!(
            require_canonical_json::<OperatorPlan>(input.as_bytes())
                .unwrap_err()
                .code(),
            "INVALID_JSON"
        );
    }
}

struct TestFeatures {
    sha256: String,
    covers: bool,
    applicable: bool,
    recipe_matches: bool,
}

impl TemplateFeatureView for TestFeatures {
    fn snapshot_sha256(&self) -> &str {
        &self.sha256
    }
    fn covers_plan_window(&self, _body: &OperatorPlanBody) -> bool {
        self.covers
    }
    fn template_is_applicable(&self, _body: &OperatorPlanBody) -> bool {
        self.applicable
    }
    fn recipe_matches(&self, _body: &OperatorPlanBody) -> bool {
        self.recipe_matches
    }
}

fn safe_plan() -> OperatorPlan {
    require_canonical_json(fixture_bytes(SAFE_BYTES_RAW)).unwrap()
}

fn features(plan: &OperatorPlan) -> TestFeatures {
    TestFeatures {
        sha256: plan.feature_snapshot.sha256.clone(),
        covers: true,
        applicable: true,
        recipe_matches: true,
    }
}

fn context<'a>(plan: &'a OperatorPlan, features: &'a TestFeatures) -> ValidationContext<'a> {
    ValidationContext {
        outgoing: ExpectedSource::from_ref(&plan.sources.outgoing),
        incoming: ExpectedSource::from_ref(&plan.sources.incoming),
        features,
    }
}

fn validation_code(plan: &OperatorPlan) -> &'static str {
    let feature_view = features(plan);
    PlanValidator::validate_structure(plan.clone(), &context(plan, &feature_view))
        .unwrap_err()
        .code()
}

#[test]
fn valid_safe_body_obtains_only_a_validated_body_token() {
    let plan = safe_plan();
    let feature_view = features(&plan);
    let validated =
        PlanValidator::validate_body(plan.body(), &context(&plan, &feature_view)).unwrap();
    assert_eq!(validated.body().template.id, "safe_crossfade");
    assert_eq!(validated.report().stages_completed(), 8);
}

#[test]
fn schema_ascii_hash_seed_and_source_identity_fail_closed() {
    let mutations: &[(fn(&mut OperatorPlan), &str)] = &[
        (
            |p| p.schema_version = "transition-operator-plan/2".into(),
            "UNSUPPORTED_SCHEMA_VERSION",
        ),
        (
            |p| p.provenance.geometry_id = "géometry".into(),
            "NON_ASCII_PLAN_STRING",
        ),
        (
            |p| p.sources.outgoing.pcm_sha256 = "ABC".into(),
            "INVALID_SHA256",
        ),
        (
            |p| p.provenance.seed_hex_u64 = "0000000000000001".into(),
            "INVALID_DSP_SEED",
        ),
        (
            |p| p.sources.incoming.pcm_profile = "container_mp3".into(),
            "INVALID_PCM_PROFILE",
        ),
    ];
    for (mutate, expected) in mutations {
        let mut plan = safe_plan();
        mutate(&mut plan);
        assert_eq!(validation_code(&plan), *expected);
    }
}

#[test]
fn timeline_and_context_bounds_are_checked_without_repair() {
    let mut plan = safe_plan();
    plan.timeline.dry_start_frame = 0;
    assert_eq!(validation_code(&plan), "INVALID_TIMELINE");
    let mut plan = safe_plan();
    plan.timeline.effect_end_frame = 264_601;
    assert_eq!(validation_code(&plan), "INVALID_TIMELINE");

    let plan = safe_plan();
    let feature_view = features(&plan);
    let mut wrong_context = context(&plan, &feature_view);
    wrong_context.outgoing.pcm_frame_count -= 1;
    assert_eq!(
        PlanValidator::validate_body(plan.body(), &wrong_context)
            .unwrap_err()
            .code(),
        "SOURCE_CONTEXT_MISMATCH"
    );

    let mut plan = safe_plan();
    plan.sources.incoming.cue_source_frame = 100;
    assert_eq!(validation_code(&plan), "SOURCE_WINDOW_OUT_OF_BOUNDS");
}

#[test]
fn operation_order_and_primary_gain_contract_are_explicit() {
    let mut plan = safe_plan();
    plan.operations.swap(0, 1);
    assert_eq!(validation_code(&plan), "NON_CANONICAL_OPERATION_ORDER");
    let mut plan = safe_plan();
    plan.operations.pop();
    assert_eq!(validation_code(&plan), "MISSING_PRIMARY_GAIN");
    let mut plan = safe_plan();
    let Operation::GainEnvelope(outgoing) = &mut plan.operations[0] else {
        unreachable!()
    };
    outgoing.points[1].value_ppm = 1;
    assert_eq!(validation_code(&plan), "INVALID_PRIMARY_GAIN_ENDPOINT");
    let mut plan = safe_plan();
    let duplicate = plan.operations[0].clone();
    plan.operations.push(duplicate);
    assert_eq!(validation_code(&plan), "DUPLICATE_OPERATION_ID");
}

#[test]
fn envelope_and_operation_resource_bounds_reject_one_outside() {
    let mut plan = safe_plan();
    let Operation::GainEnvelope(outgoing) = &mut plan.operations[0] else {
        unreachable!()
    };
    outgoing.points[0].value_ppm = 1_000_001;
    assert_eq!(validation_code(&plan), "GAIN_OUT_OF_RANGE");
    let mut plan = safe_plan();
    let Operation::GainEnvelope(outgoing) = &mut plan.operations[0] else {
        unreachable!()
    };
    outgoing.points.push(outgoing.points[1].clone());
    outgoing
        .interpolations
        .push(transition_operator::Interpolation::Linear);
    assert_eq!(validation_code(&plan), "NON_INCREASING_ENVELOPE_POINTS");
    let mut plan = safe_plan();
    plan.operations
        .extend(std::iter::repeat_n(plan.operations[0].clone(), 11));
    assert_eq!(validation_code(&plan), "TOO_MANY_OPERATIONS");
}

#[test]
fn output_safety_is_exact_and_nonboosting() {
    let mutations: &[(fn(&mut OperatorPlan), &str)] = &[
        (
            |p| p.output_safety.pair_output_gain_mdb = 1,
            "INVALID_PAIR_OUTPUT_GAIN",
        ),
        (
            |p| p.output_safety.pair_output_gain_mdb = -24_001,
            "INVALID_PAIR_OUTPUT_GAIN",
        ),
        (
            |p| p.output_safety.lookahead_frames = 220,
            "INVALID_OUTPUT_SAFETY_PROFILE",
        ),
        (
            |p| p.output_safety.true_peak_target_mdbtp = -999,
            "INVALID_OUTPUT_SAFETY_PROFILE",
        ),
        (
            |p| p.output_safety.maximum_active_fraction_ppm = 50_001,
            "INVALID_OUTPUT_SAFETY_PROFILE",
        ),
    ];
    for (mutate, expected) in mutations {
        let mut plan = safe_plan();
        mutate(&mut plan);
        assert_eq!(validation_code(&plan), *expected);
    }
}

#[test]
fn feature_context_applicability_and_recipe_equality_are_separate() {
    let plan = safe_plan();
    for (covers, applicable, recipe_matches, expected) in [
        (false, true, true, "FEATURE_WINDOW_NOT_COVERED"),
        (true, false, true, "TEMPLATE_INAPPLICABLE"),
        (true, true, false, "TEMPLATE_RECIPE_MISMATCH"),
    ] {
        let feature_view = TestFeatures {
            sha256: plan.feature_snapshot.sha256.clone(),
            covers,
            applicable,
            recipe_matches,
        };
        assert_eq!(
            PlanValidator::validate_body(plan.body(), &context(&plan, &feature_view))
                .unwrap_err()
                .code(),
            expected
        );
    }
    let feature_view = TestFeatures {
        sha256: "9999999999999999999999999999999999999999999999999999999999999999".into(),
        covers: true,
        applicable: true,
        recipe_matches: true,
    };
    assert_eq!(
        PlanValidator::validate_body(plan.body(), &context(&plan, &feature_view))
            .unwrap_err()
            .code(),
        "FEATURE_SNAPSHOT_MISMATCH"
    );
}

fn assert_valid(plan: &OperatorPlan) {
    let feature_view = features(plan);
    PlanValidator::validate_structure(plan.clone(), &context(plan, &feature_view)).unwrap();
}

fn primary_gains(plan: &OperatorPlan) -> Vec<Operation> {
    plan.operations.clone()
}

fn envelope(points: &[(i64, i64)]) -> (Vec<EnvelopePoint>, Vec<Interpolation>) {
    (
        points
            .iter()
            .map(|&(frame, value_ppm)| EnvelopePoint { frame, value_ppm })
            .collect(),
        std::iter::repeat_n(Interpolation::Linear, points.len() - 1).collect(),
    )
}

#[test]
fn time_map_rate_boundaries_are_inclusive() {
    for rate in [920_000, 1_080_000] {
        let mut plan = safe_plan();
        plan.template.id = "shaped_handoff".into();
        plan.template.recipe_id = "test".into();
        plan.operations.insert(
            0,
            Operation::TimeMap(TimeMap {
                op_id: "incoming.time_map".into(),
                target: Target::Incoming,
                source_rate_ppm: rate,
                profile: TimeMapProfile::PitchPreservingBalancedTransientsV1,
            }),
        );
        assert_valid(&plan);
    }
    let mut plan = safe_plan();
    plan.template.id = "shaped_handoff".into();
    plan.template.recipe_id = "test".into();
    plan.operations.insert(
        0,
        Operation::TimeMap(TimeMap {
            op_id: "incoming.time_map".into(),
            target: Target::Incoming,
            source_rate_ppm: 1_080_001,
            profile: TimeMapProfile::PitchPreservingBalancedTransientsV1,
        }),
    );
    assert_eq!(validation_code(&plan), "INVALID_TIME_MAP");
}

#[test]
fn filter_frequency_q_and_control_bounds_are_enforced() {
    let mut plan = safe_plan();
    plan.template.id = "spectral_handoff".into();
    plan.template.recipe_id = "test".into();
    let (wet_points, wet_interpolations) = envelope(&[(-220_500, 0), (0, 1_000_000)]);
    let filter = FilterEnvelope {
        op_id: "outgoing.filter".into(),
        target: Target::Outgoing,
        filter_kind: FilterKind::LowpassBiquadV1,
        cutoff_points: vec![
            CutoffPoint {
                frame: -220_500,
                cutoff_millihz: 20_000,
            },
            CutoffPoint {
                frame: 0,
                cutoff_millihz: 18_000_000,
            },
        ],
        cutoff_interpolations: vec![Interpolation::Linear],
        q_milli: 500,
        wet_points,
        wet_interpolations,
        control_interval_frames: 64,
    };
    plan.operations.insert(0, Operation::FilterEnvelope(filter));
    assert_valid(&plan);
    let Operation::FilterEnvelope(filter) = &mut plan.operations[0] else {
        unreachable!()
    };
    filter.q_milli = 1_001;
    assert_eq!(validation_code(&plan), "FILTER_PARAMETER_OUT_OF_RANGE");
}

fn crossover(target: Target, op_id: &str) -> CrossoverBandGain {
    let (low_points, low_interpolations) = envelope(&[
        (
            -220_500,
            if target == Target::Outgoing {
                1_000_000
            } else {
                0
            },
        ),
        (
            0,
            if target == Target::Outgoing {
                0
            } else {
                1_000_000
            },
        ),
    ]);
    let (high_points, high_interpolations) = envelope(&[(-220_500, 1_000_000), (0, 1_000_000)]);
    let (wet_points, wet_interpolations) = envelope(&[
        (-220_500, 0),
        (-219_618, 1_000_000),
        (-882, 1_000_000),
        (0, 0),
    ]);
    CrossoverBandGain {
        op_id: op_id.into(),
        target,
        profile: CrossoverProfile::LinkwitzRiley4V1,
        crossover_millihz: vec![80_000],
        bands: vec![Band::Low, Band::High],
        band_gain_envelopes: vec![
            BandGainEnvelope {
                band: Band::Low,
                points: low_points,
                interpolations: low_interpolations,
            },
            BandGainEnvelope {
                band: Band::High,
                points: high_points,
                interpolations: high_interpolations,
            },
        ],
        wet_points,
        wet_interpolations,
        control_interval_frames: 64,
    }
}

#[test]
fn crossover_topology_and_complementary_bass_ownership_are_bounded() {
    let mut plan = safe_plan();
    plan.template.id = "bass_handoff".into();
    plan.template.recipe_id = "test".into();
    plan.operations = vec![
        Operation::CrossoverBandGain(crossover(Target::Outgoing, "outgoing.crossover")),
        Operation::CrossoverBandGain(crossover(Target::Incoming, "incoming.crossover")),
    ];
    plan.operations.extend(primary_gains(&safe_plan()));
    assert_valid(&plan);
    let Operation::CrossoverBandGain(incoming) = &mut plan.operations[1] else {
        unreachable!()
    };
    incoming.crossover_millihz[0] = 5_000_001;
    assert_eq!(validation_code(&plan), "INVALID_CROSSOVER");

    let mut plan = safe_plan();
    plan.template.id = "bass_handoff".into();
    plan.template.recipe_id = "test".into();
    let mut incoming = crossover(Target::Incoming, "incoming.crossover");
    incoming.band_gain_envelopes[0].points[0].value_ppm = 1;
    plan.operations = vec![
        Operation::CrossoverBandGain(crossover(Target::Outgoing, "outgoing.crossover")),
        Operation::CrossoverBandGain(incoming),
    ];
    plan.operations.extend(primary_gains(&safe_plan()));
    assert_eq!(validation_code(&plan), "BASS_OWNERSHIP_OVERLAP");
}

#[test]
fn duck_depth_and_attack_release_bounds_are_enforced() {
    let mut plan = safe_plan();
    plan.template.id = "ducked_overlap".into();
    plan.template.recipe_id = "test".into();
    let (points, interpolations) = envelope(&[
        (-10_000, 1_000_000),
        (-9_118, 251_189),
        (-8_000, 251_189),
        (-7_118, 1_000_000),
    ]);
    plan.operations.insert(
        0,
        Operation::DuckEnvelope(DuckEnvelope {
            op_id: "outgoing.duck".into(),
            target: Target::Outgoing,
            reason: DuckReason::VocalCollision,
            points,
            interpolations,
        }),
    );
    assert_valid(&plan);
    let Operation::DuckEnvelope(duck) = &mut plan.operations[0] else {
        unreachable!()
    };
    duck.points[1].value_ppm = 251_188;
    assert_eq!(validation_code(&plan), "GAIN_OUT_OF_RANGE");
}

#[test]
fn delay_taps_and_effect_end_are_exactly_bounded() {
    let mut plan = safe_plan();
    plan.template.id = "echo_tail_handoff".into();
    plan.template.recipe_id = "test".into();
    plan.timeline.effect_end_frame = 88_200;
    plan.operations.insert(
        0,
        Operation::FeedforwardDelayTail(FeedforwardDelayTail {
            op_id: "outgoing.tail".into(),
            target: Target::Outgoing,
            capture_start_frame: -22_050,
            capture_end_frame: 0,
            taps: vec![
                DelayTap {
                    delay_frames: 882,
                    gain_ppm: 500_000,
                },
                DelayTap {
                    delay_frames: 88_200,
                    gain_ppm: 300_000,
                },
            ],
        }),
    );
    assert_valid(&plan);
    let Operation::FeedforwardDelayTail(tail) = &mut plan.operations[0] else {
        unreachable!()
    };
    tail.taps[1].gain_ppm = 300_001;
    assert_eq!(validation_code(&plan), "INVALID_DELAY_TAIL");
}

#[test]
fn rhythmic_gate_has_explicit_click_safe_edges_and_terminal_zero() {
    let mut plan = safe_plan();
    plan.template.id = "rhythmic_handoff".into();
    plan.template.recipe_id = "test".into();
    let (points, interpolations) = envelope(&[
        (-10_000, 0),
        (-9_779, 1_000_000),
        (-9_000, 1_000_000),
        (-8_779, 0),
        (0, 0),
    ]);
    plan.operations.insert(
        0,
        Operation::RhythmicGate(RhythmicGate {
            op_id: "outgoing.gate".into(),
            target: Target::Outgoing,
            points,
            interpolations,
        }),
    );
    assert_valid(&plan);
    let Operation::RhythmicGate(gate) = &mut plan.operations[0] else {
        unreachable!()
    };
    gate.points[1].frame = -9_780;
    assert_eq!(validation_code(&plan), "INVALID_RHYTHMIC_GATE");
}

#[test]
fn tail_capture_may_start_before_primary_gain_points() {
    let mut plan = safe_plan();
    plan.template.id = "echo_tail_handoff".into();
    plan.template.recipe_id = "test".into();
    plan.timeline.dry_start_frame = -22_050;
    plan.timeline.effect_end_frame = 8_820;
    for operation in &mut plan.operations {
        let Operation::GainEnvelope(gain) = operation else {
            unreachable!()
        };
        gain.points[0].frame = -882;
    }
    plan.operations.insert(
        0,
        Operation::FeedforwardDelayTail(FeedforwardDelayTail {
            op_id: "outgoing.tail".into(),
            target: Target::Outgoing,
            capture_start_frame: -22_050,
            capture_end_frame: 0,
            taps: vec![DelayTap {
                delay_frames: 8_820,
                gain_ppm: 300_000,
            }],
        }),
    );
    assert_valid(&plan);
}
