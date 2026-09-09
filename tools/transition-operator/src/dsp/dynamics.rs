use crate::dsp::{PcmBuffer, eval_envelope};
use crate::error::{Error, Result};
use crate::model::{DuckEnvelope, EnvelopePoint, GainEnvelope, Interpolation, RhythmicGate};

pub fn apply_gain(buffer: &mut PcmBuffer, start_frame: i64, value: &GainEnvelope) -> Result<()> {
    apply_multiplier(buffer, start_frame, &value.points, &value.interpolations)
}

pub fn apply_duck(buffer: &mut PcmBuffer, start_frame: i64, value: &DuckEnvelope) -> Result<()> {
    apply_multiplier(buffer, start_frame, &value.points, &value.interpolations)
}

pub fn apply_gate(buffer: &mut PcmBuffer, start_frame: i64, value: &RhythmicGate) -> Result<()> {
    apply_multiplier(buffer, start_frame, &value.points, &value.interpolations)
}

fn apply_multiplier(
    buffer: &mut PcmBuffer,
    start_frame: i64,
    points: &[EnvelopePoint],
    interpolations: &[Interpolation],
) -> Result<()> {
    for (offset, frame) in buffer.frames_mut().iter_mut().enumerate() {
        let timeline_frame = start_frame
            .checked_add(i64::try_from(offset).map_err(|_| {
                Error::new("INTEGER_OVERFLOW", "PCM offset does not fit signed frames")
            })?)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "PCM timeline frame overflow"))?;
        let gain = eval_envelope(points, interpolations, timeline_frame)?;
        frame[0] *= gain;
        frame[1] *= gain;
    }
    Ok(())
}
