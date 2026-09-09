use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use transition_operator::{
    ArtifactBackend, ArtifactFormat, CanonicalPcmBackend, ExpectedSource, FfmpegBackend,
    GeneratedCandidate, OperatorPlan, PairRenderStatus, PcmBuffer, PlanValidator,
    PrivateManifestLocator, ReferenceRenderer, RenderRequest, RendererEnvironment,
    RubberBandTimeStretch, SourceLocator, TemplateFeatureView, TemplateId, TimeStretchBackend,
    TimeStretchCue, ValidationContext, candidate_id, canonical_json,
    render_candidates_with_fallback, render_identity, resolve_render_sources,
    validate_render_request, validate_render_request_bytes,
};

const SAFE_RAW: &[u8] = include_bytes!("fixtures/plans/safe-crossfade.json");
const ALL_OPERATIONS_RAW: &[u8] = include_bytes!("fixtures/plans/all-operations.json");

#[derive(serde::Deserialize)]
struct RenderSuite {
    processing_warm_up_frames: i64,
    repeat_count: usize,
    valid_plan_variants: Vec<String>,
}

fn render_suite() -> RenderSuite {
    serde_json::from_slice(include_bytes!("fixtures/plans/render-suite.json")).unwrap()
}

fn stage_rank(stage: &str) -> usize {
    [
        "time_map",
        "spectral",
        "dynamics",
        "tail_capture",
        "primary_gain",
        "sum",
        "pair_gain",
        "limiter",
        "measurement",
        "quantization",
        "encode",
        "decode_verify",
    ]
    .iter()
    .position(|prefix| stage == *prefix || stage.starts_with(&format!("{prefix}:")))
    .unwrap()
}

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
    let expected_hash = hex(&Sha256::digest(bytes));
    assert_eq!(pcm.source_pcm_sha256(), Some(expected_hash.as_str()));
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

impl ArtifactBackend for CountingBackend {
    fn encode_pcm24_flac_bytes(&self, pcm24: &[u8]) -> transition_operator::Result<Vec<u8>> {
        let mut encoded = b"mock-flac\0".to_vec();
        encoded.extend_from_slice(pcm24);
        Ok(encoded)
    }

    fn decode_flac_pcm24_bytes(&self, encoded: &[u8]) -> transition_operator::Result<Vec<u8>> {
        Ok(encoded.strip_prefix(b"mock-flac\0").unwrap().to_vec())
    }
}

struct DeterministicStretch;

impl TimeStretchBackend for DeterministicStretch {
    fn process(
        &self,
        input: &PcmBuffer,
        rate_ppm: i64,
        output_frames: usize,
        cue: TimeStretchCue,
    ) -> transition_operator::Result<PcmBuffer> {
        assert!(cue.logical_error_frames(rate_ppm) <= 1);
        let frames = (0..output_frames)
            .map(|index| {
                let source = (index as u128 * rate_ppm as u128 + 500_000) / 1_000_000;
                input.frame(source as usize)
            })
            .collect();
        PcmBuffer::from_frames(frames)
    }
}

struct CorruptingBackend {
    source_bytes: Vec<u8>,
}

impl CanonicalPcmBackend for CorruptingBackend {
    fn decode_s16le_stereo_44100(&self, _: &Path) -> transition_operator::Result<Vec<u8>> {
        Ok(self.source_bytes.clone())
    }
}

impl ArtifactBackend for CorruptingBackend {
    fn encode_pcm24_flac_bytes(&self, pcm24: &[u8]) -> transition_operator::Result<Vec<u8>> {
        Ok(pcm24.to_vec())
    }

    fn decode_flac_pcm24_bytes(&self, encoded: &[u8]) -> transition_operator::Result<Vec<u8>> {
        let mut decoded = encoded.to_vec();
        decoded[0] ^= 1;
        Ok(decoded)
    }
}

