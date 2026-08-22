use std::{collections::VecDeque, f64::consts::PI, time::Duration};

use thiserror::Error;

use crate::{NUM_CHANNELS, SAMPLE_RATE};

const MIN_TEMPO: f64 = 0.25;
const MAX_TEMPO: f64 = 4.0;

/// One piecewise-constant playback-speed change on a source track timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeedPoint {
    /// Absolute source-track position at which this speed begins applying.
    pub from_position: Duration,
    /// Source frames consumed per wall-clock output frame.
    pub speed: f64,
}

/// Validated source-timeline speed automation.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeedAutomation {
    points: Vec<SpeedPoint>,
}

/// Invalid or currently unsupported speed automation.
#[derive(Clone, Copy, Debug, Error, PartialEq)]
pub enum SpeedAutomationError {
    /// At least one point is required.
    #[error("speed automation must contain at least one point")]
    Empty,
    /// Speeds must be finite, positive, and within the stretcher's supported range.
    #[error("speed automation contains an invalid or unsupported speed")]
    InvalidSpeed,
    /// Source positions must be strictly increasing.
    #[error("speed automation positions must be strictly increasing")]
    UnorderedPositions,
}

impl SpeedAutomation {
    /// Validate piecewise-constant speed points in source-timeline order.
    pub fn new(points: Vec<SpeedPoint>) -> Result<Self, SpeedAutomationError> {
        if points.is_empty() {
            return Err(SpeedAutomationError::Empty);
        }
        if points.iter().any(|point| {
            !point.speed.is_finite() || point.speed < MIN_TEMPO || point.speed > MAX_TEMPO
        }) {
            return Err(SpeedAutomationError::InvalidSpeed);
        }
        if points
            .windows(2)
            .any(|pair| pair[1].from_position <= pair[0].from_position)
        {
            return Err(SpeedAutomationError::UnorderedPositions);
        }
        Ok(Self { points })
    }

    /// Ordered source-timeline automation points.
    pub fn points(&self) -> &[SpeedPoint] {
        &self.points
    }

    /// Speed applying at an absolute source-track position.
    pub fn speed_at(&self, source_position: Duration) -> f64 {
        self.points
            .iter()
            .rev()
            .find(|point| point.from_position <= source_position)
            .map_or(1.0, |point| point.speed)
    }

    /// Source duration consumed during a wall-clock interval starting at `source_start`.
    pub fn source_duration_for_wall_time(
        &self,
        source_start: Duration,
        wall_duration: Duration,
    ) -> Duration {
        let start_frames = duration_to_frames(source_start);
        let wall_frames = duration_to_frames(wall_duration);
        let end_frames = self.advance_source_frames(start_frames, wall_frames);
        frames_to_duration((end_frames - start_frames).max(0.0))
    }

    /// Wall-clock duration needed to consume a source-timeline duration.
    pub fn wall_duration_for_source_time(
        &self,
        source_start: Duration,
        source_duration: Duration,
    ) -> Duration {
        let mut source_frames = duration_to_frames(source_start);
        let source_end = source_frames + duration_to_frames(source_duration);
        let mut wall_frames = 0.0;
        while source_frames < source_end {
            let speed = self.speed_at(frames_to_duration(source_frames));
            let next = self
                .points
                .iter()
                .find(|point| duration_to_frames(point.from_position) > source_frames)
                .map(|point| duration_to_frames(point.from_position).min(source_end))
                .unwrap_or(source_end);
            wall_frames += (next - source_frames) / speed;
            source_frames = next;
        }
        frames_to_duration(wall_frames)
    }

    fn advance_source_frames(&self, mut source_frames: f64, mut wall_frames: f64) -> f64 {
        while wall_frames > 0.0 {
            let source_position = frames_to_duration(source_frames);
            let speed = self.speed_at(source_position);
            let next = self
                .points
                .iter()
                .find(|point| duration_to_frames(point.from_position) > source_frames)
                .map(|point| duration_to_frames(point.from_position));

            let Some(next_source_frames) = next else {
                source_frames += wall_frames * speed;
                break;
            };
            let wall_to_next = (next_source_frames - source_frames) / speed;
            if wall_frames < wall_to_next {
                source_frames += wall_frames * speed;
                break;
            }
            source_frames = next_source_frames;
            wall_frames -= wall_to_next;
        }
        source_frames
    }

