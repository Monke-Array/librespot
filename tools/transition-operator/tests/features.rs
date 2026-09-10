use transition_operator::{
    AnalysisIdentity, CanonicalSource, CueFeature, CueKind, DurationMode, FeatureSnapshotBodyV3,
    FeatureSnapshotV3, FeatureWindow, GenerationConfig, GenerationRequest, PairFeatures, PcmBuffer,
    RhythmFeatures, SourceFeatures, SourceRef, TemplateFeatureView, TemplateId, WindowKind,
    build_feature_snapshot, extract_signal_features, feature_algorithm_sha256,
    finalize_feature_snapshot, generate_candidates, periodicity_confidence_ppm,
    phase_confidence_ppm, propose_geometries, snapshot_sha256, temporal_onset_iou_ppm,
    validate_feature_snapshot,
};

fn hash(byte: char) -> String {
    std::iter::repeat_n(byte, 64).collect()
}

fn snapshot_body() -> FeatureSnapshotBodyV3 {
    let analysis = AnalysisIdentity::finalize(
        "transition-local-features".into(),
        "2.0.0".into(),
        hash('a'),
    )
    .unwrap();
    let outgoing = SourceFeatures::finalize(
        hash('1'),
        2_646_000,
        &analysis,
        vec![CueFeature::draft(2_000_000, CueKind::Outro, 800_000)],
        Some(RhythmFeatures {
            beat_frames: vec![1_900_000, 1_922_050, 1_944_100, 1_966_150, 1_988_200],
            downbeat_frames: vec![1_900_000, 1_988_200],
            meter_beats: Some(4),
            tempo_millibpm: 120_000,
            beat_confidence_ppm: 850_000,
            downbeat_confidence_ppm: 750_000,
        }),
        vec![FeatureWindow::draft(
            WindowKind::OutgoingTransition,
            1_294_400,
            2_264_600,
            None,
            Some(700_000),
            Some(250_000),
            Some(true),
            Some(420_000),
            Some(710_000),
            Some(180_000),
            Some(240_000),
            Some(580_000),
            Some(-11_000),
            Some(1_200),
        )],
        -2_000,
        -1_500,
    )
    .unwrap();
    let incoming = SourceFeatures::finalize(
        hash('2'),
        2_646_000,
        &analysis,
        vec![CueFeature::draft(800_000, CueKind::Intro, 800_000)],
        None,
        vec![FeatureWindow::draft(
            WindowKind::IncomingTransition,
            94_400,
            1_064_600,
            None,
            Some(500_000),
            Some(150_000),
            Some(true),
            Some(350_000),
            Some(730_000),
            Some(230_000),
            Some(250_000),
            Some(520_000),
            Some(-8_000),
            Some(1_100),
        )],
        -3_000,
        -2_500,
    )
    .unwrap();
    FeatureSnapshotBodyV3 {
        schema_version: "transition-feature-snapshot/3".into(),
        analysis,
        pair: PairFeatures {
            outgoing_cue_id: outgoing.cues[0].cue_id.clone(),
            incoming_cue_id: incoming.cues[0].cue_id.clone(),
            outgoing_window_id: outgoing.windows[0].window_id.clone(),
            incoming_window_id: incoming.windows[0].window_id.clone(),
            vocal_collision_ppm: None,
            vocal_collision_span_frames: None,
            vocal_collision_start_frame: None,
            vocal_collision_end_frame: None,
            outgoing_vocal_collision_strength_ppm: None,
            incoming_vocal_collision_strength_ppm: None,
            transient_collision_ppm: Some(450_000),
            transient_collision_span_frames: Some(22_050),
            transient_collision_start_frame: Some(-50_000),
            transient_collision_end_frame: Some(-27_950),
            outgoing_transient_collision_strength_ppm: Some(700_000),
            incoming_transient_collision_strength_ppm: Some(600_000),
            bass_collision_ppm: Some(500_000),
            spectral_overlap_ppm: Some(700_000),
            energy_delta_mdb: Some(3_000),
            alignment_error_ppm_of_beat: Some(20_000),
        },
        outgoing,
        incoming,
    }
}

