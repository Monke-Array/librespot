//! Parsing for transition metadata already materialized onto Spotify Connect tracks.
//!
//! This module deliberately stops at a typed render description. It does not select a transition
//! or apply volume, EQ, filter, speed, or other DSP automation.

use std::collections::HashMap;

use librespot_protocol::player::ProvidedTrack;
use serde_json::Value;
use thiserror::Error;

const FADE_OUT_START: &str = "audio.fade_out_start_time";
const FADE_OUT_DURATION: &str = "audio.fade_out_duration";
const FADE_OUT_CURVES: &str = "audio.fade_out_curves";
const FADE_OUT_EQ_LOW_GAIN_CURVES: &str = "audio.fade_out_eq_low_gain_curves";
const FADE_OUT_FILTER_CUTOFF_CURVES: &str = "audio.fade_out_filter_cutoff_curves";
const FADE_OUT_FILTER_RESONANCE_CURVES: &str = "audio.fade_out_filter_resonance_curves";
const FADE_IN_START: &str = "audio.fade_in_start_time";
const FADE_IN_DURATION: &str = "audio.fade_in_duration";
const FADE_IN_CURVES: &str = "audio.fade_in_curves";
const FADE_IN_EQ_LOW_GAIN_CURVES: &str = "audio.fade_in_eq_low_gain_curves";
const FADE_OVERLAP: &str = "audio.fade_overlap";
const SPEED_AUTOMATION: &str = "audio.speed_automation";
const ONLY_ALLOW_FADE_ON_ADVANCE: &str = "audio.only_allow_fade_on_advance";
const AUDIO_AUTOMIX_MODE: &str = "audio.automix_mode";
const AUTO_PRESET_ID: &str = "automix.auto_preset_id";
const AUTOMIX_MODE: &str = "automix.mode";
const TRANSITION_URI: &str = "automix.transition_uri";

/// One control point in a materialized automation curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutomationCurvePoint {
    /// Curve-local horizontal coordinate.
    pub x: f64,
    /// Automation value at this control point.
    pub y: f64,
}

/// One segment of a piecewise materialized automation curve.
#[derive(Clone, Debug, PartialEq)]
pub struct AutomationCurveSegment {
    /// Segment start within the complete automation timeline.
    pub start: f64,
    /// Segment end within the complete automation timeline.
    pub end: f64,
    /// Ordered line, quadratic, or cubic control points.
    pub points: Vec<AutomationCurvePoint>,
}

/// A piecewise automation curve.
#[derive(Clone, Debug, PartialEq)]
pub struct PiecewiseAutomationCurve {
    /// Ordered, non-overlapping curve segments.
    pub segments: Vec<AutomationCurveSegment>,
}

/// One materialized playback-speed automation point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeedAutomationPoint {
    /// Position on the automation timeline, in the units supplied by Spotify.
    pub position: u64,
    /// Positive playback speed at this point.
    pub speed: f64,
}

/// Start and duration for one side of a materialized transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaterializedTransitionTiming {
    /// Source position at which this side begins participating.
    pub start_ms: u64,
    /// Materialized automation duration for this side.
    pub duration_ms: u64,
}

/// Typed, non-rendering description of one materialized A -> B transition.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterializedTransitionRenderPlan {
    /// Outgoing timing sourced only from track A.
    pub outgoing: MaterializedTransitionTiming,
    /// Incoming timing sourced only from track B.
    pub incoming: MaterializedTransitionTiming,
    /// Authoritative wall-clock overlap. `audio.fade_overlap` on B wins when present.
    pub overlap_ms: u64,
    /// Preset sourced from A's outgoing edge metadata.
    pub preset_id: u32,
    /// Optional `automix.mode` sourced from A.
    pub mode: Option<String>,
    /// Optional `audio.automix_mode` sourced from incoming track B.
    pub incoming_audio_automix_mode: Option<String>,
    /// Optional transition URI sourced from A.
    pub transition_uri: Option<String>,
    /// Whether advancing is the only operation allowed to apply the fade.
    pub only_allow_fade_on_advance: Option<bool>,
    /// Outgoing volume automation sourced from A.
    pub outgoing_volume: Option<PiecewiseAutomationCurve>,
    /// Incoming volume automation sourced from B.
    pub incoming_volume: Option<PiecewiseAutomationCurve>,
    /// Outgoing low-band EQ automation sourced from A.
    pub outgoing_eq_low_gain: Option<PiecewiseAutomationCurve>,
    /// Incoming low-band EQ automation sourced from B.
    pub incoming_eq_low_gain: Option<PiecewiseAutomationCurve>,
    /// Outgoing filter-cutoff automation sourced from A.
    pub outgoing_filter_cutoff: Option<PiecewiseAutomationCurve>,
    /// Outgoing filter-resonance automation sourced from A.
    pub outgoing_filter_resonance: Option<PiecewiseAutomationCurve>,
    /// Incoming speed automation sourced from B.
    pub incoming_speed: Vec<SpeedAutomationPoint>,
}