    fn needs_stretching(&self) -> bool {
        self.points.iter().any(|point| point.speed != 1.0)
    }
}

/// Streaming Waveform Similarity Overlap-Add over interleaved `f64` PCM.
///
/// The analysis hop follows tempo while the synthesis hop remains fixed. Adjacent grains are
/// correlation-aligned and Hann-crossfaded, changing duration without resampling their waveform.
struct WsolaEngine {
    channels: usize,
    tempo: f64,
    hop: usize,
    frame: usize,
    search: usize,
    input: Vec<f64>,
    input_origin: usize,
    output: VecDeque<f64>,
    overlap: Vec<f64>,
    primed: bool,
    draining: bool,
    ideal_source: f64,
    last_source: usize,
}

impl WsolaEngine {
    fn new(sample_rate: u32, channels: usize) -> Self {
        let hop = ((f64::from(sample_rate) * 0.015).round() as usize).max(1);
        let frame = hop * 2;
        let search = ((f64::from(sample_rate) * 0.006).round() as usize).max(1);
        Self {
            channels,
            tempo: 1.0,
            hop,
            frame,
            search,
            input: Vec::with_capacity(frame * channels * 4),
            input_origin: 0,
            output: VecDeque::with_capacity(frame * channels * 2),
            overlap: vec![0.0; hop * channels],
            primed: false,
            draining: false,
            ideal_source: 0.0,
            last_source: 0,
        }
    }

    fn set_tempo(&mut self, tempo: f64) {
        self.tempo = tempo.clamp(MIN_TEMPO, MAX_TEMPO);
    }

    fn push(&mut self, samples: &[f64]) {
        debug_assert_eq!(samples.len() % self.channels, 0);
        self.input.extend_from_slice(samples);
    }

    fn pull(&mut self, max_samples: usize) -> Vec<f64> {
        while self.output.len() < max_samples && self.step() {}
        let samples = max_samples.min(self.output.len()) / self.channels * self.channels;
        self.output.drain(..samples).collect()
    }

    fn flush(&mut self) -> Vec<f64> {
        self.draining = true;
        while self.step() {}
        if self.primed {
            self.output.extend(self.overlap.iter().copied());
            self.overlap.fill(0.0);
            self.primed = false;
        }
        self.draining = false;
        self.output.drain(..).collect()
    }

    fn step(&mut self) -> bool {
        let available_end = self.input_origin + self.input.len() / self.channels;
        if !self.primed {
            if self.input_origin + self.frame > available_end {
                return false;
            }
            let source = self.input_origin;
            for frame in 0..self.hop {
                for channel in 0..self.channels {
                    self.output.push_back(self.at(source + frame, channel));
                    self.overlap[frame * self.channels + channel] =
                        self.at(source + self.hop + frame, channel);
                }
            }
            self.last_source = source;
            self.ideal_source = source as f64 + self.hop as f64 * self.tempo;
            self.primed = true;
            self.compact();
            return true;
        }

        let base = self.ideal_source.round() as i64;
        let minimum = self.input_origin as i64;
        let maximum = available_end as i64 - self.frame as i64;
        if maximum < minimum {
            return false;
        }
        let unity_tempo = self.tempo == 1.0;
        let required_end =
            base + if unity_tempo { 0 } else { self.search as i64 } + self.frame as i64;
        if !self.draining && required_end > available_end as i64 {
            return false;
        }
        let (first, last) = if unity_tempo {
            if !(minimum..=maximum).contains(&base) {
                return false;
            }
            (base, base)
        } else {
            (
                (base - self.search as i64).max(minimum),
                (base + self.search as i64).min(maximum),
            )
        };
        if last < first {
            return false;
        }

        // At unity the previous nominal grain's tail and this nominal grain's head are the same
        // samples. Selecting the nominal source directly therefore makes their overlap-add
        // transparent and avoids both correlated displacement and the search cost.
        let best_source = if unity_tempo {
            base as usize
        } else {
            let mut best_source = first as usize;
            let mut best_score = f64::NEG_INFINITY;
            for candidate in first..=last {
                let candidate = candidate as usize;
                let mut dot = 0.0;
                let mut overlap_energy = 0.0;
                let mut candidate_energy = 0.0;
                for frame in 0..self.hop {
                    for channel in 0..self.channels {
                        let previous = self.overlap[frame * self.channels + channel];
                        let current = self.at(candidate + frame, channel);
                        dot += previous * current;
                        overlap_energy += previous * previous;
                        candidate_energy += current * current;
                    }
                }
                let score = dot
                    / (overlap_energy * candidate_energy)
                        .sqrt()
                        .max(f64::MIN_POSITIVE);
                if score > best_score {
                    best_score = score;
                    best_source = candidate;
                }
            }
            best_source
        };

        for frame in 0..self.hop {
            let phase = (frame + 1) as f64 / self.hop as f64;
            let fade_in = 0.5 - 0.5 * (PI * phase).cos();
            let fade_out = 1.0 - fade_in;
            for channel in 0..self.channels {
                let index = frame * self.channels + channel;
                let current = self.at(best_source + frame, channel);
                self.output
                    .push_back(self.overlap[index] * fade_out + current * fade_in);
                self.overlap[index] = self.at(best_source + self.hop + frame, channel);
            }
        }
        self.last_source = best_source;
        self.ideal_source += self.hop as f64 * self.tempo;
        self.compact();
        true
    }

