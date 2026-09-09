use crate::dsp::SafetyMeasurements;
use crate::error::Result;
use crate::render::{ArtifactFormat, CanonicalPcmBackend};
use serde::{Deserialize, Serialize};

pub trait ArtifactBackend: CanonicalPcmBackend {
    fn encode_pcm24_flac_bytes(&self, pcm24: &[u8]) -> Result<Vec<u8>>;
    fn decode_flac_pcm24_bytes(&self, encoded: &[u8]) -> Result<Vec<u8>>;

    fn private_encode_program_text(&self) -> Result<Option<String>> {
        Ok(None)
    }

    fn private_artifact_decode_program_text(&self) -> Result<Option<String>> {
        Ok(None)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeakMeasurements {
    pub sample_peak: f64,
    pub true_peak: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenderMeasurements {
    pub outgoing_input: PeakMeasurements,
    pub incoming_input: PeakMeasurements,
    pub summed_bus_sample_peak: f64,
    pub post_pair_gain_sample_peak: f64,
    pub predicted_limiter_demand_mdb: i64,
    pub post_limiter: SafetyMeasurements,
    pub decoded_artifact: PeakMeasurements,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRecord {
    pub format: ArtifactFormat,
    pub frame_count: i64,
    pub pre_encode_pcm_sha256: String,
    pub decoded_pcm_sha256: String,
    pub container_sha256: String,
    #[serde(skip)]
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateRenderProvenance {
    pub stage_trace: Vec<String>,
    pub backend_programs: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderQcStatus {
    Accepted,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenderRecord {
    pub schema_version: String,
    pub render_id: String,
    pub candidate_id: String,
    pub plan_sha256: String,
    pub render_request_sha256: String,
    pub renderer_environment_sha256: String,
    pub render_program_sha256: String,
    pub qc_status: RenderQcStatus,
    pub measurements: RenderMeasurements,
    pub artifact: ArtifactRecord,
    pub private_provenance: PrivateRenderProvenance,
}
