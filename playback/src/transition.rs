use std::{cmp::Ordering, f64::consts::FRAC_PI_2, time::Duration};

use thiserror::Error;

use crate::{SpeedAutomation, decoder::AudioPacket};

/// Selects whether the player should prepare a transition and, if so, its shape.
///
/// Policy deliberately has no access to the sink or PCM. It decides when and for how long a
/// transition should run; [`TransitionEngine`] is solely responsible for rendering it.
pub(crate) trait TransitionPolicy {
    fn plan(
        &self,
        current_position: Duration,
        current_duration: Duration,
    ) -> Option<TransitionSpec>;

    /// Whether an armed transition should begin consuming both sources now.
    fn should_start(&self, current_position: Duration, current_duration: Duration) -> bool;
}

/// One control point in a normalized transition gain envelope.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GainPoint {
    /// Position local to a segment, normally in the inclusive range 0..=1.
    pub x: f64,
    /// Linear source gain at this control point.
    pub y: f64,
}

/// A piecewise Bezier segment in a normalized transition gain envelope.
#[derive(Clone, Debug, PartialEq)]
pub struct GainCurveSegment {
    /// Start position within the complete transition, in the range 0..=1.
    pub start: f64,
    /// End position within the complete transition, in the range 0..=1.
    pub end: f64,
    /// Two, three, or four Bezier control points local to this segment.
    pub points: Vec<GainPoint>,
}

/// A reusable, normalized source-gain envelope made from piecewise Bezier segments.
#[derive(Clone, Debug, PartialEq)]
pub struct GainCurve {
    segments: Vec<GainCurveSegment>,
}

/// Validation error for a generic transition plan or gain curve.
#[derive(Debug, Error, PartialEq)]
pub enum TransitionPlanError {
    /// A saved plan cannot have a zero-duration overlap.
    #[error("transition duration must be greater than zero")]
    EmptyDuration,
    /// A gain envelope needs at least one segment.
    #[error("gain curve must contain at least one segment")]
    EmptyGainCurve,
    /// Segment bounds must be finite, normalized, and increasing.
    #[error("gain curve segment bounds are invalid")]
    InvalidSegmentBounds,
    /// Current Automix envelopes use line, quadratic, or cubic Bezier segments.
    #[error("gain curve segment must contain two, three, or four points")]
    InvalidPointCount,
    /// Control-point values must be finite and x positions must be normalized and monotonic.
    #[error("gain curve contains an invalid control point")]
    InvalidPoint,
    /// Segments must not overlap after sorting by start position.
    #[error("gain curve segments overlap")]
    OverlappingSegments,
    /// Evaluation requires a finite transition progress.
    #[error("gain curve progress must be finite")]
    InvalidProgress,
}

impl GainCurve {
    /// Validate and construct a generic gain envelope.
    pub fn new(mut segments: Vec<GainCurveSegment>) -> Result<Self, TransitionPlanError> {
        if segments.is_empty() {
            return Err(TransitionPlanError::EmptyGainCurve);
        }

        for segment in &segments {
            if !segment.start.is_finite()
                || !segment.end.is_finite()
                || !(0.0..=1.0).contains(&segment.start)
                || !(0.0..=1.0).contains(&segment.end)
                || segment.start >= segment.end
            {
                return Err(TransitionPlanError::InvalidSegmentBounds);
            }
            if !(2..=4).contains(&segment.points.len()) {
                return Err(TransitionPlanError::InvalidPointCount);
            }

            let mut previous_x = f64::NEG_INFINITY;
            for point in &segment.points {
                if !point.x.is_finite()
                    || !point.y.is_finite()
                    || !(0.0..=1.0).contains(&point.x)
                    || point.x < previous_x
                {
                    return Err(TransitionPlanError::InvalidPoint);
                }
                previous_x = point.x;
            }
        }

        segments.sort_by(|left, right| {
            left.start
                .partial_cmp(&right.start)
                .unwrap_or(Ordering::Equal)
        });
        if segments.windows(2).any(|pair| pair[1].start < pair[0].end) {
            return Err(TransitionPlanError::OverlappingSegments);
        }

        Ok(Self { segments })
    }

