use crate::dsp::{BiquadDf2t, PcmBuffer, eval_envelope, rbj_coefficients};
use crate::error::{Error, Result};
use crate::model::{Band, CrossoverBandGain, FilterKind};

pub fn apply_crossover_band_gain(
    buffer: &mut PcmBuffer,
    start_frame: i64,
    value: &CrossoverBandGain,
) -> Result<()> {
    if value.control_interval_frames != 64 {
        return Err(Error::new(
            "INVALID_DSP_CONTROL_INTERVAL",
            "v1 crossover control interval must be 64 frames",
        ));
    }
    let expected_bands: &[Band] = match value.crossover_millihz.as_slice() {
        [_] => &[Band::Low, Band::High],
        [_, _] => &[Band::Low, Band::Mid, Band::High],
        _ => {
            return Err(Error::new(
                "INVALID_DSP_CROSSOVER",
                "LR4 crossover requires one or two split frequencies",
            ));
        }
    };
    if value.bands != expected_bands
        || value.band_gain_envelopes.len() != expected_bands.len()
        || value
            .band_gain_envelopes
            .iter()
            .zip(expected_bands)
            .any(|(envelope, band)| envelope.band != *band)
    {
        return Err(Error::new(
            "INVALID_DSP_CROSSOVER",
            "crossover bands and gain envelopes do not match fixed topology",
        ));
    }

    let first = value.crossover_millihz[0];
    let mut low = StereoLr4::new(FilterKind::LowpassBiquadV1, first)?;
    let mut rest = StereoLr4::new(FilterKind::HighpassBiquadV1, first)?;
    let mut second = if value.crossover_millihz.len() == 2 {
        let frequency = value.crossover_millihz[1];
        Some((
            StereoLr4::new(FilterKind::LowpassBiquadV1, frequency)?,
            StereoLr4::new(FilterKind::HighpassBiquadV1, frequency)?,
        ))
    } else {
        None
    };

    for (offset, output) in buffer.frames_mut().iter_mut().enumerate() {
        let frame =
            start_frame
                .checked_add(i64::try_from(offset).map_err(|_| {
                    Error::new("INTEGER_OVERFLOW", "crossover frame offset overflow")
                })?)
                .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "crossover frame overflow"))?;
        let dry = *output;
        let low_band = low.process(dry);
        let rest_band = rest.process(dry);
        let bands = if let Some((mid_filter, high_filter)) = &mut second {
            vec![
                low_band,
                mid_filter.process(rest_band),
                high_filter.process(rest_band),
            ]
        } else {
            vec![low_band, rest_band]
        };
        let mut processed = [0.0, 0.0];
        for (band, envelope) in bands.iter().zip(&value.band_gain_envelopes) {
            let gain = eval_envelope(&envelope.points, &envelope.interpolations, frame)?;
            processed[0] += band[0] * gain;
            processed[1] += band[1] * gain;
        }
        let wet = eval_envelope(&value.wet_points, &value.wet_interpolations, frame)?;
        output[0] = dry[0] * (1.0 - wet) + processed[0] * wet;
        output[1] = dry[1] * (1.0 - wet) + processed[1] * wet;
    }
    Ok(())
}

struct StereoLr4 {
    left: Lr4,
    right: Lr4,
}

impl StereoLr4 {
    fn new(kind: FilterKind, cutoff_millihz: i64) -> Result<Self> {
        Ok(Self {
            left: Lr4::new(kind, cutoff_millihz)?,
            right: Lr4::new(kind, cutoff_millihz)?,
        })
    }

    fn process(&mut self, frame: [f64; 2]) -> [f64; 2] {
        [self.left.process(frame[0]), self.right.process(frame[1])]
    }
}

struct Lr4 {
    first: BiquadDf2t,
    second: BiquadDf2t,
}

impl Lr4 {
    fn new(kind: FilterKind, cutoff_millihz: i64) -> Result<Self> {
        Ok(Self {
            first: BiquadDf2t::new(rbj_coefficients(kind, cutoff_millihz, 707)?),
            second: BiquadDf2t::new(rbj_coefficients(kind, cutoff_millihz, 707)?),
        })
    }

    fn process(&mut self, sample: f64) -> f64 {
        self.second.process(self.first.process(sample))
    }
}