/// Failure to parse a present materialized transition.
#[derive(Debug, Error, PartialEq)]
pub enum MaterializedTransitionError {
    /// A partially materialized edge omitted a required field.
    #[error("materialized transition is missing {0}")]
    Missing(&'static str),
    /// A scalar metadata value was malformed or outside its valid range.
    #[error("materialized transition has invalid {0}")]
    InvalidScalar(&'static str),
    /// A JSON metadata value was malformed.
    #[error("materialized transition has invalid JSON in {key}: {reason}")]
    InvalidJson {
        /// Metadata key containing malformed JSON.
        key: &'static str,
        /// Narrow validation failure.
        reason: String,
    },
}

/// Parse one A -> B edge from A's outgoing and B's incoming materialized metadata.
///
/// `Ok(None)` means the pair has no materialized edge. Once any edge field is present, missing or
/// malformed required data fails closed with an error.
pub fn materialized_transition_plan_for_pair(
    outgoing: &ProvidedTrack,
    incoming: &ProvidedTrack,
) -> Result<Option<MaterializedTransitionRenderPlan>, MaterializedTransitionError> {
    let a = &outgoing.metadata;
    let b = &incoming.metadata;
    let edge_present = [FADE_OUT_START, FADE_OUT_DURATION, AUTO_PRESET_ID]
        .iter()
        .any(|key| a.contains_key(*key))
        || [FADE_IN_START, FADE_IN_DURATION, FADE_OVERLAP]
            .iter()
            .any(|key| b.contains_key(*key));
    if !edge_present {
        return Ok(None);
    }

    let outgoing = MaterializedTransitionTiming {
        start_ms: required_u64(a, FADE_OUT_START)?,
        duration_ms: required_positive_u64(a, FADE_OUT_DURATION)?,
    };
    let incoming = MaterializedTransitionTiming {
        start_ms: required_u64(b, FADE_IN_START)?,
        duration_ms: required_positive_u64(b, FADE_IN_DURATION)?,
    };
    let overlap_ms = optional_positive_u64(b, FADE_OVERLAP)?.unwrap_or(outgoing.duration_ms);

    Ok(Some(MaterializedTransitionRenderPlan {
        outgoing,
        incoming,
        overlap_ms,
        preset_id: required_u32(a, AUTO_PRESET_ID)?,
        mode: optional_nonempty_string(a, AUTOMIX_MODE)?,
        incoming_audio_automix_mode: optional_nonempty_string(b, AUDIO_AUTOMIX_MODE)?,
        transition_uri: optional_nonempty_string(a, TRANSITION_URI)?,
        only_allow_fade_on_advance: optional_bool(a, ONLY_ALLOW_FADE_ON_ADVANCE)?,
        outgoing_volume: optional_curve(a, FADE_OUT_CURVES)?,
        incoming_volume: optional_curve(b, FADE_IN_CURVES)?,
        outgoing_eq_low_gain: optional_curve(a, FADE_OUT_EQ_LOW_GAIN_CURVES)?,
        incoming_eq_low_gain: optional_curve(b, FADE_IN_EQ_LOW_GAIN_CURVES)?,
        outgoing_filter_cutoff: optional_curve(a, FADE_OUT_FILTER_CUTOFF_CURVES)?,
        outgoing_filter_resonance: optional_curve(a, FADE_OUT_FILTER_RESONANCE_CURVES)?,
        incoming_speed: optional_speed_automation(b)?.unwrap_or_default(),
    }))
}

fn required_u64(
    metadata: &HashMap<String, String>,
    key: &'static str,
) -> Result<u64, MaterializedTransitionError> {
    metadata
        .get(key)
        .ok_or(MaterializedTransitionError::Missing(key))?
        .parse()
        .map_err(|_| MaterializedTransitionError::InvalidScalar(key))
}

fn required_positive_u64(
    metadata: &HashMap<String, String>,
    key: &'static str,
) -> Result<u64, MaterializedTransitionError> {
    let value = required_u64(metadata, key)?;
    (value > 0)
        .then_some(value)
        .ok_or(MaterializedTransitionError::InvalidScalar(key))
}

fn optional_positive_u64(
    metadata: &HashMap<String, String>,
    key: &'static str,
) -> Result<Option<u64>, MaterializedTransitionError> {
    metadata
        .get(key)
        .map(|_| required_positive_u64(metadata, key))
        .transpose()
}

fn required_u32(
    metadata: &HashMap<String, String>,
    key: &'static str,
) -> Result<u32, MaterializedTransitionError> {
    metadata
        .get(key)
        .ok_or(MaterializedTransitionError::Missing(key))?
        .parse()
        .map_err(|_| MaterializedTransitionError::InvalidScalar(key))
}

fn optional_nonempty_string(
    metadata: &HashMap<String, String>,
    key: &'static str,
) -> Result<Option<String>, MaterializedTransitionError> {
    metadata
        .get(key)
        .map(|value| {
            (!value.is_empty() && !value.chars().any(char::is_control))
                .then(|| value.clone())
                .ok_or(MaterializedTransitionError::InvalidScalar(key))
        })
        .transpose()
}

fn optional_bool(
    metadata: &HashMap<String, String>,
    key: &'static str,
) -> Result<Option<bool>, MaterializedTransitionError> {
    metadata
        .get(key)
        .map(|value| match value.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(MaterializedTransitionError::InvalidScalar(key)),
        })
        .transpose()
}

fn optional_curve(
    metadata: &HashMap<String, String>,
    key: &'static str,
) -> Result<Option<PiecewiseAutomationCurve>, MaterializedTransitionError> {
    metadata
        .get(key)
        .map(|encoded| parse_curve(encoded, key))
        .transpose()
}

fn parse_curve(
    encoded: &str,
    key: &'static str,
) -> Result<PiecewiseAutomationCurve, MaterializedTransitionError> {
    let value: Value = serde_json::from_str(encoded).map_err(|error| invalid_json(key, error))?;
    let values = value
        .as_array()
        .ok_or_else(|| invalid_json_message(key, "curve segments were not an array"))?;
    if values.is_empty() {
        return Err(invalid_json_message(key, "curve contained no segments"));
    }
    let segments = values
        .iter()
        .map(|value| parse_segment(value, key))
        .collect::<Result<Vec<_>, _>>()?;
    validate_segments(&segments, key)?;
    Ok(PiecewiseAutomationCurve { segments })
}

fn parse_segment(
    value: &Value,
    key: &'static str,
) -> Result<AutomationCurveSegment, MaterializedTransitionError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid_json_message(key, "curve segment was not an object"))?;
    let start = object
        .get("start_point")
        .ok_or_else(|| invalid_json_message(key, "start_point is missing"))
        .and_then(|value| finite(value, key))?;
    let end = object
        .get("end_point")
        .ok_or_else(|| invalid_json_message(key, "end_point is missing"))
        .and_then(|value| finite(value, key))?;
    let points = object
        .get("fade_curve")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_json_message(key, "fade_curve is missing or not an array"))?;
    parse_segment_points(points, start, end, key)
}

