use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use transition_operator::{
    ArtifactFormat, CanonicalPcmBackend, ExpectedSource, FfmpegBackend, OperatorPlan, PcmBuffer,
    PlanValidator, PrivateManifestLocator, RenderRequest, RendererEnvironment, SourceLocator,
    TemplateFeatureView, ValidationContext, candidate_id, canonical_json, render_identity,
    resolve_render_sources, validate_render_request, validate_render_request_bytes,
};

const SAFE_RAW: &[u8] = include_bytes!("fixtures/plans/safe-crossfade.json");

struct Features(String);

impl TemplateFeatureView for Features {
    fn snapshot_sha256(&self) -> &str {
        &self.0
    }

    fn covers_plan_window(&self, _: &transition_operator::OperatorPlanBody) -> bool {
        true
    }

    fn template_is_applicable(&self, _: &transition_operator::OperatorPlanBody) -> bool {
        true
    }

    fn recipe_matches(&self, _: &transition_operator::OperatorPlanBody) -> bool {
        true
    }
}

fn fixture() -> OperatorPlan {
    transition_operator::require_canonical_json(SAFE_RAW.strip_suffix(b"\n").unwrap_or(SAFE_RAW))
        .unwrap()
}

fn validated_plan() -> transition_operator::ValidatedPlan {
    let plan = fixture();
    let features = Features(plan.feature_snapshot.sha256.clone());
    let context = ValidationContext {
        outgoing: ExpectedSource::from_ref(&plan.sources.outgoing),
        incoming: ExpectedSource::from_ref(&plan.sources.incoming),
        features: &features,
    };
    PlanValidator::validate_bytes(SAFE_RAW.strip_suffix(b"\n").unwrap_or(SAFE_RAW), &context)
        .unwrap()
}

fn finalized_body(plan: &OperatorPlan) -> transition_operator::ValidatedPlan {
    let features = Features(plan.feature_snapshot.sha256.clone());
    let context = ValidationContext {
        outgoing: ExpectedSource::from_ref(&plan.sources.outgoing),
        incoming: ExpectedSource::from_ref(&plan.sources.incoming),
        features: &features,
    };
    transition_operator::finalize_plan(PlanValidator::validate_body(plan.body(), &context).unwrap())
        .unwrap()
}

fn request(plan: &transition_operator::ValidatedPlan) -> RenderRequest {
    RenderRequest {
        schema_version: "transition-render-request/1".into(),
        plan_sha256: plan.plan_hash().hex(),
        candidate_id: candidate_id(plan.plan_hash()).as_str().to_owned(),
        output_start_frame: -220_500,
        output_end_frame: 44_100,
        source_locator_manifest_sha256: "a".repeat(64),
        renderer_capability_profile_sha256: "b".repeat(64),
        artifact_format: ArtifactFormat::FlacPcm24Stereo44100V1,
        processing_warm_up_frames: 4_096,
    }
}

fn environment() -> RendererEnvironment {
    RendererEnvironment {
        schema_version: "transition-renderer-environment/1".into(),
        renderer_id: "transition-operator-reference".into(),
        renderer_version: "1.0.0".into(),
        renderer_source_revision: "0123456789abcdef".into(),
        os: "windows".into(),
        architecture: "x86_64".into(),
        ffmpeg_executable: "ffmpeg".into(),
        ffmpeg_version: "9.0.1".into(),
        codec_versions: vec!["flac=1.4.3".into()],
        time_stretch_backend: "rubberband-4.0.0".into(),
        process_flags: vec!["ffmpeg_threads=1".into(), "filter_threads=1".into()],
        locale: "C".into(),
        rounding_mode: "nearest_ties_to_even_binary64".into(),
        decode_threads: 1,
        encode_threads: 1,
        capability_profile_sha256: "b".repeat(64),
        encoder_configuration: "flac_pcm24_no_metadata_v1".into(),
    }
}

#[test]
fn source_pcm_ingestion_is_exact_s16le_stereo_and_hashes_complete_bytes() {
    let bytes = [0x00, 0x80, 0xff, 0x7f, 0x00, 0x40, 0x00, 0xc0];
    let pcm = PcmBuffer::from_s16le_stereo_44100(&bytes).unwrap();
    assert_eq!(pcm.frame_count(), 2);
    assert_eq!(pcm.frame(0), [-1.0, 32_767.0 / 32_768.0]);
    assert_eq!(pcm.frame(1), [0.5, -0.5]);
    assert_eq!(pcm.pcm_sha256(), hex(&Sha256::digest(bytes)));
    assert_eq!(
        PcmBuffer::from_s16le_stereo_44100(&bytes[..7])
            .unwrap_err()
            .code(),
        "INVALID_PCM_BYTE_LENGTH"
    );
}