    /// Return the normalized linear gain at a transition progress, clamped to 0..=1.
    pub fn gain_at(&self, progress: f64) -> Result<f64, TransitionPlanError> {
        if !progress.is_finite() {
            return Err(TransitionPlanError::InvalidProgress);
        }
        let progress = progress.clamp(0.0, 1.0);
        let first = self
            .segments
            .first()
            .expect("validated gain curve is non-empty");
        if progress <= first.start {
            return Ok(Self::segment_gain(first, 0.0));
        }

        let mut previous = first;
        for (index, segment) in self.segments.iter().enumerate() {
            if progress < segment.start {
                return Ok(Self::segment_gain(previous, 1.0));
            }
            if progress < segment.end || index == self.segments.len() - 1 {
                let local = (progress - segment.start) / (segment.end - segment.start);
                return Ok(Self::segment_gain(segment, local));
            }
            previous = segment;
        }

        Ok(Self::segment_gain(previous, 1.0))
    }

    fn segment_gain(segment: &GainCurveSegment, target_x: f64) -> f64 {
        let points = &segment.points;
        let first_x = points.first().expect("validated segment has points").x;
        let last_x = points.last().expect("validated segment has points").x;
        let target_x = target_x.clamp(first_x, last_x);

        // Repeated x coordinates encode a vertical edge. Treat the control points as a
        // right-continuous polyline so the discontinuity remains a step instead of being
        // smoothed by the Bezier solver.
        if points.windows(2).any(|pair| pair[0].x == pair[1].x) {
            return Self::stepped_polyline_coordinate(points, target_x).clamp(0.0, 1.0);
        }

        // Spotify's editor emits uniformly spaced x controls, for which Bezier x(t) == t.
        // Avoid an iterative solve on every audio frame in that common case.
        let denominator = (points.len() - 1) as f64;
        let uniform_x = points
            .iter()
            .enumerate()
            .all(|(index, point)| (point.x - index as f64 / denominator).abs() <= 1e-9);
        let parameter = if uniform_x {
            target_x
        } else {
            let mut low = 0.0;
            let mut high = 1.0;
            for _ in 0..24 {
                let middle = (low + high) * 0.5;
                if Self::bezier_coordinate(points, middle, |point| point.x) < target_x {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            (low + high) * 0.5
        };

        Self::bezier_coordinate(points, parameter, |point| point.y).clamp(0.0, 1.0)
    }

    fn stepped_polyline_coordinate(points: &[GainPoint], target_x: f64) -> f64 {
        let right = points.partition_point(|point| point.x <= target_x);
        if right == 0 {
            return points[0].y;
        }
        if right == points.len() {
            return points[right - 1].y;
        }

        let left = points[right - 1];
        let right = points[right];
        let progress = (target_x - left.x) / (right.x - left.x);
        left.y + (right.y - left.y) * progress
    }

    fn bezier_coordinate(
        points: &[GainPoint],
        parameter: f64,
        coordinate: impl Fn(&GainPoint) -> f64,
    ) -> f64 {
        let mut values = [0.0; 4];
        for (value, point) in values.iter_mut().zip(points) {
            *value = coordinate(point);
        }
        for remaining in (1..points.len()).rev() {
            for index in 0..remaining {
                values[index] = values[index] * (1.0 - parameter) + values[index + 1] * parameter;
            }
        }
        values[0]
    }
}

/// A scheduled transition plan independent of Spotify's protobuf representation.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionPlan {
    current_start: Duration,
    next_start: Duration,
    duration: Duration,
    current_gain: GainCurve,
    next_gain: GainCurve,
    next_speed: Option<SpeedAutomation>,
}

impl TransitionPlan {
    /// Construct a scheduled transition using already validated gain envelopes.
    pub fn new(
        current_start: Duration,
        next_start: Duration,
        duration: Duration,
        current_gain: GainCurve,
        next_gain: GainCurve,
    ) -> Result<Self, TransitionPlanError> {
        if duration.is_zero() {
            return Err(TransitionPlanError::EmptyDuration);
        }
        Ok(Self {
            current_start,
            next_start,
            duration,
            current_gain,
            next_gain,
            next_speed: None,
        })
    }

    /// Attach validated source-timeline speed automation for the incoming source.
    pub fn with_next_speed_automation(mut self, automation: SpeedAutomation) -> Self {
        self.next_speed = Some(automation);
        self
    }

    /// Position at which the outgoing source begins the transition.
    pub fn current_start(&self) -> Duration {
        self.current_start
    }

    /// Position from which the incoming source is preloaded.
    pub fn next_start(&self) -> Duration {
        self.next_start
    }

    /// Exact overlap duration.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Incoming source-timeline speed automation, when pitch-preserving stretching is required.
    pub fn next_speed_automation(&self) -> Option<&SpeedAutomation> {
        self.next_speed.as_ref()
    }

