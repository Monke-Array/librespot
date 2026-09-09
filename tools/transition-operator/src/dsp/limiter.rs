use crate::dsp::PcmBuffer;
use crate::error::{Error, Result};
use crate::scalar::div_round_nearest_away;
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct LimiterMeasurements {
    pub applied_gains: Vec<f64>,
    pub maximum_gain_reduction_mdb: i64,
    pub active_fraction_ppm: i64,
}

#[derive(Clone, Debug)]
pub struct LookaheadPeakLimiter {
    ceiling: f64,
    lookahead_frames: usize,
    release_multiplier: f64,
    activity_threshold_mdb: i64,
}

impl LookaheadPeakLimiter {
    pub fn new(
        ceiling_mdbfs: i64,
        lookahead_frames: i64,
        release_frames: i64,
        activity_threshold_mdb: i64,
    ) -> Result<Self> {
        if ceiling_mdbfs != -1_200
            || lookahead_frames != 221
            || release_frames != 4_410
            || activity_threshold_mdb != 100
        {
            return Err(Error::new(
                "INVALID_LIMITER_PROFILE",
                "limiter parameters do not match lookahead_peak_limiter_v1",
            ));
        }
        Ok(Self {
            ceiling: 10_f64.powf(ceiling_mdbfs as f64 / 20_000.0),
            lookahead_frames: lookahead_frames as usize,
            release_multiplier: 100_f64.powf(1.0 / release_frames as f64),
            activity_threshold_mdb,
        })
    }

    pub fn process(
        &self,
        buffer: &mut PcmBuffer,
        buffer_start_frame: i64,
        analysis_start_frame: i64,
        analysis_end_frame: i64,
    ) -> Result<LimiterMeasurements> {
        let analysis_start = timeline_index(
            analysis_start_frame,
            buffer_start_frame,
            buffer.frame_count(),
            true,
        )?;
        let analysis_end = timeline_index(
            analysis_end_frame,
            buffer_start_frame,
            buffer.frame_count(),
            true,
        )?;
        if analysis_start >= analysis_end {
            return Err(Error::new(
                "INVALID_LIMITER_ANALYSIS_WINDOW",
                "limiter activity window must be nonempty",
            ));
        }
        let peaks: Vec<_> = buffer
            .frames()
            .iter()
            .map(|frame| frame[0].abs().max(frame[1].abs()))
            .collect();
        if peaks.iter().any(|peak| !peak.is_finite()) {
            return Err(Error::new(
                "NON_FINITE_PCM",
                "limiter input contains a nonfinite sample",
            ));
        }

        let mut maximum = VecDeque::<usize>::new();
        let mut next_to_add = 0usize;
        let mut previous_gain = 1.0;
        let mut applied_gains = Vec::with_capacity(peaks.len());
        let mut maximum_reduction = 0_i64;
        let mut active_frames = 0usize;
        for index in 0..peaks.len() {
            let inclusive_end = index
                .saturating_add(self.lookahead_frames)
                .min(peaks.len().saturating_sub(1));
            while next_to_add <= inclusive_end && next_to_add < peaks.len() {
                while maximum
                    .back()
                    .is_some_and(|previous| peaks[*previous] <= peaks[next_to_add])
                {
                    maximum.pop_back();
                }
                maximum.push_back(next_to_add);
                next_to_add += 1;
            }
            while maximum.front().is_some_and(|position| *position < index) {
                maximum.pop_front();
            }
            let peak = maximum.front().map_or(0.0, |position| peaks[*position]);
            let required_gain = if peak == 0.0 {
                1.0
            } else {
                (self.ceiling / peak).min(1.0)
            };
            let released_gain = (previous_gain * self.release_multiplier).min(1.0);
            let gain = required_gain.min(released_gain);
            if gain <= 0.0 || !gain.is_finite() {
                return Err(Error::new(
                    "INVALID_LIMITER_GAIN",
                    "limiter produced a nonpositive or nonfinite gain",
                ));
            }
            let reduction = (-20_000.0 * gain.log10()).round() as i64;
            maximum_reduction = maximum_reduction.max(reduction);
            if (analysis_start..analysis_end).contains(&index)
                && reduction > self.activity_threshold_mdb
            {
                active_frames += 1;
            }
            applied_gains.push(gain);
            previous_gain = gain;
        }
        for (frame, gain) in buffer.frames_mut().iter_mut().zip(&applied_gains) {
            frame[0] *= gain;
            frame[1] *= gain;
        }
        let analysis_frames = analysis_end - analysis_start;
        let active_numerator = i64::try_from(active_frames)
            .ok()
            .and_then(|value| value.checked_mul(1_000_000))
            .ok_or_else(|| {
                Error::new(
                    "INTEGER_OVERFLOW",
                    "limiter activity numerator exceeds signed 64-bit range",
                )
            })?;
        let analysis_denominator = i64::try_from(analysis_frames).map_err(|_| {
            Error::new(
                "INTEGER_OVERFLOW",
                "limiter analysis length exceeds signed 64-bit range",
            )
        })?;
        let active_fraction_ppm = div_round_nearest_away(active_numerator, analysis_denominator)?;
        Ok(LimiterMeasurements {
            applied_gains,
            maximum_gain_reduction_mdb: maximum_reduction,
            active_fraction_ppm,
        })
    }
}

pub fn apply_pair_gain(buffer: &mut PcmBuffer, gain_mdb: i64) -> Result<()> {
    if !(-24_000..=0).contains(&gain_mdb) {
        return Err(Error::new(
            "INVALID_PAIR_OUTPUT_GAIN",
            "pair output gain must be nonboosting and within v1 bounds",
        ));
    }
    let gain = 10_f64.powf(gain_mdb as f64 / 20_000.0);
    for frame in buffer.frames_mut() {
        frame[0] *= gain;
        frame[1] *= gain;
    }
    Ok(())
}

fn timeline_index(frame: i64, start: i64, frame_count: usize, allow_end: bool) -> Result<usize> {
    let offset = frame
        .checked_sub(start)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| {
            Error::new(
                "INVALID_LIMITER_ANALYSIS_WINDOW",
                "limiter activity window is outside rendered PCM",
            )
        })?;
    if offset > frame_count || (!allow_end && offset == frame_count) {
        return Err(Error::new(
            "INVALID_LIMITER_ANALYSIS_WINDOW",
            "limiter activity window is outside rendered PCM",
        ));
    }
    Ok(offset)
}