fn parse_segment_points(
    points: &[Value],
    start: f64,
    end: f64,
    key: &'static str,
) -> Result<AutomationCurveSegment, MaterializedTransitionError> {
    if !(2..=4).contains(&points.len()) {
        return Err(invalid_json_message(
            key,
            "curve segment needs two, three, or four points",
        ));
    }
    let points = points
        .iter()
        .map(|point| parse_curve_point(point, key))
        .collect::<Result<Vec<_>, _>>()?;
    if points.windows(2).any(|pair| pair[1].x < pair[0].x) {
        return Err(invalid_json_message(key, "curve point order is invalid"));
    }
    Ok(AutomationCurveSegment { start, end, points })
}

fn parse_curve_point(
    value: &Value,
    key: &'static str,
) -> Result<AutomationCurvePoint, MaterializedTransitionError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid_json_message(key, "curve point was not an object"))?;
    let x = object
        .get("x")
        .ok_or_else(|| invalid_json_message(key, "curve point x is missing"))
        .and_then(|value| finite(value, key))?;
    let y = object
        .get("y")
        .ok_or_else(|| invalid_json_message(key, "curve point y is missing"))
        .and_then(|value| finite(value, key))?;
    Ok(AutomationCurvePoint { x, y })
}

fn validate_segments(
    segments: &[AutomationCurveSegment],
    key: &'static str,
) -> Result<(), MaterializedTransitionError> {
    if segments.iter().any(|segment| {
        !segment.start.is_finite() || !segment.end.is_finite() || segment.start >= segment.end
    }) {
        return Err(invalid_json_message(
            key,
            "curve segment bounds are invalid",
        ));
    }
    if segments.windows(2).any(|pair| pair[1].start < pair[0].end) {
        return Err(invalid_json_message(
            key,
            "curve segments overlap or are unordered",
        ));
    }
    Ok(())
}