    fn spec(&self) -> TransitionSpec {
        TransitionSpec {
            duration: self.duration,
            curve: TransitionCurve::GainCurves {
                current: self.current_gain.clone(),
                next: self.next_gain.clone(),
            },
            current_gain: 1.0,
            next_gain: 1.0,
        }
    }
}

/// Selects a caller-provided scheduled plan while retaining the same bounded preparation window
/// used by the fixed-duration fallback.
pub(crate) struct ScheduledTransitionPolicy<'a> {
    plan: &'a TransitionPlan,
    preparation: Duration,
}

impl<'a> ScheduledTransitionPolicy<'a> {
    pub(crate) fn new(plan: &'a TransitionPlan, preparation: Duration) -> Self {
        Self { plan, preparation }
    }
}

impl TransitionPolicy for ScheduledTransitionPolicy<'_> {
    fn plan(
        &self,
        current_position: Duration,
        _current_duration: Duration,
    ) -> Option<TransitionSpec> {
        let preparation_start = self.plan.current_start.saturating_sub(self.preparation);
        let transition_end = self.plan.current_start.saturating_add(self.plan.duration);
        (current_position >= preparation_start && current_position < transition_end)
            .then(|| self.plan.spec())
    }

    fn should_start(&self, current_position: Duration, _current_duration: Duration) -> bool {
        current_position >= self.plan.current_start
    }
}

/// The default policy preserves librespot's existing one-track playback behavior.
#[derive(Debug, Default)]
#[allow(dead_code)] // Kept as the one-line opt-out/legacy policy and exercised in tests.
pub(crate) struct NoTransitionPolicy;

impl TransitionPolicy for NoTransitionPolicy {
    fn plan(
        &self,
        _current_position: Duration,
        _current_duration: Duration,
    ) -> Option<TransitionSpec> {
        None
    }

    fn should_start(&self, _current_position: Duration, _current_duration: Duration) -> bool {
        false
    }
}

/// A deliberately simple transition policy with a fixed overlap near natural EOF.
#[derive(Debug)]
pub(crate) struct FixedDurationTransitionPolicy {
    duration: Duration,
    preparation: Duration,
}

impl FixedDurationTransitionPolicy {
    pub(crate) const FIVE_SECONDS: Duration = Duration::from_secs(5);

    /// `preparation` gives the bounded secondary worker time to produce its first PCM chunk.
    pub(crate) fn new(duration: Duration, preparation: Duration) -> Self {
        Self {
            duration,
            preparation,
        }
    }

    fn remaining(current_position: Duration, current_duration: Duration) -> Option<Duration> {
        current_duration.checked_sub(current_position)
    }
}

impl Default for FixedDurationTransitionPolicy {
    fn default() -> Self {
        Self::new(Self::FIVE_SECONDS, Duration::from_millis(200))
    }
}

impl TransitionPolicy for FixedDurationTransitionPolicy {
    fn plan(
        &self,
        current_position: Duration,
        current_duration: Duration,
    ) -> Option<TransitionSpec> {
        let remaining = Self::remaining(current_position, current_duration)?;
        if remaining > self.duration.saturating_add(self.preparation) {
            return None;
        }

        Some(TransitionSpec {
            duration: self.duration,
            curve: TransitionCurve::Linear,
            current_gain: 1.0,
            next_gain: 1.0,
        })
    }

