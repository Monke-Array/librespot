use crate::error::{Error, Result};
use crate::geometry::{DurationMode, GeometryProposal};
use crate::model::Operation;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TemplateId {
    SafeCrossfade,
    ShapedHandoff,
    BeatCut,
    BassHandoff,
    SpectralHandoff,
    DuckedOverlap,
    EchoTailHandoff,
    EnergyRamp,
    RhythmicHandoff,
}

impl TemplateId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SafeCrossfade => "safe_crossfade",
            Self::ShapedHandoff => "shaped_handoff",
            Self::BeatCut => "beat_cut",
            Self::BassHandoff => "bass_handoff",
            Self::SpectralHandoff => "spectral_handoff",
            Self::DuckedOverlap => "ducked_overlap",
            Self::EchoTailHandoff => "echo_tail_handoff",
            Self::EnergyRamp => "energy_ramp",
            Self::RhythmicHandoff => "rhythmic_handoff",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipeId(String);

impl RecipeId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecipeDuration {
    Cut,
    Seconds(i64),
    Bars(i64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecipeSpec {
    pub id: &'static str,
    pub index: usize,
    pub duration: RecipeDuration,
}

#[derive(Clone, Copy, Debug)]
pub struct TemplateFamily {
    pub id: TemplateId,
    pub quota: usize,
    pub recipes: &'static [RecipeSpec],
}

#[derive(Clone, Debug, Default)]
pub struct TemplateInputs {
    pub cue_confidence_ppm: Option<i64>,
    pub beat_confidence_ppm: Option<i64>,
    pub downbeat_confidence_ppm: Option<i64>,
    pub alignment_error_ppm_of_beat: Option<i64>,
    pub outgoing_vocal_activity_ppm: Option<i64>,
    pub incoming_vocal_activity_ppm: Option<i64>,
    pub outgoing_vocal_sustained: Option<bool>,
    pub vocal_collision_ppm: Option<i64>,
    pub vocal_collision_span_frames: Option<i64>,
    pub vocal_collision_start_frame: Option<i64>,
    pub vocal_collision_end_frame: Option<i64>,
    pub outgoing_vocal_collision_strength_ppm: Option<i64>,
    pub incoming_vocal_collision_strength_ppm: Option<i64>,
    pub transient_collision_ppm: Option<i64>,
    pub transient_collision_span_frames: Option<i64>,
    pub transient_collision_start_frame: Option<i64>,
    pub transient_collision_end_frame: Option<i64>,
    pub outgoing_transient_collision_strength_ppm: Option<i64>,
    pub incoming_transient_collision_strength_ppm: Option<i64>,
    pub two_beats_frames: Option<i64>,
    pub outgoing_transient_activity_ppm: Option<i64>,
    pub incoming_transient_activity_ppm: Option<i64>,
    pub outgoing_transient_density_ppm: Option<i64>,
    pub outgoing_bass_occupancy_ppm: Option<i64>,
    pub incoming_bass_occupancy_ppm: Option<i64>,
    pub bass_collision_ppm: Option<i64>,
    pub outgoing_spectral_stability_ppm: Option<i64>,
    pub incoming_spectral_stability_ppm: Option<i64>,
    pub spectral_overlap_ppm: Option<i64>,
    pub outgoing_energy_variability_mdb: Option<i64>,
    pub incoming_energy_variability_mdb: Option<i64>,
    pub energy_delta_mdb: Option<i64>,
    pub outgoing_hard_cut_safe: Option<bool>,
    pub incoming_hard_cut_safe: Option<bool>,
    pub beat_frames: Vec<i64>,
    pub meter_beats: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FamilyApplicability {
    pub id: TemplateId,
    pub applicable: bool,
    pub need_score_ppm: i64,
}

#[derive(Clone, Debug)]
pub struct TemplateDraft {
    pub template_id: TemplateId,
    pub recipe_id: RecipeId,
    pub geometry_id: String,
    pub dry_start_frame: i64,
    pub effect_end_frame: i64,
    pub operations: Vec<Operation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectionRecord {
    pub template_id: TemplateId,
    pub recipe_id: String,
    pub geometry_id: Option<String>,
    pub code: &'static str,
    pub message: String,
}

impl TemplateFamily {
    pub fn applicability(self, inputs: &TemplateInputs) -> FamilyApplicability {
        let reliable_cue = inputs
            .cue_confidence_ppm
            .is_some_and(|value| value >= 700_000);
        let reliable_rhythm = reliable_cue
            && inputs
                .beat_confidence_ppm
                .is_some_and(|value| value >= 750_000)
            && inputs
                .downbeat_confidence_ppm
                .is_some_and(|value| value >= 700_000)
            && inputs
                .alignment_error_ppm_of_beat
                .is_some_and(|value| value <= 31_250);
        let vocal_localized = localized(
            inputs.vocal_collision_ppm,
            inputs.vocal_collision_span_frames,
            inputs.two_beats_frames,
        );
        let transient_localized = localized(
            inputs.transient_collision_ppm,
            inputs.transient_collision_span_frames,
            inputs.two_beats_frames,
        );
        let sustained_collision = sustained(
            inputs.vocal_collision_ppm,
            inputs.vocal_collision_span_frames,
            inputs.two_beats_frames,
        ) || sustained(
            inputs.transient_collision_ppm,
            inputs.transient_collision_span_frames,
            inputs.two_beats_frames,
        );
        let score = match self.id {
            TemplateId::SafeCrossfade => 0,
            TemplateId::ShapedHandoff => 250_000
                .max(inputs.spectral_overlap_ppm.unwrap_or(0))
                .max(inputs.bass_collision_ppm.unwrap_or(0)),
            TemplateId::BeatCut => inputs
                .vocal_collision_ppm
                .unwrap_or(0)
                .max(inputs.transient_collision_ppm.unwrap_or(0))
                .max(inputs.bass_collision_ppm.unwrap_or(0)),
            TemplateId::BassHandoff => inputs.bass_collision_ppm.unwrap_or(0),
            TemplateId::SpectralHandoff => inputs.spectral_overlap_ppm.unwrap_or(0),
            TemplateId::DuckedOverlap => {
                if vocal_localized {
                    inputs.vocal_collision_ppm.unwrap_or(0)
                } else {
                    inputs.transient_collision_ppm.unwrap_or(0)
                }
            }
            TemplateId::EchoTailHandoff => {
                1_000_000
                    - inputs
                        .outgoing_vocal_activity_ppm
                        .unwrap_or(1_000_000)
                        .max(inputs.outgoing_transient_density_ppm.unwrap_or(1_000_000))
            }
            TemplateId::EnergyRamp => inputs
                .energy_delta_mdb
                .and_then(i64::checked_abs)
                .unwrap_or(0)
                .saturating_mul(100)
                .min(1_000_000),
            TemplateId::RhythmicHandoff => inputs.outgoing_transient_activity_ppm.unwrap_or(0),
        };
        let applicable = match self.id {
            TemplateId::SafeCrossfade => true,
            TemplateId::ShapedHandoff => reliable_cue && !sustained_collision,
            TemplateId::BeatCut => {
                reliable_rhythm && inputs.outgoing_vocal_sustained == Some(false)
            }
            TemplateId::BassHandoff => {
                reliable_rhythm
                    && inputs
                        .outgoing_bass_occupancy_ppm
                        .is_some_and(|value| value >= 250_000)
                        .then_some(true)
                        .or_else(|| {
                            inputs
                                .incoming_bass_occupancy_ppm
                                .is_some_and(|value| value >= 250_000)
                                .then_some(true)
                        })
                        .unwrap_or(false)
                    && inputs
                        .bass_collision_ppm
                        .is_some_and(|value| value >= 350_000)
                    && !sustained_collision
            }
            TemplateId::SpectralHandoff => {
                reliable_cue
                    && inputs
                        .outgoing_spectral_stability_ppm
                        .is_some_and(|value| value >= 700_000)
                    && inputs
                        .incoming_spectral_stability_ppm
                        .is_some_and(|value| value >= 700_000)
                    && inputs
                        .spectral_overlap_ppm
                        .is_some_and(|value| value >= 450_000)
                    && !sustained_collision
            }
            TemplateId::DuckedOverlap => {
                reliable_cue
                    && inputs.vocal_collision_ppm.is_some()
                    && inputs.transient_collision_ppm.is_some()
                    && (vocal_localized ^ transient_localized)
                    && !sustained_collision
            }
            TemplateId::EchoTailHandoff => {
                reliable_rhythm
                    && inputs
                        .outgoing_vocal_activity_ppm
                        .is_some_and(|value| value <= 200_000)
                    && inputs
                        .outgoing_transient_density_ppm
                        .is_some_and(|value| value <= 350_000)
            }
            TemplateId::EnergyRamp => {
                reliable_cue
                    && inputs
                        .outgoing_energy_variability_mdb
                        .is_some_and(|value| value <= 2_000)
                    && inputs
                        .incoming_energy_variability_mdb
                        .is_some_and(|value| value <= 2_000)
                    && inputs
                        .energy_delta_mdb
                        .and_then(i64::checked_abs)
                        .is_some_and(|value| value >= 3_000)
                    && !sustained_collision
            }
            TemplateId::RhythmicHandoff => {
                reliable_rhythm
                    && inputs
                        .outgoing_transient_activity_ppm
                        .is_some_and(|value| value >= 600_000)
                    && inputs
                        .outgoing_vocal_activity_ppm
                        .is_some_and(|value| value <= 200_000)
            }
        };
        FamilyApplicability {
            id: self.id,
            applicable,
            need_score_ppm: score,
        }
    }
}

pub fn bind_recipe<'a>(
    recipe: &RecipeSpec,
    geometries: &'a [GeometryProposal],
) -> Result<&'a GeometryProposal> {
    let mut compatible: Vec<_> = geometries
        .iter()
        .filter(|geometry| duration_matches(recipe.duration, geometry))
        .collect();
    compatible.sort_by(|left, right| {
        right
            .geometry_quality_ppm
            .cmp(&left.geometry_quality_ppm)
            .then_with(|| left.geometry_id.cmp(&right.geometry_id))
    });
    compatible
        .get(recipe.index % compatible.len().max(1))
        .copied()
        .ok_or_else(|| {
            Error::new(
                "NO_COMPATIBLE_GEOMETRY",
                "recipe has no compatible geometry",
            )
        })
}

fn duration_matches(duration: RecipeDuration, geometry: &GeometryProposal) -> bool {
    match duration {
        RecipeDuration::Cut => geometry.duration_mode == DurationMode::Cut,
        RecipeDuration::Seconds(frames) => {
            geometry.duration_mode == DurationMode::Seconds
                && geometry.requested_dry_frames == frames
        }
        RecipeDuration::Bars(bars) => {
            geometry.duration_mode == DurationMode::Bars && geometry.resolved_bar_count == bars
        }
    }
}

fn localized(value: Option<i64>, span: Option<i64>, two_beats: Option<i64>) -> bool {
    matches!(value, Some(300_000..=700_000))
        && matches!((span, two_beats), (Some(span), Some(limit)) if span <= limit)
}

fn sustained(value: Option<i64>, span: Option<i64>, two_beats: Option<i64>) -> bool {
    value.is_some_and(|value| value > 700_000)
        || matches!((span, two_beats), (Some(span), Some(limit)) if span > limit)
}
