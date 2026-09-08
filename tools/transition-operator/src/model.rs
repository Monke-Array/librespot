use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorPlan {
    pub schema_version: String,
    pub plan_id: String,
    pub template: TemplateRef,
    pub format: AudioFormat,
    pub sources: Sources,
    pub timeline: Timeline,
    pub operations: Vec<Operation>,
    pub output_safety: OutputSafety,
    pub feature_snapshot: FeatureSnapshotRef,
    pub provenance: Provenance,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorPlanBody {
    pub schema_version: String,
    pub template: TemplateRef,
    pub format: AudioFormat,
    pub sources: Sources,
    pub timeline: Timeline,
    pub operations: Vec<Operation>,
    pub output_safety: OutputSafety,
    pub feature_snapshot: FeatureSnapshotRef,
    pub provenance: Provenance,
}

impl OperatorPlan {
    pub fn body(&self) -> OperatorPlanBody {
        OperatorPlanBody {
            schema_version: self.schema_version.clone(),
            template: self.template.clone(),
            format: self.format.clone(),
            sources: self.sources.clone(),
            timeline: self.timeline.clone(),
            operations: self.operations.clone(),
            output_safety: self.output_safety.clone(),
            feature_snapshot: self.feature_snapshot.clone(),
            provenance: self.provenance.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateRef {
    pub id: String,
    pub version: i64,
    pub recipe_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioFormat {
    pub sample_rate_hz: i64,
    pub channels: i64,
    pub channel_order: ChannelOrder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelOrder {
    StereoLr,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sources {
    pub outgoing: SourceRef,
    pub incoming: SourceRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub track_id: String,
    pub pcm_profile: String,
    pub pcm_sha256: String,
    pub pcm_frame_count: i64,
    pub cue_id: String,
    pub cue_source_frame: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timeline {
    pub dry_start_frame: i64,
    pub dry_handoff_frame: i64,
    pub effect_end_frame: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Outgoing,
    Incoming,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    Hold,
    Linear,
    Smoothstep,
    QuarterSine,
    QuarterCosine,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvelopePoint {
    pub frame: i64,
    pub value_ppm: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CutoffPoint {
    pub frame: i64,
    pub cutoff_millihz: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Operation {
    TimeMap(TimeMap),
    GainEnvelope(GainEnvelope),
    FilterEnvelope(FilterEnvelope),
    CrossoverBandGain(CrossoverBandGain),
    DuckEnvelope(DuckEnvelope),
    FeedforwardDelayTail(FeedforwardDelayTail),
    RhythmicGate(RhythmicGate),
}

impl Operation {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::TimeMap(_) => "time_map",
            Self::GainEnvelope(_) => "gain_envelope",
            Self::FilterEnvelope(_) => "filter_envelope",
            Self::CrossoverBandGain(_) => "crossover_band_gain",
            Self::DuckEnvelope(_) => "duck_envelope",
            Self::FeedforwardDelayTail(_) => "feedforward_delay_tail",
            Self::RhythmicGate(_) => "rhythmic_gate",
        }
    }

    pub fn op_id(&self) -> &str {
        match self {
            Self::TimeMap(value) => &value.op_id,
            Self::GainEnvelope(value) => &value.op_id,
            Self::FilterEnvelope(value) => &value.op_id,
            Self::CrossoverBandGain(value) => &value.op_id,
            Self::DuckEnvelope(value) => &value.op_id,
            Self::FeedforwardDelayTail(value) => &value.op_id,
            Self::RhythmicGate(value) => &value.op_id,
        }
    }

    pub fn target(&self) -> Target {
        match self {
            Self::TimeMap(value) => value.target,
            Self::GainEnvelope(value) => value.target,
            Self::FilterEnvelope(value) => value.target,
            Self::CrossoverBandGain(value) => value.target,
            Self::DuckEnvelope(value) => value.target,
            Self::FeedforwardDelayTail(value) => value.target,
            Self::RhythmicGate(value) => value.target,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeMap {
    pub op_id: String,
    pub target: Target,
    pub source_rate_ppm: i64,
    pub profile: TimeMapProfile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeMapProfile {
    PitchPreservingBalancedTransientsV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GainEnvelope {
    pub op_id: String,
    pub target: Target,
    pub points: Vec<EnvelopePoint>,
    pub interpolations: Vec<Interpolation>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterEnvelope {
    pub op_id: String,
    pub target: Target,
    pub filter_kind: FilterKind,
    pub cutoff_points: Vec<CutoffPoint>,
    pub cutoff_interpolations: Vec<Interpolation>,
    pub q_milli: i64,
    pub wet_points: Vec<EnvelopePoint>,
    pub wet_interpolations: Vec<Interpolation>,
    pub control_interval_frames: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterKind {
    LowpassBiquadV1,
    HighpassBiquadV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrossoverBandGain {
    pub op_id: String,
    pub target: Target,
    pub profile: CrossoverProfile,
    pub crossover_millihz: Vec<i64>,
    pub bands: Vec<Band>,
    pub band_gain_envelopes: Vec<BandGainEnvelope>,
    pub wet_points: Vec<EnvelopePoint>,
    pub wet_interpolations: Vec<Interpolation>,
    pub control_interval_frames: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossoverProfile {
    #[serde(rename = "linkwitz_riley_4_v1")]
    LinkwitzRiley4V1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Band {
    Low,
    Mid,
    High,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BandGainEnvelope {
    pub band: Band,
    pub points: Vec<EnvelopePoint>,
    pub interpolations: Vec<Interpolation>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DuckEnvelope {
    pub op_id: String,
    pub target: Target,
    pub reason: DuckReason,
    pub points: Vec<EnvelopePoint>,
    pub interpolations: Vec<Interpolation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuckReason {
    VocalCollision,
    TransientCollision,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedforwardDelayTail {
    pub op_id: String,
    pub target: Target,
    pub capture_start_frame: i64,
    pub capture_end_frame: i64,
    pub taps: Vec<DelayTap>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelayTap {
    pub delay_frames: i64,
    pub gain_ppm: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RhythmicGate {
    pub op_id: String,
    pub target: Target,
    pub points: Vec<EnvelopePoint>,
    pub interpolations: Vec<Interpolation>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputSafety {
    pub profile: OutputSafetyProfile,
    pub pair_output_gain_mdb: i64,
    pub sample_peak_ceiling_mdbfs: i64,
    pub true_peak_target_mdbtp: i64,
    pub limiter_profile: LimiterProfile,
    pub lookahead_frames: i64,
    pub release_frames: i64,
    pub maximum_gain_reduction_mdb: i64,
    pub maximum_active_fraction_ppm: i64,
    pub activity_threshold_mdb: i64,
    pub true_peak_measurement_profile: TruePeakProfile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputSafetyProfile {
    TransitionOutputSafetyV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimiterProfile {
    LookaheadPeakLimiterV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruePeakProfile {
    Bs1770_4xV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureSnapshotRef {
    pub schema_version: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub generator_id: String,
    pub generator_version: String,
    pub generator_config_sha256: String,
    pub seed_hex_u64: String,
    pub geometry_id: String,
}