    fn should_start(&self, current_position: Duration, current_duration: Duration) -> bool {
        Self::remaining(current_position, current_duration)
            .is_some_and(|remaining| remaining <= self.duration)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransitionState {
    Idle,
    Armed,
    Active,
    Finishing,
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)] // Equal-power remains available for the next policy iteration.
pub(crate) enum TransitionCurve {
    Linear,
    EqualPower,
    GainCurves { current: GainCurve, next: GainCurve },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TransitionSpec {
    pub duration: Duration,
    pub curve: TransitionCurve,
    pub current_gain: f64,
    pub next_gain: f64,
}

#[derive(Debug, Error, PartialEq)]
pub(crate) enum TransitionError {
    #[error("transition duration must be greater than zero")]
    EmptyDuration,
    #[error("transition duration is too large")]
    DurationTooLarge,
    #[error("transition source gains must be finite")]
    InvalidGain,
    #[error("transition gain curve evaluation failed")]
    InvalidGainCurve,
    #[error("cannot {operation} while transition is {state:?}")]
    InvalidState {
        operation: &'static str,
        state: TransitionState,
    },
    #[error("active transition requires both PCM sources")]
    MissingSource,
    #[error("active transition cannot mix raw encoded packets")]
    EncodedPacket,
    #[error("transition inputs have different sample counts ({current} and {next})")]
    MismatchedSampleCounts { current: usize, next: usize },
    #[error("transition input has {samples} samples, not aligned to {channels} channels")]
    UnalignedSamples { samples: usize, channels: usize },
}

/// Stateful renderer for interleaved PCM from a current and a next source.
///
/// In `Idle` and `Armed`, a current-only packet is returned untouched (including its allocation).
/// Once a next-source packet arrives for an armed transition, equally sized, frame-aligned PCM
/// packets are mixed. Packet alignment remains the responsibility of the future dual-decoder
/// scheduler, keeping decoder ownership and timing out of this signal-processing layer.
pub(crate) struct TransitionEngine {
    state: TransitionState,
    sample_rate: u32,
    channels: usize,
    spec: Option<TransitionSpec>,
    elapsed_frames: usize,
    total_frames: usize,
}

impl TransitionEngine {
    pub(crate) fn new(sample_rate: u32, channels: usize) -> Self {
        assert!(sample_rate > 0, "transition sample rate must be non-zero");
        assert!(channels > 0, "transition channel count must be non-zero");

        Self {
            state: TransitionState::Idle,
            sample_rate,
            channels,
            spec: None,
            elapsed_frames: 0,
            total_frames: 0,
        }
    }

    pub(crate) fn state(&self) -> TransitionState {
        self.state
    }

    #[cfg(test)]
    pub(crate) fn elapsed_frames(&self) -> usize {
        self.elapsed_frames
    }

    #[cfg(test)]
    pub(crate) fn total_frames(&self) -> usize {
        self.total_frames
    }

    pub(crate) fn remaining_frames(&self) -> usize {
        self.total_frames.saturating_sub(self.elapsed_frames)
    }

    pub(crate) fn arm(&mut self, spec: TransitionSpec) -> Result<(), TransitionError> {
        if self.state != TransitionState::Idle {
            return Err(self.invalid_state("arm"));
        }
        if spec.duration.is_zero() {
            return Err(TransitionError::EmptyDuration);
        }
        if !spec.current_gain.is_finite() || !spec.next_gain.is_finite() {
            return Err(TransitionError::InvalidGain);
        }

        let duration_nanos = spec.duration.as_nanos();
        let scaled_frames = duration_nanos
            .checked_mul(u128::from(self.sample_rate))
            .ok_or(TransitionError::DurationTooLarge)?;
        let total_frames = scaled_frames.div_ceil(1_000_000_000);
        self.total_frames =
            usize::try_from(total_frames).map_err(|_| TransitionError::DurationTooLarge)?;
        self.elapsed_frames = 0;
        debug!(
            "Transition armed: duration {:?}, curve {:?}, current gain {}, next gain {}",
            spec.duration, spec.curve, spec.current_gain, spec.next_gain
        );
        self.spec = Some(spec);
        self.state = TransitionState::Armed;
        Ok(())
    }

    /// Render one packet from the current source, optionally with a simultaneous next-source
    /// packet. The no-transition path is a move-only passthrough and performs no PCM conversion.
    pub(crate) fn render(
        &mut self,
        current: AudioPacket,
        next: Option<AudioPacket>,
    ) -> Result<AudioPacket, TransitionError> {
        match self.state {
            TransitionState::Idle => {
                if next.is_some() {
                    return Err(self.invalid_state("accept a second source"));
                }
                return Ok(current);
            }
            TransitionState::Armed if next.is_none() => return Ok(current),
            TransitionState::Armed => {
                debug!("Transition second source ready");
                self.state = TransitionState::Active;
                debug!("Transition started");
            }
            TransitionState::Active => {}
            TransitionState::Finishing => return Err(self.invalid_state("render")),
        }

        let next = next.ok_or(TransitionError::MissingSource)?;
        self.mix_pcm(current, next)
    }

    /// Reset any armed or active transition. Calling this while idle is intentionally harmless,
    /// which makes seek/stop/load cancellation paths idempotent.
    pub(crate) fn cancel(&mut self, reason: &str) {
        if self.state() != TransitionState::Idle {
            debug!("Transition cancelled while {:?}: {reason}", self.state());
            self.reset();
        }
    }

    /// A dual-stream owner calls this after promoting the next decoder to current.
    pub(crate) fn complete(&mut self) -> Result<(), TransitionError> {
        if self.state != TransitionState::Finishing {
            return Err(self.invalid_state("complete"));
        }
        debug!("Transition completed");
        self.reset();
        Ok(())
    }

