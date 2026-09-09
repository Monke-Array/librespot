use serde::Deserialize;
use transition_operator::{
    Band, BandGainEnvelope, BiquadDf2t, CrossoverBandGain, CrossoverProfile, CutoffPoint,
    EnvelopePoint, FilterEnvelope, FilterKind, Interpolation, PcmBuffer, Target,
    apply_crossover_band_gain, apply_filter_envelope, control_grid_bracket, rbj_coefficients,
};

#[derive(Deserialize)]
struct FilterVectors {
    lp_20hz_q500: [f64; 5],
    hp_18000hz_q1000: [f64; 5],
    lp_1000hz_q707_impulse: Vec<f64>,
}

fn vectors() -> FilterVectors {
    serde_json::from_slice(include_bytes!("fixtures/pcm/filter-vectors.json")).unwrap()
}

fn envelope(values: &[(i64, i64)]) -> Vec<EnvelopePoint> {
    values
        .iter()
        .map(|&(frame, value_ppm)| EnvelopePoint { frame, value_ppm })
        .collect()
}

#[test]
fn rbj_coefficients_match_independent_min_max_and_impulse_vectors() {
    let expected = vectors();
    let low = rbj_coefficients(FilterKind::LowpassBiquadV1, 20_000, 500).unwrap();
    let high = rbj_coefficients(FilterKind::HighpassBiquadV1, 18_000_000, 1_000).unwrap();
    assert_coefficients(low.as_array(), expected.lp_20hz_q500);
    assert_coefficients(high.as_array(), expected.hp_18000hz_q1000);

    let coefficients = rbj_coefficients(FilterKind::LowpassBiquadV1, 1_000_000, 707).unwrap();
    let mut filter = BiquadDf2t::new(coefficients);
    let actual: Vec<_> = (0..expected.lp_1000hz_q707_impulse.len())
        .map(|index| filter.process(if index == 0 { 1.0 } else { 0.0 }))
        .collect();
    for (actual, expected) in actual.iter().zip(expected.lp_1000hz_q707_impulse) {
        assert!((actual - expected).abs() < 1e-15, "{actual} != {expected}");
    }
}

#[test]
fn signed_control_grid_uses_mathematical_floor_across_zero() {
    assert_eq!(control_grid_bracket(-129, 64).unwrap(), (-192, -128, 63));
    assert_eq!(control_grid_bracket(-128, 64).unwrap(), (-128, -64, 0));
    assert_eq!(control_grid_bracket(-65, 64).unwrap(), (-128, -64, 63));
    assert_eq!(control_grid_bracket(-64, 64).unwrap(), (-64, 0, 0));
    assert_eq!(control_grid_bracket(-1, 64).unwrap(), (-64, 0, 63));
    assert_eq!(control_grid_bracket(0, 64).unwrap(), (0, 64, 0));
}

#[test]
fn swept_filter_runs_continuous_state_even_while_wet_is_zero() {
    let mut pcm = PcmBuffer::from_frames(vec![[1.0, 1.0], [0.0, 0.0], [0.0, 0.0]]).unwrap();
    let filter = FilterEnvelope {
        op_id: "outgoing.filter".into(),
        target: Target::Outgoing,
        filter_kind: FilterKind::LowpassBiquadV1,
        cutoff_points: vec![
            CutoffPoint {
                frame: -64,
                cutoff_millihz: 1_000_000,
            },
            CutoffPoint {
                frame: 64,
                cutoff_millihz: 2_000_000,
            },
        ],
        cutoff_interpolations: vec![Interpolation::Linear],
        q_milli: 707,
        wet_points: envelope(&[(-2, 0), (-1, 0), (0, 1_000_000)]),
        wet_interpolations: vec![Interpolation::Linear; 2],
        control_interval_frames: 64,
    };
    apply_filter_envelope(&mut pcm, -2, &filter).unwrap();
    assert_eq!(pcm.frame(0), [1.0, 1.0]);
    assert_eq!(pcm.frame(1), [0.0, 0.0]);
    assert!(
        pcm.frame(2)[0].abs() > 0.01,
        "filter state was not warmed while dry"
    );
    assert_eq!(pcm.frame(2)[0], pcm.frame(2)[1]);
}

#[test]
fn lr4_two_band_unity_recombines_with_unit_magnitude() {
    let frame_count = 44_100;
    let source: Vec<_> = (0..frame_count)
        .map(|frame| {
            let sample = (2.0 * std::f64::consts::PI * 1_000.0 * frame as f64 / 44_100.0).sin();
            [sample, sample]
        })
        .collect();
    let input_rms = rms(&source[8_192..]);
    let mut pcm = PcmBuffer::from_frames(source).unwrap();
    let crossover = two_band(
        envelope(&[(0, 1_000_000), (44_100, 1_000_000)]),
        envelope(&[(0, 1_000_000), (44_100, 1_000_000)]),
    );
    apply_crossover_band_gain(&mut pcm, 0, &crossover).unwrap();
    let output_rms = rms(&pcm.frames()[8_192..]);
    assert!((output_rms / input_rms - 1.0).abs() < 1e-3);
}

