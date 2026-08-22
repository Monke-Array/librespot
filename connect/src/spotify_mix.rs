use std::{collections::BTreeSet, time::Duration};

use data_encoding::BASE64;
use librespot_playback::{
    GainCurve, GainCurveSegment, GainPoint, TransitionPlan, TransitionPlanError,
};
use librespot_protocol::{
    automix_transition::{CurveSet, Overlap, Preset, Transition},
    player::ProvidedTrack,
};
use protobuf::{Message, MessageField};
use thiserror::Error;

pub(crate) const RECIPE_ATTRIBUTE: &str = "automix.auto_transition_recipe";
pub(crate) const BACKEND_RECIPE_ATTRIBUTE: &str = "automix.backend_auto_transition";
const ITEM_SPEED_ATTRIBUTE: &str = "item.speed";
const ITEM_SPEED_TOLERANCE: f64 = 0.001;

#[derive(Debug, Error)]
pub(crate) enum SpotifyTransitionError {
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
    #[error("preset/style resolution required (preset={0})")]
    UnsupportedPresetStyle(i32),
    #[error("tempo handling required ({0}={1})")]
    UnsupportedTempo(&'static str, f32),
    #[error("EQ/filter/FX rendering required")]
    UnsupportedEffects,
    #[error("inline volume automation is required")]
    UnsupportedVolumeAutomation,
    #[error(transparent)]
    InvalidPlan(#[from] TransitionPlanError),
}

/// Typed current Spotify Automix recipe decoded from a playlist item attribute.
#[derive(Clone, Debug)]
pub(crate) struct SpotifyTransitionRecipe {
    transition: Transition,
}

impl SpotifyTransitionRecipe {
    pub(crate) fn from_base64(encoded: &str) -> Result<Self, SpotifyTransitionError> {
        let bytes = BASE64
            .decode(encoded.as_bytes())
            .map_err(|_| SpotifyTransitionError::InvalidBase64)?;
        let transition = Transition::parse_from_bytes(&bytes)
            .map_err(|_| SpotifyTransitionError::InvalidProtobuf)?;
        let recipe = Self { transition };
        recipe.sanity_check()?;
        Ok(recipe)
    }

    pub(crate) fn from_local_auto(
        outgoing: &ProvidedTrack,
        incoming: &ProvidedTrack,
        playable_a_uri: &str,
        playable_b_uri: &str,
        bpm_a: f32,
        bpm_b: f32,
        item_speed_a: f32,
        item_speed_b: f32,
        overlap: crate::spotify_auto_mix::AutoTransitionOverlap,
        preset_id: u8,
    ) -> Result<Self, SpotifyTransitionError> {
        let start_a_ms = i32::try_from(overlap.start_a_ms)
            .map_err(|_| SpotifyTransitionError::InvalidDuration)?;
        let start_b_ms = i32::try_from(overlap.start_b_ms)
            .map_err(|_| SpotifyTransitionError::InvalidDuration)?;
        let duration_ms = i32::try_from(overlap.duration_ms)
            .map_err(|_| SpotifyTransitionError::InvalidDuration)?;
        let duration_bars = i32::try_from(overlap.duration_bars)
            .map_err(|_| SpotifyTransitionError::InvalidDuration)?;
        let transition = Transition {
            overlap: MessageField::some(Overlap {
                track_a_row_id: (!outgoing.uid.is_empty()).then(|| outgoing.uid.clone()),
                track_b_row_id: (!incoming.uid.is_empty()).then(|| incoming.uid.clone()),
                start_a_ms: Some(start_a_ms),
                start_b_ms: Some(start_b_ms),
                duration_ms: Some(duration_ms),
                speed_a: Some(overlap.speed_a),
                speed_b: Some(overlap.speed_b),
                duration_bars: Some(duration_bars),
                is_beatmatched: Some(overlap.is_beatmatched),
                track_a_uri: Some(outgoing.uri.clone()),
                track_b_uri: Some(incoming.uri.clone()),
                track_a_playable_uri: Some(playable_a_uri.to_owned()),
                track_b_playable_uri: Some(playable_b_uri.to_owned()),
                bpm_a: Some(bpm_a),
                bpm_b: Some(bpm_b),
                item_speed_a: Some(f64::from(item_speed_a)),
                item_speed_b: Some(f64::from(item_speed_b)),
                ..Default::default()
            }),
            preset: MessageField::some(Preset {
                id: Some(i32::from(preset_id)),
                ..Default::default()
            }),
            ..Default::default()
        };
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

    fn ensure_renderable(&self) -> Result<(), SpotifyTransitionError> {
        let overlap = self.overlap()?;
        let preset = self
            .transition
            .preset
            .as_ref()
            .ok_or(SpotifyTransitionError::MissingPreset)?;

        // EqSwap is confirmed to require preset semantics that the current renderer does not
        // implement. Other presets without inline volume automation need style resolution too.
        if preset.id() == 2
            || (preset.id() != 0
                && (preset.volume_out_curve_override.is_none()
                    || preset.volume_in_curve_override.is_none()))
        {
            return Err(SpotifyTransitionError::UnsupportedPresetStyle(preset.id()));
        }
        if preset.volume_out_curve_override.is_none() || preset.volume_in_curve_override.is_none() {
            return Err(SpotifyTransitionError::UnsupportedVolumeAutomation);
        }
        if has_unsupported_effects(preset) {
            return Err(SpotifyTransitionError::UnsupportedEffects);
        }
        if overlap
            .speed_a
            .is_some_and(|speed| (speed - 1.0).abs() > ITEM_SPEED_TOLERANCE as f32)
        {
            return Err(SpotifyTransitionError::UnsupportedTempo(
                "speedA",
                overlap.speed_a(),
            ));
        }
        if overlap
            .speed_b
            .is_some_and(|speed| (speed - 1.0).abs() > ITEM_SPEED_TOLERANCE as f32)
        {
            return Err(SpotifyTransitionError::UnsupportedTempo(
                "speedB",
                overlap.speed_b(),
            ));
        }
        Ok(())
    }

    fn decoded_summary(&self) -> Result<String, SpotifyTransitionError> {
        let overlap = self.overlap()?;
        Ok(format!(
            "startA={} startB={} duration={} preset={} speedA={:?} speedB={:?}",
            overlap.start_a_ms(),
            overlap.start_b_ms(),
            overlap.duration_ms(),
            self.preset().map(Preset::id).unwrap_or_default(),
            overlap.speed_a,
            overlap.speed_b
        ))
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

pub(crate) fn provided_item_speed_or_default(track: &ProvidedTrack) -> f32 {
    provided_item_speed(track)
        .map(|speed| speed as f32)
        .filter(|speed| speed.is_finite() && *speed > 0.0)
        .unwrap_or(1.0)
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

    transition_plan_for_recipe_pair(outgoing, incoming, encoded)
}

pub(crate) fn transition_plan_for_recipe_pair(
    outgoing: &ProvidedTrack,
    incoming: &ProvidedTrack,
    encoded: &str,
) -> Option<TransitionPlan> {
    match SpotifyTransitionRecipe::from_base64(encoded) {
        Ok(recipe) => transition_plan_for_decoded_pair(outgoing, incoming, &recipe),
        Err(error) => {
            warn!("[spotify-mix] invalid saved transition: {error}; using fallback");
            None
        }
    }
}

pub(crate) fn transition_plan_for_decoded_pair(
    outgoing: &ProvidedTrack,
    incoming: &ProvidedTrack,
    recipe: &SpotifyTransitionRecipe,
) -> Option<TransitionPlan> {
    transition_plan_for_decoded_pair_with_origin(outgoing, incoming, recipe, "saved", true)
}

pub(crate) fn transition_plan_for_local_auto_pair(
    outgoing: &ProvidedTrack,
    incoming: &ProvidedTrack,
    recipe: &SpotifyTransitionRecipe,
) -> Option<TransitionPlan> {
    transition_plan_for_decoded_pair_with_origin(outgoing, incoming, recipe, "local Auto", false)
}

fn transition_plan_for_decoded_pair_with_origin(
    outgoing: &ProvidedTrack,
    incoming: &ProvidedTrack,
    recipe: &SpotifyTransitionRecipe,
    origin: &str,
    warn_on_failure: bool,
) -> Option<TransitionPlan> {
    let result: Result<TransitionPlan, SpotifyTransitionError> = (|| {
        recipe.validate_pair(
            &outgoing.uri,
            &incoming.uri,
            provided_item_speed(outgoing),
            provided_item_speed(incoming),
        )?;
        let overlap = recipe.overlap()?;
        let preset = recipe.preset();
        debug!(
            "[spotify-mix] {origin} recipe decoded {}",
            recipe.decoded_summary()?
        );
        debug!(
            "[spotify-mix] preset type={:?} beatmatchPreference={:?} beatmatched={} itemSpeedA={} itemSpeedB={}",
            preset.map(Preset::type_),
            recipe.transition.beatmatch_preference(),
            overlap.is_beatmatched(),
            overlap.item_speed_a(),
            overlap.item_speed_b()
        );
        debug!(
            "[spotify-mix] durationBars={:?} bpmA={:?} bpmB={:?}",
            overlap.duration_bars, overlap.bpm_a, overlap.bpm_b
        );
        recipe.ensure_renderable()?;
        let plan = recipe.to_plan()?;
        debug!(
            "[spotify-mix] {origin} transition {} -> {}",
            outgoing.uri, incoming.uri
        );
        debug!(
            "[spotify-mix] startA={} startB={} duration={}",
            overlap.start_a_ms(),
            overlap.start_b_ms(),
            overlap.duration_ms()
        );
        Ok(plan)
    })();

    match result {
        Ok(plan) => Some(plan),
        Err(
            error @ (SpotifyTransitionError::UnsupportedPresetStyle(_)
            | SpotifyTransitionError::UnsupportedTempo(_, _)
            | SpotifyTransitionError::UnsupportedEffects
            | SpotifyTransitionError::UnsupportedVolumeAutomation),
        ) => {
            if warn_on_failure {
                warn!("[spotify-mix] {origin} recipe unsupported: {error}; using fallback");
            } else {
                debug!("[spotify-auto] {origin} recipe unsupported: {error}; using fallback");
            }
            None
        }
        Err(error) => {
            if warn_on_failure {
                warn!("[spotify-mix] invalid {origin} transition: {error}; using fallback");
            } else {
                debug!("[spotify-auto] invalid {origin} transition: {error}; using fallback");
            }
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
    fn local_auto_recipe_preserves_resolved_identities_and_oracle_geometry() {
        let mut outgoing = track(TRACK_A, None, Some("1.25"));
        outgoing.uid = "row-a".into();
        let mut incoming = track(TRACK_B, None, None);
        incoming.uid = "row-b".into();
        let playable_a = "spotify:track:2BMRUAA1oTc7e9JPlr6xbZ";
        let playable_b = "spotify:track:5g9lS8deSIxItFBmZRC4vN";
        let recipe = SpotifyTransitionRecipe::from_local_auto(
            &outgoing,
            &incoming,
            playable_a,
            playable_b,
            87.102_9,
            87.274,
            1.25,
            1.0,
            crate::spotify_auto_mix::AutoTransitionOverlap {
                start_a_ms: 208_960,
                start_b_ms: 2_763,
                duration_ms: 6_090,
                duration_bars: 2,
                speed_a: 1.0,
                speed_b: 0.903_120_4,
                is_beatmatched: true,
            },
            1,
        )
        .unwrap();
        let overlap = recipe.overlap().unwrap();

        assert_eq!(overlap.track_a_uri(), TRACK_A);
        assert_eq!(overlap.track_b_uri(), TRACK_B);
        assert_eq!(overlap.track_a_playable_uri(), playable_a);
        assert_eq!(overlap.track_b_playable_uri(), playable_b);
        assert_eq!(overlap.track_a_row_id(), "row-a");
        assert_eq!(overlap.track_b_row_id(), "row-b");
        assert_eq!(overlap.start_a_ms(), 208_960);
        assert_eq!(overlap.start_b_ms(), 2_763);
        assert_eq!(overlap.duration_ms(), 6_090);
        assert_eq!(overlap.duration_bars(), 2);
        assert_eq!(overlap.speed_b().to_bits(), 0.903_120_4_f32.to_bits());
        assert!(overlap.is_beatmatched());
        assert_eq!(overlap.item_speed_a(), 1.25);
        assert_eq!(recipe.preset().unwrap().id(), 1);
    }

    #[test]
    fn item_speed_defaults_to_one_for_missing_or_invalid_metadata() {
        assert_eq!(
            provided_item_speed_or_default(&track(TRACK_A, None, None)),
            1.0
        );
        assert_eq!(
            provided_item_speed_or_default(&track(TRACK_A, None, Some("not-a-speed"))),
            1.0
        );
        assert_eq!(
            provided_item_speed_or_default(&track(TRACK_A, None, Some("1.125"))),
            1.125
        );
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
    fn eq_swap_without_inline_curves_is_reported_as_unsupported() {
        let mut transition = transition();
        let preset = transition.preset.as_mut().unwrap();
        preset.id = Some(2);
        preset.volume_out_curve_override.clear();
        preset.volume_in_curve_override.clear();
        let recipe = SpotifyTransitionRecipe::from_base64(&encoded(&transition)).unwrap();

        assert!(matches!(
            recipe.ensure_renderable(),
            Err(SpotifyTransitionError::UnsupportedPresetStyle(2))
        ));
        let outgoing = track(TRACK_A, None, Some("1.0"));
        let incoming = track(TRACK_B, None, Some("1.0"));
        assert!(transition_plan_for_decoded_pair(&outgoing, &incoming, &recipe).is_none());
    }

    #[test]
    fn provided_metadata_keys_are_sorted_without_values() {
        let mut track = ProvidedTrack::new();
        track.metadata.insert("z-last".into(), "secret-z".into());
        track.metadata.insert("a-first".into(), "secret-a".into());

        assert_eq!(provided_metadata_keys(&track), ["a-first", "z-last"]);
    }
}
