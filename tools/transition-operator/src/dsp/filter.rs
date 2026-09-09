use crate::dsp::{PcmBuffer, eval_envelope};
use crate::error::{Error, Result};
use crate::model::{CutoffPoint, FilterEnvelope, FilterKind, Interpolation};

const SAMPLE_RATE_HZ: f64 = 44_100.0;

#[derive(Clone, Copy, Debug)]
pub struct BiquadCoefficients {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl BiquadCoefficients {
    pub fn as_array(self) -> [f64; 5] {
        [self.b0, self.b1, self.b2, self.a1, self.a2]
    }

    fn interpolate(self, right: Self, phase: f64) -> Self {
        let lerp = |left: f64, right: f64| left + (right - left) * phase;
        Self {
            b0: lerp(self.b0, right.b0),
            b1: lerp(self.b1, right.b1),
            b2: lerp(self.b2, right.b2),
            a1: lerp(self.a1, right.a1),
            a2: lerp(self.a2, right.a2),
        }
    }
}

#[derive(Clone, Debug)]
pub struct BiquadDf2t {
    coefficients: BiquadCoefficients,
    z1: f64,
    z2: f64,
}

impl BiquadDf2t {
    pub fn new(coefficients: BiquadCoefficients) -> Self {
        Self {
            coefficients,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn process(&mut self, sample: f64) -> f64 {
        let coefficients = self.coefficients;
        let output = coefficients.b0 * sample + self.z1;
        self.z1 = coefficients.b1 * sample - coefficients.a1 * output + self.z2;
        self.z2 = coefficients.b2 * sample - coefficients.a2 * output;
        output
    }

    fn set_coefficients(&mut self, coefficients: BiquadCoefficients) {
        self.coefficients = coefficients;
    }
}

pub fn rbj_coefficients(
    kind: FilterKind,
    cutoff_millihz: i64,
    q_milli: i64,
) -> Result<BiquadCoefficients> {
    if !(20_000..=18_000_000).contains(&cutoff_millihz) || !(500..=1_000).contains(&q_milli) {
        return Err(Error::new(
            "INVALID_DSP_FILTER_PARAMETERS",
            "RBJ cutoff or Q is outside the v1 profile",
        ));
    }
    rbj_coefficients_f64(kind, cutoff_millihz as f64, q_milli as f64)
}

pub fn control_grid_bracket(frame: i64, interval: i64) -> Result<(i64, i64, i64)> {
    if interval <= 0 {
        return Err(Error::new(
            "INVALID_DSP_CONTROL_INTERVAL",
            "control interval must be positive",
        ));
    }
    let remainder = frame.rem_euclid(interval);
    let left = frame
        .checked_sub(remainder)
        .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "control grid floor overflow"))?;
    let right = left
        .checked_add(interval)
        .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "control grid ceiling overflow"))?;
    Ok((left, right, remainder))
}

pub fn apply_filter_envelope(
    buffer: &mut PcmBuffer,
    start_frame: i64,
    value: &FilterEnvelope,
) -> Result<()> {
    if value.control_interval_frames != 64 {
        return Err(Error::new(
            "INVALID_DSP_CONTROL_INTERVAL",
            "v1 filter control interval must be 64 frames",
        ));
    }
    let initial = rbj_coefficients(value.filter_kind, 20_000, value.q_milli)?;
    let mut left_filter = BiquadDf2t::new(initial);
    let mut right_filter = BiquadDf2t::new(initial);
    for (offset, output) in buffer.frames_mut().iter_mut().enumerate() {
        let frame = start_frame
            .checked_add(
                i64::try_from(offset)
                    .map_err(|_| Error::new("INTEGER_OVERFLOW", "filter frame offset overflow"))?,
            )
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "filter timeline frame overflow"))?;
        let (grid_left, grid_right, remainder) = control_grid_bracket(frame, 64)?;
        let left_cutoff = eval_cutoff(
            &value.cutoff_points,
            &value.cutoff_interpolations,
            grid_left,
        )?;
        let right_cutoff = eval_cutoff(
            &value.cutoff_points,
            &value.cutoff_interpolations,
            grid_right,
        )?;
        let coefficients =
            rbj_coefficients_f64(value.filter_kind, left_cutoff, value.q_milli as f64)?
                .interpolate(
                    rbj_coefficients_f64(value.filter_kind, right_cutoff, value.q_milli as f64)?,
                    remainder as f64 / 64.0,
                );
        left_filter.set_coefficients(coefficients);
        right_filter.set_coefficients(coefficients);
        let dry = *output;
        let filtered = [left_filter.process(dry[0]), right_filter.process(dry[1])];
        let wet = eval_envelope(&value.wet_points, &value.wet_interpolations, frame)?;
        output[0] = dry[0] * (1.0 - wet) + filtered[0] * wet;
        output[1] = dry[1] * (1.0 - wet) + filtered[1] * wet;
    }
    Ok(())
}

