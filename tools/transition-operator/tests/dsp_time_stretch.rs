use serde::Deserialize;
use transition_operator::{PcmBuffer, RubberBandTimeStretch, TimeStretchBackend, TimeStretchCue};

#[derive(Deserialize)]
struct Fixture {
    rates_ppm: Vec<i64>,
    output_frames: usize,
    cue_output_frame: usize,
    maximum_cue_error_frames: usize,
    maximum_transient_error_frames: usize,
    maximum_pitch_error_cents: f64,
    maximum_level_error_mdb: i64,
}

fn fixture() -> Fixture {
    serde_json::from_slice(include_bytes!("fixtures/pcm/time-map-fixtures.json")).unwrap()
}

#[test]
fn rubberband_has_exact_length_pitch_level_and_transient_conformance() {
    let fixture = fixture();
    let backend = RubberBandTimeStretch::new("ffmpeg");
    for rate in fixture.rates_ppm {
        let required =
            ((fixture.output_frames as u64 * rate as u64 + 999_999) / 1_000_000) as usize + 8_192;
        let cue_input =
            ((fixture.cue_output_frame as u64 * rate as u64 + 500_000) / 1_000_000) as usize;
        let cue = TimeStretchCue {
            input_frame: cue_input,
            output_frame: fixture.cue_output_frame,
        };

        let tone = PcmBuffer::from_frames(
            (0..required)
                .map(|frame| {
                    let sample =
                        0.25 * (2.0 * std::f64::consts::PI * 440.0 * frame as f64 / 44_100.0).sin();
                    [sample, sample]
                })
                .collect(),
        )
        .unwrap();
        let rendered = backend
            .process(&tone, rate, fixture.output_frames, cue)
            .unwrap();
        let repeated = backend
            .process(&tone, rate, fixture.output_frames, cue)
            .unwrap();
        assert_eq!(rendered.frame_count(), fixture.output_frames);
        assert_eq!(rendered.frames(), repeated.frames());
        let measured_hz = estimate_frequency(&rendered.frames()[4_096..40_000]);
        let cents = 1_200.0 * (measured_hz / 440.0).log2().abs();
        assert!(
            cents <= fixture.maximum_pitch_error_cents,
            "rate {rate}: {cents} cents"
        );
        let level_error_mdb = (20_000.0 * (rms(rendered.frames()) / rms(tone.frames())).log10())
            .round()
            .abs() as i64;
        assert!(
            level_error_mdb <= fixture.maximum_level_error_mdb,
            "rate {rate}: {level_error_mdb} mdb"
        );

        let mut impulse = vec![[0.0, 0.0]; required];
        impulse[cue_input] = [1.0, 1.0];
        let rendered_impulse = backend
            .process(
                &PcmBuffer::from_frames(impulse).unwrap(),
                rate,
                fixture.output_frames,
                cue,
            )
            .unwrap();
        let peak = rendered_impulse
            .frames()
            .iter()
            .enumerate()
            .max_by(|left, right| left.1[0].abs().total_cmp(&right.1[0].abs()))
            .unwrap()
            .0;
        assert!(
            peak.abs_diff(fixture.cue_output_frame) <= fixture.maximum_transient_error_frames,
            "rate {rate}: transient at {peak}"
        );
        assert!(
            cue.logical_error_frames(rate) <= fixture.maximum_cue_error_frames,
            "rate {rate}: logical cue mapping drifted"
        );
    }
}

#[test]
fn time_stretch_program_hash_is_stable_and_parameter_bound() {
    let backend = RubberBandTimeStretch::new("ffmpeg");
    let first = backend.program_hash(920_000, 44_100).unwrap();
    assert_eq!(first, backend.program_hash(920_000, 44_100).unwrap());
    assert_ne!(first, backend.program_hash(1_080_000, 44_100).unwrap());
    assert_ne!(first, backend.program_hash(920_000, 44_101).unwrap());
}

#[test]
fn unity_rate_is_an_exact_continuous_window_bypass() {
    let input = PcmBuffer::from_frames(
        (0..1_024)
            .map(|frame| [frame as f64 / 1_024.0, -(frame as f64) / 1_024.0])
            .collect(),
    )
    .unwrap();
    let output = RubberBandTimeStretch::new("ffmpeg")
        .process(
            &input,
            1_000_000,
            1_000,
            TimeStretchCue {
                input_frame: 500,
                output_frame: 500,
            },
        )
        .unwrap();
    assert_eq!(output.frames(), &input.frames()[..1_000]);
}

#[test]
fn time_stretch_rejects_bounds_short_input_and_misaligned_cue_without_padding() {
    let backend = RubberBandTimeStretch::new("ffmpeg");
    let input = PcmBuffer::from_frames(vec![[0.0, 0.0]; 100]).unwrap();
    assert_eq!(
        backend
            .process(
                &input,
                919_999,
                10,
                TimeStretchCue {
                    input_frame: 9,
                    output_frame: 10,
                },
            )
            .unwrap_err()
            .code(),
        "INVALID_TIME_STRETCH_RATE"
    );
    assert_eq!(
        backend
            .process(
                &input,
                1_080_000,
                100,
                TimeStretchCue {
                    input_frame: 99,
                    output_frame: 92,
                },
            )
            .unwrap_err()
            .code(),
        "TIME_STRETCH_SOURCE_TOO_SHORT"
    );
    assert_eq!(
        backend
            .process(
                &input,
                1_000_000,
                10,
                TimeStretchCue {
                    input_frame: 20,
                    output_frame: 5,
                },
            )
            .unwrap_err()
            .code(),
        "TIME_STRETCH_CUE_MISALIGNED"
    );
}

fn estimate_frequency(frames: &[[f64; 2]]) -> f64 {
    let crossings: Vec<_> = frames
        .windows(2)
        .enumerate()
        .filter_map(|(index, pair)| {
            (pair[0][0] <= 0.0 && pair[1][0] > 0.0)
                .then(|| index as f64 + (-pair[0][0]) / (pair[1][0] - pair[0][0]))
        })
        .collect();
    let periods = crossings.len() - 1;
    44_100.0 * periods as f64 / (crossings[periods] - crossings[0])
}

fn rms(frames: &[[f64; 2]]) -> f64 {
    (frames.iter().map(|frame| frame[0] * frame[0]).sum::<f64>() / frames.len() as f64).sqrt()
}
