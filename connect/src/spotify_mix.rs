use std::{collections::BTreeSet, time::Duration};

use data_encoding::BASE64;
use librespot_playback::{
    GainCurve, GainCurveSegment, GainPoint, TransitionPlan, TransitionPlanError,
};
use librespot_protocol::{
    automix_transition::{CurveSet, Overlap, Preset, Transition},
    player::ProvidedTrack,
};
use protobuf::Message;
use thiserror::Error;

const RECIPE_ATTRIBUTE: &str = "automix.auto_transition_recipe";
const BACKEND_RECIPE_ATTRIBUTE: &str = "automix.backend_auto_transition";
const ITEM_SPEED_ATTRIBUTE: &str = "item.speed";
const ITEM_SPEED_TOLERANCE: f64 = 0.001;

#[derive(Debug, Error)]
enum SpotifyTransitionError {
    #[error("recipe is not valid base64")]
    InvalidBase64,
    #[error("recipe is not a valid Automix Transition protobuf")]
    InvalidProtobuf,
    #[error("recipe has no overlap")]
    MissingOverlap,
    #[error("recipe overlap is missing timing fields")]
    MissingTiming,
    #[error("recipe overlap duration must be greater than zero")]
    InvalidDuration,
    #[error("recipe overlap contains a non-finite analysis value")]
    InvalidAnalysisValue,
    #[error("recipe outgoing URI does not match the playlist item")]
    TrackAMismatch,
    #[error("recipe incoming URI does not match the next playlist item")]
    TrackBMismatch,
    #[error("recipe outgoing item speed does not match the playlist item")]
    ItemSpeedAMismatch,
    #[error("recipe incoming item speed does not match the playlist item")]
    ItemSpeedBMismatch,
    #[error("recipe has no preset")]
    MissingPreset,
    #[error("recipe has no outgoing volume curve override")]
    MissingOutgoingVolumeCurve,
    #[error("recipe has no incoming volume curve override")]
    MissingIncomingVolumeCurve,
    #[error(transparent)]
    InvalidPlan(#[from] TransitionPlanError),
}

/// Typed current Spotify Automix recipe decoded from a playlist item attribute.
struct SpotifyTransitionRecipe {
    transition: Transition,
}

impl SpotifyTransitionRecipe {
    fn from_base64(encoded: &str) -> Result<Self, SpotifyTransitionError> {
        let bytes = BASE64
            .decode(encoded.as_bytes())
            .map_err(|_| SpotifyTransitionError::InvalidBase64)?;
        let transition = Transition::parse_from_bytes(&bytes)
            .map_err(|_| SpotifyTransitionError::InvalidProtobuf)?;
        let recipe = Self { transition };
        recipe.sanity_check()?;
        Ok(recipe)
    }

    fn overlap(&self) -> Result<&Overlap, SpotifyTransitionError> {
        self.transition
            .overlap
            .as_ref()
            .ok_or(SpotifyTransitionError::MissingOverlap)
    }

    fn sanity_check(&self) -> Result<(), SpotifyTransitionError> {
        let overlap = self.overlap()?;
        if overlap.start_a_ms.is_none()
            || overlap.start_b_ms.is_none()
            || overlap.duration_ms.is_none()
        {
            return Err(SpotifyTransitionError::MissingTiming);
        }
        if overlap.duration_ms() <= 0 || overlap.start_a_ms() < 0 || overlap.start_b_ms() < 0 {
            return Err(SpotifyTransitionError::InvalidDuration);
        }

        let finite = overlap
            .speed_a
            .into_iter()
            .map(f64::from)
            .chain(overlap.speed_b.into_iter().map(f64::from))
            .chain(overlap.bpm_a.into_iter().map(f64::from))
            .chain(overlap.bpm_b.into_iter().map(f64::from))
            .chain(overlap.item_speed_a)
            .chain(overlap.item_speed_b)
            .all(f64::is_finite);
        if !finite {
            return Err(SpotifyTransitionError::InvalidAnalysisValue);
        }
        Ok(())
    }

