use std::time::Duration;

use librespot_core::SpotifyUri;

use crate::{SAMPLE_RATE, TransitionPlan};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PreviewGeneration(pub u64);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PreviewAuthority {
    pub connect_session_id: String,
    pub normal_ownership_generation: u64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PreviewToken {
    pub authority: PreviewAuthority,
    pub generation: PreviewGeneration,
}

#[derive(Clone, Debug)]
pub struct PreviewTrack {
    pub canonical: SpotifyUri,
    pub expected_playable: SpotifyUri,
}

#[derive(Clone, Debug)]
pub struct PreviewPlaybackRequest {
    pub token: PreviewToken,
    pub outgoing: PreviewTrack,
    pub incoming: PreviewTrack,
    pub outgoing_load_position_ms: u32,
    pub incoming_load_position_ms: u32,
    pub post_roll: Duration,
    pub plan: TransitionPlan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewCancelReason {
    Replaced,
    NormalCommand,
    SessionChanged,
    Inactive,
    Shutdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetainedPlaybackDisposition {
    Restore,
    Discard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewRestoreOutcome {
    Restored,
    DiscardedByAuthority,
    DiscardedByCommand,
    NotOwned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewFailure {
    OutgoingLoad,
    IncomingLoad,
    PlayableMismatch,
    SecondaryUnavailable,
    Render,
    PrematureOutgoingEof,
    PrematureIncomingEof,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreviewFrameBudget {
    remaining: usize,
}

impl PreviewFrameBudget {
    pub fn new(duration: Duration) -> Self {
        let frames = duration
            .as_nanos()
            .saturating_mul(u128::from(SAMPLE_RATE))
            .div_ceil(1_000_000_000);
        Self {
            remaining: usize::try_from(frames).unwrap_or(usize::MAX),
        }
    }

    pub fn remaining(&self) -> usize {
        self.remaining
    }

    pub fn take(&mut self, available: usize) -> usize {
        let taken = available.min(self.remaining);
        self.remaining -= taken;
        taken
    }

    pub fn is_complete(&self) -> bool {
        self.remaining == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn token(session: &str, preview_generation: u64, normal_generation: u64) -> PreviewToken {
        PreviewToken {
            authority: PreviewAuthority {
                connect_session_id: session.to_owned(),
                normal_ownership_generation: normal_generation,
            },
            generation: PreviewGeneration(preview_generation),
        }
    }

    #[test]
    fn token_equality_requires_session_generation_and_normal_authority() {
        let reference = token("session-a", 4, 9);
        assert_ne!(reference, token("session-b", 4, 9));
        assert_ne!(reference, token("session-a", 5, 9));
        assert_ne!(reference, token("session-a", 4, 10));
    }

    #[test]
    fn three_second_post_roll_is_exact_at_44100_hz() {
        let mut budget = PreviewFrameBudget::new(Duration::from_millis(3_000));
        assert_eq!(budget.remaining(), 132_300);
        assert_eq!(budget.take(100_000), 100_000);
        assert_eq!(budget.take(40_000), 32_300);
        assert!(budget.is_complete());
    }
}