    fn at(&self, frame: usize, channel: usize) -> f64 {
        self.input[(frame - self.input_origin) * self.channels + channel]
    }

    fn compact(&mut self) {
        let next_candidate =
            (self.ideal_source.round() as i64 - self.search as i64).max(0) as usize;
        let keep_from = self.last_source.min(next_candidate);
        if keep_from > self.input_origin + (1 << 16) {
            let samples = (keep_from - self.input_origin) * self.channels;
            self.input.drain(..samples);
            self.input_origin = keep_from;
        }
    }
}

/// Stateful pitch-preserving processor owned by one decoded playback source.
pub(crate) struct PitchPreservingTimeStretch {
    automation: SpeedAutomation,
    engine: WsolaEngine,
    output: VecDeque<f64>,
    generated_source_frames: f64,
    emitted_source_frames: f64,
}

impl PitchPreservingTimeStretch {
    pub(crate) fn new(automation: SpeedAutomation, source_position: Duration) -> Option<Self> {
        if !automation.needs_stretching() {
            return None;
        }
        let mut engine = WsolaEngine::new(SAMPLE_RATE, usize::from(NUM_CHANNELS));
        engine.set_tempo(automation.speed_at(source_position));
        Some(Self {
            automation,
            engine,
            output: VecDeque::with_capacity(4096 * usize::from(NUM_CHANNELS)),
            generated_source_frames: duration_to_frames(source_position),
            emitted_source_frames: duration_to_frames(source_position),
        })
    }

    pub(crate) fn push(&mut self, samples: &[f64]) {
        self.engine.push(samples);
    }

    pub(crate) fn fill_output(&mut self, requested_samples: usize) {
        let channels = usize::from(NUM_CHANNELS);
        while self.output.len() < requested_samples {
            let missing_frames = (requested_samples - self.output.len()).div_ceil(channels);
            let speed = self
                .automation
                .speed_at(frames_to_duration(self.generated_source_frames));
            self.engine.set_tempo(speed);
            let frames_to_speed_change = self
                .automation
                .points
                .iter()
                .find(|point| {
                    duration_to_frames(point.from_position) > self.generated_source_frames
                })
                .map(|point| {
                    ((duration_to_frames(point.from_position) - self.generated_source_frames)
                        / speed)
                        .ceil()
                        .max(1.0) as usize
                })
                .unwrap_or(usize::MAX);
            let requested_frames = missing_frames.min(frames_to_speed_change);
            let pulled = self.engine.pull(requested_frames * channels);
            if pulled.is_empty() {
                break;
            }
            let frames = pulled.len() / channels;
            self.output.extend(pulled);
            self.generated_source_frames = self
                .automation
                .advance_source_frames(self.generated_source_frames, frames as f64);
        }
    }

