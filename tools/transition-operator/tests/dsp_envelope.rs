use serde::Deserialize;
use transition_operator::{
    DelayTap, DuckEnvelope, DuckReason, EnvelopePoint, FeedforwardDelayTail, GainEnvelope,
    Interpolation, PcmBuffer, RhythmicGate, Target, apply_duck, apply_gain, apply_gate,
    eval_envelope, render_feedforward_tail,
};

#[derive(Deserialize)]
struct Vectors {
    linear_midpoint: f64,
    smoothstep_midpoint: f64,
    quarter_sine_midpoint: f64,
    quarter_cosine_midpoint: f64,
    equal_power_squared_sum: f64,
}

fn points(values: &[(i64, i64)]) -> Vec<EnvelopePoint> {
    values
        .iter()
        .map(|&(frame, value_ppm)| EnvelopePoint { frame, value_ppm })
        .collect()
}

fn vectors() -> Vectors {
    serde_json::from_slice(include_bytes!("fixtures/pcm/envelope-vectors.json")).unwrap()
}

#[test]
fn envelope_vectors_cover_holds_endpoints_and_every_curve() {
    let expected = vectors();
    let values = points(&[(0, 0), (4, 1_000_000)]);
    for (curve, midpoint) in [
        (Interpolation::Linear, expected.linear_midpoint),
        (Interpolation::Smoothstep, expected.smoothstep_midpoint),
        (Interpolation::QuarterSine, expected.quarter_sine_midpoint),
        (
            Interpolation::QuarterCosine,
            expected.quarter_cosine_midpoint,
        ),
    ] {
        assert_eq!(eval_envelope(&values, &[curve], -1).unwrap(), 0.0);
        assert_eq!(eval_envelope(&values, &[curve], 0).unwrap(), 0.0);
        assert!((eval_envelope(&values, &[curve], 2).unwrap() - midpoint).abs() < 1e-15);
        assert_eq!(eval_envelope(&values, &[curve], 4).unwrap(), 1.0);
        assert_eq!(eval_envelope(&values, &[curve], 5).unwrap(), 1.0);
    }

    let step = points(&[(-1, 1_000_000), (0, 0)]);
    assert_eq!(
        eval_envelope(&step, &[Interpolation::Hold], -1).unwrap(),
        1.0
    );
    assert_eq!(
        eval_envelope(&step, &[Interpolation::Hold], 0).unwrap(),
        0.0
    );
}

#[test]
fn matched_quarter_curves_are_equal_power() {
    let expected = vectors();
    let outgoing = points(&[(0, 1_000_000), (100, 0)]);
    let incoming = points(&[(0, 0), (100, 1_000_000)]);
    for frame in 0..=100 {
        let a = eval_envelope(&outgoing, &[Interpolation::QuarterCosine], frame).unwrap();
        let b = eval_envelope(&incoming, &[Interpolation::QuarterSine], frame).unwrap();
        assert!((a * a + b * b - expected.equal_power_squared_sum).abs() < 2e-15);
    }
}

#[test]
fn gain_duck_and_gate_multiply_without_normalization_or_state() {
    let mut gain_pcm = PcmBuffer::from_frames(vec![[1.0, -1.0]; 3]).unwrap();
    assert_eq!(gain_pcm.source_pcm_sha256(), None);
    let gain = GainEnvelope {
        op_id: "outgoing.primary_gain".into(),
        target: Target::Outgoing,
        points: points(&[(-1, 1_000_000), (1, 0)]),
        interpolations: vec![Interpolation::Linear],
    };
    apply_gain(&mut gain_pcm, -1, &gain).unwrap();
    assert_eq!(gain_pcm.frames(), &[[1.0, -1.0], [0.5, -0.5], [0.0, -0.0]]);

    let mut duck_pcm = PcmBuffer::from_frames(vec![[0.8, -0.4]; 3]).unwrap();
    let duck = DuckEnvelope {
        op_id: "outgoing.duck".into(),
        target: Target::Outgoing,
        reason: DuckReason::VocalCollision,
        points: points(&[(-1, 500_000), (1, 500_000)]),
        interpolations: vec![Interpolation::Linear],
    };
    apply_duck(&mut duck_pcm, -1, &duck).unwrap();
    assert_eq!(duck_pcm.frames(), &[[0.4, -0.2]; 3]);

    let mut gate_pcm = PcmBuffer::from_frames(vec![[1.0, 1.0]; 5]).unwrap();
    let gate = RhythmicGate {
        op_id: "outgoing.gate".into(),
        target: Target::Outgoing,
        points: points(&[(-2, 0), (-1, 1_000_000), (0, 0), (1, 0), (2, 0)]),
        interpolations: vec![Interpolation::Linear; 4],
    };
    apply_gate(&mut gate_pcm, -2, &gate).unwrap();
    assert_eq!(
        gate_pcm.frames(),
        &[[0.0, 0.0], [1.0, 1.0], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]]
    );
}

#[test]
fn envelope_rejects_malformed_topology_instead_of_repairing_it() {
    assert_eq!(
        eval_envelope(&points(&[(0, 0)]), &[], 0)
            .unwrap_err()
            .code(),
        "INVALID_DSP_ENVELOPE"
    );
    assert_eq!(
        eval_envelope(
            &points(&[(0, 0), (0, 1_000_000)]),
            &[Interpolation::Linear],
            0,
        )
        .unwrap_err()
        .code(),
        "INVALID_DSP_ENVELOPE"
    );
}

#[test]
fn feedforward_tail_uses_half_open_capture_and_never_feedbacks() {
    let input = PcmBuffer::from_frames(vec![
        [1.0, 10.0],
        [2.0, 20.0],
        [3.0, 30.0],
        [4.0, 40.0],
        [5.0, 50.0],
    ])
    .unwrap();
    let tail = FeedforwardDelayTail {
        op_id: "outgoing.delay_tail".into(),
        target: Target::Outgoing,
        capture_start_frame: -2,
        capture_end_frame: 0,
        taps: vec![
            DelayTap {
                delay_frames: 2,
                gain_ppm: 500_000,
            },
            DelayTap {
                delay_frames: 3,
                gain_ppm: 250_000,
            },
        ],
    };
    let wet = render_feedforward_tail(&input, -3, -3, 4, &tail).unwrap();
    assert_eq!(wet.frame_count(), 7);
    assert_eq!(wet.frame(0), [0.0, 0.0]);
    assert_eq!(wet.frame(1), [0.0, 0.0]);
    assert_eq!(wet.frame(2), [0.0, 0.0]);
    assert_eq!(wet.frame(3), [1.0, 10.0]);
    assert_eq!(wet.frame(4), [2.0, 20.0]);
    assert_eq!(wet.frame(5), [0.75, 7.5]);
    assert_eq!(wet.frame(6), [0.0, 0.0]);
}

#[test]
fn feedforward_tail_rejects_a_missing_captured_input_frame() {
    let input = PcmBuffer::from_frames(vec![[1.0, 1.0]]).unwrap();
    let tail = FeedforwardDelayTail {
        op_id: "outgoing.delay_tail".into(),
        target: Target::Outgoing,
        capture_start_frame: -2,
        capture_end_frame: 0,
        taps: vec![DelayTap {
            delay_frames: 2,
            gain_ppm: 500_000,
        }],
    };
    assert_eq!(
        render_feedforward_tail(&input, -1, 0, 2, &tail)
            .unwrap_err()
            .code(),
        "DSP_SOURCE_WINDOW_MISSING"
    );
}