fn optional_speed_automation(
    metadata: &HashMap<String, String>,
) -> Result<Option<Vec<SpeedAutomationPoint>>, MaterializedTransitionError> {
    let Some(encoded) = metadata.get(SPEED_AUTOMATION) else {
        return Ok(None);
    };
    let value: Value =
        serde_json::from_str(encoded).map_err(|error| invalid_json(SPEED_AUTOMATION, error))?;
    let values = value.as_array().ok_or_else(|| {
        invalid_json_message(SPEED_AUTOMATION, "speed automation was not an array")
    })?;
    if values.is_empty() {
        return Err(invalid_json_message(
            SPEED_AUTOMATION,
            "speed automation contained no points",
        ));
    }
    let points = values
        .iter()
        .map(|value| {
            let object = value.as_object().ok_or_else(|| {
                invalid_json_message(SPEED_AUTOMATION, "speed point was not an object")
            })?;
            let position = object
                .get("from_position")
                .ok_or_else(|| invalid_json_message(SPEED_AUTOMATION, "from_position is missing"))
                .and_then(|value| {
                    value.as_u64().ok_or_else(|| {
                        invalid_json_message(
                            SPEED_AUTOMATION,
                            "from_position is not a non-negative integer",
                        )
                    })
                })?;
            let speed = object
                .get("speed")
                .ok_or_else(|| invalid_json_message(SPEED_AUTOMATION, "speed is missing"))
                .and_then(|value| finite(value, SPEED_AUTOMATION))?;
            if speed <= 0.0 {
                return Err(invalid_json_message(
                    SPEED_AUTOMATION,
                    "speed point contains a non-positive speed",
                ));
            }
            Ok(SpeedAutomationPoint { position, speed })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if points
        .windows(2)
        .any(|pair| pair[1].position < pair[0].position)
    {
        return Err(invalid_json_message(
            SPEED_AUTOMATION,
            "speed points are unordered",
        ));
    }
    Ok(Some(points))
}

fn finite(value: &Value, key: &'static str) -> Result<f64, MaterializedTransitionError> {
    value
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| invalid_json_message(key, "number is missing or non-finite"))
}

fn invalid_json(key: &'static str, error: serde_json::Error) -> MaterializedTransitionError {
    invalid_json_message(key, error.to_string())
}

fn invalid_json_message(
    key: &'static str,
    reason: impl Into<String>,
) -> MaterializedTransitionError {
    MaterializedTransitionError::InvalidJson {
        key,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK_A: &str = "spotify:track:2BMRUAA1oTc7e9JPlr6xbZ";
    const TRACK_B: &str = "spotify:track:5g9lS8deSIxItFBmZRC4vN";
    const CAPTURED_OUTGOING_VOLUME_CURVE: &str = r#"[
        {
            "start_point": 0,
            "end_point": 1,
            "fade_curve": [
                {"x":0,"y":1},
                {"x":0.6,"y":1},
                {"x":0.6,"y":0},
                {"x":1,"y":0}
            ]
        }
    ]"#;
    const CAPTURED_LOW_EQ_SEGMENTED_CURVE: &str = r#"[
        {
            "start_point": 0,
            "end_point": 0.46875,
            "fade_curve": [
                {"x":0,"y":0.5},
                {"x":1,"y":0.5}
            ]
        },
        {
            "start_point": 0.46875,
            "end_point": 0.5,
            "fade_curve": [
                {"x":0,"y":0.5},
                {"x":0.5,"y":0.5},
                {"x":0.5,"y":0},
                {"x":1,"y":0}
            ]
        },
        {
            "start_point": 0.5,
            "end_point": 1,
            "fade_curve": [
                {"x":0,"y":0},
                {"x":1,"y":0}
            ]
        }
    ]"#;
    const CAPTURED_SPEED_AUTOMATION: &str = r#"[
        {"from_position":0,"speed":0.90312},
        {"from_position":8763,"speed":0.90812},
        {"from_position":27763,"speed":1.0}
    ]"#;
    const CAPTURED_INCOMING_VOLUME_CURVE: &str = r#"[
        {
            "start_point": 0,
            "end_point": 1,
            "fade_curve": [
                {"x":0,"y":0},
                {"x":0.4,"y":0},
                {"x":0.4,"y":1},
                {"x":1,"y":1}
            ]
        }
    ]"#;

    fn track(uri: &str, metadata: &[(&str, &str)]) -> ProvidedTrack {
        ProvidedTrack {
            uri: uri.to_owned(),
            metadata: metadata
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            ..Default::default()
        }
    }

    fn outgoing() -> ProvidedTrack {
        track(
            TRACK_A,
            &[
                (FADE_OUT_START, "208960"),
                (FADE_OUT_DURATION, "6090"),
                (AUTO_PRESET_ID, "1"),
                (AUTOMIX_MODE, "auto"),
                (TRANSITION_URI, "spotify:core-auto-transition"),
                (FADE_OUT_CURVES, CAPTURED_OUTGOING_VOLUME_CURVE),
                (FADE_OUT_EQ_LOW_GAIN_CURVES, CAPTURED_LOW_EQ_SEGMENTED_CURVE),
            ],
        )
    }

    fn incoming() -> ProvidedTrack {
        track(
            TRACK_B,
            &[
                (FADE_IN_START, "2763"),
                (FADE_IN_DURATION, "5500"),
                (FADE_OVERLAP, "6090"),
                (SPEED_AUTOMATION, CAPTURED_SPEED_AUTOMATION),
                (FADE_IN_CURVES, CAPTURED_INCOMING_VOLUME_CURVE),
                (AUDIO_AUTOMIX_MODE, "auto"),
                (AUTO_PRESET_ID, "19"),
            ],
        )
    }

    #[test]
    fn known_pair_combines_a_outgoing_with_b_incoming_metadata() {
        let plan = materialized_transition_plan_for_pair(&outgoing(), &incoming())
            .unwrap()
            .unwrap();

        assert_eq!(plan.outgoing.start_ms, 208_960);
        assert_eq!(plan.outgoing.duration_ms, 6_090);
        assert_eq!(plan.incoming.start_ms, 2_763);
        assert_eq!(plan.incoming.duration_ms, 5_500);
        assert_eq!(plan.overlap_ms, 6_090);
        assert!((plan.incoming_speed[0].speed - 0.90312).abs() < 1.0e-12);
        assert_eq!(plan.preset_id, 1);
        assert_eq!(plan.mode.as_deref(), Some("auto"));
        assert_eq!(plan.incoming_audio_automix_mode.as_deref(), Some("auto"));
        assert_eq!(
            plan.transition_uri.as_deref(),
            Some("spotify:core-auto-transition")
        );
        assert_eq!(plan.outgoing_volume.unwrap().segments.len(), 1);
        assert_eq!(plan.incoming_volume.unwrap().segments[0].points.len(), 4);
    }

    #[test]
    fn exact_captured_outgoing_volume_curve_parses() {
        let curve = parse_curve(CAPTURED_OUTGOING_VOLUME_CURVE, FADE_OUT_CURVES).unwrap();

        assert_eq!(curve.segments.len(), 1);
        assert_eq!(curve.segments[0].start, 0.0);
        assert_eq!(curve.segments[0].end, 1.0);
        assert_eq!(
            curve.segments[0].points,
            [
                AutomationCurvePoint { x: 0.0, y: 1.0 },
                AutomationCurvePoint { x: 0.6, y: 1.0 },
                AutomationCurvePoint { x: 0.6, y: 0.0 },
                AutomationCurvePoint { x: 1.0, y: 0.0 },
            ]
        );
    }

    #[test]
    fn exact_captured_low_eq_segmented_curve_parses() {
        let curve =
            parse_curve(CAPTURED_LOW_EQ_SEGMENTED_CURVE, FADE_OUT_EQ_LOW_GAIN_CURVES).unwrap();

        assert_eq!(curve.segments.len(), 3);
        assert_eq!(curve.segments[0].start, 0.0);
        assert_eq!(curve.segments[0].end, 0.46875);
        assert_eq!(curve.segments[1].start, 0.46875);
        assert_eq!(curve.segments[1].end, 0.5);
        assert_eq!(curve.segments[2].start, 0.5);
        assert_eq!(curve.segments[2].end, 1.0);
        assert_eq!(
            curve.segments[1].points[1],
            AutomationCurvePoint { x: 0.5, y: 0.5 }
        );
        assert_eq!(
            curve.segments[1].points[2],
            AutomationCurvePoint { x: 0.5, y: 0.0 }
        );
    }

    #[test]
    fn exact_captured_speed_automation_parses() {
        let plan = materialized_transition_plan_for_pair(&outgoing(), &incoming())
            .unwrap()
            .unwrap();

        assert_eq!(plan.incoming_speed[0].position, 0);
        assert!((plan.incoming_speed[0].speed - 0.90312).abs() < 1.0e-12);
        assert_eq!(plan.incoming_speed[1].position, 8_763);
        assert_eq!(plan.incoming_speed.last().unwrap().position, 27_763);
        assert_eq!(plan.incoming_speed.last().unwrap().speed, 1.0);
    }

    #[test]
    fn incoming_overlap_is_authoritative_when_durations_differ() {
        let mut outgoing = outgoing();
        outgoing
            .metadata
            .insert(FADE_OUT_DURATION.to_owned(), "7000".to_owned());
        let plan = materialized_transition_plan_for_pair(&outgoing, &incoming())
            .unwrap()
            .unwrap();

        assert_eq!(plan.outgoing.duration_ms, 7_000);
        assert_eq!(plan.overlap_ms, 6_090);
    }

    #[test]
    fn malformed_materialized_values_fail_closed() {
        let mut malformed_incoming = incoming();
        malformed_incoming
            .metadata
            .insert(SPEED_AUTOMATION.to_owned(), "not-json".to_owned());
        assert!(matches!(
            materialized_transition_plan_for_pair(&outgoing(), &malformed_incoming),
            Err(MaterializedTransitionError::InvalidJson {
                key: SPEED_AUTOMATION,
                ..
            })
        ));

        malformed_incoming.metadata.insert(
            SPEED_AUTOMATION.to_owned(),
            r#"[{"position":0,"speed":0.90312}]"#.to_owned(),
        );
        assert!(matches!(
            materialized_transition_plan_for_pair(&outgoing(), &malformed_incoming),
            Err(MaterializedTransitionError::InvalidJson {
                key: SPEED_AUTOMATION,
                ..
            })
        ));

        let mut malformed_outgoing = outgoing();
        malformed_outgoing.metadata.insert(
            FADE_OUT_CURVES.to_owned(),
            r#"[{"start":0,"end":1,"points":[{"x":0,"y":1},{"x":1,"y":0}]}]"#.to_owned(),
        );
        assert!(matches!(
            materialized_transition_plan_for_pair(&malformed_outgoing, &incoming()),
            Err(MaterializedTransitionError::InvalidJson {
                key: FADE_OUT_CURVES,
                ..
            })
        ));

        let mut outgoing = outgoing();
        outgoing
            .metadata
            .insert(FADE_OUT_START.to_owned(), "-1".to_owned());
        assert_eq!(
            materialized_transition_plan_for_pair(&outgoing, &incoming()),
            Err(MaterializedTransitionError::InvalidScalar(FADE_OUT_START))
        );
    }

    #[test]
    fn absent_materialized_edge_is_unavailable() {
        assert_eq!(
            materialized_transition_plan_for_pair(
                &track(TRACK_A, &[]),
                &track(TRACK_B, &[(AUTO_PRESET_ID, "22")]),
            ),
            Ok(None)
        );
    }
}
