use crate::dsp::PcmBuffer;
use crate::error::{Error, Result};
use crate::model::FeedforwardDelayTail;

pub fn render_feedforward_tail(
    input: &PcmBuffer,
    input_start_frame: i64,
    output_start_frame: i64,
    output_end_frame: i64,
    tail: &FeedforwardDelayTail,
) -> Result<PcmBuffer> {
    let output_frames = output_end_frame
        .checked_sub(output_start_frame)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| {
            Error::new(
                "INVALID_DSP_OUTPUT_WINDOW",
                "delay output window must be nonempty and addressable",
            )
        })?;
    if output_frames == 0 {
        return Err(Error::new(
            "INVALID_DSP_OUTPUT_WINDOW",
            "delay output window must be nonempty",
        ));
    }
    let mut output = vec![[0.0, 0.0]; output_frames];
    for (offset, wet) in output.iter_mut().enumerate() {
        let frame = output_start_frame
            .checked_add(
                i64::try_from(offset)
                    .map_err(|_| Error::new("INTEGER_OVERFLOW", "delay output offset overflow"))?,
            )
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "delay output frame overflow"))?;
        for tap in &tail.taps {
            let origin = frame
                .checked_sub(tap.delay_frames)
                .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "delay origin frame overflow"))?;
            if !(tail.capture_start_frame..tail.capture_end_frame).contains(&origin) {
                continue;
            }
            let input_offset = origin
                .checked_sub(input_start_frame)
                .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "delay source offset overflow"))?;
            let source = usize::try_from(input_offset)
                .ok()
                .and_then(|index| input.frames().get(index))
                .ok_or_else(|| {
                    Error::new(
                        "DSP_SOURCE_WINDOW_MISSING",
                        "captured delay source frame is unavailable",
                    )
                })?;
            let gain = tap.gain_ppm as f64 / 1_000_000.0;
            wet[0] += source[0] * gain;
            wet[1] += source[1] * gain;
        }
    }
    PcmBuffer::from_frames(output)
}