#[test]
fn snapshot_finalize_validates_hash_and_derives_template_view() {
    let snapshot = finalize_feature_snapshot(snapshot_body()).unwrap();
    validate_feature_snapshot(&snapshot).unwrap();
    assert_eq!(
        snapshot.snapshot_sha256,
        snapshot_sha256(&snapshot).unwrap()
    );
    assert_eq!(
        TemplateFeatureView::snapshot_sha256(&snapshot),
        snapshot.snapshot_sha256
    );
    let inputs = snapshot.template_inputs().unwrap();
    assert_eq!(inputs.outgoing_vocal_activity_ppm, None);
    assert_eq!(inputs.outgoing_bass_occupancy_ppm, Some(420_000));
    assert_eq!(inputs.spectral_overlap_ppm, Some(700_000));

    let mut tampered = snapshot;
    tampered.pair.energy_delta_mdb = Some(3_001);
    assert_eq!(
        validate_feature_snapshot(&tampered).unwrap_err().code(),
        "FEATURE_SNAPSHOT_HASH_MISMATCH"
    );
}

#[test]
fn snapshot_golden_fixture_is_valid_and_stable() {
    let snapshot = finalize_feature_snapshot(snapshot_body()).unwrap();
    assert_eq!(
        snapshot.snapshot_sha256,
        "66f2ed62d7c29cda75822647c8275fcb8e646e2ef9f5bad0355b804d8a619c69"
    );
}

#[test]
fn snapshot_json_is_strict_and_missing_values_are_absent_not_null() {
    let snapshot = finalize_feature_snapshot(snapshot_body()).unwrap();
    let bytes = serde_json::to_vec(&snapshot).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(!text.contains("vocal_activity_ppm"));
    assert!(!text.contains(":null"));

    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("unknown".into(), true.into());
    assert!(serde_json::from_value::<FeatureSnapshotV3>(value).is_err());

    let with_null = String::from_utf8(bytes).unwrap().replacen(
        "\"windows\":[{",
        "\"windows\":[{\"vocal_activity_ppm\":null,",
        1,
    );
    assert_eq!(
        transition_operator::parse_feature_snapshot(with_null.as_bytes())
            .unwrap_err()
            .code(),
        "FEATURE_NULL_FORBIDDEN"
    );
}

#[test]
fn snapshot_rejects_out_of_bounds_window_and_unordered_beats() {
    let mut body = snapshot_body();
    body.outgoing.windows[0].end_frame = body.outgoing.source_frames + 1;
    assert_eq!(
        finalize_feature_snapshot(body).unwrap_err().code(),
        "INVALID_FEATURE_WINDOW"
    );

    let mut body = snapshot_body();
    body.outgoing
        .rhythm
        .as_mut()
        .unwrap()
        .beat_frames
        .swap(0, 1);
    assert_eq!(
        finalize_feature_snapshot(body).unwrap_err().code(),
        "INVALID_RHYTHM_GRID"
    );
}

#[test]
fn pilot_v1_import_does_not_promote_recompute_or_omitted_values() {
    let old = serde_json::json!({
        "track_id": "sanitized-track",
        "duration_ms": 60000,
        "bpm": 120.0,
        "bpm_confidence": 1.0,
        "true_peak_dbfs": -1.0,
        "vocal_probability": null,
        "downbeat_times_ms": [0, 2000, 4000]
    });
    let imported = transition_operator::import_pilot_v1(&old).unwrap();
    assert!(imported.source_identity.is_none());
    assert!(imported.cues.is_empty());
    assert!(imported.rhythm.is_none());
    assert!(imported.vocal_activity_ppm.is_none());
    assert!(imported.whole_source_true_peak_mdbtp.is_none());
}

fn pulse_source(tempo_bpm: i64, phase_frames: usize, bass_hz: f64) -> CanonicalSource {
    pulse_source_with_meter(tempo_bpm, phase_frames, bass_hz, true)
}

fn pulse_source_with_meter(
    tempo_bpm: i64,
    phase_frames: usize,
    bass_hz: f64,
    accent_downbeats: bool,
) -> CanonicalSource {
    let frame_count = 44_100 * 40;
    let beat_frames = (60 * 44_100 / tempo_bpm) as usize;
    let mut frames = Vec::with_capacity(frame_count);
    for frame in 0..frame_count {
        let beat_offset = frame.saturating_sub(phase_frames) % beat_frames;
        let beat_index = frame.saturating_sub(phase_frames) / beat_frames;
        let pulse = if frame >= phase_frames && beat_offset < 220 {
            if accent_downbeats && beat_index % 4 == 0 {
                0.75
            } else {
                0.35
            }
        } else {
            0.0
        };
        let tone = 0.08 * (2.0 * std::f64::consts::PI * bass_hz * frame as f64 / 44_100.0).sin();
        frames.push([pulse + tone, pulse + tone]);
    }
    CanonicalSource::from_pcm(PcmBuffer::from_frames(frames).unwrap()).unwrap()
}