#[test]
fn render_request_is_canonical_bound_to_plan_and_covers_the_effect() {
    let plan = validated_plan();
    let request = request(&plan);
    let validated = validate_render_request(request.clone(), &plan).unwrap();
    let bytes = canonical_json(&request).unwrap();
    assert!(
        std::str::from_utf8(&bytes)
            .unwrap()
            .contains("\"artifact_format\":\"flac_pcm24_stereo_44100_v1\"")
    );
    let reparsed = validate_render_request_bytes(&bytes, &plan).unwrap();
    assert_eq!(validated.canonical_bytes(), reparsed.canonical_bytes());
    assert_eq!(validated.request_hash(), reparsed.request_hash());
    let mut unknown: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    unknown["backend_filter"] = serde_json::json!("untrusted syntax");
    let unknown = canonical_json(&unknown).unwrap();
    assert!(validate_render_request_bytes(&unknown, &plan).is_err());

    let mut wrong = request.clone();
    wrong.processing_warm_up_frames = 4_095;
    assert_eq!(
        validate_render_request(wrong, &plan).unwrap_err().code(),
        "INVALID_RENDER_WARM_UP"
    );
    let mut truncated = request.clone();
    truncated.output_end_frame = -1;
    assert_eq!(
        validate_render_request(truncated, &plan)
            .unwrap_err()
            .code(),
        "INCOMPLETE_RENDER_WINDOW"
    );
    let mut late = request;
    late.output_start_frame = -220_499;
    assert_eq!(
        validate_render_request(late, &plan).unwrap_err().code(),
        "INCOMPLETE_RENDER_WINDOW"
    );
}

#[test]
fn render_identity_has_exact_domain_bytes_and_excludes_absolute_source_paths() {
    let plan = validated_plan();
    let request = validate_render_request(request(&plan), &plan).unwrap();
    let environment = environment();
    let first = PrivateManifestLocator::new(
        "a".repeat(64),
        BTreeMap::from([(
            plan.plan().sources.outgoing.track_id.clone(),
            PathBuf::from(r"C:\private\one.mp3"),
        )]),
    )
    .unwrap();
    let second = PrivateManifestLocator::new(
        "a".repeat(64),
        BTreeMap::from([(
            plan.plan().sources.outgoing.track_id.clone(),
            PathBuf::from(r"D:\different\private\two.flac"),
        )]),
    )
    .unwrap();
    assert_eq!(first.manifest_sha256(), second.manifest_sha256());
    let identity = render_identity(&plan, &request, &environment).unwrap();
    let mut expected = Sha256::new();
    expected.update(b"transition-render/1\0");
    expected.update(plan.plan_hash().as_bytes());
    expected.update(request.request_hash());
    expected.update(environment.sha256().unwrap().as_bytes());
    assert_eq!(
        identity.as_str(),
        format!("render1-{}", hex(&expected.finalize()))
    );
    assert_eq!(
        identity.as_str(),
        "render1-9ec0e842c41acfd5c42c79b58624afe3542926f6edaa91656c606c04d362d918"
    );
    assert!(
        !std::str::from_utf8(request.canonical_bytes())
            .unwrap()
            .contains("private")
    );
}

struct CountingLocator {
    calls: Cell<usize>,
    manifest: String,
    path: Option<PathBuf>,
}

impl SourceLocator for CountingLocator {
    fn manifest_sha256(&self) -> &str {
        &self.manifest
    }

    fn resolve(&self, _: &transition_operator::SourceRef) -> Option<PathBuf> {
        self.calls.set(self.calls.get() + 1);
        self.path.clone()
    }
}

struct CountingBackend {
    calls: Cell<usize>,
    bytes: Vec<u8>,
}

impl CanonicalPcmBackend for CountingBackend {
    fn decode_s16le_stereo_44100(&self, _: &Path) -> transition_operator::Result<Vec<u8>> {
        self.calls.set(self.calls.get() + 1);
        Ok(self.bytes.clone())
    }
}

#[test]
fn invalid_plan_bytes_cannot_resolve_sources_or_invoke_a_backend() {
    let plan = fixture();
    let features = Features(plan.feature_snapshot.sha256.clone());
    let context = ValidationContext {
        outgoing: ExpectedSource::from_ref(&plan.sources.outgoing),
        incoming: ExpectedSource::from_ref(&plan.sources.incoming),
        features: &features,
    };
    let locator = CountingLocator {
        calls: Cell::new(0),
        manifest: "a".repeat(64),
        path: Some(PathBuf::from("never-used")),
    };
    let backend = CountingBackend {
        calls: Cell::new(0),
        bytes: Vec::new(),
    };
    assert!(resolve_render_sources(b"{}", b"{}", &context, &locator, &backend).is_err());
    assert_eq!(locator.calls.get(), 0);
    assert_eq!(backend.calls.get(), 0);
}

