use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use transition_operator::{
    ArtifactBackend, ArtifactFormat, CanonicalPcmBackend, ExpectedSource, OperatorPlan, PcmBuffer,
    PlanValidator, PrivateManifestLocator, ReferenceRenderer, RenderRequest, RendererEnvironment,
    TemplateFeatureView, TimeStretchBackend, TimeStretchCue, ValidationContext, candidate_id,
    finalize_plan, validate_render_request,
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

struct MemoryBackend(Vec<u8>);

impl CanonicalPcmBackend for MemoryBackend {
    fn decode_s16le_stereo_44100(&self, _: &Path) -> transition_operator::Result<Vec<u8>> {
        Ok(self.0.clone())
    }
}

impl ArtifactBackend for MemoryBackend {
    fn encode_pcm24_flac_bytes(&self, pcm24: &[u8]) -> transition_operator::Result<Vec<u8>> {
        Ok(pcm24.to_vec())
    }

    fn decode_flac_pcm24_bytes(&self, encoded: &[u8]) -> transition_operator::Result<Vec<u8>> {
        Ok(encoded.to_vec())
    }
}

struct UnusedStretch;

impl TimeStretchBackend for UnusedStretch {
    fn process(
        &self,
        _: &PcmBuffer,
        _: i64,
        _: usize,
        _: TimeStretchCue,
    ) -> transition_operator::Result<PcmBuffer> {
        panic!("safe_crossfade does not invoke time stretch")
    }
}

#[test]
#[ignore = "non-normative 64-candidate local performance characterization"]
fn renders_sixty_four_synthetic_thirty_six_second_candidates_within_architecture_bounds() {
    const CANDIDATES: usize = 64;
    const OUTPUT_FRAMES: i64 = 1_587_600;
    const SOURCE_FRAMES: usize = 1_600_000;
    let source_bytes = [100_i16.to_le_bytes(), (-100_i16).to_le_bytes()]
        .concat()
        .repeat(SOURCE_FRAMES);
    let source_hash = hex(&Sha256::digest(&source_bytes));
    let mut base: OperatorPlan = transition_operator::require_canonical_json(SAFE_RAW).unwrap();
    for source in [&mut base.sources.outgoing, &mut base.sources.incoming] {
        source.pcm_sha256 = source_hash.clone();
        source.pcm_frame_count = SOURCE_FRAMES as i64;
        source.cue_source_frame = 710_000;
    }
    let locator = PrivateManifestLocator::new(
        "a".repeat(64),
        BTreeMap::from([
            (
                base.sources.outgoing.track_id.clone(),
                PathBuf::from("outgoing"),
            ),
            (
                base.sources.incoming.track_id.clone(),
                PathBuf::from("incoming"),
            ),
        ]),
    )
    .unwrap();
    let renderer =
        ReferenceRenderer::new(MemoryBackend(source_bytes), UnusedStretch, environment()).unwrap();
    let started = Instant::now();
    let mut expected_pcm_hash = None;
    let mut maximum_artifact_bytes = 0usize;
    for index in 0..CANDIDATES {
        let mut body = base.body();
        body.provenance.geometry_id = format!("performance-{index:02}");
        let features = Features(body.feature_snapshot.sha256.clone());
        let context = ValidationContext {
            outgoing: ExpectedSource::from_ref(&body.sources.outgoing),
            incoming: ExpectedSource::from_ref(&body.sources.incoming),
            features: &features,
        };
        let plan = finalize_plan(PlanValidator::validate_body(body, &context).unwrap()).unwrap();
        let request = validate_render_request(
            RenderRequest {
                schema_version: "transition-render-request/1".into(),
                plan_sha256: plan.plan_hash().hex(),
                candidate_id: candidate_id(plan.plan_hash()).as_str().to_owned(),
                output_start_frame: -705_600,
                output_end_frame: 882_000,
                source_locator_manifest_sha256: "a".repeat(64),
                renderer_capability_profile_sha256: "b".repeat(64),
                artifact_format: ArtifactFormat::FlacPcm24Stereo44100V1,
                processing_warm_up_frames: 4_096,
            },
            &plan,
        )
        .unwrap();
        let record = renderer.render(&plan, &request, &locator).unwrap();
        assert_eq!(record.artifact.frame_count, OUTPUT_FRAMES);
        match &expected_pcm_hash {
            Some(expected) => assert_eq!(&record.artifact.decoded_pcm_sha256, expected),
            None => expected_pcm_hash = Some(record.artifact.decoded_pcm_sha256.clone()),
        }
        maximum_artifact_bytes = maximum_artifact_bytes.max(record.artifact.bytes.len());
        drop(record);
    }
    let elapsed = started.elapsed().as_secs_f64();
    let rendered_audio_seconds = CANDIDATES as f64 * OUTPUT_FRAMES as f64 / 44_100.0;
    let render_audio_ratio = elapsed / rendered_audio_seconds;
    let peak_rss_bytes = peak_rss_bytes();
    let report = serde_json::json!({
        "schema_version": "transition-renderer-performance-observation/1",
        "candidate_count": CANDIDATES,
        "frames_per_candidate": OUTPUT_FRAMES,
        "elapsed_seconds": elapsed,
        "render_audio_ratio": render_audio_ratio,
        "peak_rss_bytes": peak_rss_bytes,
        "maximum_artifact_bytes": maximum_artifact_bytes,
        "maximum_live_candidate_artifacts": 1,
        "normative": false
    });
    let report_directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("performance");
    fs::create_dir_all(&report_directory).unwrap();
    fs::write(
        report_directory.join("reference-renderer.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();

    assert!(elapsed <= 300.0, "elapsed {elapsed:.3}s");
    assert!(
        render_audio_ratio <= 0.131,
        "render/audio ratio {render_audio_ratio:.6}"
    );
    assert!(
        peak_rss_bytes <= 512 * 1024 * 1024,
        "peak RSS {peak_rss_bytes}"
    );
    assert!(maximum_artifact_bytes <= 512 * 1024 * 1024);
}

fn environment() -> RendererEnvironment {
    RendererEnvironment {
        schema_version: "transition-renderer-environment/1".into(),
        renderer_id: "transition-operator-reference".into(),
        renderer_version: "1.0.0".into(),
        renderer_source_revision: "0123456789abcdef".into(),
        os: "windows".into(),
        architecture: "x86_64".into(),
        ffmpeg_executable: "in-memory".into(),
        ffmpeg_version: "not-used".into(),
        codec_versions: vec!["mock=1".into()],
        time_stretch_backend: "not-used".into(),
        process_flags: vec!["threads=1".into()],
        locale: "C".into(),
        rounding_mode: "nearest_ties_to_even_binary64".into(),
        decode_threads: 1,
        encode_threads: 1,
        capability_profile_sha256: "b".repeat(64),
        encoder_configuration: "in_memory_pcm24_v1".into(),
    }
}

#[cfg(windows)]
fn peak_rss_bytes() -> usize {
    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(
            process: isize,
            counters: *mut ProcessMemoryCounters,
            size: u32,
        ) -> i32;
    }
    let mut counters = ProcessMemoryCounters {
        cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
        page_fault_count: 0,
        peak_working_set_size: 0,
        working_set_size: 0,
        quota_peak_paged_pool_usage: 0,
        quota_paged_pool_usage: 0,
        quota_peak_non_paged_pool_usage: 0,
        quota_non_paged_pool_usage: 0,
        pagefile_usage: 0,
        peak_pagefile_usage: 0,
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<ProcessMemoryCounters>() as u32,
        )
    };
    assert_ne!(ok, 0, "GetProcessMemoryInfo failed");
    counters.peak_working_set_size
}

#[cfg(not(windows))]
fn peak_rss_bytes() -> usize {
    0
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