    pub(crate) fn available_samples(&self) -> usize {
        self.output.len()
    }

    pub(crate) fn take(&mut self, samples: usize) -> Vec<f64> {
        let samples: Vec<_> = self.output.drain(..samples).collect();
        let frames = samples.len() / usize::from(NUM_CHANNELS);
        self.emitted_source_frames = self
            .automation
            .advance_source_frames(self.emitted_source_frames, frames as f64);
        samples
    }

    pub(crate) fn flush(&mut self) {
        let tail = self.engine.flush();
        if tail.is_empty() {
            return;
        }
        let frames = tail.len() / usize::from(NUM_CHANNELS);
        self.output.extend(tail);
        self.generated_source_frames = self
            .automation
            .advance_source_frames(self.generated_source_frames, frames as f64);
    }

    pub(crate) fn source_position(&self) -> Duration {
        frames_to_duration(self.emitted_source_frames)
    }
}

fn duration_to_frames(duration: Duration) -> f64 {
    duration.as_secs_f64() * f64::from(SAMPLE_RATE)
}

fn frames_to_duration(frames: f64) -> Duration {
    Duration::from_secs_f64(frames.max(0.0) / f64::from(SAMPLE_RATE))
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use super::*;

    fn automation(points: &[(u64, f64)]) -> SpeedAutomation {
        SpeedAutomation::new(
            points
                .iter()
                .map(|&(position_ms, speed)| SpeedPoint {
                    from_position: Duration::from_millis(position_ms),
                    speed,
                })
                .collect(),
        )
        .unwrap()
    }

    fn stretch_all(input: &[f64], speed: SpeedAutomation) -> Vec<f64> {
        let mut stretch = PitchPreservingTimeStretch::new(speed, Duration::ZERO).unwrap();
        stretch.push(input);
        stretch.fill_output(usize::MAX / 2);
        stretch.flush();
        stretch.take(stretch.available_samples())
    }

    #[test]
    fn unity_speed_does_not_insert_a_processor() {
        assert!(PitchPreservingTimeStretch::new(automation(&[(0, 1.0)]), Duration::ZERO).is_none());
    }

    #[test]
    fn constant_speed_maps_source_time_to_wall_time() {
        let speed = automation(&[(0, 0.90312)]);
        let source = speed.source_duration_for_wall_time(
            Duration::from_millis(2_763),
            Duration::from_millis(6_090),
        );
        assert!(source.abs_diff(Duration::from_millis(5_500)) <= Duration::from_micros(2));
        let wall = speed.wall_duration_for_source_time(
            Duration::from_millis(2_763),
            Duration::from_millis(5_500),
        );
        assert!(wall.abs_diff(Duration::from_millis(6_090)) <= Duration::from_micros(2));
    }

    #[test]
    fn source_clock_applies_piecewise_speed_points_without_interpolation() {
        let speed = automation(&[(0, 0.5), (100, 0.75), (200, 1.0)]);
        let source =
            speed.source_duration_for_wall_time(Duration::ZERO, Duration::from_micros(400_000));

        assert!(source.abs_diff(Duration::from_micros(266_667)) <= Duration::from_micros(2));
        assert_eq!(speed.speed_at(Duration::from_millis(99)), 0.5);
        assert_eq!(speed.speed_at(Duration::from_millis(100)), 0.75);
        assert_eq!(speed.speed_at(Duration::from_millis(200)), 1.0);
    }

    #[test]
    fn speed_automation_rejects_invalid_values_and_order() {
        assert_eq!(
            SpeedAutomation::new(vec![SpeedPoint {
                from_position: Duration::ZERO,
                speed: f64::NAN,
            }]),
            Err(SpeedAutomationError::InvalidSpeed)
        );
        assert_eq!(
            SpeedAutomation::new(vec![SpeedPoint {
                from_position: Duration::ZERO,
                speed: 0.0,
            }]),
            Err(SpeedAutomationError::InvalidSpeed)
        );
        assert_eq!(
            SpeedAutomation::new(vec![SpeedPoint {
                from_position: Duration::ZERO,
                speed: f64::INFINITY,
            }]),
            Err(SpeedAutomationError::InvalidSpeed)
        );
        assert_eq!(
            SpeedAutomation::new(vec![
                SpeedPoint {
                    from_position: Duration::from_millis(10),
                    speed: 1.0,
                },
                SpeedPoint {
                    from_position: Duration::from_millis(10),
                    speed: 0.9,
                },
            ]),
            Err(SpeedAutomationError::UnorderedPositions)
        );
    }

    #[test]
    fn wsola_changes_duration_while_preserving_sine_pitch() {
        let seconds = 2.0;
        let frequency = 440.0;
        let frames = (seconds * f64::from(SAMPLE_RATE)) as usize;
        let mut input = Vec::with_capacity(frames * usize::from(NUM_CHANNELS));
        for frame in 0..frames {
            let sample = (TAU * frequency * frame as f64 / f64::from(SAMPLE_RATE)).sin() * 0.5;
            input.extend([sample, sample]);
        }

        let output = stretch_all(&input, automation(&[(0, 0.90312)]));
        let output_frames = output.len() / usize::from(NUM_CHANNELS);
        let expected_frames = frames as f64 / 0.90312;
        let duration_error_frames = (output_frames as f64 - expected_frames).abs();
        assert!(
            duration_error_frames < 400.0,
            "duration error was {duration_error_frames} frames"
        );

        let left: Vec<_> = output
            .chunks_exact(usize::from(NUM_CHANNELS))
            .map(|frame| frame[0])
            .collect();
        let start = 2_000.min(left.len());
        let crossings = left[start..]
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        let measured_hz = crossings as f64 * f64::from(SAMPLE_RATE) / (left.len() - start) as f64;
        assert!(
            (measured_hz - frequency).abs() < 0.1,
            "pitch was {measured_hz} Hz"
        );
    }

    #[test]
    fn adjacent_speed_changes_remain_crossfaded() {
        let frames = (1.2 * f64::from(SAMPLE_RATE)) as usize;
        let mut input = Vec::with_capacity(frames * usize::from(NUM_CHANNELS));
        for frame in 0..frames {
            let sample = (TAU * 220.0 * frame as f64 / f64::from(SAMPLE_RATE)).sin() * 0.5;
            input.extend([sample, sample]);
        }

        let output = stretch_all(&input, automation(&[(0, 0.90312), (400, 0.95), (800, 1.0)]));
        let maximum_step = output
            .chunks_exact(usize::from(NUM_CHANNELS))
            .map(|frame| frame[0])
            .collect::<Vec<_>>()
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0, f64::max);

        assert!(
            maximum_step < 0.1,
            "largest adjacent-sample step was {maximum_step}"
        );
    }

    #[test]
    fn unity_after_non_unity_uses_nominal_source_without_drift() {
        let mut engine = WsolaEngine::new(SAMPLE_RATE, 1);
        let frames = engine.frame + engine.hop * 20 + engine.search;
        let signal: Vec<_> = (0..frames)
            .map(|frame| {
                let phase = frame as f64 / f64::from(SAMPLE_RATE);
                (TAU * 317.0 * phase).sin() * 0.37
                    + (TAU * 733.0 * phase).cos() * 0.19
                    + (frame % 97) as f64 * 0.000_01
            })
            .collect();
        engine.push(&signal);

        engine.set_tempo(0.90312);
        for _ in 0..6 {
            assert!(engine.step());
            engine.output.clear();
        }

        engine.set_tempo(1.0);
        let first_unity_source = engine.ideal_source.round() as usize;
        assert!(engine.step());
        assert_eq!(engine.last_source, first_unity_source);
        engine.output.clear();

        for step in 1..=8 {
            let nominal_source = engine.ideal_source.round() as usize;
            assert_eq!(nominal_source, first_unity_source + step * engine.hop);
            assert!(engine.step());
            assert_eq!(engine.last_source, nominal_source);

            let output: Vec<_> = engine.output.drain(..).collect();
            assert_eq!(output.len(), engine.hop);
            for (actual, expected) in output
                .iter()
                .zip(&signal[nominal_source..nominal_source + engine.hop])
            {
                assert!(
                    (actual - expected).abs() <= f64::EPSILON,
                    "unity overlap-add changed the waveform: {actual} != {expected}"
                );
            }
        }
    }
}