#[test]
fn source_resolution_rejects_missing_and_hash_mismatched_content() {
    let plan = validated_plan();
    let fixture_plan = plan.plan();
    let features = Features(fixture_plan.feature_snapshot.sha256.clone());
    let context = ValidationContext {
        outgoing: ExpectedSource::from_ref(&fixture_plan.sources.outgoing),
        incoming: ExpectedSource::from_ref(&fixture_plan.sources.incoming),
        features: &features,
    };
    let request_bytes = canonical_json(&request(&plan)).unwrap();
    let missing = CountingLocator {
        calls: Cell::new(0),
        manifest: "a".repeat(64),
        path: None,
    };
    let backend = CountingBackend {
        calls: Cell::new(0),
        bytes: vec![0; 4],
    };
    assert_eq!(
        resolve_render_sources(
            plan.canonical_bytes(),
            &request_bytes,
            &context,
            &missing,
            &backend,
        )
        .unwrap_err()
        .code(),
        "MISSING_SOURCE"
    );
    assert_eq!(backend.calls.get(), 0);

    let present = CountingLocator {
        calls: Cell::new(0),
        manifest: "a".repeat(64),
        path: Some(PathBuf::from("opaque-input")),
    };
    assert_eq!(
        resolve_render_sources(
            plan.canonical_bytes(),
            &request_bytes,
            &context,
            &present,
            &backend,
        )
        .unwrap_err()
        .code(),
        "SOURCE_PCM_HASH_MISMATCH"
    );
}

#[test]
fn source_resolution_checks_complete_pcm_hash_before_declared_frame_count() {
    let bytes = vec![0_u8; 4];
    let mut body = fixture();
    let pcm_hash = hex(&Sha256::digest(&bytes));
    body.sources.outgoing.pcm_sha256 = pcm_hash.clone();
    body.sources.incoming.pcm_sha256 = pcm_hash;
    let plan = finalized_body(&body);
    let features = Features(body.feature_snapshot.sha256.clone());
    let context = ValidationContext {
        outgoing: ExpectedSource::from_ref(&body.sources.outgoing),
        incoming: ExpectedSource::from_ref(&body.sources.incoming),
        features: &features,
    };
    let request_bytes = canonical_json(&request(&plan)).unwrap();
    let locator = CountingLocator {
        calls: Cell::new(0),
        manifest: "a".repeat(64),
        path: Some(PathBuf::from("opaque-input")),
    };
    let backend = CountingBackend {
        calls: Cell::new(0),
        bytes,
    };
    assert_eq!(
        resolve_render_sources(
            plan.canonical_bytes(),
            &request_bytes,
            &context,
            &locator,
            &backend,
        )
        .unwrap_err()
        .code(),
        "SOURCE_PCM_FRAME_COUNT_MISMATCH"
    );
}

#[test]
fn ffmpeg_backend_uses_the_frozen_canonical_profiles() {
    let backend = FfmpegBackend::new("ffmpeg");
    assert_eq!(
        backend.decode_arguments(Path::new("input with spaces.mp3")),
        [
            "-nostdin",
            "-v",
            "error",
            "-threads",
            "1",
            "-i",
            "input with spaces.mp3",
            "-map_metadata",
            "-1",
            "-vn",
            "-sn",
            "-dn",
            "-ac",
            "2",
            "-ar",
            "44100",
            "-sample_fmt",
            "s16",
            "-f",
            "s16le",
            "pipe:1",
        ]
    );
    assert!(backend.decode_arguments(Path::new("literal;not-shell"))[6].contains(';'));
}

#[test]
fn source_ffmpeg_decodes_a_synthetic_wav_to_exact_canonical_pcm() {
    let canonical = [0x00, 0x80, 0xff, 0x7f, 0x00, 0x40, 0x00, 0xc0];
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("synthetic input.wav");
    fs::write(&input, pcm16_wav(&canonical)).unwrap();
    let decoded = FfmpegBackend::new("ffmpeg")
        .decode_s16le_stereo_44100(&input)
        .unwrap();
    assert_eq!(decoded, canonical);
}

#[test]
fn source_ffmpeg_encodes_complete_synthetic_pcm24_frames_to_flac() {
    let pcm24 = [0, 0, 0, 0xff, 0xff, 0x7f].repeat(4_096);
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("synthetic.flac");
    FfmpegBackend::new("ffmpeg")
        .encode_pcm24_flac(&pcm24, &output)
        .unwrap();
    assert!(fs::metadata(output).unwrap().len() > 0);
}

fn pcm16_wav(pcm: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(44 + pcm.len());
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36_u32 + pcm.len() as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&44_100_u32.to_le_bytes());
    bytes.extend_from_slice(&(44_100_u32 * 4).to_le_bytes());
    bytes.extend_from_slice(&4_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    bytes.extend_from_slice(pcm);
    bytes
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
