use crate::error::{Error, Result};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug)]
pub struct PcmBuffer {
    frames: Vec<[f64; 2]>,
    source_pcm_sha256: Option<String>,
}

impl PcmBuffer {
    pub fn from_frames(frames: Vec<[f64; 2]>) -> Result<Self> {
        if frames.iter().flatten().any(|sample| !sample.is_finite()) {
            return Err(Error::new(
                "NON_FINITE_PCM",
                "PCM buffers may contain only finite binary64 samples",
            ));
        }
        Ok(Self {
            frames,
            // Internal DSP buffers are not canonical source identities. Only
            // s16le ingestion populates this field.
            source_pcm_sha256: None,
        })
    }

    pub fn from_s16le_stereo_44100(bytes: &[u8]) -> Result<Self> {
        if bytes.len() % 4 != 0 {
            return Err(Error::new(
                "INVALID_PCM_BYTE_LENGTH",
                "canonical stereo s16le PCM must contain complete frames",
            ));
        }
        let frames = bytes
            .chunks_exact(4)
            .map(|frame| {
                let left = i16::from_le_bytes([frame[0], frame[1]]);
                let right = i16::from_le_bytes([frame[2], frame[3]]);
                [f64::from(left) / 32_768.0, f64::from(right) / 32_768.0]
            })
            .collect();
        Ok(Self {
            frames,
            source_pcm_sha256: Some(hex(&Sha256::digest(bytes))),
        })
    }

    pub fn from_s24le_stereo_44100(bytes: &[u8]) -> Result<Self> {
        if bytes.len() % 6 != 0 {
            return Err(Error::new(
                "INVALID_PCM24_BYTE_LENGTH",
                "canonical stereo s24le PCM must contain complete frames",
            ));
        }
        let frames = bytes
            .chunks_exact(6)
            .map(|frame| {
                let left = i32::from_le_bytes([
                    frame[0],
                    frame[1],
                    frame[2],
                    if frame[2] & 0x80 == 0 { 0 } else { 0xff },
                ]);
                let right = i32::from_le_bytes([
                    frame[3],
                    frame[4],
                    frame[5],
                    if frame[5] & 0x80 == 0 { 0 } else { 0xff },
                ]);
                [left as f64 / 8_388_607.0, right as f64 / 8_388_607.0]
            })
            .collect();
        Ok(Self {
            frames,
            source_pcm_sha256: None,
        })
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn frame(&self, index: usize) -> [f64; 2] {
        self.frames[index]
    }

    pub fn frames(&self) -> &[[f64; 2]] {
        &self.frames
    }

    pub(crate) fn frames_mut(&mut self) -> &mut [[f64; 2]] {
        &mut self.frames
    }

    pub fn source_pcm_sha256(&self) -> Option<&str> {
        self.source_pcm_sha256.as_deref()
    }

    pub(crate) fn slice(&self, start: usize, end: usize) -> Result<Self> {
        let frames = self.frames.get(start..end).ok_or_else(|| {
            Error::new(
                "DSP_SOURCE_WINDOW_MISSING",
                "PCM slice is outside the available source window",
            )
        })?;
        Self::from_frames(frames.to_vec())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
