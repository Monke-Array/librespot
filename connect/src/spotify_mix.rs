use std::{collections::BTreeSet, time::Duration};

use data_encoding::BASE64;
use librespot_playback::{
    GainCurve, GainCurveSegment, GainPoint, SpeedAutomation, SpeedPoint, TransitionPlan,
    TransitionPlanError,
};
use librespot_protocol::{
    automix_transition::{CurveSet, Overlap, Preset, Transition},
    player::ProvidedTrack,
};
use protobuf::Message;
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

    #[cfg(test)]
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
            overlap: protobuf::MessageField::some(Overlap {
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
            preset: protobuf::MessageField::some(Preset {
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

    if let Some(encoded) = outgoing.metadata.get(RECIPE_ATTRIBUTE) {
        if let Some(plan) = transition_plan_for_recipe_pair(outgoing, incoming, encoded) {
            return Some(plan);
        }
    } else {
        debug!("[spotify-mix] no saved recipe; checking materialized transition");
    }

    match crate::spotify_materialized_transition::materialized_transition_plan_for_pair(
        outgoing, incoming,
    ) {
        Ok(Some(materialized)) => match materialized.to_transition_plan() {
            Ok(plan) => {
                debug!(
                    "[spotify-mix] materialized volume transition {} -> {}",
                    outgoing.uri, incoming.uri
                );
                Some(plan)
            }
            Err(error) => {
                debug!(
                    "[spotify-mix] materialized transition unsupported: {error}; using fallback"
                );
                None
            }
        },
        Ok(None) => {
            debug!("[spotify-mix] no materialized transition; using fallback");
            None
        }
        Err(error) => {
            debug!("[spotify-mix] invalid materialized transition: {error}; using fallback");
            None
        }
    }
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

pub(crate) fn transition_plan_for_local_auto_transition(
    transition: &crate::spotify_auto_mix::AutoRankedTransition,
) -> Result<TransitionPlan, SpotifyTransitionError> {
    let overlap = transition.overlap;
    if overlap.duration_ms <= 0 || overlap.start_a_ms < 0 || overlap.start_b_ms < 0 {
        return Err(SpotifyTransitionError::InvalidDuration);
    }
    if !overlap.speed_a.is_finite()
        || !overlap.speed_b.is_finite()
        || overlap.speed_a <= 0.0
        || overlap.speed_b <= 0.0
    {
        return Err(SpotifyTransitionError::InvalidAnalysisValue);
    }
    if (overlap.speed_a - 1.0).abs() > ITEM_SPEED_TOLERANCE as f32 {
        return Err(SpotifyTransitionError::UnsupportedTempo(
            "speedA",
            overlap.speed_a,
        ));
    }

    let mut plan = TransitionPlan::new(
        Duration::from_millis(u64::try_from(overlap.start_a_ms).unwrap()),
        Duration::from_millis(u64::try_from(overlap.start_b_ms).unwrap()),
        Duration::from_millis(u64::try_from(overlap.duration_ms).unwrap()),
        linear_gain_curve(1.0, 0.0)?,
        linear_gain_curve(0.0, 1.0)?,
    )?;
    if (overlap.speed_b - 1.0).abs() > ITEM_SPEED_TOLERANCE as f32 {
        let speed = SpeedAutomation::new(vec![SpeedPoint {
            from_position: Duration::from_millis(u64::try_from(overlap.start_b_ms).unwrap()),
            speed: f64::from(overlap.speed_b),
        }])
        .map_err(|_| SpotifyTransitionError::UnsupportedTempo("speedB", overlap.speed_b))?;
        plan = plan.with_next_speed_automation(speed);
    }
    Ok(plan)
}

fn linear_gain_curve(from: f64, to: f64) -> Result<GainCurve, TransitionPlanError> {
    GainCurve::new(vec![GainCurveSegment {
        start: 0.0,
        end: 1.0,
        points: vec![GainPoint { x: 0.0, y: from }, GainPoint { x: 1.0, y: to }],
    }])
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
    use serde_json::Value;

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

    fn materialized_volume_pair() -> (ProvidedTrack, ProvidedTrack) {
        let mut outgoing = track(TRACK_A, None, None);
        outgoing
            .metadata
            .insert("audio.fade_out_start_time".into(), "208960".into());
        outgoing
            .metadata
            .insert("audio.fade_out_duration".into(), "6090".into());
        outgoing
            .metadata
            .insert("automix.auto_preset_id".into(), "1".into());
        outgoing.metadata.insert(
            "audio.fade_out_curves".into(),
            r#"[{"start_point":0,"end_point":1,"fade_curve":[{"x":0,"y":1},{"x":0.6,"y":1},{"x":0.6,"y":0},{"x":1,"y":0}]}]"#.into(),
        );

        let mut incoming = track(TRACK_B, None, None);
        incoming
            .metadata
            .insert("audio.fade_in_start_time".into(), "2763".into());
        incoming
            .metadata
            .insert("audio.fade_in_duration".into(), "6090".into());
        incoming
            .metadata
            .insert("audio.fade_overlap".into(), "6090".into());
        incoming.metadata.insert(
            "audio.fade_in_curves".into(),
            r#"[{"start_point":0,"end_point":1,"fade_curve":[{"x":0,"y":0},{"x":0.4,"y":0},{"x":0.4,"y":1},{"x":1,"y":1}]}]"#.into(),
        );

        (outgoing, incoming)
    }

    fn read_json(path: impl AsRef<std::path::Path>) -> Value {
        let path = path.as_ref();
        serde_json::from_str(
            &std::fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display())),
        )
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
    }

    fn required<'a>(value: &'a Value, key: &str) -> &'a Value {
        value
            .get(key)
            .unwrap_or_else(|| panic!("fixture field {key:?} is missing from {value}"))
    }

    fn fixture_i64(value: &Value, key: &str) -> i64 {
        required(value, key)
            .as_i64()
            .unwrap_or_else(|| panic!("fixture field {key:?} is not an integer in {value}"))
    }

    fn fixture_f32(value: &Value, key: &str) -> f32 {
        required(value, key)
            .as_f64()
            .unwrap_or_else(|| panic!("fixture field {key:?} is not a number in {value}"))
            as f32
    }

    fn fixture_bool(value: &Value, key: &str) -> bool {
        required(value, key)
            .as_bool()
            .unwrap_or_else(|| panic!("fixture field {key:?} is not a bool in {value}"))
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
    fn local_auto_transition_materializes_oracle_geometry_and_speed() {
        let transition = crate::spotify_auto_mix::AutoRankedTransition {
            overlap: crate::spotify_auto_mix::AutoTransitionOverlap {
                start_a_ms: 208_960,
                start_b_ms: 2_763,
                duration_ms: 6_090,
                duration_bars: 2,
                speed_a: 1.0,
                speed_b: 0.903_120_4,
                is_beatmatched: true,
            },
            computed_score: 3.755_000_1,
            components: None,
            pareto_layer: Some(0),
            ranked_presets: vec![crate::spotify_auto_mix::AutoRankedPreset {
                preset_id: 1,
                computed_score: 1.0,
            }],
        };

        let plan = transition_plan_for_local_auto_transition(&transition)
            .expect("oracle local Auto transition should materialize");

        assert_eq!(plan.current_start(), Duration::from_millis(208_960));
        assert_eq!(plan.next_start(), Duration::from_millis(2_763));
        assert_eq!(plan.duration(), Duration::from_millis(6_090));
        let speed = plan
            .next_speed_automation()
            .expect("non-1.0 speedB should be attached");
        assert!((speed.speed_at(Duration::from_millis(2_763)) - 0.903_120_398_5).abs() < 1e-6);
    }

    #[test]
    fn top_ranked_oracle_beatmatched_local_auto_transitions_materialize() {
        let oracle = read_json(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../tools/spotify-automix-oracle/get_computed_transitions_2026-08-14.json"),
        );
        let computed = required(
            required(&oracle["runs"][0], "response"),
            "computedTransitions",
        )
        .as_array()
        .expect("computedTransitions must be an array");
        let mut checked = 0;

        for entry in computed {
            let ranked = &required(entry, "rankedTransitions")
                .as_array()
                .expect("rankedTransitions must be an array")[0];
            let overlap = required(ranked, "overlap");
            if !fixture_bool(overlap, "isBeatmatched") {
                continue;
            }
            let start_a_ms = fixture_i64(overlap, "startAMs");
            let start_b_ms = fixture_i64(overlap, "startBMs");
            let duration_ms = fixture_i64(overlap, "durationMs");
            let speed_b = fixture_f32(overlap, "speedB");
            let transition = crate::spotify_auto_mix::AutoRankedTransition {
                overlap: crate::spotify_auto_mix::AutoTransitionOverlap {
                    start_a_ms,
                    start_b_ms,
                    duration_ms,
                    duration_bars: fixture_i64(overlap, "durationBars") as usize,
                    speed_a: fixture_f32(overlap, "speedA"),
                    speed_b,
                    is_beatmatched: true,
                },
                computed_score: fixture_f32(ranked, "computedScore"),
                components: None,
                pareto_layer: Some(0),
                ranked_presets: vec![crate::spotify_auto_mix::AutoRankedPreset {
                    preset_id: fixture_i64(
                        required(&required(ranked, "rankedPresets")[0], "preset"),
                        "id",
                    ) as u8,
                    computed_score: 1.0,
                }],
            };

            let plan = transition_plan_for_local_auto_transition(&transition)
                .expect("top-ranked oracle beatmatch should materialize");
            assert_eq!(
                plan.current_start(),
                Duration::from_millis(start_a_ms as u64)
            );
            assert_eq!(plan.next_start(), Duration::from_millis(start_b_ms as u64));
            assert_eq!(plan.duration(), Duration::from_millis(duration_ms as u64));
            let speed = plan
                .next_speed_automation()
                .expect("beatmatched oracle transition should preserve speedB");
            assert!(
                (speed.speed_at(Duration::from_millis(start_b_ms as u64)) - f64::from(speed_b))
                    .abs()
                    < 1e-6
            );
            checked += 1;
        }

        assert_eq!(checked, 6);
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
    fn materialized_volume_pair_yields_existing_transition_plan() {
        let (outgoing, incoming) = materialized_volume_pair();
        let plan = transition_plan_for_pair(&outgoing, &incoming).unwrap();

        assert_eq!(plan.current_start(), Duration::from_millis(208_960));
        assert_eq!(plan.next_start(), Duration::from_millis(2_763));
        assert_eq!(plan.duration(), Duration::from_millis(6_090));
    }

    #[test]
    fn unsupported_materialized_dsp_falls_through() {
        let (outgoing, mut incoming) = materialized_volume_pair();
        incoming.metadata.insert(
            "audio.speed_automation".into(),
            r#"[{"from_position":0,"speed":0.90312}]"#.into(),
        );

        assert!(transition_plan_for_pair(&outgoing, &incoming).is_none());
    }

    #[test]
    fn valid_saved_recipe_precedes_materialized_volume_plan() {
        let (mut outgoing, incoming) = materialized_volume_pair();
        outgoing
            .metadata
            .insert(RECIPE_ATTRIBUTE.into(), encoded(&transition()));

        let plan = transition_plan_for_pair(&outgoing, &incoming).unwrap();
        assert_eq!(plan.current_start(), Duration::from_millis(175_000));
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
