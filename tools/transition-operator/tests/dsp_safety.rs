use serde::Deserialize;
use transition_operator::{
    LookaheadPeakLimiter, OperatorPlan, PcmBuffer, SafetyMeasurements, apply_pair_gain,
    measure_loudness, measure_true_peak_bs1770_4x, quantize_pcm24, validate_safety_measurements,
};

#[derive(Deserialize)]
struct Vectors {
    impulse_true_peak: f64,
    constant_half_boundary_peak: f64,
    near_nyquist_hz: f64,
}

fn vectors() -> Vectors {
    serde_json::from_slice(include_bytes!("fixtures/pcm/bs1770-4x-vectors.json")).unwrap()
}

#[test]
fn pair_gain_is_one_nonboosting_mdb_conversion() {
    let mut pcm = PcmBuffer::from_frames(vec![[1.0, -0.5], [0.25, -0.125]]).unwrap();
    apply_pair_gain(&mut pcm, -6_020).unwrap();
    let expected = 10_f64.powf(-6_020.0 / 20_000.0);
    assert_eq!(pcm.frame(0), [expected, -0.5 * expected]);
    assert_eq!(pcm.frame(1), [0.25 * expected, -0.125 * expected]);
    assert_eq!(
        apply_pair_gain(&mut pcm, 1).unwrap_err().code(),
        "INVALID_PAIR_OUTPUT_GAIN"
    );
}

#[test]
fn limiter_has_inclusive_lookahead_immediate_attack_release_and_linked_stereo() {
    let mut frames = vec![[0.0, 0.0]; 223];
    frames[221] = [2.0, -0.5];
    let mut pcm = PcmBuffer::from_frames(frames).unwrap();
    let limiter = LookaheadPeakLimiter::new(-1_200, 221, 4_410, 100).unwrap();
    let result = limiter.process(&mut pcm, 0, 0, 223).unwrap();
    let ceiling = 10_f64.powf(-1_200.0 / 20_000.0);
    assert_eq!(result.applied_gains[0], ceiling / 2.0);
    assert_eq!(result.applied_gains[221], ceiling / 2.0);
    let release = 100_f64.powf(1.0 / 4_410.0);
    assert!((result.applied_gains[222] - ceiling / 2.0 * release).abs() < 1e-15);
    assert_eq!(pcm.frame(221), [ceiling, -0.25 * ceiling]);
    assert_eq!(
        result.maximum_gain_reduction_mdb,
        (-20_000.0 * (ceiling / 2.0).log10()).round() as i64
    );
    let active = result
        .applied_gains
        .iter()
        .filter(|gain| -20_000.0 * gain.log10() > 100.0)
        .count();
    assert_eq!(
        result.active_fraction_ppm,
        ((active as f64 * 1_000_000.0 / 223.0).round()) as i64
    );
}

#[test]
fn limiter_zero_input_stays_unity_and_activity_uses_only_transition_window() {
    let mut pcm = PcmBuffer::from_frames(vec![[0.0, 0.0]; 500]).unwrap();
    let limiter = LookaheadPeakLimiter::new(-1_200, 221, 4_410, 100).unwrap();
    let result = limiter.process(&mut pcm, -250, -100, 0).unwrap();
    assert!(result.applied_gains.iter().all(|gain| *gain == 1.0));
    assert_eq!(result.maximum_gain_reduction_mdb, 0);
    assert_eq!(result.active_fraction_ppm, 0);
}

#[test]
fn annex_two_true_peak_covers_impulse_phases_boundaries_and_stereo_maximum() {
    let expected = vectors();
    let mut measured = Vec::new();
    for phase in 0..4 {
        let mut frames = vec![[0.0, 0.0]; 64];
        frames[24 + phase] = [1.0, 0.25];
        measured
            .push(measure_true_peak_bs1770_4x(&PcmBuffer::from_frames(frames).unwrap()).unwrap());
    }
    assert!(
        measured
            .iter()
            .all(|peak| (*peak - expected.impulse_true_peak).abs() < 1e-15)
    );

    let constant = PcmBuffer::from_frames(vec![[0.5, -0.25]; 256]).unwrap();
    let peak = measure_true_peak_bs1770_4x(&constant).unwrap();
    assert!(
        (peak - expected.constant_half_boundary_peak).abs() < 1e-15,
        "constant boundary peak {peak}"
    );
}

