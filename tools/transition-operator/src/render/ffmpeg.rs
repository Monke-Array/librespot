use crate::error::{Error, Result};
use crate::render::source::CanonicalPcmBackend;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug)]
pub struct FfmpegBackend {
    executable: PathBuf,
}

impl FfmpegBackend {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    pub fn decode_arguments(&self, input: &Path) -> Vec<String> {
        vec![
            "-nostdin".into(),
            "-v".into(),
            "error".into(),
            "-threads".into(),
            "1".into(),
            "-i".into(),
            input.to_string_lossy().into_owned(),
            "-map_metadata".into(),
            "-1".into(),
            "-vn".into(),
            "-sn".into(),
            "-dn".into(),
            "-ac".into(),
            "2".into(),
            "-ar".into(),
            "44100".into(),
            "-sample_fmt".into(),
            "s16".into(),
            "-f".into(),
            "s16le".into(),
            "pipe:1".into(),
        ]
    }

    pub fn encode_pcm24_flac(&self, pcm24: &[u8], output: &Path) -> Result<()> {
        if pcm24.len() % 6 != 0 {
            return Err(Error::new(
                "INVALID_PCM24_BYTE_LENGTH",
                "stereo PCM24 must contain complete frames",
            ));
        }
        let mut child = Command::new(&self.executable)
            .args([
                "-nostdin",
                "-v",
                "error",
                "-threads",
                "1",
                "-f",
                "s24le",
                "-ac",
                "2",
                "-ar",
                "44100",
                "-i",
                "pipe:0",
                "-map_metadata",
                "-1",
                "-c:a",
                "flac",
                "-sample_fmt",
                "s32",
                "-bits_per_raw_sample",
                "24",
                "-y",
            ])
            .arg(output)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| Error::new("FFMPEG_SPAWN_FAILED", "unable to start FLAC encoder"))?;
        child
            .stdin
            .take()
            .ok_or_else(|| Error::new("FFMPEG_STDIN_FAILED", "encoder stdin is unavailable"))?
            .write_all(pcm24)
            .map_err(|_| Error::new("FFMPEG_STDIN_FAILED", "unable to write encoder input"))?;
        let output = child
            .wait_with_output()
            .map_err(|_| Error::new("FFMPEG_WAIT_FAILED", "unable to wait for FLAC encoder"))?;
        if !output.status.success() {
            return Err(Error::new(
                "FFMPEG_ENCODE_FAILED",
                "FLAC encoder returned failure",
            ));
        }
        Ok(())
    }
}

impl CanonicalPcmBackend for FfmpegBackend {
    fn decode_s16le_stereo_44100(&self, path: &Path) -> Result<Vec<u8>> {
        let output = Command::new(&self.executable)
            .args(self.decode_arguments(path))
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|_| Error::new("FFMPEG_SPAWN_FAILED", "unable to start source decoder"))?;
        if !output.status.success() {
            return Err(Error::new(
                "FFMPEG_DECODE_FAILED",
                "source decoder returned failure",
            ));
        }
        Ok(output.stdout)
    }
}
