use serde_json::Value;
use std::collections::BTreeSet;

const AUDIT: &[u8] = include_bytes!("fixtures/features/pilot-v1-audit.json");

#[test]
fn pilot_v1_audit_covers_every_formal_feature_category_with_evidence() {
    let audit: Value = serde_json::from_slice(AUDIT).expect("audit fixture must be JSON");
    assert_eq!(audit["schema_version"], "transition-feature-audit/1");

    let expected: BTreeSet<_> = [
        "source_identity_duration",
        "cue",
        "beat_downbeat_meter_tempo",
        "vocal",
        "transient",
        "bass",
        "spectral",
        "energy",
        "whole_source_peaks",
    ]
    .into_iter()
    .collect();
    let entries = audit["categories"]
        .as_array()
        .expect("audit categories must be an array");
    let actual: BTreeSet<_> = entries
        .iter()
        .map(|entry| {
            entry["category"]
                .as_str()
                .expect("category must be a string")
        })
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(entries.len(), expected.len(), "categories must be unique");

    for entry in entries {
        let category = entry["category"].as_str().unwrap();
        for field in [
            "pilot_v1_fields",
            "v2_fields",
            "old_units",
            "new_units",
            "old_definition",
            "new_definition",
            "source_window_semantics",
            "missing_value_semantics",
            "normalization_semantics",
            "extractor_provenance",
            "evidence",
        ] {
            assert!(
                nonempty(&entry[field]),
                "{category}.{field} must contain explicit evidence"
            );
        }
        assert!(
            matches!(
                entry["disposition"].as_str(),
                Some("reuse_exact")
                    | Some("migrate_with_named_transform")
                    | Some("recompute_v2")
                    | Some("omit_v2")
            ),
            "{category} must have an approved disposition"
        );
        let provenance = &entry["extractor_provenance"];
        for field in ["repository", "revision", "path", "symbols"] {
            assert!(
                nonempty(&provenance[field]),
                "{category}.extractor_provenance.{field} is required"
            );
        }
    }
}

fn nonempty(value: &Value) -> bool {
    match value {
        Value::String(value) => !value.trim().is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(values) => !values.is_empty(),
        _ => false,
    }
}