#[test]
fn reference_renderer_runs_the_fixed_pipeline_and_verifies_encoded_pcm() {
    let frame_count = 230_000usize;
    let source_bytes = [1_000_i16.to_le_bytes(), (-1_000_i16).to_le_bytes()]
        .concat()
        .repeat(frame_count);
    let source_hash = hex(&Sha256::digest(&source_bytes));
    let mut body = fixture();
    for source in [&mut body.sources.outgoing, &mut body.sources.incoming] {
        source.pcm_sha256 = source_hash.clone();
        source.pcm_frame_count = frame_count as i64;
        source.cue_source_frame = 224_596;
    }
    let plan = finalized_body(&body);
    let mut render_request = request(&plan);
    render_request.output_end_frame = 100;
    let render_request = validate_render_request(render_request, &plan).unwrap();
    let locator = PrivateManifestLocator::new(
        "a".repeat(64),
        BTreeMap::from([
            (
                body.sources.outgoing.track_id.clone(),
                PathBuf::from("outgoing"),
            ),
            (
                body.sources.incoming.track_id.clone(),
                PathBuf::from("incoming"),
            ),
        ]),
    )
    .unwrap();
    let renderer = ReferenceRenderer::new(
        CountingBackend {
            calls: Cell::new(0),
            bytes: source_bytes,
        },
        DeterministicStretch,
        environment(),
    )
    .unwrap();

    let record = renderer.render(&plan, &render_request, &locator).unwrap();
    assert_eq!(record.artifact.frame_count, 220_600);
    assert_eq!(
        record.artifact.decoded_pcm_sha256,
        record.artifact.pre_encode_pcm_sha256
    );
    assert_eq!(
        record.private_provenance.stage_trace,
        [
            "primary_gain:outgoing",
            "primary_gain:incoming",
            "sum",
            "pair_gain",
            "limiter",
            "measurement",
            "quantization",
            "encode",
            "decode_verify",
        ]
    );
}

#[test]
fn reference_renderer_rejects_encode_decode_pcm_mismatch() {
    let frame_count = 230_000usize;
    let source_bytes = [1_000_i16.to_le_bytes(), (-1_000_i16).to_le_bytes()]
        .concat()
        .repeat(frame_count);
    let source_hash = hex(&Sha256::digest(&source_bytes));
    let mut body = fixture();
    for source in [&mut body.sources.outgoing, &mut body.sources.incoming] {
        source.pcm_sha256 = source_hash.clone();
        source.pcm_frame_count = frame_count as i64;
        source.cue_source_frame = 224_596;
    }
    let plan = finalized_body(&body);
    let mut render_request = request(&plan);
    render_request.output_end_frame = 100;
    let render_request = validate_render_request(render_request, &plan).unwrap();
    let locator = PrivateManifestLocator::new(
        "a".repeat(64),
        BTreeMap::from([
            (
                body.sources.outgoing.track_id.clone(),
                PathBuf::from("outgoing"),
            ),
            (
                body.sources.incoming.track_id.clone(),
                PathBuf::from("incoming"),
            ),
        ]),
    )
    .unwrap();
    let renderer = ReferenceRenderer::new(
        CorruptingBackend { source_bytes },
        DeterministicStretch,
        environment(),
    )
    .unwrap();
    assert_eq!(
        renderer
            .render(&plan, &render_request, &locator)
            .unwrap_err()
            .code(),
        "ENCODE_DECODE_PCM_MISMATCH"
    );
}