#[test]
fn rhythm_confidence_calibration_is_monotonic_and_keeps_ambiguous_evidence_below_gates() {
    assert_eq!(
        periodicity_confidence_ppm(400_000, 400_000).unwrap(),
        500_000
    );
    assert_eq!(
        periodicity_confidence_ppm(300_000, 200_000).unwrap(),
        750_000
    );
    assert_eq!(
        periodicity_confidence_ppm(700_000, 400_000).unwrap(),
        1_000_000
    );
    assert!(
        periodicity_confidence_ppm(460_000, 400_000).unwrap()
            < periodicity_confidence_ppm(520_000, 400_000).unwrap()
    );

    assert_eq!(phase_confidence_ppm(1_000_000, 0).unwrap(), 500_000);
    assert_eq!(phase_confidence_ppm(1_000_000, 100_000).unwrap(), 750_000);
    assert_eq!(phase_confidence_ppm(1_000_000, 200_000).unwrap(), 1_000_000);
    assert!(
        phase_confidence_ppm(900_000, 40_000).unwrap()
            < phase_confidence_ppm(900_000, 160_000).unwrap()
    );

    let ambiguous =
        extract_signal_features(&pulse_source_with_meter(120, 4_410, 80.0, false)).unwrap();
    let ambiguous = ambiguous
        .rhythm
        .expect("uniform pulses still define a beat grid");
    assert!(ambiguous.beat_confidence_ppm >= 750_000);
    assert!(ambiguous.downbeat_confidence_ppm < 700_000);
}

#[test]
fn temporal_onset_collision_measures_alignment_not_aggregate_activity() {
    let aligned = [1_000_000, 0, 500_000, 0, 250_000];
    let partially_aligned = [500_000, 0, 250_000, 0, 0];
    let displaced = [0, 1_000_000, 0, 500_000, 0];

    assert_eq!(
        temporal_onset_iou_ppm(&aligned, &aligned).unwrap(),
        1_000_000
    );
    assert_eq!(temporal_onset_iou_ppm(&aligned, &displaced).unwrap(), 0);
    let partial = temporal_onset_iou_ppm(&aligned, &partially_aligned).unwrap();
    assert!(partial > 0 && partial < 1_000_000);
    assert!(partial > temporal_onset_iou_ppm(&aligned, &displaced).unwrap());
}

#[test]
fn extract_signal_features_detects_real_periodicity_without_synthetic_fallback() {
    let fixture: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/pcm/feature-signals.json")).unwrap();
    assert_eq!(
        feature_algorithm_sha256(),
        fixture["algorithm_sha256"].as_str().unwrap()
    );
    let source = pulse_source(120, 4_410, 80.0);
    let features = extract_signal_features(&source).unwrap();
    let repeated = extract_signal_features(&source).unwrap();
    assert_eq!(features.analysis, repeated.analysis);
    assert_eq!(features.source_pcm_sha256, repeated.source_pcm_sha256);
    assert_eq!(features.source_frames, repeated.source_frames);
    assert_eq!(features.sample_peak_mdbfs, repeated.sample_peak_mdbfs);
    assert_eq!(features.true_peak_mdbtp, repeated.true_peak_mdbtp);
    assert_eq!(features.rhythm, repeated.rhythm);
    let rhythm = features
        .rhythm
        .as_ref()
        .expect("pulse train has a measured rhythm");
    assert!((119_000..=121_000).contains(&rhythm.tempo_millibpm));
    assert!(rhythm.beat_confidence_ppm >= 750_000);
    assert!(rhythm.downbeat_confidence_ppm >= 700_000);
    assert_eq!(rhythm.meter_beats, Some(4));
    assert!(rhythm.beat_frames.windows(2).all(|pair| pair[0] < pair[1]));

    let silence =
        CanonicalSource::from_pcm(PcmBuffer::from_frames(vec![[0.0, 0.0]; 44_100 * 20]).unwrap())
            .unwrap();
    assert!(extract_signal_features(&silence).unwrap().rhythm.is_none());
}

#[test]
fn extract_rejects_pcm_that_would_require_source_clamping() {
    assert_eq!(
        CanonicalSource::from_pcm(PcmBuffer::from_frames(vec![[1.0, 0.0]]).unwrap())
            .unwrap_err()
            .code(),
        "NON_CANONICAL_PCM"
    );
}

#[test]
fn extract_signal_features_distinguishes_bass_and_treble_windows() {
    let bass = extract_signal_features(&pulse_source(120, 0, 80.0)).unwrap();
    let treble = extract_signal_features(&pulse_source(120, 0, 6_000.0)).unwrap();
    let bass_window = bass.measure_window(44_100 * 8, 44_100 * 24).unwrap();
    let treble_window = treble.measure_window(44_100 * 8, 44_100 * 24).unwrap();
    assert!(bass_window.bass_occupancy_ppm > treble_window.bass_occupancy_ppm);
    assert!(bass_window.low_occupancy_ppm > treble_window.low_occupancy_ppm);
    assert!(treble_window.high_occupancy_ppm > bass_window.high_occupancy_ppm);
    assert_eq!(
        bass_window.low_occupancy_ppm
            + bass_window.mid_occupancy_ppm
            + bass_window.high_occupancy_ppm,
        1_000_000
    );
}