#[test]
fn near_nyquist_true_peak_is_not_reduced_to_sample_peak() {
    let expected = vectors();
    let frames: Vec<_> = (0..4_096)
        .map(|frame| {
            let sample = 0.8
                * (2.0 * std::f64::consts::PI * expected.near_nyquist_hz * frame as f64 / 44_100.0)
                    .sin();
            [sample, -sample]
        })
        .collect();
    let pcm = PcmBuffer::from_frames(frames).unwrap();
    let sample_peak = pcm
        .frames()
        .iter()
        .flatten()
        .map(|sample| sample.abs())
        .fold(0.0, f64::max);
    assert!(measure_true_peak_bs1770_4x(&pcm).unwrap() >= sample_peak);
}

#[test]
fn pcm24_quantization_rounds_ties_away_and_rejects_required_clamps() {
    let half_lsb = 0.5 / 8_388_607.0;
    let pcm = PcmBuffer::from_frames(vec![[half_lsb, -half_lsb], [1.0, -1.0]]).unwrap();
    assert_eq!(
        quantize_pcm24(&pcm).unwrap(),
        [1, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f, 1, 0, 0x80,]
    );
    let clipped = PcmBuffer::from_frames(vec![[1.1, 0.0]]).unwrap();
    assert_eq!(
        quantize_pcm24(&clipped).unwrap_err().code(),
        "PCM24_CLAMP_REQUIRED"
    );
}

#[test]
fn loudness_measurement_is_descriptive_and_finite() {
    let pcm = PcmBuffer::from_frames(vec![[0.5, 0.5]; 44_100]).unwrap();
    let measured = measure_loudness(&pcm).unwrap();
    assert!((measured - (-6_021)).abs() <= 1);
}

fn output_safety() -> transition_operator::OutputSafety {
    serde_json::from_slice::<OperatorPlan>(include_bytes!("fixtures/plans/safe-crossfade.json"))
        .unwrap()
        .output_safety
}

#[test]
fn safety_qc_uses_stable_fail_closed_rejection_codes() {
    let safety = output_safety();
    let passing = SafetyMeasurements {
        sample_peak: 10_f64.powf(-1_200.0 / 20_000.0),
        true_peak: 10_f64.powf(-1_000.0 / 20_000.0),
        loudness_mdb: -12_000,
        maximum_gain_reduction_mdb: 3_000,
        limiter_active_fraction_ppm: 50_000,
    };
    validate_safety_measurements(&passing, &safety).unwrap();

    let mut failed = passing.clone();
    failed.sample_peak = 10_f64.powf(-1_199.0 / 20_000.0);
    assert_eq!(
        validate_safety_measurements(&failed, &safety)
            .unwrap_err()
            .code(),
        "SAMPLE_PEAK_LIMIT_EXCEEDED"
    );

    failed = passing.clone();
    failed.true_peak = 10_f64.powf(-999.0 / 20_000.0);
    assert_eq!(
        validate_safety_measurements(&failed, &safety)
            .unwrap_err()
            .code(),
        "TRUE_PEAK_LIMIT_EXCEEDED"
    );

    failed = passing.clone();
    failed.maximum_gain_reduction_mdb = 3_001;
    assert_eq!(
        validate_safety_measurements(&failed, &safety)
            .unwrap_err()
            .code(),
        "LIMITER_REDUCTION_LIMIT_EXCEEDED"
    );

    failed = passing.clone();
    failed.limiter_active_fraction_ppm = 50_001;
    assert_eq!(
        validate_safety_measurements(&failed, &safety)
            .unwrap_err()
            .code(),
        "LIMITER_ACTIVITY_LIMIT_EXCEEDED"
    );

    failed = passing.clone();
    failed.sample_peak = f64::NAN;
    assert_eq!(
        validate_safety_measurements(&failed, &safety)
            .unwrap_err()
            .code(),
        "NON_FINITE_PCM"
    );

    failed = passing;
    failed.maximum_gain_reduction_mdb = -1;
    assert_eq!(
        validate_safety_measurements(&failed, &safety)
            .unwrap_err()
            .code(),
        "INVALID_SAFETY_MEASUREMENTS"
    );
}
