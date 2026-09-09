use transition_operator::{
    CueIdentityCore, DurationMode, FeatureWindowIdentityCore, GeometryProposal,
    GeometryProposalCore, JSON_SAFE_INTEGER_MAX, cue_id, feature_window_id, geometry_id,
    shortlist_geometries, validate_fallback_geometry,
};

fn hash(ch: char) -> String {
    std::iter::repeat_n(ch, 64).collect()
}

fn core(mode: DurationMode, dry_frames: i64, quality: i64, suffix: &str) -> GeometryProposalCore {
    GeometryProposalCore {
        outgoing_pcm_sha256: hash('a'),
        incoming_pcm_sha256: hash('b'),
        feature_snapshot_sha256: hash('c'),
        outgoing_pcm_frame_count: 2_000_000,
        incoming_pcm_frame_count: 2_000_000,
        outgoing_cue_id: format!("cue-out-{suffix}"),
        outgoing_cue_source_frame: 1_000_000,
        incoming_cue_id: format!("cue-in-{suffix}"),
        incoming_cue_source_frame: 1_000_000,
        duration_mode: mode,
        requested_dry_frames: dry_frames,
        resolved_bar_count: match mode {
            DurationMode::Bars => 2,
            _ => 0,
        },
        incoming_source_rate_ppm: 1_000_000,
        cue_confidence_ppm: 900_000,
        beat_confidence_ppm: 900_000,
        downbeat_confidence_ppm: 900_000,
        alignment_error_ppm_of_beat: 0,
        geometry_quality_ppm: quality,
        feature_window_ids: vec![format!("window-z-{suffix}"), format!("window-a-{suffix}")],
    }
}

fn proposal(mode: DurationMode, dry_frames: i64, quality: i64, suffix: &str) -> GeometryProposal {
    GeometryProposal::finalize(core(mode, dry_frames, quality, suffix)).unwrap()
}

#[test]
fn cue_and_window_ids_are_content_derived_golden_values() {
    let cue = CueIdentityCore {
        source_pcm_sha256: hash('1'),
        analysis_sha256: hash('2'),
        kind: "downbeat".into(),
        frame: 12_345,
    };
    assert_eq!(
        cue_id(&cue).unwrap().as_str(),
        "cue1-3e180262009a63aa45c165712b4bfdaff46072d13d62ce72c92d685abe42c508"
    );

    let window = FeatureWindowIdentityCore {
        source_pcm_sha256: hash('1'),
        analysis_sha256: hash('2'),
        kind: "bass".into(),
        start_frame: 10_000,
        end_frame: 20_000,
    };
    assert_eq!(
        feature_window_id(&window).unwrap().as_str(),
        "window1-e18bf9f4bfa11567d2aac2ee28aea1df7b660234a0067dff78b0e6bdd3acaa98"
    );
}

#[test]
fn geometry_id_ignores_quality_and_window_input_order_but_not_render_semantics() {
    let original = core(DurationMode::Bars, 176_400, 900_000, "same");
    let original_id = geometry_id(&original).unwrap();

    let mut reordered = original.clone();
    reordered.feature_window_ids.reverse();
    reordered.geometry_quality_ppm = 1;
    assert_eq!(geometry_id(&reordered).unwrap(), original_id);

    let mut moved = original;
    moved.incoming_cue_source_frame += 1;
    assert_ne!(geometry_id(&moved).unwrap(), original_id);
}

