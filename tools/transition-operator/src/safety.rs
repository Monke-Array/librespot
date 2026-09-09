use crate::error::{Error, Result};

pub fn calculate_pair_output_gain(
    outgoing_true_peak_mdbtp: Option<i64>,
    incoming_true_peak_mdbtp: Option<i64>,
    operation_margin_mdb: i64,
) -> Result<i64> {
    let outgoing = outgoing_true_peak_mdbtp.ok_or_else(|| {
        Error::new(
            "MISSING_SOURCE_TRUE_PEAK",
            "whole-source outgoing true peak is required",
        )
    })?;
    let incoming = incoming_true_peak_mdbtp.ok_or_else(|| {
        Error::new(
            "MISSING_SOURCE_TRUE_PEAK",
            "whole-source incoming true peak is required",
        )
    })?;
    if !matches!(operation_margin_mdb, 0 | 1_000 | 3_000) {
        return Err(Error::new(
            "INVALID_OPERATION_MARGIN",
            "operation margin must be a named v1 value",
        ));
    }
    let peak = outgoing.max(incoming);
    let gain = (-1_200_i64)
        .checked_sub(peak)
        .and_then(|value| value.checked_sub(6_021))
        .and_then(|value| value.checked_sub(operation_margin_mdb))
        .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "pair headroom calculation overflow"))?
        .min(0);
    if gain < -24_000 {
        return Err(Error::new(
            "PAIR_HEADROOM_EXCEEDS_LIMIT",
            "required pair output gain is below -24000 mdb",
        ));
    }
    Ok(gain)
}
