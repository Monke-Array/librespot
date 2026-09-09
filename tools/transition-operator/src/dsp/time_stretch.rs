use crate::canonical::canonical_json;
use crate::dsp::PcmBuffer;
use crate::error::{Error, Result};
use crate::render::FfmpegBackend;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimeStretchCue {
    pub input_frame: usize,
    pub output_frame: usize,
}

impl TimeStretchCue {
    pub fn logical_error_frames(self, rate_ppm: i64) -> usize {
        if rate_ppm <= 0 {
            return usize::MAX;
        }
        let expected_input =
            ((self.output_frame as u128 * rate_ppm as u128 + 500_000) / 1_000_000) as usize;
        self.input_frame.abs_diff(expected_input)
    }
}

pub trait TimeStretchBackend {
    fn process(
        &self,
        input: &PcmBuffer,
        rate_ppm: i64,
        output_frames: usize,
        cue: TimeStretchCue,
    ) -> Result<PcmBuffer>;
}

#[derive(Clone, Debug)]
pub struct RubberBandTimeStretch {
    ffmpeg: FfmpegBackend,
}

impl RubberBandTimeStretch {
    pub fn new(executable: impl Into<std::path::PathBuf>) -> Self {
        Self {
            ffmpeg: FfmpegBackend::new(executable),
        }
    }

    pub fn program_hash(&self, rate_ppm: i64, output_frames: usize) -> Result<String> {
        if !(920_000..=1_080_000).contains(&rate_ppm) || output_frames == 0 {
            return Err(Error::new(
                "INVALID_TIME_STRETCH_PROGRAM",
                "time-stretch program parameters are outside the v1 profile",
            ));
        }
        let arguments = FfmpegBackend::rubberband_arguments(rate_ppm, output_frames);
        Ok(hex(&Sha256::digest(canonical_json(&arguments)?)))
    }
}

impl TimeStretchBackend for RubberBandTimeStretch {
    fn process(
        &self,
        input: &PcmBuffer,
        rate_ppm: i64,
        output_frames: usize,
        cue: TimeStretchCue,
    ) -> Result<PcmBuffer> {
        if !(920_000..=1_080_000).contains(&rate_ppm) {
            return Err(Error::new(
                "INVALID_TIME_STRETCH_RATE",
                "time-stretch rate is outside the v1 profile",
            ));
        }
        if output_frames == 0
            || cue.output_frame >= output_frames
            || cue.input_frame >= input.frame_count()
        {
            return Err(Error::new(
                "INVALID_TIME_STRETCH_CUE",
                "time-stretch cue or output interval is invalid",
            ));
        }
        if cue.logical_error_frames(rate_ppm) > 1 {
            return Err(Error::new(
                "TIME_STRETCH_CUE_MISALIGNED",
                "input and output cue frames differ from the declared constant rate",
            ));
        }
        let required_input = output_frames
            .checked_mul(usize::try_from(rate_ppm).map_err(|_| {
                Error::new("INTEGER_OVERFLOW", "time-stretch rate conversion overflow")
            })?)
            .and_then(|value| value.checked_add(999_999))
            .map(|value| value / 1_000_000)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "time-stretch source bound overflow"))?;
        if input.frame_count() < required_input {
            return Err(Error::new(
                "TIME_STRETCH_SOURCE_TOO_SHORT",
                "time-stretch source is shorter than the requested continuous output",
            ));
        }
        if rate_ppm == 1_000_000 {
            return PcmBuffer::from_frames(input.frames()[..output_frames].to_vec());
        }

        let input_bytes = f64le_bytes(input.frames());
        let output_bytes =
            self.ffmpeg
                .rubberband_f64_stereo_44100(&input_bytes, rate_ppm, output_frames)?;
        if output_bytes.len() % 16 != 0 {
            return Err(Error::new(
                "INVALID_TIME_STRETCH_OUTPUT",
                "time-stretch backend returned incomplete binary64 stereo frames",
            ));
        }
        let frames: Vec<_> = output_bytes
            .chunks_exact(16)
            .map(|frame| {
                [
                    f64::from_le_bytes(frame[..8].try_into().expect("exact chunk")),
                    f64::from_le_bytes(frame[8..].try_into().expect("exact chunk")),
                ]
            })
            .collect();
        if frames.len() != output_frames {
            return Err(Error::new(
                "TIME_STRETCH_OUTPUT_LENGTH_MISMATCH",
                format!(
                    "time-stretch backend returned {} frames instead of {output_frames}",
                    frames.len()
                ),
            ));
        }
        PcmBuffer::from_frames(frames)
    }
}

fn f64le_bytes(frames: &[[f64; 2]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(frames.len() * 16);
    for frame in frames {
        bytes.extend_from_slice(&frame[0].to_le_bytes());
        bytes.extend_from_slice(&frame[1].to_le_bytes());
    }
    bytes
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
