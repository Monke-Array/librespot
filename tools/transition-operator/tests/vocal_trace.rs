use transition_operator::{
    VOCAL_HOP_FRAMES, VOCAL_PAIR_END_FRAME, VOCAL_PAIR_START_FRAME, VocalAnalysisIdentityV1,
    VocalTraceBodyV1, VocalTraceV1, finalize_vocal_trace, measure_vocal_collision,
    vocal_activity_ppm,
};

fn hash(byte: char) -> String {
    std::iter::repeat_n(byte, 64).collect()
}

fn trace(evidence_ppm: Vec<i64>, threshold_ppm: i64) -> VocalTraceV1 {
    let source_frames = i64::try_from(evidence_ppm.len()).unwrap() * VOCAL_HOP_FRAMES;
    finalize_vocal_trace(VocalTraceBodyV1 {
        schema_version: "vocal-evidence-trace/1".into(),
        source_pcm_sha256: hash('a'),
        source_frames,
        hop_frames: VOCAL_HOP_FRAMES,
        analysis: VocalAnalysisIdentityV1 {
            extractor_id: "synthetic-test".into(),
            extractor_version: "1.0.0".into(),
            algorithm_sha256: hash('b'),
            positive_evidence_threshold_ppm: threshold_ppm,
        },
        evidence_ppm,
    })
    .unwrap()
}

#[test]
fn trace_identity_is_canonical_source_and_algorithm_bound() {
    let original = trace(vec![0, 500_000, 1_000_000], 500_000);
    let repeat = trace(vec![0, 500_000, 1_000_000], 500_000);
    assert_eq!(original.trace_sha256, repeat.trace_sha256);

    let changed_evidence = trace(vec![0, 500_001, 1_000_000], 500_000);
    assert_ne!(original.trace_sha256, changed_evidence.trace_sha256);
    let changed_threshold = trace(vec![0, 500_000, 1_000_000], 500_001);
    assert_ne!(original.trace_sha256, changed_threshold.trace_sha256);

    let mut invalid = VocalTraceBodyV1 {
        schema_version: "vocal-evidence-trace/1".into(),
        source_pcm_sha256: hash('a'),
        source_frames: 3 * VOCAL_HOP_FRAMES,
        hop_frames: VOCAL_HOP_FRAMES,
        analysis: original.analysis.clone(),
        evidence_ppm: vec![0, 500_000],
    };
    assert_eq!(
        finalize_vocal_trace(invalid.clone()).unwrap_err().code(),
        "INVALID_VOCAL_TRACE"
    );
    invalid.evidence_ppm = vec![0, 500_000, 1_000_001];
    assert_eq!(
        finalize_vocal_trace(invalid).unwrap_err().code(),
        "INVALID_VOCAL_TRACE"
    );
}

#[test]
fn activity_occupancy_has_exact_threshold_boundaries_and_clipping() {
    for (active_hops, expected) in [
        (0, 0),
        (1, 100_000),
        (2, 200_000),
        (3, 300_000),
        (6, 600_000),
        (7, 700_000),
        (8, 800_000),
        (10, 1_000_000),
    ] {
        let mut evidence = vec![0; 10];
        evidence[..active_hops].fill(700_000);
        let trace = trace(evidence, 500_000);
        assert_eq!(
            vocal_activity_ppm(&trace, 0, 10 * VOCAL_HOP_FRAMES).unwrap(),
            expected,
            "active_hops={active_hops}"
        );
    }

    let clipped = trace(vec![700_000, 0, 700_000], 500_000);
    assert_eq!(
        vocal_activity_ppm(&clipped, -10_000, 10_000).unwrap(),
        666_667
    );
    assert_eq!(
        vocal_activity_ppm(&clipped, 221, 662).unwrap(),
        0,
        "only the second hop center is inside the half-open window"
    );
}

fn pair_traces(
    outgoing_runs: &[(usize, usize, i64)],
    incoming_runs: &[(usize, usize, i64)],
) -> (VocalTraceV1, VocalTraceV1, i64) {
    let mut outgoing = vec![0; 400];
    let mut incoming = vec![0; 400];
    let cue_frame = 300 * VOCAL_HOP_FRAMES;
    let pair_start_hop =
        usize::try_from((cue_frame + VOCAL_PAIR_START_FRAME) / VOCAL_HOP_FRAMES).unwrap();
    for &(start, end, strength) in outgoing_runs {
        outgoing[pair_start_hop + start..pair_start_hop + end].fill(strength);
    }
    for &(start, end, strength) in incoming_runs {
        incoming[pair_start_hop + start..pair_start_hop + end].fill(strength);
    }
    (
        trace(outgoing, 500_000),
        trace(incoming, 500_000),
        cue_frame,
    )
}