    fn mix_pcm(
        &mut self,
        current: AudioPacket,
        next: AudioPacket,
    ) -> Result<AudioPacket, TransitionError> {
        let (AudioPacket::Samples(mut current), AudioPacket::Samples(next)) = (current, next)
        else {
            return Err(TransitionError::EncodedPacket);
        };

        if current.len() != next.len() {
            return Err(TransitionError::MismatchedSampleCounts {
                current: current.len(),
                next: next.len(),
            });
        }
        if current.len() % self.channels != 0 {
            return Err(TransitionError::UnalignedSamples {
                samples: current.len(),
                channels: self.channels,
            });
        }

        let spec = self
            .spec
            .as_ref()
            .expect("active transition must retain its specification");
        let was_active = self.elapsed_frames < self.total_frames;

        for (current_frame, next_frame) in current
            .chunks_exact_mut(self.channels)
            .zip(next.chunks_exact(self.channels))
        {
            if self.elapsed_frames < self.total_frames {
                let progress = if self.total_frames == 1 {
                    1.0
                } else {
                    self.elapsed_frames as f64 / (self.total_frames - 1) as f64
                };
                let (current_fade, next_fade) = match &spec.curve {
                    TransitionCurve::Linear => (1.0 - progress, progress),
                    TransitionCurve::EqualPower => {
                        let angle = progress * FRAC_PI_2;
                        (angle.cos(), angle.sin())
                    }
                    TransitionCurve::GainCurves { current, next } => (
                        current
                            .gain_at(progress)
                            .map_err(|_| TransitionError::InvalidGainCurve)?,
                        next.gain_at(progress)
                            .map_err(|_| TransitionError::InvalidGainCurve)?,
                    ),
                };

                for (current_sample, next_sample) in current_frame.iter_mut().zip(next_frame.iter())
                {
                    *current_sample = *current_sample * current_fade * spec.current_gain
                        + *next_sample * next_fade * spec.next_gain;
                }
                self.elapsed_frames += 1;
            } else {
                for (current_sample, next_sample) in current_frame.iter_mut().zip(next_frame.iter())
                {
                    *current_sample = *next_sample * spec.next_gain;
                }
            }
        }

        if was_active && self.elapsed_frames == self.total_frames {
            self.state = TransitionState::Finishing;
            debug!("Transition mix finished; awaiting next-source promotion");
        }

        Ok(AudioPacket::Samples(current))
    }

    fn invalid_state(&self, operation: &'static str) -> TransitionError {
        TransitionError::InvalidState {
            operation,
            state: self.state,
        }
    }

