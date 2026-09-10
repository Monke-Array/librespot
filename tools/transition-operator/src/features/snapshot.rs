use crate::canonical::canonical_json;
use crate::error::{Error, Result};
use crate::geometry::{CueIdentityCore, FeatureWindowIdentityCore, cue_id, feature_window_id};
use crate::model::{FeatureSnapshotRef, Operation, OperatorPlanBody};
use crate::scalar::div_round_nearest_away;
use crate::templates::{TemplateId, TemplateInputs, template_registry};
use crate::validation::TemplateFeatureView;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const FEATURE_SNAPSHOT_SCHEMA_VERSION: &str = "transition-feature-snapshot/3";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisIdentity {
    pub analysis_id: String,
    pub extractor_id: String,
    pub extractor_version: String,
    pub algorithm_sha256: String,
    pub analysis_sha256: String,
}

#[derive(Serialize)]
struct AnalysisIdentityBody<'a> {
    extractor_id: &'a str,
    extractor_version: &'a str,
    algorithm_sha256: &'a str,
}

impl AnalysisIdentity {
    pub fn finalize(
        extractor_id: String,
        extractor_version: String,
        algorithm_sha256: String,
    ) -> Result<Self> {
        require_ascii(&extractor_id, "INVALID_ANALYSIS_IDENTITY")?;
        require_ascii(&extractor_version, "INVALID_ANALYSIS_IDENTITY")?;
        require_hash(&algorithm_sha256)?;
        let body = AnalysisIdentityBody {
            extractor_id: &extractor_id,
            extractor_version: &extractor_version,
            algorithm_sha256: &algorithm_sha256,
        };
        let analysis_sha256 = domain_hash(b"transition-feature-analysis/2\0", &body)?;
        Ok(Self {
            analysis_id: format!("analysis2-{analysis_sha256}"),
            extractor_id,
            extractor_version,
            algorithm_sha256,
            analysis_sha256,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueKind {
    Intro,
    Outro,
}

impl CueKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Intro => "intro",
            Self::Outro => "outro",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CueFeature {
    pub cue_id: String,
    pub source_frame: i64,
    pub kind: CueKind,
    pub confidence_ppm: i64,
}

impl CueFeature {
    pub fn draft(source_frame: i64, kind: CueKind, confidence_ppm: i64) -> Self {
        Self {
            cue_id: String::new(),
            source_frame,
            kind,
            confidence_ppm,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RhythmFeatures {
    pub beat_frames: Vec<i64>,
    pub downbeat_frames: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meter_beats: Option<i64>,
    pub tempo_millibpm: i64,
    pub beat_confidence_ppm: i64,
    pub downbeat_confidence_ppm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    OutgoingTransition,
    IncomingTransition,
}

impl WindowKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::OutgoingTransition => "outgoing_transition",
            Self::IncomingTransition => "incoming_transition",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureWindow {
    pub window_id: String,
    pub kind: WindowKind,
    pub start_frame: i64,
    pub end_frame: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vocal_activity_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transient_activity_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transient_density_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hard_cut_safe: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bass_occupancy_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectral_stability_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low_occupancy_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mid_occupancy_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high_occupancy_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_term_loudness_mlu: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_variability_mdb: Option<i64>,
}

impl FeatureWindow {
    #[allow(clippy::too_many_arguments)]
    pub fn draft(
        kind: WindowKind,
        start_frame: i64,
        end_frame: i64,
        vocal_activity_ppm: Option<i64>,
        transient_activity_ppm: Option<i64>,
        transient_density_ppm: Option<i64>,
        hard_cut_safe: Option<bool>,
        bass_occupancy_ppm: Option<i64>,
        spectral_stability_ppm: Option<i64>,
        low_occupancy_ppm: Option<i64>,
        mid_occupancy_ppm: Option<i64>,
        high_occupancy_ppm: Option<i64>,
        short_term_loudness_mlu: Option<i64>,
        energy_variability_mdb: Option<i64>,
    ) -> Self {
        Self {
            window_id: String::new(),
            kind,
            start_frame,
            end_frame,
            vocal_activity_ppm,
            transient_activity_ppm,
            transient_density_ppm,
            hard_cut_safe,
            bass_occupancy_ppm,
            spectral_stability_ppm,
            low_occupancy_ppm,
            mid_occupancy_ppm,
            high_occupancy_ppm,
            short_term_loudness_mlu,
            energy_variability_mdb,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFeatures {
    pub source_pcm_sha256: String,
    pub source_frames: i64,
    pub cues: Vec<CueFeature>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rhythm: Option<RhythmFeatures>,
    pub windows: Vec<FeatureWindow>,
    pub sample_peak_mdbfs: i64,
    pub true_peak_mdbtp: i64,
}

impl SourceFeatures {
    #[allow(clippy::too_many_arguments)]
    pub fn finalize(
        source_pcm_sha256: String,
        source_frames: i64,
        analysis: &AnalysisIdentity,
        mut cues: Vec<CueFeature>,
        rhythm: Option<RhythmFeatures>,
        mut windows: Vec<FeatureWindow>,
        sample_peak_mdbfs: i64,
        true_peak_mdbtp: i64,
    ) -> Result<Self> {
        require_hash(&source_pcm_sha256)?;
        for cue in &mut cues {
            cue.cue_id = cue_id(&CueIdentityCore {
                source_pcm_sha256: source_pcm_sha256.clone(),
                analysis_sha256: analysis.analysis_sha256.clone(),
                kind: cue.kind.as_str().to_owned(),
                frame: cue.source_frame,
            })?
            .as_str()
            .to_owned();
        }
        cues.sort_by(|left, right| left.cue_id.cmp(&right.cue_id));
        for window in &mut windows {
            window.window_id = feature_window_id(&FeatureWindowIdentityCore {
                source_pcm_sha256: source_pcm_sha256.clone(),
                analysis_sha256: analysis.analysis_sha256.clone(),
                kind: window.kind.as_str().to_owned(),
                start_frame: window.start_frame,
                end_frame: window.end_frame,
            })?
            .as_str()
            .to_owned();
        }
        windows.sort_by(|left, right| left.window_id.cmp(&right.window_id));
        let value = Self {
            source_pcm_sha256,
            source_frames,
            cues,
            rhythm,
            windows,
            sample_peak_mdbfs,
            true_peak_mdbtp,
        };
        validate_source(&value, analysis)?;
        Ok(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairFeatures {
    pub outgoing_cue_id: String,
    pub incoming_cue_id: String,
    pub outgoing_window_id: String,
    pub incoming_window_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vocal_collision_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vocal_collision_span_frames: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vocal_collision_start_frame: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vocal_collision_end_frame: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outgoing_vocal_collision_strength_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incoming_vocal_collision_strength_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transient_collision_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transient_collision_span_frames: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transient_collision_start_frame: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transient_collision_end_frame: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outgoing_transient_collision_strength_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incoming_transient_collision_strength_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bass_collision_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spectral_overlap_ppm: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_delta_mdb: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment_error_ppm_of_beat: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureSnapshotBodyV3 {
    pub schema_version: String,
    pub analysis: AnalysisIdentity,
    pub pair: PairFeatures,
    pub outgoing: SourceFeatures,
    pub incoming: SourceFeatures,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureSnapshotV3 {
    pub schema_version: String,
    pub snapshot_sha256: String,
    pub analysis: AnalysisIdentity,
    pub pair: PairFeatures,
    pub outgoing: SourceFeatures,
    pub incoming: SourceFeatures,
}

impl FeatureSnapshotV3 {
    pub fn body(&self) -> FeatureSnapshotBodyV3 {
        FeatureSnapshotBodyV3 {
            schema_version: self.schema_version.clone(),
            analysis: self.analysis.clone(),
            pair: self.pair.clone(),
            outgoing: self.outgoing.clone(),
            incoming: self.incoming.clone(),
        }
    }

    pub fn feature_ref(&self) -> FeatureSnapshotRef {
        FeatureSnapshotRef {
            schema_version: self.schema_version.clone(),
            sha256: self.snapshot_sha256.clone(),
        }
    }

    pub fn template_inputs(&self) -> Result<TemplateInputs> {
        validate_feature_snapshot(self)?;
        let outgoing_window = self
            .outgoing
            .windows
            .iter()
            .find(|window| window.window_id == self.pair.outgoing_window_id)
            .ok_or_else(|| Error::new("INVALID_PAIR_FEATURES", "outgoing window is missing"))?;
        let incoming_window = self
            .incoming
            .windows
            .iter()
            .find(|window| window.window_id == self.pair.incoming_window_id)
            .ok_or_else(|| Error::new("INVALID_PAIR_FEATURES", "incoming window is missing"))?;
        let outgoing_cue = self
            .outgoing
            .cues
            .iter()
            .find(|cue| cue.cue_id == self.pair.outgoing_cue_id)
            .ok_or_else(|| Error::new("INVALID_PAIR_FEATURES", "outgoing cue is missing"))?;
        let incoming_cue = self
            .incoming
            .cues
            .iter()
            .find(|cue| cue.cue_id == self.pair.incoming_cue_id)
            .ok_or_else(|| Error::new("INVALID_PAIR_FEATURES", "incoming cue is missing"))?;
        let cue_confidence_ppm = Some(outgoing_cue.confidence_ppm.min(incoming_cue.confidence_ppm));
        let (
            beat_confidence_ppm,
            downbeat_confidence_ppm,
            beat_frames,
            meter_beats,
            two_beats_frames,
        ) = if let Some(rhythm) = &self.outgoing.rhythm {
            let relative = rhythm
                .beat_frames
                .iter()
                .map(|frame| frame - outgoing_cue.source_frame)
                .collect();
            let beat_length = 2_i64
                .checked_mul(60_000_i64.checked_mul(44_100).unwrap() / rhythm.tempo_millibpm)
                .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "beat duration overflow"))?;
            (
                Some(rhythm.beat_confidence_ppm),
                Some(rhythm.downbeat_confidence_ppm),
                relative,
                rhythm.meter_beats,
                Some(beat_length),
            )
        } else {
            (None, None, Vec::new(), None, None)
        };
        Ok(TemplateInputs {
            cue_confidence_ppm,
            beat_confidence_ppm,
            downbeat_confidence_ppm,
            alignment_error_ppm_of_beat: self.pair.alignment_error_ppm_of_beat,
            outgoing_vocal_activity_ppm: outgoing_window.vocal_activity_ppm,
            incoming_vocal_activity_ppm: incoming_window.vocal_activity_ppm,
            outgoing_vocal_sustained: outgoing_window
                .vocal_activity_ppm
                .map(|value| value > 700_000),
            vocal_collision_ppm: self.pair.vocal_collision_ppm,
            vocal_collision_span_frames: self.pair.vocal_collision_span_frames,
            vocal_collision_start_frame: self.pair.vocal_collision_start_frame,
            vocal_collision_end_frame: self.pair.vocal_collision_end_frame,
            outgoing_vocal_collision_strength_ppm: self.pair.outgoing_vocal_collision_strength_ppm,
            incoming_vocal_collision_strength_ppm: self.pair.incoming_vocal_collision_strength_ppm,
            transient_collision_ppm: self.pair.transient_collision_ppm,
            transient_collision_span_frames: self.pair.transient_collision_span_frames,
            transient_collision_start_frame: self.pair.transient_collision_start_frame,
            transient_collision_end_frame: self.pair.transient_collision_end_frame,
            outgoing_transient_collision_strength_ppm: self
                .pair
                .outgoing_transient_collision_strength_ppm,
            incoming_transient_collision_strength_ppm: self
                .pair
                .incoming_transient_collision_strength_ppm,
            two_beats_frames,
            outgoing_transient_activity_ppm: outgoing_window.transient_activity_ppm,
            incoming_transient_activity_ppm: incoming_window.transient_activity_ppm,
            outgoing_transient_density_ppm: outgoing_window.transient_density_ppm,
            outgoing_bass_occupancy_ppm: outgoing_window.bass_occupancy_ppm,
            incoming_bass_occupancy_ppm: incoming_window.bass_occupancy_ppm,
            bass_collision_ppm: self.pair.bass_collision_ppm,
            outgoing_spectral_stability_ppm: outgoing_window.spectral_stability_ppm,
            incoming_spectral_stability_ppm: incoming_window.spectral_stability_ppm,
            spectral_overlap_ppm: self.pair.spectral_overlap_ppm,
            outgoing_energy_variability_mdb: outgoing_window.energy_variability_mdb,
            incoming_energy_variability_mdb: incoming_window.energy_variability_mdb,
            energy_delta_mdb: self.pair.energy_delta_mdb,
            outgoing_hard_cut_safe: outgoing_window.hard_cut_safe,
            incoming_hard_cut_safe: incoming_window.hard_cut_safe,
            beat_frames,
            meter_beats,
        })
    }
}

pub fn finalize_feature_snapshot(body: FeatureSnapshotBodyV3) -> Result<FeatureSnapshotV3> {
    validate_body(&body)?;
    let snapshot_sha256 = body_hash(&body)?;
    Ok(FeatureSnapshotV3 {
        schema_version: body.schema_version,
        snapshot_sha256,
        analysis: body.analysis,
        pair: body.pair,
        outgoing: body.outgoing,
        incoming: body.incoming,
    })
}

pub fn snapshot_sha256(snapshot: &FeatureSnapshotV3) -> Result<String> {
    body_hash(&snapshot.body())
}

pub fn validate_feature_snapshot(snapshot: &FeatureSnapshotV3) -> Result<()> {
    validate_body(&snapshot.body())?;
    require_hash(&snapshot.snapshot_sha256)?;
    if snapshot.snapshot_sha256 != snapshot_sha256(snapshot)? {
        return Err(Error::new(
            "FEATURE_SNAPSHOT_HASH_MISMATCH",
            "feature snapshot hash does not match its semantic body",
        ));
    }
    Ok(())
}

pub fn parse_feature_snapshot(bytes: &[u8]) -> Result<FeatureSnapshotV3> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| {
        Error::new(
            "INVALID_FEATURE_SNAPSHOT_JSON",
            format!("feature snapshot is not valid JSON: {error}"),
        )
    })?;
    if contains_null(&value) {
        return Err(Error::new(
            "FEATURE_NULL_FORBIDDEN",
            "optional feature values must be omitted rather than null",
        ));
    }
    let snapshot: FeatureSnapshotV3 = serde_json::from_value(value).map_err(|error| {
        Error::new(
            "INVALID_FEATURE_SNAPSHOT_JSON",
            format!("feature snapshot does not match v3 schema: {error}"),
        )
    })?;
    validate_feature_snapshot(&snapshot)?;
    Ok(snapshot)
}

impl TemplateFeatureView for FeatureSnapshotV3 {
    fn snapshot_sha256(&self) -> &str {
        &self.snapshot_sha256
    }

    fn covers_plan_window(&self, body: &OperatorPlanBody) -> bool {
        let outgoing = self.outgoing.windows.iter().any(|window| {
            window.window_id == self.pair.outgoing_window_id
                && window.start_frame
                    <= body.sources.outgoing.cue_source_frame + body.timeline.dry_start_frame
                && window.end_frame >= body.sources.outgoing.cue_source_frame
        });
        let incoming_rate = body
            .operations
            .iter()
            .find_map(|operation| match operation {
                Operation::TimeMap(value) => Some(value.source_rate_ppm),
                _ => None,
            })
            .unwrap_or(1_000_000);
        let incoming_start = body
            .timeline
            .dry_start_frame
            .checked_mul(incoming_rate)
            .and_then(|value| div_round_nearest_away(value, 1_000_000).ok());
        let incoming_end = body
            .timeline
            .effect_end_frame
            .checked_mul(incoming_rate)
            .and_then(|value| div_round_nearest_away(value, 1_000_000).ok());
        let incoming = self.incoming.windows.iter().any(|window| {
            window.window_id == self.pair.incoming_window_id
                && incoming_start.is_some_and(|offset| {
                    window.start_frame <= body.sources.incoming.cue_source_frame + offset
                })
                && incoming_end.is_some_and(|offset| {
                    window.end_frame >= body.sources.incoming.cue_source_frame + offset
                })
        });
        outgoing && incoming
    }

    fn template_is_applicable(&self, body: &OperatorPlanBody) -> bool {
        let Some(id) = template_id(&body.template.id) else {
            return false;
        };
        self.template_inputs().is_ok_and(|inputs| {
            template_registry()
                .iter()
                .find(|family| family.id == id)
                .is_some_and(|family| family.applicability(&inputs).applicable)
        })
    }

    fn recipe_matches(&self, body: &OperatorPlanBody) -> bool {
        let Some(id) = template_id(&body.template.id) else {
            return false;
        };
        body.template.version == 1
            && template_registry()
                .iter()
                .find(|family| family.id == id)
                .is_some_and(|family| {
                    family
                        .recipes
                        .iter()
                        .any(|recipe| recipe.id == body.template.recipe_id)
                })
    }
}

fn validate_body(body: &FeatureSnapshotBodyV3) -> Result<()> {
    if body.schema_version != FEATURE_SNAPSHOT_SCHEMA_VERSION {
        return Err(Error::new(
            "INVALID_FEATURE_SNAPSHOT_VERSION",
            "feature snapshot schema version is unsupported",
        ));
    }
    validate_analysis(&body.analysis)?;
    validate_source(&body.outgoing, &body.analysis)?;
    validate_source(&body.incoming, &body.analysis)?;
    validate_pair(body)?;
    Ok(())
}

fn validate_analysis(value: &AnalysisIdentity) -> Result<()> {
    let expected = AnalysisIdentity::finalize(
        value.extractor_id.clone(),
        value.extractor_version.clone(),
        value.algorithm_sha256.clone(),
    )?;
    if &expected != value {
        return Err(Error::new(
            "ANALYSIS_IDENTITY_MISMATCH",
            "analysis identity does not match its versioned algorithm",
        ));
    }
    Ok(())
}

fn validate_source(value: &SourceFeatures, analysis: &AnalysisIdentity) -> Result<()> {
    require_hash(&value.source_pcm_sha256)?;
    if value.source_frames <= 0 || value.cues.is_empty() || value.windows.is_empty() {
        return Err(Error::new(
            "INVALID_SOURCE_FEATURES",
            "source feature identity, cues, and windows are required",
        ));
    }
    if !strict_ids(value.cues.iter().map(|item| item.cue_id.as_str()))
        || !strict_ids(value.windows.iter().map(|item| item.window_id.as_str()))
    {
        return Err(Error::new(
            "NON_CANONICAL_FEATURE_ORDER",
            "cue and window arrays must be sorted and unique by semantic ID",
        ));
    }
    for cue in &value.cues {
        probability(cue.confidence_ppm)?;
        if cue.source_frame < 0 || cue.source_frame >= value.source_frames {
            return Err(Error::new("INVALID_CUE", "cue is outside source bounds"));
        }
        let expected = cue_id(&CueIdentityCore {
            source_pcm_sha256: value.source_pcm_sha256.clone(),
            analysis_sha256: analysis.analysis_sha256.clone(),
            kind: cue.kind.as_str().to_owned(),
            frame: cue.source_frame,
        })?;
        if cue.cue_id != expected.as_str() {
            return Err(Error::new(
                "CUE_ID_MISMATCH",
                "cue ID does not match its content",
            ));
        }
    }
    if let Some(rhythm) = &value.rhythm {
        if !(20_000..=400_000).contains(&rhythm.tempo_millibpm)
            || rhythm.beat_frames.len() < 2
            || !strict_frames(&rhythm.beat_frames, value.source_frames)
            || !strict_frames(&rhythm.downbeat_frames, value.source_frames)
            || rhythm
                .downbeat_frames
                .iter()
                .any(|frame| rhythm.beat_frames.binary_search(frame).is_err())
            || rhythm
                .meter_beats
                .is_some_and(|meter| !(2..=12).contains(&meter))
        {
            return Err(Error::new(
                "INVALID_RHYTHM_GRID",
                "rhythm grid is unordered, inconsistent, or outside bounds",
            ));
        }
        probability(rhythm.beat_confidence_ppm)?;
        probability(rhythm.downbeat_confidence_ppm)?;
    }
    for window in &value.windows {
        if window.start_frame < 0
            || window.start_frame >= window.end_frame
            || window.end_frame > value.source_frames
        {
            return Err(Error::new(
                "INVALID_FEATURE_WINDOW",
                "feature window is outside source bounds",
            ));
        }
        let expected = feature_window_id(&FeatureWindowIdentityCore {
            source_pcm_sha256: value.source_pcm_sha256.clone(),
            analysis_sha256: analysis.analysis_sha256.clone(),
            kind: window.kind.as_str().to_owned(),
            start_frame: window.start_frame,
            end_frame: window.end_frame,
        })?;
        if window.window_id != expected.as_str() {
            return Err(Error::new(
                "FEATURE_WINDOW_ID_MISMATCH",
                "feature window ID does not match its content",
            ));
        }
        for value in [
            window.vocal_activity_ppm,
            window.transient_activity_ppm,
            window.transient_density_ppm,
            window.bass_occupancy_ppm,
            window.spectral_stability_ppm,
            window.low_occupancy_ppm,
            window.mid_occupancy_ppm,
            window.high_occupancy_ppm,
        ]
        .into_iter()
        .flatten()
        {
            probability(value)?;
        }
        if window
            .short_term_loudness_mlu
            .is_some_and(|value| !(-120_000..=24_000).contains(&value))
            || window
                .energy_variability_mdb
                .is_some_and(|value| !(0..=120_000).contains(&value))
        {
            return Err(Error::new(
                "INVALID_WINDOW_MEASUREMENT",
                "window energy measurement is outside v3 bounds",
            ));
        }
        if let (Some(low), Some(mid), Some(high)) = (
            window.low_occupancy_ppm,
            window.mid_occupancy_ppm,
            window.high_occupancy_ppm,
        ) {
            if low + mid + high != 1_000_000 {
                return Err(Error::new(
                    "INVALID_SPECTRAL_OCCUPANCY",
                    "spectral band occupancies must sum to one million ppm",
                ));
            }
        }
    }
    if !(-120_000..=12_000).contains(&value.sample_peak_mdbfs)
        || !(-120_000..=12_000).contains(&value.true_peak_mdbtp)
        || value.true_peak_mdbtp < value.sample_peak_mdbfs
    {
        return Err(Error::new(
            "INVALID_SOURCE_PEAK",
            "whole-source peaks are outside v3 bounds",
        ));
    }
    Ok(())
}

fn validate_pair(body: &FeatureSnapshotBodyV3) -> Result<()> {
    let pair = &body.pair;
    if !body
        .outgoing
        .cues
        .iter()
        .any(|value| value.cue_id == pair.outgoing_cue_id)
        || !body
            .incoming
            .cues
            .iter()
            .any(|value| value.cue_id == pair.incoming_cue_id)
        || !body
            .outgoing
            .windows
            .iter()
            .any(|value| value.window_id == pair.outgoing_window_id)
        || !body
            .incoming
            .windows
            .iter()
            .any(|value| value.window_id == pair.incoming_window_id)
    {
        return Err(Error::new(
            "INVALID_PAIR_FEATURES",
            "pair references unknown cue or feature window",
        ));
    }
    for value in [
        pair.vocal_collision_ppm,
        pair.transient_collision_ppm,
        pair.bass_collision_ppm,
        pair.spectral_overlap_ppm,
        pair.alignment_error_ppm_of_beat,
        pair.outgoing_vocal_collision_strength_ppm,
        pair.incoming_vocal_collision_strength_ppm,
        pair.outgoing_transient_collision_strength_ppm,
        pair.incoming_transient_collision_strength_ppm,
    ]
    .into_iter()
    .flatten()
    {
        probability(value)?;
    }
    if pair
        .energy_delta_mdb
        .is_some_and(|value| !(-120_000..=120_000).contains(&value))
        || pair
            .vocal_collision_span_frames
            .is_some_and(|value| value <= 0)
        || pair
            .transient_collision_span_frames
            .is_some_and(|value| value <= 0)
        || invalid_collision_group(
            pair.vocal_collision_ppm,
            pair.vocal_collision_span_frames,
            pair.vocal_collision_start_frame,
            pair.vocal_collision_end_frame,
            pair.outgoing_vocal_collision_strength_ppm,
            pair.incoming_vocal_collision_strength_ppm,
        )
        || invalid_collision_group(
            pair.transient_collision_ppm,
            pair.transient_collision_span_frames,
            pair.transient_collision_start_frame,
            pair.transient_collision_end_frame,
            pair.outgoing_transient_collision_strength_ppm,
            pair.incoming_transient_collision_strength_ppm,
        )
    {
        return Err(Error::new(
            "INVALID_PAIR_MEASUREMENT",
            "pair measurement is outside v3 bounds",
        ));
    }
    Ok(())
}

fn invalid_collision_group(
    value: Option<i64>,
    span: Option<i64>,
    start: Option<i64>,
    end: Option<i64>,
    outgoing_strength: Option<i64>,
    incoming_strength: Option<i64>,
) -> bool {
    let details = [span, start, end, outgoing_strength, incoming_strength];
    let has_any_detail = details.into_iter().any(|value| value.is_some());
    if !has_any_detail {
        return false;
    }
    value.is_none()
        || value == Some(0)
        || details.into_iter().any(|value| value.is_none())
        || !matches!((start, end, span), (Some(start), Some(end), Some(span)) if (-705_600..=264_600).contains(&start) && (-705_600..=264_600).contains(&end) && start < end && end - start == span)
}

fn template_id(value: &str) -> Option<TemplateId> {
    match value {
        "safe_crossfade" => Some(TemplateId::SafeCrossfade),
        "shaped_handoff" => Some(TemplateId::ShapedHandoff),
        "beat_cut" => Some(TemplateId::BeatCut),
        "bass_handoff" => Some(TemplateId::BassHandoff),
        "spectral_handoff" => Some(TemplateId::SpectralHandoff),
        "ducked_overlap" => Some(TemplateId::DuckedOverlap),
        "echo_tail_handoff" => Some(TemplateId::EchoTailHandoff),
        "energy_ramp" => Some(TemplateId::EnergyRamp),
        "rhythmic_handoff" => Some(TemplateId::RhythmicHandoff),
        _ => None,
    }
}

fn strict_ids<'a>(values: impl Iterator<Item = &'a str>) -> bool {
    let mut previous: Option<&str> = None;
    for value in values {
        if previous.is_some_and(|old| old >= value) {
            return false;
        }
        previous = Some(value);
    }
    true
}

fn strict_frames(values: &[i64], source_frames: i64) -> bool {
    values
        .iter()
        .all(|frame| (0..source_frames).contains(frame))
        && values.windows(2).all(|pair| pair[0] < pair[1])
}

fn contains_null(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => true,
        serde_json::Value::Array(values) => values.iter().any(contains_null),
        serde_json::Value::Object(values) => values.values().any(contains_null),
        _ => false,
    }
}

fn probability(value: i64) -> Result<()> {
    if (0..=1_000_000).contains(&value) {
        Ok(())
    } else {
        Err(Error::new(
            "INVALID_FEATURE_PROBABILITY",
            "probability-like feature must be integer ppm",
        ))
    }
}

fn body_hash<T: Serialize>(value: &T) -> Result<String> {
    domain_hash(b"transition-feature-snapshot/3\0", value)
}

fn domain_hash<T: Serialize>(domain: &[u8], value: &T) -> Result<String> {
    let mut digest = Sha256::new();
    digest.update(domain);
    digest.update(canonical_json(value)?);
    Ok(hex(&digest.finalize()))
}

fn require_hash(value: &str) -> Result<()> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(Error::new(
            "INVALID_SHA256",
            "hash must be lowercase SHA-256",
        ))
    }
}

fn require_ascii(value: &str, code: &'static str) -> Result<()> {
    if !value.is_empty() && value.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        Ok(())
    } else {
        Err(Error::new(code, "identity text must be printable ASCII"))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