#[test]
fn collision_is_pair_interval_occupancy_not_iou() {
    assert_eq!(VOCAL_PAIR_END_FRAME - VOCAL_PAIR_START_FRAME, 88_200);
    let (outgoing, incoming, cue) = pair_traces(&[(0, 120, 800_000)], &[(60, 180, 600_000)]);
    let measured = measure_vocal_collision(Some(&outgoing), Some(&incoming), cue, cue, 1_000_000)
        .unwrap()
        .unwrap();
    assert_eq!(measured.collision_ppm, 300_000);
    assert_eq!(measured.span_frames, Some(60 * VOCAL_HOP_FRAMES));
    assert_eq!(
        measured.start_frame,
        Some(VOCAL_PAIR_START_FRAME + 60 * VOCAL_HOP_FRAMES)
    );
    assert_eq!(
        measured.end_frame,
        Some(VOCAL_PAIR_START_FRAME + 120 * VOCAL_HOP_FRAMES)
    );
    assert_eq!(measured.outgoing_strength_ppm, Some(800_000));
    assert_eq!(measured.incoming_strength_ppm, Some(600_000));
}

#[test]
fn collision_handles_empty_unknown_thresholds_and_longest_island() {
    let (silent_out, silent_in, cue) = pair_traces(&[], &[]);
    let empty = measure_vocal_collision(Some(&silent_out), Some(&silent_in), cue, cue, 1_000_000)
        .unwrap()
        .unwrap();
    assert_eq!(empty.collision_ppm, 0);
    assert_eq!(empty.span_frames, None);
    assert!(
        measure_vocal_collision(None, Some(&silent_in), cue, cue, 1_000_000)
            .unwrap()
            .is_none()
    );

    for (active_hops, expected) in [
        (59, 295_000),
        (60, 300_000),
        (61, 305_000),
        (139, 695_000),
        (140, 700_000),
        (141, 705_000),
    ] {
        let (outgoing, incoming, cue) =
            pair_traces(&[(0, active_hops, 800_000)], &[(0, active_hops, 600_000)]);
        let measured =
            measure_vocal_collision(Some(&outgoing), Some(&incoming), cue, cue, 1_000_000)
                .unwrap()
                .unwrap();
        assert_eq!(
            measured.collision_ppm, expected,
            "active_hops={active_hops}"
        );
    }

    let (outgoing, incoming, cue) = pair_traces(
        &[(5, 15, 700_000), (40, 65, 900_000), (100, 125, 800_000)],
        &[(5, 15, 600_000), (40, 65, 500_000), (100, 125, 700_000)],
    );
    let measured = measure_vocal_collision(Some(&outgoing), Some(&incoming), cue, cue, 1_000_000)
        .unwrap()
        .unwrap();
    assert_eq!(measured.collision_ppm, 300_000);
    assert_eq!(
        measured.start_frame,
        Some(VOCAL_PAIR_START_FRAME + 40 * VOCAL_HOP_FRAMES),
        "equal longest islands choose the earliest"
    );
    assert_eq!(measured.outgoing_strength_ppm, Some(900_000));
    assert_eq!(measured.incoming_strength_ppm, Some(500_000));
}

#[test]
fn collision_uses_round_away_incoming_rate_mapping() {
    let (outgoing, mut incoming, cue) = pair_traces(&[(100, 101, 800_000)], &[]);
    let timeline = VOCAL_PAIR_START_FRAME + 100 * VOCAL_HOP_FRAMES;
    let mapped =
        transition_operator::div_round_nearest_away(timeline * 1_010_000, 1_000_000).unwrap();
    let incoming_index = usize::try_from((cue + mapped) / VOCAL_HOP_FRAMES).unwrap();
    incoming.evidence_ppm[incoming_index] = 700_000;
    incoming = finalize_vocal_trace(incoming.body()).unwrap();
    let measured = measure_vocal_collision(Some(&outgoing), Some(&incoming), cue, cue, 1_010_000)
        .unwrap()
        .unwrap();
    assert_eq!(measured.collision_ppm, 5_000);
}
