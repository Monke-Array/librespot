use crate::error::Result;
use serde_json::Value;

/// Sanitized result of the audited Pilot V1 import boundary.
///
/// Task 20 found no Pilot V1 measurement with semantics identical to snapshot
/// v2. The explicit empty result prevents generic JSON passthrough or accidental
/// promotion of the old constant/synthetic values.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PilotV1Import {
    pub source_identity: Option<String>,
    pub cues: Vec<String>,
    pub rhythm: Option<String>,
    pub vocal_activity_ppm: Option<i64>,
    pub whole_source_true_peak_mdbtp: Option<i64>,
}

pub fn import_pilot_v1(_value: &Value) -> Result<PilotV1Import> {
    Ok(PilotV1Import::default())
}