#[test]
fn reference_renderer_rejects_true_peak_instead_of_clamping_or_repairing() {
    let frame_count = 230_000usize;
    let source_bytes = [i16::MAX.to_le_bytes(), i16::MIN.to_le_bytes()]
        .concat()
        .repeat(frame_count);
    let source_hash = hex(&Sha256::digest(&source_bytes));
    let mut body = fixture();
    body.output_safety.pair_output_gain_mdb = 0;
    for source in [&mut body.sources.outgoing, &mut body.sources.incoming] {
        source.pcm_sha256 = source_hash.clone();
        source.pcm_frame_count = frame_count as i64;
        source.cue_source_frame = 224_596;
    }
    let plan = finalized_body(&body);
    let mut request_body = request(&plan);
    request_body.output_end_frame = 100;
    let request = validate_render_request(request_body, &plan).unwrap();
    let locator = PrivateManifestLocator::new(
        "a".repeat(64),
        BTreeMap::from([
            (
                body.sources.outgoing.track_id.clone(),
                PathBuf::from("outgoing"),
            ),
            (
                body.sources.incoming.track_id.clone(),
                PathBuf::from("incoming"),
            ),
        ]),
    )
    .unwrap();
    let renderer = ReferenceRenderer::new(
        CountingBackend {
            calls: Cell::new(0),
            bytes: source_bytes,
        },
        DeterministicStretch,
        environment(),
    )
    .unwrap();
    assert_eq!(
        renderer
            .render(&plan, &request, &locator)
            .unwrap_err()
            .code(),
        "TRUE_PEAK_LIMIT_EXCEEDED"
    );
}

#[test]
fn approved_operation_subsets_render_in_normative_order_with_frozen_warm_up_and_repeatability() {
    let suite = render_suite();
    let frame_count = 400_000usize;
    let source_bytes = [100_i16.to_le_bytes(), (-100_i16).to_le_bytes()]
        .concat()
        .repeat(frame_count);
    let source_hash = hex(&Sha256::digest(&source_bytes));
    let mut body: OperatorPlan =
        transition_operator::require_canonical_json(ALL_OPERATIONS_RAW).unwrap();
    for source in [&mut body.sources.outgoing, &mut body.sources.incoming] {
        source.pcm_sha256 = source_hash.clone();
        source.pcm_frame_count = frame_count as i64;
        source.cue_source_frame = 230_000;
    }
    let locator = PrivateManifestLocator::new(
        "a".repeat(64),
        BTreeMap::from([
            (
                body.sources.outgoing.track_id.clone(),
                PathBuf::from("outgoing"),
            ),
            (
                body.sources.incoming.track_id.clone(),
                PathBuf::from("incoming"),
            ),
        ]),
    )
    .unwrap();
    let renderer = ReferenceRenderer::new(
        CountingBackend {
            calls: Cell::new(0),
            bytes: source_bytes,
        },
        DeterministicStretch,
        environment(),
    )
    .unwrap();

    for variant in &suite.valid_plan_variants {
        let mut variant_body = body.clone();
        variant_body.timeline.effect_end_frame = 0;
        variant_body.template.recipe_id = format!("renderer_{variant}");
        variant_body.operations.retain(|operation| {
            matches!(
                operation,
                transition_operator::Operation::TimeMap(_)
                    | transition_operator::Operation::GainEnvelope(_)
            ) || matches!(
                (variant.as_str(), operation),
                ("filter", transition_operator::Operation::FilterEnvelope(_))
                    | (
                        "crossover",
                        transition_operator::Operation::CrossoverBandGain(_)
                    )
                    | ("duck", transition_operator::Operation::DuckEnvelope(_))
                    | (
                        "tail",
                        transition_operator::Operation::FeedforwardDelayTail(_)
                    )
                    | ("gate", transition_operator::Operation::RhythmicGate(_))
            )
        });
        match variant.as_str() {
            "filter" => variant_body.template.id = "spectral_handoff".into(),
            "crossover" => {
                variant_body.template.id = "bass_handoff".into();
                let incoming = variant_body
                    .operations
                    .iter()
                    .find_map(|operation| match operation {
                        transition_operator::Operation::CrossoverBandGain(value) => {
                            Some(value.clone())
                        }
                        _ => None,
                    })
                    .unwrap();
                let mut outgoing = incoming;
                outgoing.target = transition_operator::Target::Outgoing;
                outgoing.op_id = "outgoing.crossover".into();
                for point in &mut outgoing.band_gain_envelopes[0].points {
                    point.value_ppm = 1_000_000 - point.value_ppm;
                }
                variant_body.operations.insert(
                    1,
                    transition_operator::Operation::CrossoverBandGain(outgoing),
                );
            }
            "duck" => {
                variant_body.template.id = "ducked_overlap".into();
                let duck = variant_body
                    .operations
                    .iter_mut()
                    .find_map(|operation| match operation {
                        transition_operator::Operation::DuckEnvelope(value) => Some(value),
                        _ => None,
                    })
                    .unwrap();
                duck.points[0].frame = -60_882;
                duck.points[1].frame = -60_000;
                duck.points[2].frame = -59_118;
            }
            "tail" => {
                variant_body.template.id = "echo_tail_handoff".into();
                variant_body.timeline.effect_end_frame = 17_640;
            }
            "gate" => {
                variant_body.template.id = "rhythmic_handoff".into();
                let gate = variant_body
                    .operations
                    .iter_mut()
                    .find_map(|operation| match operation {
                        transition_operator::Operation::RhythmicGate(value) => Some(value),
                        _ => None,
                    })
                    .unwrap();
                gate.target = transition_operator::Target::Outgoing;
                gate.op_id = "outgoing.gate".into();
            }
            _ => unreachable!(),
        }
        let plan = finalized_body(&variant_body);
        let mut variant_request = request(&plan);
        variant_request.output_start_frame = -88_200;
        let render_request = validate_render_request(variant_request, &plan).unwrap();
        assert_eq!(
            render_request.request().processing_warm_up_frames,
            suite.processing_warm_up_frames
        );
        let records: Vec<_> = (0..suite.repeat_count)
            .map(|_| renderer.render(&plan, &render_request, &locator).unwrap())
            .collect();
        assert!(records.windows(2).all(|pair| {
            pair[0].artifact.pre_encode_pcm_sha256 == pair[1].artifact.pre_encode_pcm_sha256
                && pair[0].artifact.container_sha256 == pair[1].artifact.container_sha256
                && pair[0].measurements.post_limiter.sample_peak
                    == pair[1].measurements.post_limiter.sample_peak
        }));
        let trace = &records[0].private_provenance.stage_trace;
        assert!(
            trace
                .windows(2)
                .all(|pair| stage_rank(&pair[0]) <= stage_rank(&pair[1]))
        );
    }
}

