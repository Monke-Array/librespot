use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequirements {
    pub schema_version: String,
    pub plan_schema_version: String,
    pub required: Vec<String>,
    pub max_envelope_points: i64,
    pub max_bands: i64,
    pub max_taps: i64,
    pub max_state_span_frames: i64,
    pub max_abs_rate_delta_ppm: i64,
    pub lookahead_frames: i64,
}

impl CapabilityRequirements {
    pub fn requirements(&self) -> &[String] {
        &self.required
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererCapabilities {
    pub schema_version: String,
    pub supported_plan_versions: Vec<String>,
    pub supported_requirements: Vec<String>,
    pub max_envelope_points: i64,
    pub max_bands: i64,
    pub max_taps: i64,
    pub max_state_span_frames: i64,
    pub max_abs_rate_delta_ppm: i64,
    pub max_lookahead_frames: i64,
    pub simplifications: Vec<CapabilitySimplification>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySimplification {
    pub unsupported_requirement: String,
    pub transform_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SupportResult {
    Supported,
    SupportedWithSimplification { transform_id: String },
    Unsupported,
}

pub fn derive_capabilities(plan: &OperatorPlan) -> CapabilityRequirements {
    let mut required = BTreeSet::from([
        "format:pcm_f64_stereo_44100_v1".to_string(),
        "limiter:lookahead_peak_limiter_v1".to_string(),
        "output_safety:transition_output_safety_v1".to_string(),
        "source:pcm_s16le_stereo_44100_v1".to_string(),
        "true_peak:bs1770_4x_v1".to_string(),
    ]);
    required.insert(format!("plan:{}", plan.schema_version));
    let mut max_envelope_points = 0usize;
    let mut max_bands = 0usize;
    let mut max_taps = 0usize;
    let mut max_state_span_frames = plan.output_safety.lookahead_frames;
    let mut max_abs_rate_delta_ppm = 0i64;
    for operation in &plan.operations {
        required.insert(format!("operation:{}", operation.kind()));
        match operation {
            Operation::TimeMap(value) => {
                required.insert("profile:pitch_preserving_balanced_transients_v1".into());
                required.insert(format!("time_stretch_rate_ppm:{}", value.source_rate_ppm));
                max_abs_rate_delta_ppm = (value.source_rate_ppm - 1_000_000).abs();
            }
            Operation::GainEnvelope(value) => add_envelope(
                &mut required,
                &mut max_envelope_points,
                &value.points,
                &value.interpolations,
            ),
            Operation::FilterEnvelope(value) => {
                required.insert(format!(
                    "profile:{}",
                    match value.filter_kind {
                        FilterKind::LowpassBiquadV1 => "lowpass_biquad_v1",
                        FilterKind::HighpassBiquadV1 => "highpass_biquad_v1",
                    }
                ));
                max_envelope_points = max_envelope_points
                    .max(value.cutoff_points.len())
                    .max(value.wet_points.len());
                add_interpolations(&mut required, &value.cutoff_interpolations);
                add_interpolations(&mut required, &value.wet_interpolations);
            }
            Operation::CrossoverBandGain(value) => {
                required.insert("profile:linkwitz_riley_4_v1".into());
                max_bands = max_bands.max(value.bands.len());
                for envelope in &value.band_gain_envelopes {
                    add_envelope(
                        &mut required,
                        &mut max_envelope_points,
                        &envelope.points,
                        &envelope.interpolations,
                    );
                }
                add_envelope(
                    &mut required,
                    &mut max_envelope_points,
                    &value.wet_points,
                    &value.wet_interpolations,
                );
            }
            Operation::DuckEnvelope(value) => add_envelope(
                &mut required,
                &mut max_envelope_points,
                &value.points,
                &value.interpolations,
            ),
            Operation::FeedforwardDelayTail(value) => {
                max_taps = max_taps.max(value.taps.len());
                if let Some(last) = value.taps.last() {
                    max_state_span_frames = max_state_span_frames.max(last.delay_frames);
                }
            }
            Operation::RhythmicGate(value) => add_envelope(
                &mut required,
                &mut max_envelope_points,
                &value.points,
                &value.interpolations,
            ),
        }
    }
    required.insert(format!(
        "limit:max_abs_rate_delta_ppm:{max_abs_rate_delta_ppm}"
    ));
    required.insert(format!("limit:max_bands:{max_bands}"));
    required.insert(format!("limit:max_envelope_points:{max_envelope_points}"));
    required.insert(format!(
        "limit:max_state_span_frames:{max_state_span_frames}"
    ));
    required.insert(format!("limit:max_taps:{max_taps}"));
    required.insert(format!(
        "lookahead_frames:{}",
        plan.output_safety.lookahead_frames
    ));
    CapabilityRequirements {
        schema_version: "capability-derivation/1".into(),
        plan_schema_version: plan.schema_version.clone(),
        required: required.into_iter().collect(),
        max_envelope_points: max_envelope_points as i64,
        max_bands: max_bands as i64,
        max_taps: max_taps as i64,
        max_state_span_frames,
        max_abs_rate_delta_ppm,
        lookahead_frames: plan.output_safety.lookahead_frames,
    }
}

fn add_envelope(
    required: &mut BTreeSet<String>,
    max: &mut usize,
    points: &[EnvelopePoint],
    interpolations: &[Interpolation],
) {
    *max = (*max).max(points.len());
    add_interpolations(required, interpolations);
}

fn add_interpolations(required: &mut BTreeSet<String>, values: &[Interpolation]) {
    for value in values {
        required.insert(format!(
            "interpolation:{}",
            match value {
                Interpolation::Hold => "hold",
                Interpolation::Linear => "linear",
                Interpolation::Smoothstep => "smoothstep",
                Interpolation::QuarterSine => "quarter_sine",
                Interpolation::QuarterCosine => "quarter_cosine",
            }
        ));
    }
}

pub fn compare_capabilities(
    requirements: &CapabilityRequirements,
    profile: &RendererCapabilities,
) -> SupportResult {
    if profile.schema_version != "renderer-capabilities/1"
        || !strictly_sorted_unique(&profile.supported_plan_versions)
        || !strictly_sorted_unique(&profile.supported_requirements)
        || !profile
            .supported_plan_versions
            .contains(&requirements.plan_schema_version)
        || requirements.max_envelope_points > profile.max_envelope_points
        || requirements.max_bands > profile.max_bands
        || requirements.max_taps > profile.max_taps
        || requirements.max_state_span_frames > profile.max_state_span_frames
        || requirements.max_abs_rate_delta_ppm > profile.max_abs_rate_delta_ppm
        || requirements.lookahead_frames > profile.max_lookahead_frames
    {
        return SupportResult::Unsupported;
    }
    let missing: Vec<_> = requirements
        .required
        .iter()
        .filter(|value| !profile.supported_requirements.contains(value))
        .collect();
    if missing.is_empty() {
        return SupportResult::Supported;
    }
    let transforms: BTreeSet<_> = missing
        .iter()
        .filter_map(|missing| {
            profile
                .simplifications
                .iter()
                .find(|entry| &entry.unsupported_requirement == *missing)
                .map(|entry| entry.transform_id.clone())
        })
        .collect();
    if transforms.len() == 1
        && missing.iter().all(|missing| {
            profile
                .simplifications
                .iter()
                .any(|entry| &entry.unsupported_requirement == *missing)
        })
    {
        SupportResult::SupportedWithSimplification {
            transform_id: transforms.into_iter().next().unwrap(),
        }
    } else {
        SupportResult::Unsupported
    }
}

fn strictly_sorted_unique(values: &[String]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}
