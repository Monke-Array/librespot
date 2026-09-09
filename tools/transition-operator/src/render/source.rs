use crate::dsp::PcmBuffer;
use crate::error::{Error, Result};
use crate::model::SourceRef;
use crate::render::validate_render_request_bytes;
use crate::validation::{PlanValidator, ValidationContext};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub trait SourceLocator {
    fn manifest_sha256(&self) -> &str;
    fn resolve(&self, source: &SourceRef) -> Option<PathBuf>;
}

#[derive(Clone, Debug)]
pub struct PrivateManifestLocator {
    manifest_sha256: String,
    by_track_id: BTreeMap<String, PathBuf>,
}

impl PrivateManifestLocator {
    pub fn new(manifest_sha256: String, by_track_id: BTreeMap<String, PathBuf>) -> Result<Self> {
        if !is_hash(&manifest_sha256) {
            return Err(Error::new(
                "INVALID_SOURCE_MANIFEST_HASH",
                "source locator manifest hash must be lowercase sha256",
            ));
        }
        Ok(Self {
            manifest_sha256,
            by_track_id,
        })
    }
}

impl SourceLocator for PrivateManifestLocator {
    fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    fn resolve(&self, source: &SourceRef) -> Option<PathBuf> {
        self.by_track_id.get(&source.track_id).cloned()
    }
}

pub trait CanonicalPcmBackend {
    fn decode_s16le_stereo_44100(&self, path: &Path) -> Result<Vec<u8>>;
}

#[derive(Clone, Debug)]
pub struct ResolvedSources {
    pub outgoing: PcmBuffer,
    pub incoming: PcmBuffer,
}

pub fn resolve_render_sources(
    plan_bytes: &[u8],
    request_bytes: &[u8],
    context: &ValidationContext<'_>,
    locator: &impl SourceLocator,
    backend: &impl CanonicalPcmBackend,
) -> Result<ResolvedSources> {
    // Validation is deliberately first: malformed bytes have no filesystem or
    // process side effects.
    let plan = PlanValidator::validate_bytes(plan_bytes, context)?;
    let request = validate_render_request_bytes(request_bytes, &plan)?;
    if request.request().source_locator_manifest_sha256 != locator.manifest_sha256() {
        return Err(Error::new(
            "SOURCE_MANIFEST_MISMATCH",
            "request and source locator manifest identities differ",
        ));
    }
    let outgoing = resolve_one(&plan.plan().sources.outgoing, locator, backend)?;
    let incoming = resolve_one(&plan.plan().sources.incoming, locator, backend)?;
    Ok(ResolvedSources { outgoing, incoming })
}

fn resolve_one(
    source: &SourceRef,
    locator: &impl SourceLocator,
    backend: &impl CanonicalPcmBackend,
) -> Result<PcmBuffer> {
    let path = locator
        .resolve(source)
        .ok_or_else(|| Error::new("MISSING_SOURCE", "source is absent from private locator"))?;
    let bytes = backend.decode_s16le_stereo_44100(&path)?;
    let pcm = PcmBuffer::from_s16le_stereo_44100(&bytes)?;
    if pcm.source_pcm_sha256() != Some(source.pcm_sha256.as_str()) {
        return Err(Error::new(
            "SOURCE_PCM_HASH_MISMATCH",
            "decoded source PCM does not match declared identity",
        ));
    }
    if i64::try_from(pcm.frame_count()).ok() != Some(source.pcm_frame_count) {
        return Err(Error::new(
            "SOURCE_PCM_FRAME_COUNT_MISMATCH",
            "decoded source PCM frame count does not match declared identity",
        ));
    }
    Ok(pcm)
}

fn is_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
