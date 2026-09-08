use transition_operator::{Operation, OperatorPlan, require_canonical_json};

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