fn rbj_coefficients_f64(
    kind: FilterKind,
    cutoff_millihz: f64,
    q_milli: f64,
) -> Result<BiquadCoefficients> {
    if !(20_000.0..=18_000_000.0).contains(&cutoff_millihz) || !(500.0..=1_000.0).contains(&q_milli)
    {
        return Err(Error::new(
            "INVALID_DSP_FILTER_PARAMETERS",
            "evaluated RBJ cutoff or Q is outside the v1 profile",
        ));
    }
    let cutoff_hz = cutoff_millihz / 1_000.0;
    let q = q_milli / 1_000.0;
    let w0 = 2.0 * std::f64::consts::PI * cutoff_hz / SAMPLE_RATE_HZ;
    let cosine = w0.cos();
    let alpha = w0.sin() / (2.0 * q);
    let (b0, b1, b2) = match kind {
        FilterKind::LowpassBiquadV1 => {
            let b0 = (1.0 - cosine) / 2.0;
            (b0, 1.0 - cosine, b0)
        }
        FilterKind::HighpassBiquadV1 => {
            let b0 = (1.0 + cosine) / 2.0;
            (b0, -(1.0 + cosine), b0)
        }
    };
    let a0 = 1.0 + alpha;
    Ok(BiquadCoefficients {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: -2.0 * cosine / a0,
        a2: (1.0 - alpha) / a0,
    })
}

fn eval_cutoff(
    points: &[CutoffPoint],
    interpolations: &[Interpolation],
    frame: i64,
) -> Result<f64> {
    if points.len() < 2
        || interpolations.len() + 1 != points.len()
        || points.windows(2).any(|pair| pair[0].frame >= pair[1].frame)
    {
        return Err(Error::new(
            "INVALID_DSP_CUTOFF_ENVELOPE",
            "cutoff envelope topology is invalid",
        ));
    }
    if frame < points[0].frame {
        return Ok(points[0].cutoff_millihz as f64);
    }
    if frame >= points[points.len() - 1].frame {
        return Ok(points[points.len() - 1].cutoff_millihz as f64);
    }
    let segment = points
        .windows(2)
        .position(|pair| frame >= pair[0].frame && frame < pair[1].frame)
        .ok_or_else(|| Error::new("INVALID_DSP_CUTOFF_ENVELOPE", "cutoff segment is missing"))?;
    let left = &points[segment];
    let right = &points[segment + 1];
    let x = (frame - left.frame) as f64 / (right.frame - left.frame) as f64;
    let phase = match interpolations[segment] {
        Interpolation::Linear => x,
        Interpolation::Smoothstep => 3.0 * x * x - 2.0 * x * x * x,
        _ => {
            return Err(Error::new(
                "INVALID_DSP_CUTOFF_ENVELOPE",
                "cutoff interpolation is unsupported",
            ));
        }
    };
    Ok(left.cutoff_millihz as f64 + (right.cutoff_millihz - left.cutoff_millihz) as f64 * phase)
}
