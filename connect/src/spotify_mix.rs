use std::time::Duration;

#[cfg(test)]
use std::collections::BTreeSet;

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

use crate::spotify_mix_style::{
    ResolvedSpotifyStyle, SpotifyStyleResolutionError, resolve_spotify_style,
};

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
    #[cfg(test)]
    #[error("recipe outgoing URI does not match the playlist item")]
    TrackAMismatch,
    #[cfg(test)]
    #[error("recipe incoming URI does not match the next playlist item")]
    TrackBMismatch,
    #[cfg(test)]
    #[error("recipe outgoing item speed does not match the playlist item")]
    ItemSpeedAMismatch,
    #[cfg(test)]
    #[error("recipe incoming item speed does not match the playlist item")]
    ItemSpeedBMismatch,
    #[cfg(test)]
    #[error("recipe has no preset")]
    MissingPreset,
    #[cfg(test)]
    #[error("recipe has no outgoing volume curve override")]
    MissingOutgoingVolumeCurve,
    #[cfg(test)]
    #[error("recipe has no incoming volume curve override")]
    MissingIncomingVolumeCurve,
    #[error("preset/style resolution required (preset={0})")]
    UnsupportedPresetStyle(i32),
    #[error("tempo handling required ({0}={1})")]
    UnsupportedTempo(&'static str, f32),
    #[cfg(test)]
    #[error("EQ/filter/FX rendering required")]
    UnsupportedEffects,
    #[cfg(test)]
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

/// Semantic source selected for an authoritative Mixer edge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SpotifyTransitionSource {
    Saved,
    BackendAuto,
    LocalAuto,
    DeterministicFallback,
}

/// Where a transition candidate entered the live resolver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SpotifyTransitionProvenance {
    InlineMetadata,
    TransitionUriHydration,
    BackendMetadata,
    LocalCalculation,
    PreviewSignal,
    DeterministicPolicy,
}

