use crate::canonical::canonical_json;
use crate::error::{Error, Result};
use crate::scalar::{JSON_SAFE_INTEGER_MAX, div_round_nearest_away};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct CueId(String);

impl CueId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct FeatureWindowId(String);

impl FeatureWindowId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CueIdentityCore {
    pub source_pcm_sha256: String,
    pub analysis_sha256: String,
    pub kind: String,
    pub frame: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureWindowIdentityCore {
    pub source_pcm_sha256: String,
    pub analysis_sha256: String,
    pub kind: String,
    pub start_frame: i64,
    pub end_frame: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurationMode {
    Cut,
    Seconds,
    Bars,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryProposalCore {
    pub outgoing_pcm_sha256: String,
    pub incoming_pcm_sha256: String,
    pub feature_snapshot_sha256: String,
    pub outgoing_pcm_frame_count: i64,
    pub incoming_pcm_frame_count: i64,
    pub outgoing_cue_id: String,
    pub outgoing_cue_source_frame: i64,
    pub incoming_cue_id: String,
    pub incoming_cue_source_frame: i64,
    pub duration_mode: DurationMode,
    pub requested_dry_frames: i64,
    pub resolved_bar_count: i64,
    pub incoming_source_rate_ppm: i64,
    pub cue_confidence_ppm: i64,
    pub beat_confidence_ppm: i64,
    pub downbeat_confidence_ppm: i64,
    pub alignment_error_ppm_of_beat: i64,
    pub geometry_quality_ppm: i64,
    pub feature_window_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryProposal {
    pub geometry_id: String,
    pub outgoing_pcm_sha256: String,
    pub incoming_pcm_sha256: String,
    pub feature_snapshot_sha256: String,
    pub outgoing_pcm_frame_count: i64,
    pub incoming_pcm_frame_count: i64,
    pub outgoing_cue_id: String,
    pub outgoing_cue_source_frame: i64,
    pub incoming_cue_id: String,
    pub incoming_cue_source_frame: i64,
    pub duration_mode: DurationMode,
    pub requested_dry_frames: i64,
    pub resolved_bar_count: i64,
    pub incoming_source_rate_ppm: i64,
    pub cue_confidence_ppm: i64,
    pub beat_confidence_ppm: i64,
    pub downbeat_confidence_ppm: i64,
    pub alignment_error_ppm_of_beat: i64,
    pub geometry_quality_ppm: i64,
    pub feature_window_ids: Vec<String>,
}

impl GeometryProposal {
    pub fn finalize(mut core: GeometryProposalCore) -> Result<Self> {
        core.feature_window_ids.sort();
        core.feature_window_ids.dedup();
        validate_core(&core)?;
        let geometry_id = geometry_id(&core)?;
        Ok(Self::from_core(geometry_id, core))
    }

    pub fn core(&self) -> GeometryProposalCore {
        GeometryProposalCore {
            outgoing_pcm_sha256: self.outgoing_pcm_sha256.clone(),
            incoming_pcm_sha256: self.incoming_pcm_sha256.clone(),
            feature_snapshot_sha256: self.feature_snapshot_sha256.clone(),
            outgoing_pcm_frame_count: self.outgoing_pcm_frame_count,
            incoming_pcm_frame_count: self.incoming_pcm_frame_count,
            outgoing_cue_id: self.outgoing_cue_id.clone(),
            outgoing_cue_source_frame: self.outgoing_cue_source_frame,
            incoming_cue_id: self.incoming_cue_id.clone(),
            incoming_cue_source_frame: self.incoming_cue_source_frame,
            duration_mode: self.duration_mode,
            requested_dry_frames: self.requested_dry_frames,
            resolved_bar_count: self.resolved_bar_count,
            incoming_source_rate_ppm: self.incoming_source_rate_ppm,
            cue_confidence_ppm: self.cue_confidence_ppm,
            beat_confidence_ppm: self.beat_confidence_ppm,
            downbeat_confidence_ppm: self.downbeat_confidence_ppm,
            alignment_error_ppm_of_beat: self.alignment_error_ppm_of_beat,
            geometry_quality_ppm: self.geometry_quality_ppm,
            feature_window_ids: self.feature_window_ids.clone(),
        }
    }

    fn from_core(geometry_id: String, core: GeometryProposalCore) -> Self {
        Self {
            geometry_id,
            outgoing_pcm_sha256: core.outgoing_pcm_sha256,
            incoming_pcm_sha256: core.incoming_pcm_sha256,
            feature_snapshot_sha256: core.feature_snapshot_sha256,
            outgoing_pcm_frame_count: core.outgoing_pcm_frame_count,
            incoming_pcm_frame_count: core.incoming_pcm_frame_count,
            outgoing_cue_id: core.outgoing_cue_id,
            outgoing_cue_source_frame: core.outgoing_cue_source_frame,
            incoming_cue_id: core.incoming_cue_id,
            incoming_cue_source_frame: core.incoming_cue_source_frame,
            duration_mode: core.duration_mode,
            requested_dry_frames: core.requested_dry_frames,
            resolved_bar_count: core.resolved_bar_count,
            incoming_source_rate_ppm: core.incoming_source_rate_ppm,
            cue_confidence_ppm: core.cue_confidence_ppm,
            beat_confidence_ppm: core.beat_confidence_ppm,
            downbeat_confidence_ppm: core.downbeat_confidence_ppm,
            alignment_error_ppm_of_beat: core.alignment_error_ppm_of_beat,
            geometry_quality_ppm: core.geometry_quality_ppm,
            feature_window_ids: core.feature_window_ids,
        }
    }
}

pub fn cue_id(core: &CueIdentityCore) -> Result<CueId> {
    validate_hash(&core.source_pcm_sha256)?;
    validate_hash(&core.analysis_sha256)?;
    validate_ascii(&core.kind)?;
    if !(0..=JSON_SAFE_INTEGER_MAX).contains(&core.frame) {
        return Err(Error::new(
            "INVALID_CUE_IDENTITY",
            "cue frame must be an interoperable source frame",
        ));
    }
    Ok(CueId(format!(
        "cue1-{}",
        sha256_hex(&canonical_json(core)?)
    )))
}

pub fn feature_window_id(core: &FeatureWindowIdentityCore) -> Result<FeatureWindowId> {
    validate_hash(&core.source_pcm_sha256)?;
    validate_hash(&core.analysis_sha256)?;
    validate_ascii(&core.kind)?;
    if core.start_frame < 0
        || core.start_frame >= core.end_frame
        || core.end_frame > JSON_SAFE_INTEGER_MAX
    {
        return Err(Error::new(
            "INVALID_FEATURE_WINDOW",
            "feature window must be nonempty",
        ));
    }
    Ok(FeatureWindowId(format!(
        "window1-{}",
        sha256_hex(&canonical_json(core)?)
    )))
}

#[derive(Serialize)]
struct GeometryIdentityProjection<'a> {
    duration_mode: DurationMode,
    feature_snapshot_sha256: &'a str,
    feature_window_ids: Vec<&'a str>,
    incoming_cue_id: &'a str,
    incoming_cue_source_frame: i64,
    incoming_pcm_sha256: &'a str,
    incoming_source_rate_ppm: i64,
    outgoing_cue_id: &'a str,
    outgoing_cue_source_frame: i64,
    outgoing_pcm_sha256: &'a str,
    requested_dry_frames: i64,
    resolved_bar_count: i64,
}

pub fn geometry_id(core: &GeometryProposalCore) -> Result<String> {
    let mut windows: Vec<_> = core.feature_window_ids.iter().map(String::as_str).collect();
    windows.sort_unstable();
    windows.dedup();
    let projection = GeometryIdentityProjection {
        duration_mode: core.duration_mode,
        feature_snapshot_sha256: &core.feature_snapshot_sha256,
        feature_window_ids: windows,
        incoming_cue_id: &core.incoming_cue_id,
        incoming_cue_source_frame: core.incoming_cue_source_frame,
        incoming_pcm_sha256: &core.incoming_pcm_sha256,
        incoming_source_rate_ppm: core.incoming_source_rate_ppm,
        outgoing_cue_id: &core.outgoing_cue_id,
        outgoing_cue_source_frame: core.outgoing_cue_source_frame,
        outgoing_pcm_sha256: &core.outgoing_pcm_sha256,
        requested_dry_frames: core.requested_dry_frames,
        resolved_bar_count: core.resolved_bar_count,
    };
    Ok(format!(
        "geom1-{}",
        sha256_hex(&canonical_json(&projection)?)
    ))
}

pub fn shortlist_geometries(values: Vec<GeometryProposal>) -> Result<Vec<GeometryProposal>> {
    let mut unique = BTreeMap::<String, GeometryProposal>::new();
    for value in values {
        let core = value.core();
        validate_core(&core)?;
        if value.cue_confidence_ppm < 700_000 {
            return Err(Error::new(
                "UNRELIABLE_CUE_GEOMETRY",
                "rich geometry requires reliable cues",
            ));
        }
        if value
            .feature_window_ids
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err(Error::new(
                "NON_CANONICAL_GEOMETRY_WINDOWS",
                "feature window IDs must be sorted and unique",
            ));
        }
        if value.geometry_id != geometry_id(&core)? {
            return Err(Error::new(
                "GEOMETRY_ID_MISMATCH",
                "geometry ID does not match semantic projection",
            ));
        }
        match unique.get(&value.geometry_id) {
            None => {
                unique.insert(value.geometry_id.clone(), value);
            }
            Some(previous) => {
                let replace = value.geometry_quality_ppm > previous.geometry_quality_ppm
                    || (value.geometry_quality_ppm == previous.geometry_quality_ppm
                        && canonical_json(&value)? < canonical_json(previous)?);
                if replace {
                    unique.insert(value.geometry_id.clone(), value);
                }
            }
        }
    }

    let mut buckets = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for value in unique.into_values() {
        buckets[bucket(&value)].push(value);
    }
    for values in &mut buckets {
        values.sort_by(|left, right| {
            right
                .geometry_quality_ppm
                .cmp(&left.geometry_quality_ppm)
                .then_with(|| left.geometry_id.cmp(&right.geometry_id))
        });
    }

    let mut positions = [0usize; 4];
    let mut result = Vec::new();
    while result.len() < 12 {
        let mut selected = false;
        for bucket_index in 0..4 {
            if let Some(value) = buckets[bucket_index].get(positions[bucket_index]) {
                result.push(value.clone());
                positions[bucket_index] += 1;
                selected = true;
                if result.len() == 12 {
                    break;
                }
            }
        }
        if !selected {
            break;
        }
    }
    Ok(result)
}

pub fn validate_fallback_geometry(value: &GeometryProposal) -> Result<()> {
    let core = value.core();
    validate_core(&core)?;
    if value.geometry_id != geometry_id(&core)?
        || value.duration_mode != DurationMode::Seconds
        || value.requested_dry_frames != 220_500
        || value.resolved_bar_count != 0
        || value.incoming_source_rate_ppm != 1_000_000
    {
        return Err(Error::new(
            "INVALID_FALLBACK_GEOMETRY",
            "fallback must be an exact bounds-safe five-second geometry",
        ));
    }
    Ok(())
}

fn validate_core(core: &GeometryProposalCore) -> Result<()> {
    for hash in [
        &core.outgoing_pcm_sha256,
        &core.incoming_pcm_sha256,
        &core.feature_snapshot_sha256,
    ] {
        validate_hash(hash)?;
    }
    for value in [&core.outgoing_cue_id, &core.incoming_cue_id] {
        validate_ascii(value)?;
    }
    if core.feature_window_ids.is_empty() {
        return Err(Error::new(
            "GEOMETRY_WINDOWS_MISSING",
            "geometry must reference at least one feature window",
        ));
    }
    if core
        .feature_window_ids
        .iter()
        .any(|value| validate_ascii(value).is_err())
    {
        return Err(Error::new(
            "INVALID_GEOMETRY_TEXT",
            "geometry IDs must be printable ASCII",
        ));
    }
    if core.outgoing_pcm_frame_count <= 0
        || core.incoming_pcm_frame_count <= 0
        || core.outgoing_pcm_frame_count > JSON_SAFE_INTEGER_MAX
        || core.incoming_pcm_frame_count > JSON_SAFE_INTEGER_MAX
        || !(0..=JSON_SAFE_INTEGER_MAX).contains(&core.outgoing_cue_source_frame)
        || !(0..=JSON_SAFE_INTEGER_MAX).contains(&core.incoming_cue_source_frame)
        || !(1..=705_600).contains(&core.requested_dry_frames)
        || !(920_000..=1_080_000).contains(&core.incoming_source_rate_ppm)
        || !probability(core.cue_confidence_ppm)
        || !probability(core.beat_confidence_ppm)
        || !probability(core.downbeat_confidence_ppm)
        || !probability(core.alignment_error_ppm_of_beat)
        || !probability(core.geometry_quality_ppm)
    {
        return Err(Error::new(
            "INVALID_GEOMETRY_SCALAR",
            "geometry scalar is outside v1 bounds",
        ));
    }
    match core.duration_mode {
        DurationMode::Cut if core.resolved_bar_count != 0 || core.requested_dry_frames > 1_323 => {
            return Err(Error::new(
                "INVALID_GEOMETRY_DURATION",
                "cut geometry duration is invalid",
            ));
        }
        DurationMode::Seconds if core.resolved_bar_count != 0 => {
            return Err(Error::new(
                "INVALID_GEOMETRY_DURATION",
                "seconds geometry cannot declare bars",
            ));
        }
        DurationMode::Bars
            if !matches!(core.resolved_bar_count, 1 | 2 | 4)
                || core.beat_confidence_ppm < 750_000
                || core.downbeat_confidence_ppm < 700_000
                || core.alignment_error_ppm_of_beat > 31_250 =>
        {
            return Err(Error::new(
                "UNRELIABLE_RHYTHMIC_GEOMETRY",
                "bar geometry does not meet v1 rhythmic reliability",
            ));
        }
        _ => {}
    }
    let incoming_start = div_round_nearest_away(
        core.requested_dry_frames
            .checked_neg()
            .and_then(|value| value.checked_mul(core.incoming_source_rate_ppm))
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "geometry mapping overflow"))?,
        1_000_000,
    )?;
    if core.outgoing_cue_source_frame < core.requested_dry_frames
        || core.outgoing_cue_source_frame >= core.outgoing_pcm_frame_count
        || core
            .incoming_cue_source_frame
            .checked_add(incoming_start)
            .is_none_or(|value| value < 0)
        || core.incoming_cue_source_frame >= core.incoming_pcm_frame_count
    {
        return Err(Error::new(
            "GEOMETRY_SOURCE_OUT_OF_BOUNDS",
            "geometry dry interval exceeds canonical PCM",
        ));
    }
    Ok(())
}

fn bucket(value: &GeometryProposal) -> usize {
    match value.duration_mode {
        DurationMode::Cut => 0,
        DurationMode::Bars if value.resolved_bar_count <= 2 => 1,
        DurationMode::Seconds if value.requested_dry_frames <= 220_500 => 1,
        DurationMode::Bars if value.resolved_bar_count <= 4 => 2,
        DurationMode::Seconds if value.requested_dry_frames <= 441_000 => 2,
        _ => 3,
    }
}

fn validate_hash(value: &str) -> Result<()> {
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

fn validate_ascii(value: &str) -> Result<()> {
    if !value.is_empty() && value.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        Ok(())
    } else {
        Err(Error::new(
            "INVALID_GEOMETRY_TEXT",
            "geometry identity text must be printable ASCII",
        ))
    }
}

fn probability(value: i64) -> bool {
    (0..=1_000_000).contains(&value)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