#[test]
fn shortlist_is_deduplicated_and_round_robined_independent_of_input_order() {
    let mut inputs = Vec::new();
    for index in 0..5 {
        inputs.push(proposal(
            DurationMode::Cut,
            1,
            900_000 - index,
            &format!("c{index}"),
        ));
        inputs.push(proposal(
            DurationMode::Seconds,
            132_300,
            800_000 - index,
            &format!("s{index}"),
        ));
        inputs.push(proposal(
            DurationMode::Seconds,
            300_000,
            700_000 - index,
            &format!("m{index}"),
        ));
        inputs.push(proposal(
            DurationMode::Seconds,
            600_000,
            600_000 - index,
            &format!("l{index}"),
        ));
    }
    inputs.push(inputs[0].clone());

    let first = shortlist_geometries(inputs.clone()).unwrap();
    inputs.reverse();
    let second = shortlist_geometries(inputs).unwrap();
    let ids = |values: &[GeometryProposal]| {
        values
            .iter()
            .map(|value| value.geometry_id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&first), ids(&second));
    assert_eq!(first.len(), 12);
    assert_eq!(
        first
            .iter()
            .map(|value| value.requested_dry_frames)
            .collect::<Vec<_>>(),
        [
            1, 132_300, 300_000, 600_000, 1, 132_300, 300_000, 600_000, 1, 132_300, 300_000,
            600_000,
        ]
    );
}

#[test]
fn malformed_or_out_of_bounds_geometry_fails_closed() {
    let mut missing_windows = proposal(DurationMode::Seconds, 220_500, 1, "missing");
    missing_windows.feature_window_ids.clear();
    assert_eq!(
        shortlist_geometries(vec![missing_windows])
            .unwrap_err()
            .code(),
        "GEOMETRY_WINDOWS_MISSING"
    );

    let mut out_of_bounds = core(DurationMode::Seconds, 220_500, 1, "bounds");
    out_of_bounds.outgoing_cue_source_frame = 100;
    assert_eq!(
        GeometryProposal::finalize(out_of_bounds)
            .unwrap_err()
            .code(),
        "GEOMETRY_SOURCE_OUT_OF_BOUNDS"
    );

    let mut wrong_id = proposal(DurationMode::Seconds, 220_500, 1, "wrong-id");
    wrong_id.geometry_id = format!("geom1-{}", hash('0'));
    assert_eq!(
        shortlist_geometries(vec![wrong_id]).unwrap_err().code(),
        "GEOMETRY_ID_MISMATCH"
    );
}

#[test]
fn fallback_requires_an_exact_bounds_safe_five_second_geometry() {
    let fallback = proposal(DurationMode::Seconds, 220_500, 900_000, "fallback");
    validate_fallback_geometry(&fallback).unwrap();

    let short = proposal(DurationMode::Seconds, 132_300, 900_000, "short");
    assert_eq!(
        validate_fallback_geometry(&short).unwrap_err().code(),
        "INVALID_FALLBACK_GEOMETRY"
    );
}

#[test]
fn geometry_identity_scalars_and_rich_cue_confidence_fail_closed() {
    let mut cue = CueIdentityCore {
        source_pcm_sha256: hash('1'),
        analysis_sha256: hash('2'),
        kind: "downbeat".into(),
        frame: -1,
    };
    assert_eq!(cue_id(&cue).unwrap_err().code(), "INVALID_CUE_IDENTITY");
    cue.frame = JSON_SAFE_INTEGER_MAX + 1;
    assert_eq!(cue_id(&cue).unwrap_err().code(), "INVALID_CUE_IDENTITY");

    let window = FeatureWindowIdentityCore {
        source_pcm_sha256: hash('1'),
        analysis_sha256: hash('2'),
        kind: "bass".into(),
        start_frame: -1,
        end_frame: 10,
    };
    assert_eq!(
        feature_window_id(&window).unwrap_err().code(),
        "INVALID_FEATURE_WINDOW"
    );

    let mut unsafe_integer = core(DurationMode::Seconds, 220_500, 1, "integer");
    unsafe_integer.outgoing_pcm_frame_count = JSON_SAFE_INTEGER_MAX + 1;
    assert_eq!(
        GeometryProposal::finalize(unsafe_integer)
            .unwrap_err()
            .code(),
        "INVALID_GEOMETRY_SCALAR"
    );

    let mut unreliable = proposal(DurationMode::Seconds, 220_500, 1, "confidence");
    unreliable.cue_confidence_ppm = 699_999;
    unreliable.geometry_id = geometry_id(&unreliable.core()).unwrap();
    assert_eq!(
        shortlist_geometries(vec![unreliable]).unwrap_err().code(),
        "UNRELIABLE_CUE_GEOMETRY"
    );
}