#[test]
fn pair_rendering_renders_fallback_first_and_isolates_rich_failures() {
    let plan = validated_plan();
    let fallback = GeneratedCandidate {
        candidate_id: candidate_id(plan.plan_hash()),
        template_id: TemplateId::SafeCrossfade,
        recipe_id: "fallback".into(),
        plan: plan.clone(),
    };
    let rich = GeneratedCandidate {
        candidate_id: candidate_id(plan.plan_hash()),
        template_id: TemplateId::BeatCut,
        recipe_id: "rich".into(),
        plan,
    };
    let mut calls = Vec::new();
    let rendered =
        render_candidates_with_fallback(&[rich.clone(), fallback.clone()], |candidate| {
            calls.push(candidate.template_id);
            if candidate.template_id == TemplateId::BeatCut {
                PcmBuffer::from_frames(vec![[f64::NAN, 0.0]]).map(|_| "never".to_owned())
            } else {
                Ok("fallback-render".to_owned())
            }
        });
    assert_eq!(calls, [TemplateId::SafeCrossfade, TemplateId::BeatCut]);
    assert_eq!(rendered.status, PairRenderStatus::Accepted);
    assert_eq!(rendered.fallback.as_deref(), Some("fallback-render"));
    assert!(rendered.rich.is_empty());
    assert_eq!(rendered.rejections[0].code, "NON_FINITE_PCM");

    calls.clear();
    let failed = render_candidates_with_fallback(&[rich, fallback], |candidate| {
        calls.push(candidate.template_id);
        transition_operator::measure_loudness(&PcmBuffer::from_frames(Vec::new()).unwrap())
            .map(|_| "never".to_owned())
    });
    assert_eq!(calls, [TemplateId::SafeCrossfade]);
    assert_eq!(failed.status, PairRenderStatus::FallbackRenderFailed);
    assert!(failed.fallback.is_none());
    assert_eq!(failed.rejections[0].code, "FALLBACK_RENDER_FAILED");
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

    let mut permuted: serde_json::Value = serde_json::from_slice(ALL_OPERATIONS_RAW).unwrap();
    permuted["operations"].as_array_mut().unwrap().swap(0, 1);
    let permuted = canonical_json(&permuted).unwrap();
    assert!(resolve_render_sources(&permuted, b"{}", &context, &locator, &backend).is_err());
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

#[test]
fn ffmpeg_artifact_roundtrip_preserves_exact_canonical_pcm24() {
    let pcm24 = [0, 0, 0, 1, 0, 0, 0xff, 0xff, 0x7f, 1, 0, 0x80].repeat(4_096);
    let backend = FfmpegBackend::new("ffmpeg");
    let encoded = backend.encode_pcm24_flac_bytes(&pcm24).unwrap();
    assert!(!encoded.is_empty());
    assert_eq!(backend.decode_flac_pcm24_bytes(&encoded).unwrap(), pcm24);
}

#[test]
fn pinned_ffmpeg_reference_render_is_pcm_and_container_repeatable() {
    let frame_count = 230_000usize;
    let source_bytes = [1_000_i16.to_le_bytes(), (-1_000_i16).to_le_bytes()]
        .concat()
        .repeat(frame_count);
    let source_hash = hex(&Sha256::digest(&source_bytes));
    let mut body = fixture();
    for source in [&mut body.sources.outgoing, &mut body.sources.incoming] {
        source.pcm_sha256 = source_hash.clone();
        source.pcm_frame_count = frame_count as i64;
        source.cue_source_frame = 224_596;
    }
    let plan = finalized_body(&body);
    let mut request_body = request(&plan);
    request_body.output_end_frame = 100;
    let request = validate_render_request(request_body, &plan).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("synthetic.wav");
    fs::write(&source_path, pcm16_wav(&source_bytes)).unwrap();
    let locator = PrivateManifestLocator::new(
        "a".repeat(64),
        BTreeMap::from([
            (body.sources.outgoing.track_id.clone(), source_path.clone()),
            (body.sources.incoming.track_id.clone(), source_path),
        ]),
    )
    .unwrap();
    let renderer = ReferenceRenderer::new(
        FfmpegBackend::new("ffmpeg"),
        RubberBandTimeStretch::new("ffmpeg"),
        environment(),
    )
    .unwrap();
    let records: Vec<_> = (0..3)
        .map(|_| renderer.render(&plan, &request, &locator).unwrap())
        .collect();
    assert!(
        records
            .iter()
            .all(|record| record.artifact.bytes.starts_with(b"fLaC"))
    );
    assert!(records.windows(2).all(|pair| {
        pair[0].artifact.pre_encode_pcm_sha256 == pair[1].artifact.pre_encode_pcm_sha256
            && pair[0].artifact.container_sha256 == pair[1].artifact.container_sha256
            && pair[0].measurements.post_limiter.true_peak
                == pair[1].measurements.post_limiter.true_peak
    }));
    assert!(
        records[0]
            .private_provenance
            .backend_programs
            .iter()
            .all(|program| !program.contains(directory.path().to_string_lossy().as_ref()))
    );

    let alternate_path = directory.path().join("different-name.wav");
    fs::write(&alternate_path, pcm16_wav(&source_bytes)).unwrap();
    let alternate_locator = PrivateManifestLocator::new(
        "a".repeat(64),
        BTreeMap::from([
            (
                body.sources.outgoing.track_id.clone(),
                alternate_path.clone(),
            ),
            (body.sources.incoming.track_id.clone(), alternate_path),
        ]),
    )
    .unwrap();
    let alternate = renderer
        .render(&plan, &request, &alternate_locator)
        .unwrap();
    assert_eq!(
        alternate.render_program_sha256,
        records[0].render_program_sha256
    );
    assert_eq!(
        alternate.artifact.container_sha256,
        records[0].artifact.container_sha256
    );
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