#[test]
fn crossover_state_is_per_render_and_repeated_runs_are_identical() {
    let source: Vec<_> = (0..2_000)
        .map(|frame| {
            let sample = if frame == 0 { 1.0 } else { 0.0 };
            [sample, -sample]
        })
        .collect();
    let crossover = two_band(
        envelope(&[(0, 1_000_000), (2_000, 500_000)]),
        envelope(&[(0, 0), (2_000, 1_000_000)]),
    );
    let mut first = PcmBuffer::from_frames(source.clone()).unwrap();
    let mut second = PcmBuffer::from_frames(source).unwrap();
    apply_crossover_band_gain(&mut first, 0, &crossover).unwrap();
    apply_crossover_band_gain(&mut second, 0, &crossover).unwrap();
    assert_eq!(first.frames(), second.frames());

    let mut invalid = crossover;
    invalid.control_interval_frames = 63;
    let mut pcm = PcmBuffer::from_frames(vec![[0.0, 0.0]; 4]).unwrap();
    assert_eq!(
        apply_crossover_band_gain(&mut pcm, 0, &invalid)
            .unwrap_err()
            .code(),
        "INVALID_DSP_CONTROL_INTERVAL"
    );
}

#[test]
fn fixed_three_band_topology_is_frequency_selective() {
    let low_only = three_band([1_000_000, 0, 0]);
    let mid_only = three_band([0, 1_000_000, 0]);
    let high_only = three_band([0, 0, 1_000_000]);
    assert!(rendered_sine_ratio(100.0, &low_only) > 0.85);
    assert!(rendered_sine_ratio(1_000.0, &mid_only) > 0.75);
    assert!(rendered_sine_ratio(8_000.0, &high_only) > 0.85);
    assert!(rendered_sine_ratio(100.0, &high_only) < 0.02);
    assert!(rendered_sine_ratio(8_000.0, &low_only) < 0.02);
}

fn two_band(low: Vec<EnvelopePoint>, high: Vec<EnvelopePoint>) -> CrossoverBandGain {
    CrossoverBandGain {
        op_id: "outgoing.crossover".into(),
        target: Target::Outgoing,
        profile: CrossoverProfile::LinkwitzRiley4V1,
        crossover_millihz: vec![1_000_000],
        bands: vec![Band::Low, Band::High],
        band_gain_envelopes: vec![
            BandGainEnvelope {
                band: Band::Low,
                points: low,
                interpolations: vec![Interpolation::Linear],
            },
            BandGainEnvelope {
                band: Band::High,
                points: high,
                interpolations: vec![Interpolation::Linear],
            },
        ],
        wet_points: envelope(&[(0, 1_000_000), (44_100, 1_000_000)]),
        wet_interpolations: vec![Interpolation::Linear],
        control_interval_frames: 64,
    }
}

fn three_band(gains: [i64; 3]) -> CrossoverBandGain {
    let bands = [Band::Low, Band::Mid, Band::High];
    CrossoverBandGain {
        op_id: "outgoing.crossover".into(),
        target: Target::Outgoing,
        profile: CrossoverProfile::LinkwitzRiley4V1,
        crossover_millihz: vec![500_000, 3_000_000],
        bands: bands.to_vec(),
        band_gain_envelopes: bands
            .into_iter()
            .zip(gains)
            .map(|(band, gain)| BandGainEnvelope {
                band,
                points: envelope(&[(0, gain), (44_100, gain)]),
                interpolations: vec![Interpolation::Linear],
            })
            .collect(),
        wet_points: envelope(&[(0, 1_000_000), (44_100, 1_000_000)]),
        wet_interpolations: vec![Interpolation::Linear],
        control_interval_frames: 64,
    }
}

fn rendered_sine_ratio(frequency_hz: f64, crossover: &CrossoverBandGain) -> f64 {
    let source: Vec<_> = (0..44_100)
        .map(|frame| {
            let value = (2.0 * std::f64::consts::PI * frequency_hz * frame as f64 / 44_100.0).sin();
            [value, value]
        })
        .collect();
    let input = rms(&source[8_192..]);
    let mut output = PcmBuffer::from_frames(source).unwrap();
    apply_crossover_band_gain(&mut output, 0, crossover).unwrap();
    rms(&output.frames()[8_192..]) / input
}

fn rms(frames: &[[f64; 2]]) -> f64 {
    (frames.iter().map(|frame| frame[0] * frame[0]).sum::<f64>() / frames.len() as f64).sqrt()
}

fn assert_coefficients(actual: [f64; 5], expected: [f64; 5]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 1e-15, "{actual} != {expected}");
    }
}