#[test]
fn extract_pair_snapshot_and_geometry_are_deterministic_and_bounds_safe() {
    let outgoing = extract_signal_features(&pulse_source(120, 4_410, 80.0)).unwrap();
    let incoming = extract_signal_features(&pulse_source(121, 8_820, 100.0)).unwrap();
    let first = build_feature_snapshot(&outgoing, &incoming).unwrap();
    let second = build_feature_snapshot(&outgoing, &incoming).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.outgoing.windows[0].vocal_activity_ppm, None);
    assert_eq!(first.incoming.windows[0].vocal_activity_ppm, None);
    assert!(first.pair.transient_collision_ppm.unwrap() < 700_000);

    let geometries = propose_geometries(&first).unwrap();
    assert_eq!(geometries.fallback_geometry.requested_dry_frames, 220_500);
    assert!(
        geometries
            .geometries
            .iter()
            .any(|value| value.duration_mode == DurationMode::Seconds)
    );
    assert!(
        geometries
            .geometries
            .iter()
            .any(|value| value.duration_mode == DurationMode::Bars)
    );
    assert!(geometries.geometries.iter().all(|value| {
        value.feature_snapshot_sha256 == first.snapshot_sha256
            && value.outgoing_cue_source_frame >= value.requested_dry_frames
            && value.incoming_cue_source_frame >= value.requested_dry_frames
    }));
}

#[test]
fn extract_snapshot_drives_real_m2_rich_generation() {
    let outgoing = extract_signal_features(&pulse_source(120, 4_410, 80.0)).unwrap();
    let incoming = extract_signal_features(&pulse_source(121, 8_820, 100.0)).unwrap();
    let snapshot = build_feature_snapshot(&outgoing, &incoming).unwrap();
    let geometries = propose_geometries(&snapshot).unwrap();
    let outgoing_cue = snapshot
        .outgoing
        .cues
        .iter()
        .find(|cue| cue.cue_id == snapshot.pair.outgoing_cue_id)
        .unwrap();
    let incoming_cue = snapshot
        .incoming
        .cues
        .iter()
        .find(|cue| cue.cue_id == snapshot.pair.incoming_cue_id)
        .unwrap();
    let result = generate_candidates(GenerationRequest {
        outgoing: SourceRef {
            track_id: "synthetic-outgoing".into(),
            pcm_profile: "pcm_s16le_stereo_44100_v1".into(),
            pcm_sha256: snapshot.outgoing.source_pcm_sha256.clone(),
            pcm_frame_count: snapshot.outgoing.source_frames,
            cue_id: outgoing_cue.cue_id.clone(),
            cue_source_frame: outgoing_cue.source_frame,
        },
        incoming: SourceRef {
            track_id: "synthetic-incoming".into(),
            pcm_profile: "pcm_s16le_stereo_44100_v1".into(),
            pcm_sha256: snapshot.incoming.source_pcm_sha256.clone(),
            pcm_frame_count: snapshot.incoming.source_frames,
            cue_id: incoming_cue.cue_id.clone(),
            cue_source_frame: incoming_cue.source_frame,
        },
        feature_snapshot: snapshot.feature_ref(),
        fallback_geometry: geometries.fallback_geometry,
        geometries: geometries.geometries,
        features: snapshot.template_inputs().unwrap(),
        outgoing_true_peak_mdbtp: Some(snapshot.outgoing.true_peak_mdbtp),
        incoming_true_peak_mdbtp: Some(snapshot.incoming.true_peak_mdbtp),
        config: GenerationConfig {
            generator_id: "transition-candidate-generator".into(),
            generator_version: "1.0.0".into(),
            generator_config_sha256: hash('f'),
            seed_hex_u64: "0000000000000000".into(),
        },
    })
    .unwrap();
    let families: std::collections::BTreeSet<_> = result
        .candidate_set
        .candidates
        .iter()
        .map(|candidate| candidate.template_id)
        .collect();
    assert!(families.contains(&TemplateId::SafeCrossfade));
    assert!(families.contains(&TemplateId::ShapedHandoff));
    assert!(families.contains(&TemplateId::BassHandoff));
    assert!(families.contains(&TemplateId::SpectralHandoff));
}