/// Complete Connect ownership for one authoritative outgoing-to-incoming edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SpotifyTransitionEdge {
    pub context_uri: String,
    pub outgoing_row_uid: String,
    pub incoming_row_uid: String,
    pub canonical_a_uri: String,
    pub canonical_b_uri: String,
    pub playable_a_uri: Option<String>,
    pub playable_b_uri: Option<String>,
    pub item_speed_a_bits: Option<u64>,
    pub item_speed_b_bits: Option<u64>,
    pub session_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct SpotifyTransitionCandidate {
    pub source: SpotifyTransitionSource,
    pub provenance: SpotifyTransitionProvenance,
    pub edge: SpotifyTransitionEdge,
    pub recipe: SpotifyTransitionRecipe,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedSpotifyTransition {
    pub source: SpotifyTransitionSource,
    pub provenance: SpotifyTransitionProvenance,
    pub edge: SpotifyTransitionEdge,
    pub recipe: SpotifyTransitionRecipe,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SpotifyTransitionRejection {
    BackendDisabled,
    PreviewOnly,
    StaleEdge,
    CanonicalMismatch,
    PlayableMismatch,
    ItemSpeedMismatch,
    RowMismatch,
    MissingPreset,
    UnknownPreset(i32),
    UnsupportedRenderer { preset_id: i32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SpotifyTransitionAttempt {
    pub source: SpotifyTransitionSource,
    pub provenance: SpotifyTransitionProvenance,
    pub reason: SpotifyTransitionRejection,
}

#[derive(Clone, Debug)]
pub(crate) enum SpotifyTransitionResolution {
    Selected(ResolvedSpotifyTransition),
    #[cfg(test)]
    TerminalNone {
        source: SpotifyTransitionSource,
        provenance: SpotifyTransitionProvenance,
        edge: SpotifyTransitionEdge,
    },
    #[cfg(test)]
    LocalAuto {
        rejections: Vec<SpotifyTransitionAttempt>,
    },
}

#[derive(Clone, Debug)]
pub(crate) enum SpotifyTransitionPlanResolution {
    Selected {
        transition: ResolvedSpotifyTransition,
        style: Box<ResolvedSpotifyStyle>,
        plan: TransitionPlan,
        rejections: Vec<SpotifyTransitionAttempt>,
    },
    TerminalNone {
        source: SpotifyTransitionSource,
        provenance: SpotifyTransitionProvenance,
        edge: SpotifyTransitionEdge,
        rejections: Vec<SpotifyTransitionAttempt>,
    },
    LocalAuto {
        rejections: Vec<SpotifyTransitionAttempt>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SpotifyRecipePlanError {
    MissingPreset,
    Style(SpotifyStyleResolutionError),
    UnsupportedRenderer,
}

#[derive(Clone, Debug)]
pub(crate) enum SpotifyRecipeMaterialization {
    None {
        preset_id: i32,
    },
    Playable {
        preset_id: i32,
        style: Box<ResolvedSpotifyStyle>,
        plan: TransitionPlan,
    },
}

fn reject_candidate(
    candidate: &SpotifyTransitionCandidate,
    reason: SpotifyTransitionRejection,
) -> SpotifyTransitionAttempt {
    SpotifyTransitionAttempt {
        source: candidate.source,
        provenance: candidate.provenance,
        reason,
    }
}

fn evaluate_recipe_candidate(
    active_edge: &SpotifyTransitionEdge,
    candidate: SpotifyTransitionCandidate,
) -> Result<SpotifyTransitionResolution, SpotifyTransitionAttempt> {
    if candidate.provenance == SpotifyTransitionProvenance::PreviewSignal {
        return Err(reject_candidate(
            &candidate,
            SpotifyTransitionRejection::PreviewOnly,
        ));
    }
    if candidate.edge != *active_edge {
        return Err(reject_candidate(
            &candidate,
            SpotifyTransitionRejection::StaleEdge,
        ));
    }

    let overlap = candidate
        .recipe
        .overlap()
        .expect("decoded recipes retain a validated overlap");
    if overlap.track_a_uri() != active_edge.canonical_a_uri
        || overlap.track_b_uri() != active_edge.canonical_b_uri
    {
        return Err(reject_candidate(
            &candidate,
            SpotifyTransitionRejection::CanonicalMismatch,
        ));
    }
    if (!overlap.track_a_row_id().is_empty()
        && overlap.track_a_row_id() != active_edge.outgoing_row_uid)
        || (!overlap.track_b_row_id().is_empty()
            && overlap.track_b_row_id() != active_edge.incoming_row_uid)
    {
        return Err(reject_candidate(
            &candidate,
            SpotifyTransitionRejection::RowMismatch,
        ));
    }
    if active_edge.playable_a_uri.as_deref().is_some_and(|active| {
        !overlap.track_a_playable_uri().is_empty() && overlap.track_a_playable_uri() != active
    }) || active_edge.playable_b_uri.as_deref().is_some_and(|active| {
        !overlap.track_b_playable_uri().is_empty() && overlap.track_b_playable_uri() != active
    }) {
        return Err(reject_candidate(
            &candidate,
            SpotifyTransitionRejection::PlayableMismatch,
        ));
    }
    if active_edge
        .item_speed_a_bits
        .is_some_and(|bits| !item_speeds_match(overlap.item_speed_a(), f64::from_bits(bits)))
        || active_edge
            .item_speed_b_bits
            .is_some_and(|bits| !item_speeds_match(overlap.item_speed_b(), f64::from_bits(bits)))
    {
        return Err(reject_candidate(
            &candidate,
            SpotifyTransitionRejection::ItemSpeedMismatch,
        ));
    }

    Ok(SpotifyTransitionResolution::Selected(
        ResolvedSpotifyTransition {
            source: candidate.source,
            provenance: candidate.provenance,
            edge: candidate.edge,
            recipe: candidate.recipe,
        },
    ))
}

#[cfg(test)]
pub(crate) fn resolve_recipe_sources(
    active_edge: &SpotifyTransitionEdge,
    saved: Option<SpotifyTransitionCandidate>,
    backend: Option<SpotifyTransitionCandidate>,
    backend_enabled: bool,
) -> SpotifyTransitionResolution {
    let mut rejections = Vec::new();
    if let Some(saved) = saved {
        match evaluate_recipe_candidate(active_edge, saved) {
            Ok(SpotifyTransitionResolution::Selected(transition)) => {
                match classify_recipe_source(transition) {
                    Ok(resolution) => return resolution,
                    Err(rejection) => rejections.push(rejection),
                }
            }
            Ok(SpotifyTransitionResolution::TerminalNone { .. }) => {
                unreachable!("candidate evaluation returns validated recipes")
            }
            Ok(SpotifyTransitionResolution::LocalAuto { .. }) => {
                unreachable!("candidate evaluation returns validated recipes")
            }
            Err(rejection) => rejections.push(rejection),
        }
    }
    if let Some(backend) = backend {
        if backend_enabled {
            match evaluate_recipe_candidate(active_edge, backend) {
                Ok(SpotifyTransitionResolution::Selected(transition)) => {
                    match classify_recipe_source(transition) {
                        Ok(resolution) => return resolution,
                        Err(rejection) => rejections.push(rejection),
                    }
                }
                Ok(SpotifyTransitionResolution::TerminalNone { .. }) => {
                    unreachable!("candidate evaluation returns validated recipes")
                }
                Ok(SpotifyTransitionResolution::LocalAuto { .. }) => {
                    unreachable!("candidate evaluation returns validated recipes")
                }
                Err(rejection) => rejections.push(rejection),
            }
        } else {
            rejections.push(reject_candidate(
                &backend,
                SpotifyTransitionRejection::BackendDisabled,
            ));
        }
    }
    SpotifyTransitionResolution::LocalAuto { rejections }
}

pub(crate) fn resolve_transition_plan_sources(
    active_edge: &SpotifyTransitionEdge,
    saved: Option<SpotifyTransitionCandidate>,
    backend: Option<SpotifyTransitionCandidate>,
    backend_enabled: bool,
) -> SpotifyTransitionPlanResolution {
    let mut rejections = Vec::new();
    for candidate in [saved, backend].into_iter().flatten() {
        if candidate.source == SpotifyTransitionSource::BackendAuto && !backend_enabled {
            rejections.push(reject_candidate(
                &candidate,
                SpotifyTransitionRejection::BackendDisabled,
            ));
            continue;
        }

        match evaluate_recipe_candidate(active_edge, candidate) {
            Ok(SpotifyTransitionResolution::Selected(transition)) => {
                match materialize_spotify_recipe(&transition.recipe) {
                    Ok(SpotifyRecipeMaterialization::Playable { style, plan, .. }) => {
                        return SpotifyTransitionPlanResolution::Selected {
                            transition,
                            style,
                            plan,
                            rejections,
                        };
                    }
                    Ok(SpotifyRecipeMaterialization::None { .. }) => {
                        return SpotifyTransitionPlanResolution::TerminalNone {
                            source: transition.source,
                            provenance: transition.provenance,
                            edge: transition.edge,
                            rejections,
                        };
                    }
                    Err(SpotifyRecipePlanError::MissingPreset) => {
                        rejections.push(SpotifyTransitionAttempt {
                            source: transition.source,
                            provenance: transition.provenance,
                            reason: SpotifyTransitionRejection::MissingPreset,
                        });
                    }
                    Err(SpotifyRecipePlanError::Style(
                        SpotifyStyleResolutionError::UnknownPreset(preset_id),
                    )) => {
                        rejections.push(SpotifyTransitionAttempt {
                            source: transition.source,
                            provenance: transition.provenance,
                            reason: SpotifyTransitionRejection::UnknownPreset(preset_id),
                        });
                    }
                    Err(SpotifyRecipePlanError::Style(
                        SpotifyStyleResolutionError::UnsupportedVolumeStyle(_),
                    ))
                    | Err(SpotifyRecipePlanError::UnsupportedRenderer) => {
                        rejections.push(SpotifyTransitionAttempt {
                            source: transition.source,
                            provenance: transition.provenance,
                            reason: SpotifyTransitionRejection::UnsupportedRenderer {
                                preset_id: transition
                                    .recipe
                                    .preset()
                                    .map(Preset::id)
                                    .unwrap_or_default(),
                            },
                        });
                    }
                }
            }
            #[cfg(test)]
            Ok(SpotifyTransitionResolution::TerminalNone {
                source,
                provenance,
                edge,
            }) => {
                return SpotifyTransitionPlanResolution::TerminalNone {
                    source,
                    provenance,
                    edge,
                    rejections,
                };
            }
            #[cfg(test)]
            Ok(SpotifyTransitionResolution::LocalAuto { .. }) => {
                unreachable!("candidate evaluation only selects a recipe or terminal NONE")
            }
            Err(rejection) => rejections.push(rejection),
        }
    }

    SpotifyTransitionPlanResolution::LocalAuto { rejections }
}

pub(crate) fn materialize_spotify_recipe(
    recipe: &SpotifyTransitionRecipe,
) -> Result<SpotifyRecipeMaterialization, SpotifyRecipePlanError> {
    let overlap = recipe
        .overlap()
        .expect("decoded recipes retain a validated overlap");
    let preset = recipe
        .preset()
        .ok_or(SpotifyRecipePlanError::MissingPreset)?;
    let preset_id = preset.id();
    if preset_id == 0 {
        return Ok(SpotifyRecipeMaterialization::None { preset_id });
    }
    let style =
        resolve_spotify_style(preset, overlap, false).map_err(SpotifyRecipePlanError::Style)?;
    if !style.unsupported.is_empty()
        || style.volume_override_ignored
        || style.optional_curve_overrides_ignored
    {
        return Err(SpotifyRecipePlanError::UnsupportedRenderer);
    }

    let plan = materialize_volume_plan(overlap, &style)
        .map_err(|_| SpotifyRecipePlanError::UnsupportedRenderer)?;
    Ok(SpotifyRecipeMaterialization::Playable {
        preset_id,
        style: Box::new(style),
        plan,
    })
}

#[cfg(test)]
fn classify_recipe_source(
    transition: ResolvedSpotifyTransition,
) -> Result<SpotifyTransitionResolution, SpotifyTransitionAttempt> {
    match materialize_spotify_recipe(&transition.recipe) {
        Ok(SpotifyRecipeMaterialization::None { .. }) => {
            Ok(SpotifyTransitionResolution::TerminalNone {
                source: transition.source,
                provenance: transition.provenance,
                edge: transition.edge,
            })
        }
        Ok(SpotifyRecipeMaterialization::Playable { .. })
        | Err(SpotifyRecipePlanError::Style(
            SpotifyStyleResolutionError::UnsupportedVolumeStyle(_),
        ))
        | Err(SpotifyRecipePlanError::UnsupportedRenderer) => {
            Ok(SpotifyTransitionResolution::Selected(transition))
        }
        Err(SpotifyRecipePlanError::MissingPreset) => Err(SpotifyTransitionAttempt {
            source: transition.source,
            provenance: transition.provenance,
            reason: SpotifyTransitionRejection::MissingPreset,
        }),
        Err(SpotifyRecipePlanError::Style(SpotifyStyleResolutionError::UnknownPreset(
            preset_id,
        ))) => Err(SpotifyTransitionAttempt {
            source: transition.source,
            provenance: transition.provenance,
            reason: SpotifyTransitionRejection::UnknownPreset(preset_id),
        }),
    }
}

fn materialize_volume_plan(
    overlap: &Overlap,
    style: &ResolvedSpotifyStyle,
) -> Result<TransitionPlan, SpotifyTransitionError> {
    let speed_a = overlap.speed_a.unwrap_or(1.0);
    let speed_b = overlap.speed_b.unwrap_or(1.0);
    if !speed_a.is_finite()
        || !speed_b.is_finite()
        || speed_a <= 0.0
        || speed_b <= 0.0
        || (speed_a - 1.0).abs() > ITEM_SPEED_TOLERANCE as f32
    {
        return Err(SpotifyTransitionError::UnsupportedTempo("speedA", speed_a));
    }

    let outgoing_gain =
        adapt_curve_set(&style.outgoing_volume).map_err(SpotifyTransitionError::InvalidPlan)?;
    let incoming_gain =
        adapt_curve_set(&style.incoming_volume).map_err(SpotifyTransitionError::InvalidPlan)?;
    let mut plan = TransitionPlan::new(
        Duration::from_millis(overlap.start_a_ms() as u64),
        Duration::from_millis(overlap.start_b_ms() as u64),
        Duration::from_millis(overlap.duration_ms() as u64),
        outgoing_gain,
        incoming_gain,
    )
    .map_err(SpotifyTransitionError::InvalidPlan)?;

    if (speed_b - 1.0).abs() > ITEM_SPEED_TOLERANCE as f32 {
        let start_b = plan.next_start();
        let transition_speed = f64::from(speed_b);
        let unbounded_speed = SpeedAutomation::new(vec![SpeedPoint {
            from_position: start_b,
            speed: transition_speed,
        }])
        .map_err(|_| SpotifyTransitionError::UnsupportedTempo("speedB", speed_b))?;
        let unity_position =
            start_b + unbounded_speed.source_duration_for_wall_time(start_b, plan.duration());
        let speed = SpeedAutomation::new(vec![
            SpeedPoint {
                from_position: start_b,
                speed: transition_speed,
            },
            SpeedPoint {
                from_position: unity_position,
                speed: 1.0,
            },
        ])
        .map_err(|_| SpotifyTransitionError::UnsupportedTempo("speedB", speed_b))?;
        plan = plan.with_next_speed_automation(speed);
    }
    Ok(plan)
}

impl SpotifyTransitionRecipe {
    pub(crate) fn from_transition(transition: Transition) -> Result<Self, SpotifyTransitionError> {
        let recipe = Self { transition };
        recipe.sanity_check()?;
        Ok(recipe)
    }

    pub(crate) fn from_base64(encoded: &str) -> Result<Self, SpotifyTransitionError> {
        let bytes = BASE64
            .decode(encoded.as_bytes())
            .map_err(|_| SpotifyTransitionError::InvalidBase64)?;
        let transition = Transition::parse_from_bytes(&bytes)
            .map_err(|_| SpotifyTransitionError::InvalidProtobuf)?;
        Self::from_transition(transition)
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
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

    pub(crate) fn overlap(&self) -> Result<&Overlap, SpotifyTransitionError> {
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

    #[cfg(test)]
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

    #[cfg(test)]
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

    #[cfg(test)]
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

    #[cfg(test)]
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

    pub(crate) fn preset(&self) -> Option<&Preset> {
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

pub(crate) fn item_speeds_match(left: f64, right: f64) -> bool {
    (normalized_item_speed(left) - normalized_item_speed(right)).abs() <= ITEM_SPEED_TOLERANCE
}

fn provided_item_speed(track: &ProvidedTrack) -> Option<f64> {
    track
        .metadata
        .get(ITEM_SPEED_ATTRIBUTE)
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|speed| speed.is_finite() && *speed > 0.0)
}

pub(crate) fn provided_item_speed_bits(track: &ProvidedTrack) -> Option<u64> {
    provided_item_speed(track).map(f64::to_bits)
}

pub(crate) fn provided_item_speed_or_default(track: &ProvidedTrack) -> f32 {
    provided_item_speed(track)
        .map(|speed| speed as f32)
        .filter(|speed| speed.is_finite() && *speed > 0.0)
        .unwrap_or(1.0)
}

#[cfg(test)]
fn provided_metadata_keys(track: &ProvidedTrack) -> Vec<&str> {
    track
        .metadata
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
pub(crate) fn transition_plan_for_decoded_pair(
    outgoing: &ProvidedTrack,
    incoming: &ProvidedTrack,
    recipe: &SpotifyTransitionRecipe,
) -> Option<TransitionPlan> {
    transition_plan_for_decoded_pair_with_origin(outgoing, incoming, recipe, "saved", true)
}

pub(crate) fn transition_plan_for_local_auto_transition(
    transition: &crate::spotify_auto_mix::AutoRankedTransition,
    preset_id: u8,
) -> Result<(TransitionPlan, ResolvedSpotifyStyle), SpotifyTransitionError> {
    let overlap = transition.overlap;
    if overlap.duration_ms <= 0 || overlap.start_a_ms < 0 || overlap.start_b_ms < 0 {
        return Err(SpotifyTransitionError::InvalidDuration);
    }
    let overlap = Overlap {
        start_a_ms: Some(
            i32::try_from(overlap.start_a_ms)
                .map_err(|_| SpotifyTransitionError::InvalidDuration)?,
        ),
        start_b_ms: Some(
            i32::try_from(overlap.start_b_ms)
                .map_err(|_| SpotifyTransitionError::InvalidDuration)?,
        ),
        duration_ms: Some(
            i32::try_from(overlap.duration_ms)
                .map_err(|_| SpotifyTransitionError::InvalidDuration)?,
        ),
        speed_a: Some(overlap.speed_a),
        speed_b: Some(overlap.speed_b),
        duration_bars: Some(
            i32::try_from(overlap.duration_bars)
                .map_err(|_| SpotifyTransitionError::InvalidDuration)?,
        ),
        is_beatmatched: Some(overlap.is_beatmatched),
        ..Default::default()
    };
    let preset = Preset {
        id: Some(i32::from(preset_id)),
        ..Default::default()
    };
    let style = resolve_spotify_style(&preset, &overlap, false)
        .map_err(|_| SpotifyTransitionError::UnsupportedPresetStyle(i32::from(preset_id)))?;
    let plan = materialize_volume_plan(&overlap, &style)?;
    Ok((plan, style))
}

#[cfg(test)]
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
    use librespot_protocol::automix_transition::{Curve, CurvePoint, EqCurveOverrides, PresetType};
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
    fn local_auto_transition_bounds_incoming_speed_to_overlap_source_time() {
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
            computed_score: 3.755,
            components: None,
            pareto_layer: Some(0),
            ranked_presets: vec![crate::spotify_auto_mix::AutoRankedPreset {
                preset_id: 1,
                computed_score: 1.0,
            }],
        };

        let (plan, style) = transition_plan_for_local_auto_transition(&transition, 1)
            .expect("oracle local Auto transition should materialize");

        assert_eq!(style.preset_id, 1);
        assert_eq!(style.styles.volume, 6);
        assert_eq!(plan.current_start(), Duration::from_millis(208_960));
        assert_eq!(plan.next_start(), Duration::from_millis(2_763));
        assert_eq!(plan.duration(), Duration::from_millis(6_090));
        let speed = plan
            .next_speed_automation()
            .expect("non-1.0 speedB should be attached");
        assert!((speed.speed_at(Duration::from_millis(2_763)) - 0.903_120_398_5).abs() < 1e-6);
        let expected_unity_position = Duration::from_secs_f64(8.263_003_226_995_468);
        assert_eq!(speed.points().len(), 2);
        assert!(
            speed.points()[1]
                .from_position
                .abs_diff(expected_unity_position)
                <= Duration::from_nanos(1)
        );
        assert_eq!(speed.points()[1].speed, 1.0);
        assert_eq!(
            speed.speed_at(expected_unity_position - Duration::from_nanos(1)),
            f64::from(0.903_120_4_f32)
        );
        assert_eq!(speed.speed_at(expected_unity_position), 1.0);
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
            let preset_id = fixture_i64(
                required(&required(ranked, "rankedPresets")[0], "preset"),
                "id",
            ) as u8;
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
                    preset_id,
                    computed_score: 1.0,
                }],
            };

            let (plan, style) = transition_plan_for_local_auto_transition(&transition, preset_id)
                .expect("top-ranked oracle beatmatch should materialize");
            assert_eq!(style.preset_id, i32::from(preset_id));
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

    fn resolver_edge(generation: u64) -> SpotifyTransitionEdge {
        SpotifyTransitionEdge {
            context_uri: "spotify:playlist:mixer".into(),
            outgoing_row_uid: "row-a".into(),
            incoming_row_uid: "row-b".into(),
            canonical_a_uri: TRACK_A.into(),
            canonical_b_uri: TRACK_B.into(),
            playable_a_uri: Some(TRACK_A.into()),
            playable_b_uri: Some(TRACK_B.into()),
            item_speed_a_bits: None,
            item_speed_b_bits: None,
            session_id: "session-41".into(),
            generation,
        }
    }

    fn resolver_candidate(
        source: SpotifyTransitionSource,
        provenance: SpotifyTransitionProvenance,
        recipe: Transition,
        generation: u64,
    ) -> SpotifyTransitionCandidate {
        SpotifyTransitionCandidate {
            source,
            provenance,
            edge: resolver_edge(generation),
            recipe: SpotifyTransitionRecipe::from_base64(&encoded(&recipe)).unwrap(),
        }
    }

    #[test]
    fn resolver_saved_recipe_precedes_backend() {
        let active = resolver_edge(7);
        let mut saved = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::InlineMetadata,
            transition(),
            7,
        );
        saved.edge = active.clone();
        let backend = resolver_candidate(
            SpotifyTransitionSource::BackendAuto,
            SpotifyTransitionProvenance::BackendMetadata,
            transition(),
            7,
        );

        let resolved = resolve_recipe_sources(&active, Some(saved), Some(backend), true);
        assert!(matches!(
            resolved,
            SpotifyTransitionResolution::Selected(ResolvedSpotifyTransition {
                source: SpotifyTransitionSource::Saved,
                ..
            })
        ));
    }

    #[test]
    fn resolver_backend_is_selected_when_enabled_and_saved_is_absent() {
        let active = resolver_edge(7);
        let backend = resolver_candidate(
            SpotifyTransitionSource::BackendAuto,
            SpotifyTransitionProvenance::BackendMetadata,
            transition(),
            7,
        );

        let resolved = resolve_recipe_sources(&active, None, Some(backend), true);
        assert!(matches!(
            resolved,
            SpotifyTransitionResolution::Selected(ResolvedSpotifyTransition {
                source: SpotifyTransitionSource::BackendAuto,
                ..
            })
        ));
        assert!(matches!(
            resolve_recipe_sources(&active, None, None, true),
            SpotifyTransitionResolution::LocalAuto { .. }
        ));
    }

    #[test]
    fn resolver_known_none_is_terminal_even_with_overrides() {
        let active = resolver_edge(7);
        let mut none = transition();
        let preset = none.preset.as_mut().unwrap();
        preset.id = Some(0);
        preset.eq_style_override = MessageField::some(Default::default());
        let saved = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::InlineMetadata,
            none,
            7,
        );

        assert!(matches!(
            resolve_recipe_sources(&active, Some(saved), None, true),
            SpotifyTransitionResolution::TerminalNone {
                source: SpotifyTransitionSource::Saved,
                ..
            }
        ));
    }

    #[test]
    fn resolver_unknown_preset_continues_to_local_auto() {
        let active = resolver_edge(7);
        let mut unknown = transition();
        unknown.preset.as_mut().unwrap().id = Some(999);
        let saved = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::InlineMetadata,
            unknown,
            7,
        );

        let resolved = resolve_recipe_sources(&active, Some(saved), None, true);
        let SpotifyTransitionResolution::LocalAuto { rejections } = resolved else {
            panic!("unknown preset must continue fallback");
        };
        assert_eq!(
            rejections[0].reason,
            SpotifyTransitionRejection::UnknownPreset(999)
        );
    }

    #[test]
    fn resolver_stale_edge_continues_to_local_auto() {
        let active = resolver_edge(8);
        let stale = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::TransitionUriHydration,
            transition(),
            7,
        );

        let resolved = resolve_recipe_sources(&active, Some(stale), None, true);
        let SpotifyTransitionResolution::LocalAuto { rejections } = resolved else {
            panic!("stale saved candidate must continue fallback");
        };
        assert_eq!(rejections[0].reason, SpotifyTransitionRejection::StaleEdge);
    }

    #[test]
    fn resolver_canonical_and_playable_mismatches_are_rejected() {
        let active = resolver_edge(7);
        let mut canonical_mismatch = transition();
        canonical_mismatch.overlap.as_mut().unwrap().track_b_uri = Some(TRACK_A.into());
        let saved = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::InlineMetadata,
            canonical_mismatch,
            7,
        );
        let SpotifyTransitionResolution::LocalAuto { rejections } =
            resolve_recipe_sources(&active, Some(saved), None, true)
        else {
            panic!("canonical mismatch must continue fallback");
        };
        assert_eq!(
            rejections[0].reason,
            SpotifyTransitionRejection::CanonicalMismatch
        );

        let mut playable_mismatch = transition();
        let overlap = playable_mismatch.overlap.as_mut().unwrap();
        overlap.track_a_playable_uri = Some(TRACK_B.into());
        overlap.track_b_playable_uri = Some(TRACK_B.into());
        let saved = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::InlineMetadata,
            playable_mismatch,
            7,
        );
        let SpotifyTransitionResolution::LocalAuto { rejections } =
            resolve_recipe_sources(&active, Some(saved), None, true)
        else {
            panic!("playable mismatch must continue fallback");
        };
        assert_eq!(
            rejections[0].reason,
            SpotifyTransitionRejection::PlayableMismatch
        );

        let mut active = resolver_edge(7);
        active.item_speed_a_bits = Some(1.25_f64.to_bits());
        let mut saved = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::InlineMetadata,
            transition(),
            7,
        );
        saved.edge = active.clone();
        let SpotifyTransitionResolution::LocalAuto { rejections } =
            resolve_recipe_sources(&active, Some(saved), None, true)
        else {
            panic!("item-speed mismatch must continue fallback");
        };
        assert_eq!(
            rejections[0].reason,
            SpotifyTransitionRejection::ItemSpeedMismatch
        );
    }

    #[test]
    fn resolver_preview_payload_cannot_authorize_live_playback() {
        let active = resolver_edge(7);
        let preview = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::PreviewSignal,
            transition(),
            7,
        );

        let SpotifyTransitionResolution::LocalAuto { rejections } =
            resolve_recipe_sources(&active, Some(preview), None, true)
        else {
            panic!("preview candidate must continue fallback");
        };
        assert_eq!(
            rejections[0].reason,
            SpotifyTransitionRejection::PreviewOnly
        );

        let preview = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::PreviewSignal,
            transition(),
            7,
        );
        let SpotifyTransitionPlanResolution::LocalAuto { rejections } =
            resolve_transition_plan_sources(&active, Some(preview), None, true)
        else {
            panic!("preview candidate must not materialize live playback");
        };
        assert_eq!(
            rejections[0].reason,
            SpotifyTransitionRejection::PreviewOnly
        );
    }

    fn volume_only_transition() -> Transition {
        let mut value = transition();
        let preset = value.preset.as_mut().unwrap();
        preset.id = Some(11);
        preset.volume_out_curve_override.clear();
        preset.volume_in_curve_override.clear();
        value
    }

    #[test]
    fn adapter_materializes_resolved_volume_style_without_inline_curves() {
        let active = resolver_edge(7);
        let saved = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::InlineMetadata,
            volume_only_transition(),
            7,
        );

        let SpotifyTransitionPlanResolution::Selected { plan, style, .. } =
            resolve_transition_plan_sources(&active, Some(saved), None, true)
        else {
            panic!("volume-only preset should materialize");
        };
        assert_eq!(style.preset_id, 11);
        assert_eq!(style.styles.volume, 1);
        assert_eq!(plan.current_start(), Duration::from_millis(175_000));
        assert_eq!(plan.next_start(), Duration::from_millis(12_000));
        assert_eq!(plan.duration(), Duration::from_millis(8_000));
    }

    #[test]
    fn shared_materializer_preserves_saved_live_plan() {
        let active = resolver_edge(7);
        let saved = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::InlineMetadata,
            volume_only_transition(),
            7,
        );
        let expected = match materialize_spotify_recipe(&saved.recipe).unwrap() {
            SpotifyRecipeMaterialization::Playable { plan, .. } => plan,
            SpotifyRecipeMaterialization::None { .. } => panic!("saved recipe must render"),
        };

        let SpotifyTransitionPlanResolution::Selected { plan, .. } =
            resolve_transition_plan_sources(&active, Some(saved), None, true)
        else {
            panic!("saved recipe must remain selected");
        };
        assert_eq!(plan.current_start(), expected.current_start());
        assert_eq!(plan.next_start(), expected.next_start());
        assert_eq!(plan.duration(), expected.duration());
        assert_eq!(
            plan.next_speed_automation(),
            expected.next_speed_automation()
        );
        assert_eq!(plan, expected, "gain curves and complete plan must match");
    }

    #[test]
    fn adapter_unsupported_saved_style_continues_to_backend() {
        let active = resolver_edge(7);
        let mut unsupported = transition();
        unsupported.preset.as_mut().unwrap().id = Some(2);
        let saved = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::InlineMetadata,
            unsupported,
            7,
        );
        let backend = resolver_candidate(
            SpotifyTransitionSource::BackendAuto,
            SpotifyTransitionProvenance::BackendMetadata,
            volume_only_transition(),
            7,
        );

        let SpotifyTransitionPlanResolution::Selected {
            transition,
            rejections,
            ..
        } = resolve_transition_plan_sources(&active, Some(saved), Some(backend), true)
        else {
            panic!("backend should follow unsupported saved style");
        };
        assert_eq!(transition.source, SpotifyTransitionSource::BackendAuto);
        assert!(matches!(
            rejections[0].reason,
            SpotifyTransitionRejection::UnsupportedRenderer { preset_id: 2 }
        ));
    }

    #[test]
    fn adapter_none_is_terminal_before_backend() {
        let active = resolver_edge(7);
        let mut none = volume_only_transition();
        none.preset.as_mut().unwrap().id = Some(0);
        let saved = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::InlineMetadata,
            none,
            7,
        );
        let backend = resolver_candidate(
            SpotifyTransitionSource::BackendAuto,
            SpotifyTransitionProvenance::BackendMetadata,
            volume_only_transition(),
            7,
        );

        assert!(matches!(
            resolve_transition_plan_sources(&active, Some(saved), Some(backend), true),
            SpotifyTransitionPlanResolution::TerminalNone {
                source: SpotifyTransitionSource::Saved,
                ..
            }
        ));
    }

    #[test]
    fn adapter_preserves_supported_incoming_speed_automation() {
        let active = resolver_edge(7);
        let mut value = volume_only_transition();
        value.overlap.as_mut().unwrap().speed_b = Some(0.9);
        let saved = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::InlineMetadata,
            value,
            7,
        );

        let SpotifyTransitionPlanResolution::Selected { plan, .. } =
            resolve_transition_plan_sources(&active, Some(saved), None, true)
        else {
            panic!("supported incoming speed should materialize");
        };
        let speed = plan.next_speed_automation().unwrap();
        assert_eq!(
            speed.points()[0].from_position,
            Duration::from_millis(12_000)
        );
        assert!((speed.points()[0].speed - 0.9).abs() < 1e-6);
        assert_eq!(speed.points()[1].speed, 1.0);
    }

    #[test]
    fn adapter_backend_failure_continues_to_local_auto() {
        let active = resolver_edge(7);
        let mut unknown = volume_only_transition();
        unknown.preset.as_mut().unwrap().id = Some(999);
        let backend = resolver_candidate(
            SpotifyTransitionSource::BackendAuto,
            SpotifyTransitionProvenance::BackendMetadata,
            unknown,
            7,
        );

        assert!(matches!(
            resolve_transition_plan_sources(&active, None, Some(backend), true),
            SpotifyTransitionPlanResolution::LocalAuto { .. }
        ));
    }

    #[test]
    fn adapter_rejects_curve_overrides_while_effective_gate_is_unknown() {
        let active = resolver_edge(7);
        let mut value = transition();
        value.preset.as_mut().unwrap().id = Some(11);
        let saved = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::InlineMetadata,
            value,
            7,
        );

        let SpotifyTransitionPlanResolution::LocalAuto { rejections } =
            resolve_transition_plan_sources(&active, Some(saved), None, true)
        else {
            panic!("unverified curve override must continue fallback");
        };
        assert_eq!(
            rejections[0].reason,
            SpotifyTransitionRejection::UnsupportedRenderer { preset_id: 11 }
        );

        let mut value = volume_only_transition();
        value.preset.as_mut().unwrap().eq_out_curve_overrides =
            MessageField::some(EqCurveOverrides::default());
        let saved = resolver_candidate(
            SpotifyTransitionSource::Saved,
            SpotifyTransitionProvenance::InlineMetadata,
            value,
            7,
        );
        let SpotifyTransitionPlanResolution::LocalAuto { rejections } =
            resolve_transition_plan_sources(&active, Some(saved), None, true)
        else {
            panic!("unverified optional curve override must continue fallback");
        };
        assert_eq!(
            rejections[0].reason,
            SpotifyTransitionRejection::UnsupportedRenderer { preset_id: 11 }
        );
    }
}