    fn validate_pair(
        &self,
        outgoing_uri: &str,
        incoming_uri: &str,
        outgoing_item_speed: Option<f64>,
        incoming_item_speed: Option<f64>,
    ) -> Result<(), SpotifyTransitionError> {
        let overlap = self.overlap()?;
        if overlap.track_a_uri() != outgoing_uri {
            return Err(SpotifyTransitionError::TrackAMismatch);
        }
        if overlap.track_b_uri() != incoming_uri {
            return Err(SpotifyTransitionError::TrackBMismatch);
        }
        if outgoing_item_speed
            .is_some_and(|actual| !item_speeds_match(overlap.item_speed_a(), actual))
        {
            return Err(SpotifyTransitionError::ItemSpeedAMismatch);
        }
        if incoming_item_speed
            .is_some_and(|actual| !item_speeds_match(overlap.item_speed_b(), actual))
        {
            return Err(SpotifyTransitionError::ItemSpeedBMismatch);
        }
        Ok(())
    }

    fn to_plan(&self) -> Result<TransitionPlan, SpotifyTransitionError> {
        let overlap = self.overlap()?;
        let preset = self
            .transition
            .preset
            .as_ref()
            .ok_or(SpotifyTransitionError::MissingPreset)?;
        let outgoing = preset
            .volume_out_curve_override
            .as_ref()
            .ok_or(SpotifyTransitionError::MissingOutgoingVolumeCurve)?;
        let incoming = preset
            .volume_in_curve_override
            .as_ref()
            .ok_or(SpotifyTransitionError::MissingIncomingVolumeCurve)?;

        TransitionPlan::new(
            Duration::from_millis(overlap.start_a_ms() as u64),
            Duration::from_millis(overlap.start_b_ms() as u64),
            Duration::from_millis(overlap.duration_ms() as u64),
            adapt_curve_set(outgoing)?,
            adapt_curve_set(incoming)?,
        )
        .map_err(Into::into)
    }

    fn preset(&self) -> Option<&Preset> {
        self.transition.preset.as_ref()
    }
}

fn normalized_item_speed(speed: f64) -> f64 {
    if speed.is_finite() && speed > 0.0 {
        speed
    } else {
        1.0
    }
}

fn item_speeds_match(left: f64, right: f64) -> bool {
    (normalized_item_speed(left) - normalized_item_speed(right)).abs() <= ITEM_SPEED_TOLERANCE
}

fn provided_item_speed(track: &ProvidedTrack) -> Option<f64> {
    track
        .metadata
        .get(ITEM_SPEED_ATTRIBUTE)
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|speed| speed.is_finite() && *speed > 0.0)
}

fn provided_metadata_keys(track: &ProvidedTrack) -> Vec<&str> {
    track
        .metadata
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn log_transition_track(label: &str, track: &ProvidedTrack) {
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }

    debug!(
        "[spotify-mix] {label} uri={:?} uid={:?}",
        track.uri, track.uid
    );
    debug!(
        "[spotify-mix] {label} metadata_keys={:?}",
        provided_metadata_keys(track)
    );
    // ProvidedTrack has no format-list-attribute field in the current schema.
    debug!("[spotify-mix] {label} format_attribute_keys=[]");
    debug!(
        "[spotify-mix] {label} recipe_present={} backend_recipe_present={}",
        track.metadata.contains_key(RECIPE_ATTRIBUTE),
        track.metadata.contains_key(BACKEND_RECIPE_ATTRIBUTE)
    );
}

fn adapt_curve_set(curve_set: &CurveSet) -> Result<GainCurve, TransitionPlanError> {
    let segments = curve_set
        .curves
        .iter()
        .map(|curve| GainCurveSegment {
            start: curve.start(),
            end: curve.end(),
            points: curve
                .points
                .iter()
                .map(|point| GainPoint {
                    x: point.x(),
                    y: point.y(),
                })
                .collect(),
        })
        .collect();
    GainCurve::new(segments)
}

fn has_unsupported_effects(preset: &Preset) -> bool {
    preset.eq_style_override.is_some()
        || preset.filter_fx_style_override.is_some()
        || preset.eq_out_curve_overrides.is_some()
        || preset.eq_in_curve_overrides.is_some()
        || preset.filter_out_curve_overrides.is_some()
        || preset.filter_in_curve_overrides.is_some()
        || preset.fx_style_override.is_some()
        || preset.fx_out_curve_overrides.is_some()
        || preset.fx_in_curve_overrides.is_some()
        || preset.jogwheel_style_override.is_some()
        || preset.looping_style_override.is_some()
}

