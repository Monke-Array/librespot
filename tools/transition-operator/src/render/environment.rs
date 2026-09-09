use crate::canonical::canonical_json;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RendererEnvironment {
    pub schema_version: String,
    pub renderer_id: String,
    pub renderer_version: String,
    pub renderer_source_revision: String,
    pub os: String,
    pub architecture: String,
    pub ffmpeg_executable: String,
    pub ffmpeg_version: String,
    pub codec_versions: Vec<String>,
    pub time_stretch_backend: String,
    pub process_flags: Vec<String>,
    pub locale: String,
    pub rounding_mode: String,
    pub decode_threads: i64,
    pub encode_threads: i64,
    pub capability_profile_sha256: String,
    pub encoder_configuration: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererEnvironmentHash([u8; 32]);

impl RendererEnvironmentHash {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

impl RendererEnvironment {
    pub fn sha256(&self) -> Result<RendererEnvironmentHash> {
        if self.schema_version != "transition-renderer-environment/1"
            || self.decode_threads != 1
            || self.encode_threads != 1
            || !is_hash(&self.capability_profile_sha256)
            || !all_ascii([
                &self.renderer_id,
                &self.renderer_version,
                &self.renderer_source_revision,
                &self.os,
                &self.architecture,
                &self.ffmpeg_executable,
                &self.ffmpeg_version,
                &self.time_stretch_backend,
                &self.locale,
                &self.rounding_mode,
                &self.encoder_configuration,
            ])
            || !sorted_ascii(&self.codec_versions)
            || !sorted_ascii(&self.process_flags)
        {
            return Err(Error::new(
                "INVALID_RENDERER_ENVIRONMENT",
                "renderer environment is incomplete or noncanonical",
            ));
        }
        Ok(RendererEnvironmentHash(
            Sha256::digest(canonical_json(self)?).into(),
        ))
    }
}

fn sorted_ascii(values: &[String]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1]) && values.iter().all(|value| ascii(value))
}

fn all_ascii<'a>(values: impl IntoIterator<Item = &'a String>) -> bool {
    values.into_iter().all(|value| ascii(value))
}

fn ascii(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
}

fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
