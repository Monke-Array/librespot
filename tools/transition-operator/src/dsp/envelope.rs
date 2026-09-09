use crate::error::{Error, Result};
use crate::model::{EnvelopePoint, Interpolation};

pub fn eval_envelope(
    points: &[EnvelopePoint],
    interpolations: &[Interpolation],
    frame: i64,
) -> Result<f64> {
    if points.len() < 2
        || interpolations.len() + 1 != points.len()
        || points.windows(2).any(|pair| pair[0].frame >= pair[1].frame)
    {
        return Err(Error::new(
            "INVALID_DSP_ENVELOPE",
            "DSP envelope topology is invalid",
        ));
    }
    if frame < points[0].frame {
        return Ok(ppm(points[0].value_ppm));
    }
    if frame >= points[points.len() - 1].frame {
        return Ok(ppm(points[points.len() - 1].value_ppm));
    }
    let segment = points
        .windows(2)
        .position(|pair| frame >= pair[0].frame && frame < pair[1].frame)
        .ok_or_else(|| Error::new("INVALID_DSP_ENVELOPE", "envelope segment is missing"))?;
    let left = &points[segment];
    let right = &points[segment + 1];
    let x = (frame - left.frame) as f64 / (right.frame - left.frame) as f64;
    let phase = match interpolations[segment] {
        Interpolation::Hold => 0.0,
        Interpolation::Linear => x,
        Interpolation::Smoothstep => 3.0 * x * x - 2.0 * x * x * x,
        Interpolation::QuarterSine => (std::f64::consts::FRAC_PI_2 * x).sin(),
        Interpolation::QuarterCosine => 1.0 - (std::f64::consts::FRAC_PI_2 * x).cos(),
    };
    let left = ppm(left.value_ppm);
    Ok(left + (ppm(right.value_ppm) - left) * phase)
}

fn ppm(value: i64) -> f64 {
    value as f64 / 1_000_000.0
}
