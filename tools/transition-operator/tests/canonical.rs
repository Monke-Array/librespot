use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use transition_operator::{canonical_json, div_round_nearest_away, require_canonical_json};

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct StrictFixture {
    z: i64,
    a: i64,
}

#[test]
fn division_rounds_nearest_with_ties_away_from_zero() {
    let cases = [
        (3, 2, 2),
        (-3, 2, -2),
        (3, -2, -2),
        (-3, -2, 2),
        (2, 3, 1),
        (-2, 3, -1),
        (1, 3, 0),
        (-1, 3, 0),
    ];
    for (numerator, denominator, expected) in cases {
        assert_eq!(
            div_round_nearest_away(numerator, denominator).unwrap(),
            expected,
            "{numerator}/{denominator}"
        );
    }
    assert_eq!(
        div_round_nearest_away(1, 0).unwrap_err().code(),
        "INVALID_DIVISOR"
    );
    assert_eq!(
        div_round_nearest_away(i64::MIN, -1).unwrap_err().code(),
        "INTEGER_OVERFLOW"
    );
}

#[test]
fn canonical_json_sorts_keys_and_has_no_incidental_bytes() {
    let mut value = HashMap::new();
    value.insert("z", 0_i64);
    value.insert("a", 1_i64);
    assert_eq!(canonical_json(&value).unwrap(), br#"{"a":1,"z":0}"#);

    let fixture = include_bytes!("fixtures/plans/canonical-object.json");
    assert_eq!(
        fixture.strip_suffix(b"\n").unwrap_or(fixture),
        br#"{"a":1,"z":0}"#
    );
}

#[test]
fn canonical_json_preserves_utf8_values_but_rejects_non_integer_numbers() {
    #[derive(Serialize)]
    struct Text<'a> {
        label: &'a str,
    }
    assert_eq!(
        canonical_json(&Text { label: "Música" }).unwrap(),
        "{\"label\":\"Música\"}".as_bytes()
    );
    assert_eq!(
        canonical_json(&serde_json::json!({"value": 1.5}))
            .unwrap_err()
            .code(),
        "NON_INTEGER_JSON_NUMBER"
    );
}

#[test]
fn canonical_json_enforces_interoperable_integer_range() {
    const LIMIT: i64 = 9_007_199_254_740_991;
    assert_eq!(canonical_json(&LIMIT).unwrap(), b"9007199254740991");
    assert_eq!(canonical_json(&-LIMIT).unwrap(), b"-9007199254740991");
    assert_eq!(
        canonical_json(&(LIMIT + 1)).unwrap_err().code(),
        "INTEGER_OUT_OF_RANGE"
    );
    assert_eq!(
        canonical_json(&(-LIMIT - 1)).unwrap_err().code(),
        "INTEGER_OUT_OF_RANGE"
    );
}

#[test]
fn strict_input_must_already_equal_canonical_bytes() {
    let accepted = require_canonical_json::<StrictFixture>(br#"{"a":1,"z":0}"#).unwrap();
    assert_eq!(accepted, StrictFixture { z: 0, a: 1 });

    for rejected in [
        br#"{"z":0,"a":1}"#.as_slice(),
        br#" {"a":1,"z":0}"#.as_slice(),
        br#"{"a":1,"z":0}
"#
        .as_slice(),
    ] {
        assert_eq!(
            require_canonical_json::<StrictFixture>(rejected)
                .unwrap_err()
                .code(),
            "NON_CANONICAL_JSON"
        );
    }
}

#[test]
fn strict_input_fails_closed_on_malformed_or_unsupported_json() {
    let cases: &[(&[u8], &str)] = &[
        (b"\xef\xbb\xbf{\"a\":1,\"z\":0}", "INVALID_JSON"),
        (br#"{"a":1.0,"z":0}"#, "INVALID_JSON"),
        (br#"{"a":1e0,"z":0}"#, "INVALID_JSON"),
        (br#"{"a":null,"z":0}"#, "INVALID_JSON"),
        (br#"{"a":1,"a":1,"z":0}"#, "INVALID_JSON"),
        (br#"{"a":1,"extra":2,"z":0}"#, "INVALID_JSON"),
    ];
    for (bytes, expected_code) in cases {
        assert_eq!(
            require_canonical_json::<StrictFixture>(bytes)
                .unwrap_err()
                .code(),
            *expected_code,
            "input: {}",
            String::from_utf8_lossy(bytes)
        );
    }
}
