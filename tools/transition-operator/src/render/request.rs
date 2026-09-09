use crate::canonical::{canonical_json, require_canonical_json};
use crate::error::{Error, Result};
use crate::identity::ValidatedPlan;
use crate::scalar::JSON_SAFE_INTEGER_MAX;
use crate::{RendererEnvironment, candidate_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ArtifactFormat {
    #[serde(rename = "flac_pcm24_stereo_44100_v1")]
    FlacPcm24Stereo44100V1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenderRequest {
    pub schema_version: String,
    pub plan_sha256: String,
    pub candidate_id: String,
    pub output_start_frame: i64,
    pub output_end_frame: i64,
    pub source_locator_manifest_sha256: String,
    pub renderer_capability_profile_sha256: String,
    pub artifact_format: ArtifactFormat,
    pub processing_warm_up_frames: i64,
}

#[derive(Clone, Debug)]
pub struct ValidatedRenderRequest {
    request: RenderRequest,
    canonical_bytes: Vec<u8>,
    request_hash: [u8; 32],
}

impl ValidatedRenderRequest {
    pub fn request(&self) -> &RenderRequest {
        &self.request
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    pub fn request_hash(&self) -> &[u8; 32] {
        &self.request_hash
    }

    pub fn request_hash_hex(&self) -> String {
        hex(&self.request_hash)
    }
}

pub fn validate_render_request(
    request: RenderRequest,
    plan: &ValidatedPlan,
) -> Result<ValidatedRenderRequest> {
    if request.schema_version != "transition-render-request/1" {
        return Err(Error::new(
            "UNSUPPORTED_RENDER_REQUEST",
            "render request schema version is unsupported",
        ));
    }
    if request.plan_sha256 != plan.plan_hash().hex()
        || request.candidate_id != candidate_id(plan.plan_hash()).as_str()
    {
        return Err(Error::new(
            "RENDER_REQUEST_PLAN_MISMATCH",
            "render request does not identify the validated plan",
        ));
    }
    if !is_hash(&request.source_locator_manifest_sha256)
        || !is_hash(&request.renderer_capability_profile_sha256)
    {
        return Err(Error::new(
            "INVALID_RENDER_REQUEST_HASH",
            "render request hashes must be lowercase sha256",
        ));
    }
    if request.processing_warm_up_frames != 4_096 {
        return Err(Error::new(
            "INVALID_RENDER_WARM_UP",
            "v1 render warm-up must be exactly 4096 frames",
        ));
    }
    if request.output_start_frame < -JSON_SAFE_INTEGER_MAX
        || request.output_end_frame > JSON_SAFE_INTEGER_MAX
        || request.output_start_frame >= request.output_end_frame
    {
        return Err(Error::new(
            "INVALID_RENDER_WINDOW",
            "render output interval must be nonempty and interoperable",
        ));
    }
    let timeline = &plan.plan().timeline;
    if request.output_start_frame > timeline.dry_start_frame
        || request.output_end_frame < timeline.effect_end_frame
    {
        return Err(Error::new(
            "INCOMPLETE_RENDER_WINDOW",
            "render output must include the complete transition effect",
        ));
    }
    let canonical_bytes = canonical_json(&request)?;
    let request_hash = Sha256::digest(&canonical_bytes).into();
    Ok(ValidatedRenderRequest {
        request,
        canonical_bytes,
        request_hash,
    })
}

pub fn validate_render_request_bytes(
    bytes: &[u8],
    plan: &ValidatedPlan,
) -> Result<ValidatedRenderRequest> {
    if bytes.len() > 16_384 {
        return Err(Error::new(
            "RENDER_REQUEST_TOO_LARGE",
            "canonical render request exceeds 16384 bytes",
        ));
    }
    let request: RenderRequest = require_canonical_json(bytes)?;
    let validated = validate_render_request(request, plan)?;
    if validated.canonical_bytes() != bytes {
        return Err(Error::new(
            "NON_CANONICAL_JSON",
            "render request bytes are not canonical",
        ));
    }
    Ok(validated)
}

#[derive(Clone, Eq, PartialEq)]
pub struct RenderIdentity(String);

impl RenderIdentity {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RenderIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

pub fn render_identity(
    plan: &ValidatedPlan,
    request: &ValidatedRenderRequest,
    environment: &RendererEnvironment,
) -> Result<RenderIdentity> {
    if request.request().renderer_capability_profile_sha256 != environment.capability_profile_sha256
    {
        return Err(Error::new(
            "RENDERER_CAPABILITY_MISMATCH",
            "request and renderer capability identities differ",
        ));
    }
    let environment_hash = environment.sha256()?;
    let mut digest = Sha256::new();
    digest.update(b"transition-render/1\0");
    digest.update(plan.plan_hash().as_bytes());
    digest.update(request.request_hash());
    digest.update(environment_hash.as_bytes());
    Ok(RenderIdentity(format!(
        "render1-{}",
        hex(&digest.finalize())
    )))
}

fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