    fn reset(&mut self) {
        self.state = TransitionState::Idle;
        self.spec = None;
        self.elapsed_frames = 0;
        self.total_frames = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(duration_secs: u64, curve: TransitionCurve) -> TransitionSpec {
        TransitionSpec {
            duration: Duration::from_secs(duration_secs),
            curve,
            current_gain: 1.0,
            next_gain: 1.0,
        }
    }

    fn into_samples(packet: AudioPacket) -> Vec<f64> {
        match packet {
            AudioPacket::Samples(samples) => samples,
            AudioPacket::Raw(_) => panic!("expected PCM samples"),
        }
    }

    fn smooth_gain(from: f64, to: f64) -> GainCurve {
        GainCurve::new(vec![GainCurveSegment {
            start: 0.0,
            end: 1.0,
            points: vec![
                GainPoint { x: 0.0, y: from },
                GainPoint {
                    x: 1.0 / 3.0,
                    y: from,
                },
                GainPoint {
                    x: 2.0 / 3.0,
                    y: to,
                },
                GainPoint { x: 1.0, y: to },
            ],
        }])
        .expect("test curve should validate")
    }

    fn gain_at(curve: &GainCurve, progress: f64) -> f64 {
        curve
            .gain_at(progress)
            .expect("test progress should evaluate")
    }

    fn one_segment(points: &[(f64, f64)]) -> GainCurve {
        GainCurve::new(vec![GainCurveSegment {
            start: 0.0,
            end: 1.0,
            points: points.iter().map(|&(x, y)| GainPoint { x, y }).collect(),
        }])
        .expect("test curve should validate")
    }

    #[test]
    fn bezier_gain_curve_interpolates_spotify_editor_shape() {
        let curve = smooth_gain(0.0, 1.0);
        assert_eq!(gain_at(&curve, 0.0), 0.0);
        assert!((gain_at(&curve, 0.25) - 0.15625).abs() < 1e-12);
        assert!((gain_at(&curve, 0.5) - 0.5).abs() < 1e-12);
        assert!((gain_at(&curve, 0.75) - 0.84375).abs() < 1e-12);
        assert_eq!(gain_at(&curve, 1.0), 1.0);
    }

    #[test]
    fn gain_curve_evaluates_constant_linear_and_quadratic_segments() {
        let constant = one_segment(&[(0.0, 0.25), (1.0, 0.25)]);
        assert_eq!(gain_at(&constant, -1.0), 0.25);
        assert_eq!(gain_at(&constant, 0.5), 0.25);
        assert_eq!(gain_at(&constant, 2.0), 0.25);

        let linear = one_segment(&[(0.0, 1.0), (1.0, 0.0)]);
        assert_eq!(gain_at(&linear, 0.0), 1.0);
        assert_eq!(gain_at(&linear, 0.25), 0.75);
        assert_eq!(gain_at(&linear, 1.0), 0.0);

        let quadratic = one_segment(&[(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)]);
        assert_eq!(gain_at(&quadratic, 0.0), 0.0);
        assert_eq!(gain_at(&quadratic, 0.5), 0.5);
        assert_eq!(gain_at(&quadratic, 1.0), 0.0);
    }

    #[test]
    fn captured_volume_steps_are_right_continuous() {
        let outgoing = one_segment(&[(0.0, 1.0), (0.6, 1.0), (0.6, 0.0), (1.0, 0.0)]);
        assert_eq!(gain_at(&outgoing, 0.0), 1.0);
        assert_eq!(gain_at(&outgoing, 0.6 - 1.0e-9), 1.0);
        assert_eq!(gain_at(&outgoing, 0.6), 0.0);
        assert_eq!(gain_at(&outgoing, 0.6 + 1.0e-9), 0.0);
        assert_eq!(gain_at(&outgoing, 1.0), 0.0);

        let incoming = one_segment(&[(0.0, 0.0), (0.4, 0.0), (0.4, 1.0), (1.0, 1.0)]);
        assert_eq!(gain_at(&incoming, 0.0), 0.0);
        assert_eq!(gain_at(&incoming, 0.4 - 1.0e-9), 0.0);
        assert_eq!(gain_at(&incoming, 0.4), 1.0);
        assert_eq!(gain_at(&incoming, 0.4 + 1.0e-9), 1.0);
        assert_eq!(gain_at(&incoming, 1.0), 1.0);
    }

    #[test]
    fn gain_curve_maps_global_progress_into_piecewise_segments() {
        let curve = GainCurve::new(vec![
            GainCurveSegment {
                start: 0.0,
                end: 0.25,
                points: vec![GainPoint { x: 0.0, y: 0.0 }, GainPoint { x: 1.0, y: 0.5 }],
            },
            GainCurveSegment {
                start: 0.25,
                end: 1.0,
                points: vec![GainPoint { x: 0.0, y: 0.75 }, GainPoint { x: 1.0, y: 1.0 }],
            },
        ])
        .expect("piecewise curve should validate");

        assert_eq!(gain_at(&curve, 0.125), 0.25);
        assert!(gain_at(&curve, 0.25 - 1.0e-9) < 0.500_000_001);
        assert_eq!(gain_at(&curve, 0.25), 0.75);
        assert_eq!(gain_at(&curve, 0.625), 0.875);
    }

    #[test]
    fn gain_curve_rejects_invalid_topology_and_progress() {
        let invalid_bounds = GainCurve::new(vec![GainCurveSegment {
            start: f64::NAN,
            end: 1.0,
            points: vec![GainPoint { x: 0.0, y: 0.0 }, GainPoint { x: 1.0, y: 1.0 }],
        }]);
        assert_eq!(
            invalid_bounds,
            Err(TransitionPlanError::InvalidSegmentBounds)
        );

        let invalid_point = GainCurve::new(vec![GainCurveSegment {
            start: 0.0,
            end: 1.0,
            points: vec![
                GainPoint {
                    x: 0.0,
                    y: f64::INFINITY,
                },
                GainPoint { x: 1.0, y: 1.0 },
            ],
        }]);
        assert_eq!(invalid_point, Err(TransitionPlanError::InvalidPoint));

        let curve = one_segment(&[(0.0, 0.0), (1.0, 1.0)]);
        assert_eq!(
            curve.gain_at(f64::NAN),
            Err(TransitionPlanError::InvalidProgress)
        );
        assert_eq!(
            curve.gain_at(f64::INFINITY),
            Err(TransitionPlanError::InvalidProgress)
        );
    }

    #[test]
    fn transition_engine_applies_custom_gain_curves() {
        let mut engine = TransitionEngine::new(3, 1);
        engine
            .arm(TransitionSpec {
                duration: Duration::from_secs(1),
                curve: TransitionCurve::GainCurves {
                    current: smooth_gain(1.0, 0.0),
                    next: smooth_gain(0.0, 1.0),
                },
                current_gain: 1.0,
                next_gain: 1.0,
            })
            .expect("custom transition should arm");

        let output = engine
            .render(
                AudioPacket::Samples(vec![1.0; 3]),
                Some(AudioPacket::Samples(vec![0.0; 3])),
            )
            .expect("custom transition should render");
        assert_eq!(into_samples(output), [1.0, 0.5, 0.0]);
    }

    #[test]
    fn transition_engine_applies_source_gain_curves_independently_before_summing() {
        let mut engine = TransitionEngine::new(3, 1);
        engine
            .arm(TransitionSpec {
                duration: Duration::from_secs(1),
                curve: TransitionCurve::GainCurves {
                    current: one_segment(&[(0.0, 0.25), (1.0, 0.25)]),
                    next: one_segment(&[(0.0, 0.5), (1.0, 0.5)]),
                },
                current_gain: 1.0,
                next_gain: 1.0,
            })
            .expect("custom transition should arm");

        let output = engine
            .render(
                AudioPacket::Samples(vec![2.0; 3]),
                Some(AudioPacket::Samples(vec![3.0; 3])),
            )
            .expect("custom transition should render");

        assert_eq!(into_samples(output), [2.0, 2.0, 2.0]);
    }

    #[test]
    fn scheduled_policy_prepares_and_starts_at_saved_position() {
        let plan = TransitionPlan::new(
            Duration::from_secs(90),
            Duration::from_secs(12),
            Duration::from_secs(8),
            smooth_gain(1.0, 0.0),
            smooth_gain(0.0, 1.0),
        )
        .expect("test plan should validate");
        let policy = ScheduledTransitionPolicy::new(&plan, Duration::from_millis(200));

        assert_eq!(
            policy.plan(Duration::from_millis(89_799), Duration::from_secs(100)),
            None
        );
        assert!(
            policy
                .plan(Duration::from_millis(89_800), Duration::from_secs(100))
                .is_some()
        );
        assert!(!policy.should_start(Duration::from_millis(89_999), Duration::from_secs(100)));
        assert!(policy.should_start(Duration::from_millis(90_000), Duration::from_secs(100)));
    }

    #[test]
    fn default_policy_selects_no_transition() {
        assert_eq!(
            NoTransitionPolicy.plan(Duration::from_secs(150), Duration::from_secs(180)),
            None
        );
    }

    #[test]
    fn fixed_policy_prepares_then_starts_at_five_seconds_remaining() {
        let policy = FixedDurationTransitionPolicy::default();
        let duration = Duration::from_secs(100);

        assert_eq!(policy.plan(Duration::from_millis(94_799), duration), None);
        let spec = policy
            .plan(Duration::from_millis(94_800), duration)
            .expect("bounded preparation should begin 200 ms before the overlap");
        assert_eq!(spec.duration, Duration::from_secs(5));
        assert_eq!(spec.curve, TransitionCurve::Linear);
        assert!(!policy.should_start(Duration::from_millis(94_999), duration));
        assert!(policy.should_start(Duration::from_millis(95_000), duration));
    }

    #[test]
    fn five_second_transition_has_exact_frame_count() {
        let mut engine = TransitionEngine::new(44_100, 2);
        let spec = FixedDurationTransitionPolicy::default()
            .plan(Duration::from_secs(95), Duration::from_secs(100))
            .expect("policy should select the five-second transition");
        engine.arm(spec).expect("fixed transition should arm");

        assert_eq!(engine.total_frames(), 220_500);
        let samples = 220_500 * 2;
        let _ = engine
            .render(
                AudioPacket::Samples(vec![1.0; samples]),
                Some(AudioPacket::Samples(vec![0.0; samples])),
            )
            .expect("exact five-second PCM should render");
        assert_eq!(engine.elapsed_frames(), 220_500);
        assert_eq!(engine.state(), TransitionState::Finishing);
    }

    #[test]
    fn linear_fade_has_exact_endpoints_and_midpoint() {
        let mut engine = TransitionEngine::new(3, 1);
        engine
            .arm(spec(1, TransitionCurve::Linear))
            .expect("linear transition should arm");

        let output = engine
            .render(
                AudioPacket::Samples(vec![1.0; 3]),
                Some(AudioPacket::Samples(vec![0.0; 3])),
            )
            .expect("linear transition should render");
        assert_eq!(into_samples(output), [1.0, 0.5, 0.0]);
    }

    #[test]
    fn single_source_passthrough_is_exact_and_reuses_allocation() {
        let mut engine = TransitionEngine::new(44_100, 2);
        let samples = vec![0.25, -0.5, 0.75, -1.0];
        let allocation = samples.as_ptr();

        let output = engine
            .render(AudioPacket::Samples(samples), None)
            .expect("idle passthrough should succeed");
        let output = into_samples(output);

        assert_eq!(output, [0.25, -0.5, 0.75, -1.0]);
        assert_eq!(output.as_ptr(), allocation);
        assert_eq!(engine.state(), TransitionState::Idle);

        let encoded = vec![1, 2, 3, 4];
        let allocation = encoded.as_ptr();
        let output = engine
            .render(AudioPacket::Raw(encoded), None)
            .expect("idle encoded passthrough should succeed");
        match output {
            AudioPacket::Raw(output) => {
                assert_eq!(output, [1, 2, 3, 4]);
                assert_eq!(output.as_ptr(), allocation);
            }
            AudioPacket::Samples(_) => panic!("expected encoded packet"),
        }
    }

    #[test]
    fn linear_crossfade_mixes_both_sources_by_frame() {
        let mut engine = TransitionEngine::new(5, 2);
        engine
            .arm(spec(1, TransitionCurve::Linear))
            .expect("valid transition should arm");

        let output = engine
            .render(
                AudioPacket::Samples(vec![1.0; 10]),
                Some(AudioPacket::Samples(vec![-1.0; 10])),
            )
            .expect("aligned PCM should mix");

        assert_eq!(
            into_samples(output),
            [1.0, 1.0, 0.5, 0.5, 0.0, 0.0, -0.5, -0.5, -1.0, -1.0]
        );
        assert_eq!(engine.state(), TransitionState::Finishing);
        engine
            .complete()
            .expect("finished transition should complete after source promotion");
        assert_eq!(engine.state(), TransitionState::Idle);
    }

    #[test]
    fn equal_power_mix_preserves_headroom_without_internal_clipping() {
        let mut engine = TransitionEngine::new(3, 1);
        engine
            .arm(spec(1, TransitionCurve::EqualPower))
            .expect("valid transition should arm");

        let output = engine
            .render(
                AudioPacket::Samples(vec![1.0, 1.0, 1.0]),
                Some(AudioPacket::Samples(vec![1.0, 1.0, 1.0])),
            )
            .expect("aligned PCM should mix");
        let output = into_samples(output);

        assert_eq!(output[0], 1.0);
        assert!((output[1] - 2.0_f64.sqrt()).abs() < 1e-12);
        assert!(output[1] > 1.0, "the internal mixer must not hard-clip");
        assert!((output[2] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn cancellation_resets_an_active_transition() {
        let mut engine = TransitionEngine::new(4, 1);
        engine
            .arm(spec(2, TransitionCurve::Linear))
            .expect("valid transition should arm");
        let _ = engine
            .render(
                AudioPacket::Samples(vec![1.0; 2]),
                Some(AudioPacket::Samples(vec![0.0; 2])),
            )
            .expect("aligned PCM should mix");
        assert_eq!(engine.state(), TransitionState::Active);

        engine.cancel("test cancellation");

        assert_eq!(engine.state(), TransitionState::Idle);
        let output = engine
            .render(AudioPacket::Samples(vec![0.4]), None)
            .expect("cancelled engine should return to passthrough");
        assert_eq!(into_samples(output), [0.4]);
    }

    #[test]
    fn state_machine_rejects_illegal_operations() {
        let mut engine = TransitionEngine::new(4, 1);
        assert!(matches!(
            engine.complete(),
            Err(TransitionError::InvalidState {
                operation: "complete",
                state: TransitionState::Idle
            })
        ));
        assert!(matches!(
            engine.render(
                AudioPacket::Samples(vec![1.0]),
                Some(AudioPacket::Samples(vec![0.0]))
            ),
            Err(TransitionError::InvalidState {
                operation: "accept a second source",
                state: TransitionState::Idle
            })
        ));

        engine
            .arm(spec(1, TransitionCurve::Linear))
            .expect("valid transition should arm");
        assert!(matches!(
            engine.arm(spec(1, TransitionCurve::Linear)),
            Err(TransitionError::InvalidState {
                operation: "arm",
                state: TransitionState::Armed
            })
        ));
    }
}