pub(crate) fn transition_plan_for_pair(
    outgoing: &ProvidedTrack,
    incoming: &ProvidedTrack,
) -> Option<TransitionPlan> {
    log_transition_track("A", outgoing);
    log_transition_track("B", incoming);

    let Some(encoded) = outgoing.metadata.get(RECIPE_ATTRIBUTE) else {
        debug!("[spotify-mix] no saved recipe; using fallback");
        return None;
    };

    let result = SpotifyTransitionRecipe::from_base64(encoded).and_then(|recipe| {
        recipe.validate_pair(
            &outgoing.uri,
            &incoming.uri,
            provided_item_speed(outgoing),
            provided_item_speed(incoming),
        )?;
        let plan = recipe.to_plan()?;
        let overlap = recipe.overlap()?;
        let preset = recipe.preset();
        debug!(
            "[spotify-mix] saved transition {} -> {}",
            outgoing.uri, incoming.uri
        );
        debug!(
            "[spotify-mix] startA={} startB={} duration={}",
            overlap.start_a_ms(),
            overlap.start_b_ms(),
            overlap.duration_ms()
        );
        debug!(
            "[spotify-mix] preset={} type={:?} beatmatchPreference={:?} beatmatched={} speedA={:?} speedB={:?} itemSpeedA={} itemSpeedB={}",
            preset.map(Preset::id).unwrap_or_default(),
            preset.map(Preset::type_),
            recipe.transition.beatmatch_preference(),
            overlap.is_beatmatched(),
            overlap.speed_a,
            overlap.speed_b,
            overlap.item_speed_a(),
            overlap.item_speed_b()
        );
        debug!(
            "[spotify-mix] durationBars={:?} bpmA={:?} bpmB={:?}",
            overlap.duration_bars, overlap.bpm_a, overlap.bpm_b
        );
        if preset.is_some_and(has_unsupported_effects) {
            debug!("[spotify-mix] unsupported EQ/filter/FX fields present");
        }
        Ok(plan)
    });

    match result {
        Ok(plan) => Some(plan),
        Err(error) => {
            warn!("[spotify-mix] invalid saved transition: {error}; using fallback");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librespot_protocol::automix_transition::{Curve, CurvePoint, PresetType};
    use protobuf::{EnumOrUnknown, MessageField};

    const TRACK_A: &str = "spotify:track:2TpxZ7JUBn3uw46aR7qd6V";
    const TRACK_B: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";

    fn curve(from: f64, to: f64) -> CurveSet {
        CurveSet {
            curves: vec![Curve {
                points: vec![
                    CurvePoint {
                        x: Some(0.0),
                        y: Some(from),
                        ..Default::default()
                    },
                    CurvePoint {
                        x: Some(1.0 / 3.0),
                        y: Some(from),
                        ..Default::default()
                    },
                    CurvePoint {
                        x: Some(2.0 / 3.0),
                        y: Some(to),
                        ..Default::default()
                    },
                    CurvePoint {
                        x: Some(1.0),
                        y: Some(to),
                        ..Default::default()
                    },
                ],
                start: Some(0.0),
                end: Some(1.0),
                ..Default::default()
            }],
            minimum: Some(0.0),
            maximum: Some(1.0),
            ..Default::default()
        }
    }

    fn transition() -> Transition {
        Transition {
            overlap: MessageField::some(Overlap {
                start_a_ms: Some(175_000),
                start_b_ms: Some(12_000),
                duration_ms: Some(8_000),
                speed_a: Some(1.0),
                speed_b: Some(1.0),
                duration_bars: Some(4),
                is_beatmatched: Some(false),
                track_a_uri: Some(TRACK_A.into()),
                track_b_uri: Some(TRACK_B.into()),
                bpm_a: Some(120.0),
                bpm_b: Some(122.0),
                item_speed_a: Some(1.0),
                item_speed_b: Some(1.0),
                ..Default::default()
            }),
            preset: MessageField::some(Preset {
                id: Some(1),
                type_: Some(EnumOrUnknown::new(PresetType::PRESET_TYPE_FADE)),
                volume_out_curve_override: MessageField::some(curve(1.0, 0.0)),
                volume_in_curve_override: MessageField::some(curve(0.0, 1.0)),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn encoded(transition: &Transition) -> String {
        BASE64.encode(&transition.write_to_bytes().expect("fixture should encode"))
    }

    fn track(uri: &str, recipe: Option<String>, speed: Option<&str>) -> ProvidedTrack {
        let mut track = ProvidedTrack {
            uri: uri.into(),
            ..Default::default()
        };
        if let Some(recipe) = recipe {
            track.metadata.insert(RECIPE_ATTRIBUTE.into(), recipe);
        }
        if let Some(speed) = speed {
            track
                .metadata
                .insert(ITEM_SPEED_ATTRIBUTE.into(), speed.into());
        }
        track
    }

    #[test]
    fn base64_recipe_decodes_and_converts_to_plan() {
        let recipe = SpotifyTransitionRecipe::from_base64(&encoded(&transition()))
            .expect("fixture should decode");
        recipe
            .validate_pair(TRACK_A, TRACK_B, Some(1.0), Some(1.0))
            .expect("pair should validate");
        let plan = recipe.to_plan().expect("fixture should adapt");
        assert_eq!(plan.current_start(), Duration::from_millis(175_000));
        assert_eq!(plan.next_start(), Duration::from_millis(12_000));
        assert_eq!(plan.duration(), Duration::from_millis(8_000));
    }

    #[test]
    fn malformed_base64_is_rejected() {
        assert!(matches!(
            SpotifyTransitionRecipe::from_base64("not base64!"),
            Err(SpotifyTransitionError::InvalidBase64)
        ));
    }

    #[test]
    fn malformed_protobuf_is_rejected() {
        assert!(matches!(
            SpotifyTransitionRecipe::from_base64(&BASE64.encode(&[0xff])),
            Err(SpotifyTransitionError::InvalidProtobuf)
        ));
    }

    #[test]
    fn pair_uris_are_validated_in_both_directions() {
        let recipe = SpotifyTransitionRecipe::from_base64(&encoded(&transition())).unwrap();
        assert!(matches!(
            recipe.validate_pair(TRACK_B, TRACK_B, None, None),
            Err(SpotifyTransitionError::TrackAMismatch)
        ));
        assert!(matches!(
            recipe.validate_pair(TRACK_A, TRACK_A, None, None),
            Err(SpotifyTransitionError::TrackBMismatch)
        ));
    }

    #[test]
    fn item_speed_uses_spotify_tolerance() {
        assert!(item_speeds_match(1.0, 1.001));
        assert!(!item_speeds_match(1.0, 1.001_1));
        assert!(item_speeds_match(0.0, 1.0));

        let recipe = SpotifyTransitionRecipe::from_base64(&encoded(&transition())).unwrap();
        recipe
            .validate_pair(TRACK_A, TRACK_B, Some(1.001), Some(1.0005))
            .expect("boundary item speeds should validate");
        assert!(matches!(
            recipe.validate_pair(TRACK_A, TRACK_B, Some(1.001_1), Some(1.0)),
            Err(SpotifyTransitionError::ItemSpeedAMismatch)
        ));
    }

    #[test]
    fn missing_or_invalid_recipe_selects_fallback() {
        let outgoing = track(TRACK_A, None, None);
        let incoming = track(TRACK_B, None, None);
        assert!(transition_plan_for_pair(&outgoing, &incoming).is_none());

        let outgoing = track(TRACK_A, Some("invalid".into()), None);
        assert!(transition_plan_for_pair(&outgoing, &incoming).is_none());
    }

    #[test]
    fn metadata_pair_yields_saved_plan() {
        let outgoing = track(TRACK_A, Some(encoded(&transition())), Some("1.0"));
        let incoming = track(TRACK_B, None, Some("1.0005"));
        assert!(transition_plan_for_pair(&outgoing, &incoming).is_some());
    }

    #[test]
    fn provided_metadata_keys_are_sorted_without_values() {
        let mut track = ProvidedTrack::new();
        track.metadata.insert("z-last".into(), "secret-z".into());
        track.metadata.insert("a-first".into(), "secret-a".into());

        assert_eq!(provided_metadata_keys(&track), ["a-first", "z-last"]);
    }
}
