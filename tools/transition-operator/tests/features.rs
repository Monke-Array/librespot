use transition_operator::{
    AnalysisIdentity, CueFeature, CueKind, FeatureSnapshotBodyV2, FeatureSnapshotV2, FeatureWindow,
    PairFeatures, RhythmFeatures, SourceFeatures, TemplateFeatureView, WindowKind,
    finalize_feature_snapshot, snapshot_sha256, validate_feature_snapshot,
};

fn hash(byte: char) -> String {
    std::iter::repeat_n(byte, 64).collect()
}

fn snapshot_body() -> FeatureSnapshotBodyV2 {
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
    FeatureSnapshotBodyV2 {
        schema_version: "transition-feature-snapshot/2".into(),
        analysis,
        pair: PairFeatures {
            outgoing_cue_id: outgoing.cues[0].cue_id.clone(),
            incoming_cue_id: incoming.cues[0].cue_id.clone(),
            outgoing_window_id: outgoing.windows[0].window_id.clone(),
            incoming_window_id: incoming.windows[0].window_id.clone(),
            vocal_collision_ppm: None,
            vocal_collision_span_frames: None,
            transient_collision_ppm: Some(450_000),
            transient_collision_span_frames: Some(22_050),
            bass_collision_ppm: Some(500_000),
            spectral_overlap_ppm: Some(700_000),
            energy_delta_mdb: Some(3_000),
            alignment_error_ppm_of_beat: Some(20_000),
            collision_start_frame: None,
            collision_end_frame: None,
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
    let snapshot = transition_operator::parse_feature_snapshot(include_bytes!(
        "fixtures/features/snapshot-v2.json"
    ))
    .unwrap();
    assert_eq!(
        snapshot.snapshot_sha256,
        "f71d6b2893defb699d37930794cd5bc03c571ef0e41c752900748cb94b301ec5"
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
    assert!(serde_json::from_value::<FeatureSnapshotV2>(value).is_err());

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
