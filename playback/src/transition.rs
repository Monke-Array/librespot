use std::{f64::consts::FRAC_PI_2, time::Duration};

use thiserror::Error;

use crate::decoder::AudioPacket;

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
}

/// The default policy preserves librespot's existing one-track playback behavior.
#[derive(Debug, Default)]
pub(crate) struct NoTransitionPolicy;

impl TransitionPolicy for NoTransitionPolicy {
    fn plan(
        &self,
        _current_position: Duration,
        _current_duration: Duration,
    ) -> Option<TransitionSpec> {
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransitionState {
    Idle,
    Armed,
    Active,
    Finishing,
}

#[allow(dead_code)] // Both curves are part of the next, dual-decoder integration step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransitionCurve {
    Linear,
    EqualPower,
}

#[derive(Clone, Copy, Debug, PartialEq)]
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
        self.spec = Some(spec);
        self.state = TransitionState::Armed;
        debug!(
            "Transition armed: duration {:?}, curve {:?}, current gain {}, next gain {}",
            spec.duration, spec.curve, spec.current_gain, spec.next_gain
        );
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
    #[allow(dead_code)] // Promotion is intentionally deferred until the dual-decoder player pass.
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
                let (current_fade, next_fade) = match spec.curve {
                    TransitionCurve::Linear => (1.0 - progress, progress),
                    TransitionCurve::EqualPower => {
                        let angle = progress * FRAC_PI_2;
                        (angle.cos(), angle.sin())
                    }
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

    #[test]
    fn default_policy_selects_no_transition() {
        assert_eq!(
            NoTransitionPolicy.plan(Duration::from_secs(150), Duration::from_secs(180)),
            None
        );
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
