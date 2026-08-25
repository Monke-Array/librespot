//! Live Spotify Auto transition selection.
//!
//! Metadata acquisition and the pure ranking algorithm remain separate; this module only joins
//! them and selects the native first-ranked transition/preset for the local materializer.

use librespot_core::Session;
use librespot_protocol::player::ProvidedTrack;
use thiserror::Error;

use crate::{
    spotify_auto_mix::{
        AutoGeometryConfig, AutoPairScoringInput, AutoPipelineResult, AutoRankedPreset,
        AutoRankedTransition, generate_ranked_transitions,
    },
    spotify_auto_mix_metadata::{
        AutoMetadataError, AutoMixMetadataClient, AutoTrackIdentity, LoadedAutoTrackMetadata,
    },
};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct LocalAutoPairKey {
    pub outgoing_uri: String,
    pub incoming_uri: String,
}

#[derive(Clone, Debug)]
pub(crate) struct LocalAutoRequest {
    pub key: LocalAutoPairKey,
    pub track_a: AutoTrackIdentity,
    pub track_b: AutoTrackIdentity,
    pub item_speed_a: f32,
    pub item_speed_b: f32,
}

impl LocalAutoRequest {
    pub(crate) fn from_resolved_pair(
        outgoing: ProvidedTrack,
        incoming: ProvidedTrack,
        track_a: AutoTrackIdentity,
        track_b: AutoTrackIdentity,
    ) -> Self {
        Self {
            key: LocalAutoPairKey {
                outgoing_uri: outgoing.uri.clone(),
                incoming_uri: incoming.uri.clone(),
            },
            item_speed_a: crate::spotify_mix::provided_item_speed_or_default(&outgoing),
            item_speed_b: crate::spotify_mix::provided_item_speed_or_default(&incoming),
            track_a,
            track_b,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LocalAutoTransition {
    pub transition: AutoRankedTransition,
    pub preset: AutoRankedPreset,
}

#[derive(Debug, Error)]
pub(crate) enum LocalAutoError {
    #[error(transparent)]
    Metadata(#[from] AutoMetadataError),
    #[error("Auto ranking returned no transition")]
    MissingTransition,
    #[error("Auto ranking returned no preset")]
    MissingPreset,
}

pub(crate) async fn generate_local_auto(
    session: Session,
    request: &LocalAutoRequest,
) -> Result<LocalAutoTransition, LocalAutoError> {
    let loaded = AutoMixMetadataClient::new(session)
        .load_pair(&request.track_a, &request.track_b)
        .await;
    adapt_loaded_pair(request, loaded)
}

fn adapt_loaded_pair(
    request: &LocalAutoRequest,
    loaded: Result<(LoadedAutoTrackMetadata, LoadedAutoTrackMetadata), AutoMetadataError>,
) -> Result<LocalAutoTransition, LocalAutoError> {
    let (track_a, track_b) = loaded?;
    let pipeline = generate_ranked_transitions(
        AutoPairScoringInput {
            track_a: &track_a.scoring_input,
            track_b: &track_b.scoring_input,
            item_speed_a: request.item_speed_a,
            item_speed_b: request.item_speed_b,
        },
        AutoGeometryConfig::default(),
    );
    let (transition, preset) = select_native_result(pipeline)?;
    Ok(LocalAutoTransition { transition, preset })
}

fn select_native_result(
    pipeline: AutoPipelineResult,
) -> Result<(AutoRankedTransition, AutoRankedPreset), LocalAutoError> {
    let transition = pipeline
        .ranked_transitions
        .into_iter()
        .next()
        .ok_or(LocalAutoError::MissingTransition)?;
    let preset = transition
        .ranked_presets
        .first()
        .copied()
        .ok_or(LocalAutoError::MissingPreset)?;
    Ok((transition, preset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spotify_auto_mix::{AutoPipelineResult, AutoTransitionOverlap};

    const CANONICAL_A: &str = "spotify:track:7tFiyTwD0nx5a1eklYtX2J";
    const PLAYABLE_A: &str = "spotify:track:2JiDi0qAXsPwhPqA2qaKGt";
    const CANONICAL_B: &str = "spotify:track:003vvx7Niy0yvhvHt4a68B";
    const PLAYABLE_B: &str = "spotify:track:1NHWG8zxSEypSRF3UufrnO";

    fn request() -> LocalAutoRequest {
        let mut outgoing = ProvidedTrack {
            uri: CANONICAL_A.to_owned(),
            ..Default::default()
        };
        outgoing
            .metadata
            .insert("item.speed".to_owned(), "1.125".to_owned());
        LocalAutoRequest::from_resolved_pair(
            outgoing,
            ProvidedTrack {
                uri: CANONICAL_B.to_owned(),
                ..Default::default()
            },
            AutoTrackIdentity {
                canonical_uri: CANONICAL_A.to_owned(),
                playable_uri: PLAYABLE_A.to_owned(),
                canonical_duration_ms: 354_320,
            },
            AutoTrackIdentity {
                canonical_uri: CANONICAL_B.to_owned(),
                playable_uri: PLAYABLE_B.to_owned(),
                canonical_duration_ms: 222_973,
            },
        )
    }

    fn transition(
        start_a_ms: i64,
        start_b_ms: i64,
        duration_ms: i64,
        duration_bars: usize,
        speed_b: f32,
        score: f32,
        presets: &[u8],
    ) -> AutoRankedTransition {
        AutoRankedTransition {
            overlap: AutoTransitionOverlap {
                start_a_ms,
                start_b_ms,
                duration_ms,
                duration_bars,
                speed_a: 1.0,
                speed_b,
                is_beatmatched: true,
            },
            computed_score: score,
            components: None,
            pareto_layer: Some(0),
            ranked_presets: presets
                .iter()
                .copied()
                .map(|preset_id| AutoRankedPreset {
                    preset_id,
                    computed_score: if preset_id == presets[0] { 1.0 } else { 0.5 },
                })
                .collect(),
        }
    }

    #[test]
    fn native_first_transition_and_first_preset_are_selected() {
        // First result from the durable tested-playlist oracle pair.
        let expected = transition(
            208_960,
            2_763,
            6_090,
            2,
            0.903_120_4,
            3.755_000_1,
            &[1, 0, 2],
        );
        let later = transition(100, 200, 5_000, 4, 1.0, 99.0, &[19, 1]);
        let (selected, preset) = select_native_result(AutoPipelineResult {
            raw_candidate_count: 2,
            base_valid_candidate_count: 2,
            per_bar_retained_count: 2,
            ranked_transitions: vec![expected.clone(), later],
        })
        .unwrap();

        assert_eq!(selected, expected);
        assert_eq!(preset, expected.ranked_presets[0]);
        assert_eq!(selected.overlap.start_a_ms, 208_960);
        assert_eq!(selected.overlap.start_b_ms, 2_763);
        assert_eq!(selected.overlap.duration_ms, 6_090);
        assert_eq!(selected.overlap.duration_bars, 2);
        assert_eq!(preset.preset_id, 1);
    }

    #[test]
    fn resolved_canonical_and_playable_identities_reach_the_loader_request_unchanged() {
        let request = request();

        assert_eq!(request.key.outgoing_uri, CANONICAL_A);
        assert_eq!(request.key.incoming_uri, CANONICAL_B);
        assert_eq!(request.track_a.canonical_uri, CANONICAL_A);
        assert_eq!(request.track_a.playable_uri, PLAYABLE_A);
        assert_eq!(request.track_a.canonical_duration_ms, 354_320);
        assert_eq!(request.track_b.canonical_uri, CANONICAL_B);
        assert_eq!(request.track_b.playable_uri, PLAYABLE_B);
        assert_eq!(request.track_b.canonical_duration_ms, 222_973);
        assert_eq!(request.item_speed_a, 1.125);
        assert_eq!(request.item_speed_b, 1.0);
    }

    #[test]
    fn metadata_failure_makes_local_auto_unavailable() {
        assert!(matches!(
            adapt_loaded_pair(
                &request(),
                Err(AutoMetadataError::Missing("vocal activity")),
            ),
            Err(LocalAutoError::Metadata(AutoMetadataError::Missing(
                "vocal activity"
            )))
        ));
    }

    #[test]
    fn absent_ranked_data_is_unavailable_without_panicking() {
        assert!(matches!(
            select_native_result(AutoPipelineResult {
                raw_candidate_count: 0,
                base_valid_candidate_count: 0,
                per_bar_retained_count: 0,
                ranked_transitions: Vec::new(),
            }),
            Err(LocalAutoError::MissingTransition)
        ));

        let without_presets = transition(0, 0, 5_000, 0, 1.0, 0.0, &[]);
        assert!(matches!(
            select_native_result(AutoPipelineResult {
                raw_candidate_count: 0,
                base_valid_candidate_count: 0,
                per_bar_retained_count: 0,
                ranked_transitions: vec![without_presets],
            }),
            Err(LocalAutoError::MissingPreset)
        ));
    }
}
