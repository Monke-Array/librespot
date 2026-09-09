use crate::dsp::PcmBuffer;
use crate::error::{Error, Result};
use crate::model::{LimiterProfile, OutputSafety, OutputSafetyProfile, TruePeakProfile};

// ITU-R BS.1770-4 Annex 2 four-phase, 12-tap interpolation coefficients.
const BS1770_4X: [[f64; 12]; 4] = [
    [
        0.001708984375,
        -0.0291748046875,
        0.11962890625,
        -0.4359130859375,
        1.45263671875,
        -0.65966796875,
        0.2392578125,
        -0.08251953125,
        0.02392578125,
        -0.005126953125,
        0.0003662109375,
        0.0,
    ],
    [
        -0.00152587890625,
        0.022216796875,
        -0.09814453125,
        0.465087890625,
        0.77978515625,
        -0.2003173828125,
        0.1015625,
        -0.041748046875,
        0.012939453125,
        -0.002685546875,
        0.000244140625,
        0.0,
    ],
    [
        -0.0010986328125,
        0.0189208984375,
        -0.0792236328125,
        0.292236328125,
        0.93603515625,
        -0.1817626953125,
        0.07049560546875,
        -0.0247802734375,
        0.0067138671875,
        -0.001220703125,
        0.0,
        0.0,
    ],
    [
        -0.00048828125,
        0.0091552734375,
        -0.038330078125,
        0.1383056640625,
        0.98583984375,
        -0.1290283203125,
        0.047607421875,
        -0.0159912109375,
        0.00439453125,
        -0.0006103515625,
        0.0,
        0.0,
    ],
];

#[derive(Clone, Debug)]
pub struct SafetyMeasurements {
    pub sample_peak: f64,
    pub true_peak: f64,
    pub loudness_mdb: i64,
    pub maximum_gain_reduction_mdb: i64,
    pub limiter_active_fraction_ppm: i64,
}

pub fn validate_safety_measurements(
    measurements: &SafetyMeasurements,
    safety: &OutputSafety,
) -> Result<()> {
    if safety.profile != OutputSafetyProfile::TransitionOutputSafetyV1
        || safety.sample_peak_ceiling_mdbfs != -1_200
        || safety.true_peak_target_mdbtp != -1_000
        || safety.limiter_profile != LimiterProfile::LookaheadPeakLimiterV1
        || safety.lookahead_frames != 221
        || safety.release_frames != 4_410
        || safety.maximum_gain_reduction_mdb != 3_000
        || safety.maximum_active_fraction_ppm != 50_000
        || safety.activity_threshold_mdb != 100
        || safety.true_peak_measurement_profile != TruePeakProfile::Bs1770_4xV1
    {
        return Err(Error::new(
            "INVALID_OUTPUT_SAFETY_PROFILE",
            "output safety does not match transition_output_safety_v1",
        ));
    }
    if !measurements.sample_peak.is_finite()
        || !measurements.true_peak.is_finite()
        || measurements.sample_peak < 0.0
        || measurements.true_peak < 0.0
    {
        return Err(Error::new(
            "NON_FINITE_PCM",
            "safety measurements contain an invalid peak",
        ));
    }
    if measurements.maximum_gain_reduction_mdb < 0
        || !(0..=1_000_000).contains(&measurements.limiter_active_fraction_ppm)
    {
        return Err(Error::new(
            "INVALID_SAFETY_MEASUREMENTS",
            "limiter measurements are outside their physical domains",
        ));
    }
    let sample_ceiling = 10_f64.powf(safety.sample_peak_ceiling_mdbfs as f64 / 20_000.0);
    if measurements.sample_peak > sample_ceiling {
        return Err(Error::new(
            "SAMPLE_PEAK_LIMIT_EXCEEDED",
            "post-limiter sample peak exceeds the limiter ceiling",
        ));
    }
    let true_peak_ceiling = 10_f64.powf(safety.true_peak_target_mdbtp as f64 / 20_000.0);
    if measurements.true_peak > true_peak_ceiling {
        return Err(Error::new(
            "TRUE_PEAK_LIMIT_EXCEEDED",
            "post-limiter true peak exceeds the declared target",
        ));
    }
    if measurements.maximum_gain_reduction_mdb > safety.maximum_gain_reduction_mdb {
        return Err(Error::new(
            "LIMITER_REDUCTION_LIMIT_EXCEEDED",
            "limiter gain reduction exceeds the declared maximum",
        ));
    }
    if measurements.limiter_active_fraction_ppm > safety.maximum_active_fraction_ppm {
        return Err(Error::new(
            "LIMITER_ACTIVITY_LIMIT_EXCEEDED",
            "limiter active fraction exceeds the declared maximum",
        ));
    }
    Ok(())
}

pub fn measure_true_peak_bs1770_4x(buffer: &PcmBuffer) -> Result<f64> {
    if buffer
        .frames()
        .iter()
        .flatten()
        .any(|sample| !sample.is_finite())
    {
        return Err(Error::new(
            "NON_FINITE_PCM",
            "true-peak input contains a nonfinite sample",
        ));
    }
    let mut peak = 0.0_f64;
    for channel in 0..2 {
        for output_frame in 0..buffer.frame_count().saturating_add(11) {
            for coefficients in BS1770_4X {
                let mut sample = 0.0;
                for (tap, coefficient) in coefficients.into_iter().enumerate() {
                    if let Some(input_frame) = output_frame.checked_sub(tap) {
                        if let Some(frame) = buffer.frames().get(input_frame) {
                            sample += coefficient * frame[channel];
                        }
                    }
                }
                peak = peak.max(sample.abs());
            }
        }
    }
    Ok(peak)
}

/// Returns deterministic, ungated full-buffer stereo RMS level in millidecibels.
/// This value is descriptive only and is never used for QC or normalization.
pub fn measure_loudness(buffer: &PcmBuffer) -> Result<i64> {
    if buffer.frame_count() == 0 {
        return Err(Error::new(
            "EMPTY_PCM",
            "loudness requires at least one PCM frame",
        ));
    }
    let mean_square = buffer
        .frames()
        .iter()
        .flatten()
        .map(|sample| sample * sample)
        .sum::<f64>()
        / (buffer.frame_count() * 2) as f64;
    if !mean_square.is_finite() || mean_square <= 0.0 {
        return Err(Error::new(
            "UNMEASURABLE_LOUDNESS",
            "loudness input must have finite nonzero energy",
        ));
    }
    Ok((10_000.0 * mean_square.log10()).round() as i64)
}

pub fn quantize_pcm24(buffer: &PcmBuffer) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(buffer.frame_count().saturating_mul(6));
    for sample in buffer.frames().iter().flatten() {
        if !sample.is_finite() {
            return Err(Error::new(
                "NON_FINITE_PCM",
                "PCM24 quantization input contains a nonfinite sample",
            ));
        }
        let quantized = (sample * 8_388_607.0).round();
        if !(-8_388_608.0..=8_388_607.0).contains(&quantized) {
            return Err(Error::new(
                "PCM24_CLAMP_REQUIRED",
                "PCM24 conversion would require safety clamping",
            ));
        }
        let bytes = (quantized as i32).to_le_bytes();
        output.extend_from_slice(&bytes[..3]);
    }
    Ok(output)
}
