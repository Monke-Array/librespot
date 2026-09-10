use crate::canonical::canonical_json;
use crate::error::{Error, Result};
use crate::render::artifact::ArtifactBackend;
use crate::render::source::CanonicalPcmBackend;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

const ARTIFACT_ENCODE_ARGUMENTS: &[&str] = &[
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
    "-f",
    "flac",
    "pipe:1",
];

const ARTIFACT_DECODE_ARGUMENTS: &[&str] = &[
    "-nostdin",
    "-v",
    "error",
    "-threads",
    "1",
    "-i",
    "pipe:0",
    "-map_metadata",
    "-1",
    "-vn",
    "-sn",
    "-dn",
    "-ac",
    "2",
    "-ar",
    "44100",
    "-c:a",
    "pcm_s24le",
    "-f",
    "s24le",
    "pipe:1",
];

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

    pub(crate) fn rubberband_f64_stereo_44100(
        &self,
        input: &[u8],
        rate_ppm: i64,
        output_frames: usize,
    ) -> Result<Vec<u8>> {
        let arguments = Self::rubberband_arguments(rate_ppm, output_frames);
        let child = Command::new(&self.executable)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| {
                Error::new(
                    "FFMPEG_SPAWN_FAILED",
                    "unable to start time-stretch backend",
                )
            })?;
        let output = write_input_while_collecting_output(
            child,
            input,
            "time-stretch stdin is unavailable",
            "unable to write time-stretch input",
            "unable to wait for time-stretch backend",
        )?;
        if !output.status.success() {
            return Err(Error::new(
                "TIME_STRETCH_BACKEND_FAILED",
                "time-stretch backend returned failure",
            ));
        }
        Ok(output.stdout)
    }

    pub(crate) fn rubberband_arguments(rate_ppm: i64, output_frames: usize) -> Vec<String> {
        let rate = format!("{}.{:06}", rate_ppm / 1_000_000, rate_ppm % 1_000_000);
        let filter = format!(
            "rubberband=tempo={rate}:pitch=1.000000:transients=mixed:detector=compound:phase=laminar:window=standard:smoothing=off:formant=shifted:pitchq=quality:channels=together,atrim=start_sample=0:end_sample={output_frames},asetpts=N/SR/TB"
        );
        [
            "-nostdin",
            "-v",
            "error",
            "-threads",
            "1",
            "-filter_threads",
            "1",
            "-f",
            "f64le",
            "-ac",
            "2",
            "-ar",
            "44100",
            "-i",
            "pipe:0",
            "-af",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain(std::iter::once(filter))
        .chain(
            [
                "-map_metadata",
                "-1",
                "-ac",
                "2",
                "-ar",
                "44100",
                "-f",
                "f64le",
                "pipe:1",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .collect()
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

    fn private_decode_program_text(&self, path: &Path) -> Result<Option<String>> {
        let mut arguments = self.decode_arguments(path);
        arguments[6] = "${SOURCE_PATH}".to_owned();
        Ok(Some(String::from_utf8(canonical_json(&arguments)?).expect(
            "canonical JSON produced from FFmpeg arguments is UTF-8",
        )))
    }
}

impl ArtifactBackend for FfmpegBackend {
    fn encode_pcm24_flac_bytes(&self, pcm24: &[u8]) -> Result<Vec<u8>> {
        if pcm24.len() % 6 != 0 {
            return Err(Error::new(
                "INVALID_PCM24_BYTE_LENGTH",
                "stereo PCM24 must contain complete frames",
            ));
        }
        pipe_bytes(
            &self.executable,
            ARTIFACT_ENCODE_ARGUMENTS,
            pcm24,
            "FFMPEG_ENCODE_FAILED",
        )
    }

    fn decode_flac_pcm24_bytes(&self, encoded: &[u8]) -> Result<Vec<u8>> {
        pipe_bytes(
            &self.executable,
            ARTIFACT_DECODE_ARGUMENTS,
            encoded,
            "FFMPEG_DECODE_FAILED",
        )
    }

    fn private_encode_program_text(&self) -> Result<Option<String>> {
        Ok(Some(program_text(ARTIFACT_ENCODE_ARGUMENTS)?))
    }

    fn private_artifact_decode_program_text(&self) -> Result<Option<String>> {
        Ok(Some(program_text(ARTIFACT_DECODE_ARGUMENTS)?))
    }
}

fn program_text(arguments: &[&str]) -> Result<String> {
    Ok(String::from_utf8(canonical_json(&arguments)?)
        .expect("canonical JSON produced from ASCII FFmpeg arguments is UTF-8"))
}

fn pipe_bytes(
    executable: &Path,
    arguments: &[&str],
    input: &[u8],
    failure_code: &'static str,
) -> Result<Vec<u8>> {
    let child = Command::new(executable)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| Error::new("FFMPEG_SPAWN_FAILED", "unable to start artifact backend"))?;
    let output = write_input_while_collecting_output(
        child,
        input,
        "artifact backend stdin is unavailable",
        "unable to write artifact backend input",
        "unable to wait for artifact backend",
    )?;
    if !output.status.success() {
        return Err(Error::new(
            failure_code,
            "artifact backend returned failure",
        ));
    }
    Ok(output.stdout)
}

fn write_input_while_collecting_output(
    mut child: Child,
    input: &[u8],
    missing_stdin_message: &'static str,
    write_message: &'static str,
    wait_message: &'static str,
) -> Result<Output> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::new("FFMPEG_STDIN_FAILED", missing_stdin_message))?;
    std::thread::scope(|scope| {
        // FFmpeg may produce more than an OS pipe buffer before it consumes all
        // input. Feed stdin on a scoped thread while wait_with_output drains
        // stdout/stderr, avoiding a bidirectional-pipe deadlock on real tracks.
        let writer = scope.spawn(move || stdin.write_all(input));
        let output = child.wait_with_output();
        let write_result = writer
            .join()
            .map_err(|_| Error::new("FFMPEG_STDIN_FAILED", "artifact input writer panicked"))?;
        write_result.map_err(|_| Error::new("FFMPEG_STDIN_FAILED", write_message))?;
        output.map_err(|_| Error::new("FFMPEG_WAIT_FAILED", wait_message))
    })
}
