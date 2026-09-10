use crate::canonical::canonical_json;
use crate::error::{Error, Result};
use crate::scalar::div_round_nearest_away;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const VOCAL_TRACE_SCHEMA_VERSION: &str = "vocal-evidence-trace/1";
pub const VOCAL_HOP_FRAMES: i64 = 441;
pub const VOCAL_PAIR_START_FRAME: i64 = -110_250;
pub const VOCAL_PAIR_END_FRAME: i64 = -22_050;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VocalAnalysisIdentityV1 {
    pub extractor_id: String,
    pub extractor_version: String,
    pub algorithm_sha256: String,
    pub positive_evidence_threshold_ppm: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VocalTraceBodyV1 {
    pub schema_version: String,
    pub source_pcm_sha256: String,
    pub source_frames: i64,
    pub hop_frames: i64,
    pub analysis: VocalAnalysisIdentityV1,
    pub evidence_ppm: Vec<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VocalTraceV1 {
    pub schema_version: String,
    pub trace_sha256: String,
    pub source_pcm_sha256: String,
    pub source_frames: i64,
    pub hop_frames: i64,
    pub analysis: VocalAnalysisIdentityV1,
    pub evidence_ppm: Vec<i64>,
}

impl VocalTraceV1 {
    pub fn body(&self) -> VocalTraceBodyV1 {
        VocalTraceBodyV1 {
            schema_version: self.schema_version.clone(),
            source_pcm_sha256: self.source_pcm_sha256.clone(),
            source_frames: self.source_frames,
            hop_frames: self.hop_frames,
            analysis: self.analysis.clone(),
            evidence_ppm: self.evidence_ppm.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VocalCollisionMeasurements {
    pub collision_ppm: i64,
    pub span_frames: Option<i64>,
    pub start_frame: Option<i64>,
    pub end_frame: Option<i64>,
    pub outgoing_strength_ppm: Option<i64>,
    pub incoming_strength_ppm: Option<i64>,
}

pub fn finalize_vocal_trace(body: VocalTraceBodyV1) -> Result<VocalTraceV1> {
    validate_body(&body)?;
    let trace_sha256 = trace_hash(&body)?;
    Ok(VocalTraceV1 {
        schema_version: body.schema_version,
        trace_sha256,
        source_pcm_sha256: body.source_pcm_sha256,
        source_frames: body.source_frames,
        hop_frames: body.hop_frames,
        analysis: body.analysis,
        evidence_ppm: body.evidence_ppm,
    })
}

pub fn vocal_activity_ppm(trace: &VocalTraceV1, start_frame: i64, end_frame: i64) -> Result<i64> {
    validate_trace(trace)?;
    let start = start_frame.max(0);
    let end = end_frame.min(trace.source_frames);
    if start >= end {
        return Err(Error::new(
            "INVALID_VOCAL_WINDOW",
            "clipped vocal window is empty",
        ));
    }
    let mut valid = 0_i64;
    let mut active = 0_i64;
    for (index, evidence) in trace.evidence_ppm.iter().enumerate() {
        let center = i64::try_from(index)
            .ok()
            .and_then(|value| value.checked_mul(VOCAL_HOP_FRAMES))
            .and_then(|value| value.checked_add(VOCAL_HOP_FRAMES / 2))
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "vocal hop center overflow"))?;
        if (start..end).contains(&center) && center < trace.source_frames {
            valid += 1;
            if *evidence >= trace.analysis.positive_evidence_threshold_ppm {
                active += 1;
            }
        }
    }
    if valid == 0 {
        return Err(Error::new(
            "INVALID_VOCAL_WINDOW",
            "vocal window contains no valid hop center",
        ));
    }
    div_round_nearest_away(
        active
            .checked_mul(1_000_000)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "vocal occupancy overflow"))?,
        valid,
    )
}

pub fn measure_vocal_collision(
    outgoing: Option<&VocalTraceV1>,
    incoming: Option<&VocalTraceV1>,
    outgoing_cue_frame: i64,
    incoming_cue_frame: i64,
    incoming_rate_ppm: i64,
) -> Result<Option<VocalCollisionMeasurements>> {
    let (Some(outgoing), Some(incoming)) = (outgoing, incoming) else {
        return Ok(None);
    };
    validate_trace(outgoing)?;
    validate_trace(incoming)?;
    if outgoing.hop_frames != incoming.hop_frames
        || outgoing.analysis != incoming.analysis
        || incoming_rate_ppm <= 0
    {
        return Err(Error::new(
            "VOCAL_TRACE_IDENTITY_MISMATCH",
            "pair traces must use one positive-evidence algorithm and valid rate",
        ));
    }

    let total_hops = (VOCAL_PAIR_END_FRAME - VOCAL_PAIR_START_FRAME) / VOCAL_HOP_FRAMES;
    let mut collision_hops = 0_i64;
    let mut current: Option<Island> = None;
    let mut best: Option<Island> = None;
    let mut timeline = VOCAL_PAIR_START_FRAME;
    while timeline < VOCAL_PAIR_END_FRAME {
        let outgoing_frame = outgoing_cue_frame
            .checked_add(timeline)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "vocal collision frame overflow"))?;
        let incoming_offset = div_round_nearest_away(
            timeline.checked_mul(incoming_rate_ppm).ok_or_else(|| {
                Error::new("INTEGER_OVERFLOW", "vocal collision mapping overflow")
            })?,
            1_000_000,
        )?;
        let incoming_frame = incoming_cue_frame
            .checked_add(incoming_offset)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "vocal collision frame overflow"))?;
        let outgoing_strength = evidence_at(outgoing, outgoing_frame)?;
        let incoming_strength = evidence_at(incoming, incoming_frame)?;
        let simultaneous = outgoing_strength >= outgoing.analysis.positive_evidence_threshold_ppm
            && incoming_strength >= incoming.analysis.positive_evidence_threshold_ppm;
        if simultaneous {
            collision_hops += 1;
            let island = current.get_or_insert_with(|| Island::new(timeline));
            island.end = timeline + VOCAL_HOP_FRAMES;
            island.outgoing_sum += i128::from(outgoing_strength);
            island.incoming_sum += i128::from(incoming_strength);
            island.hops += 1;
        } else if let Some(island) = current.take() {
            if best.as_ref().is_none_or(|old| island.hops > old.hops) {
                best = Some(island);
            }
        }
        timeline += VOCAL_HOP_FRAMES;
    }
    if let Some(island) = current {
        if best.as_ref().is_none_or(|old| island.hops > old.hops) {
            best = Some(island);
        }
    }

    let collision_ppm = div_round_nearest_away(
        collision_hops
            .checked_mul(1_000_000)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "vocal collision overflow"))?,
        total_hops,
    )?;
    let (span_frames, start_frame, end_frame, outgoing_strength_ppm, incoming_strength_ppm) =
        if let Some(best) = best {
            let outgoing_mean = div_round_nearest_away_i128(best.outgoing_sum, best.hops)?;
            let incoming_mean = div_round_nearest_away_i128(best.incoming_sum, best.hops)?;
            (
                Some(best.end - best.start),
                Some(best.start),
                Some(best.end),
                Some(outgoing_mean),
                Some(incoming_mean),
            )
        } else {
            (None, None, None, None, None)
        };
    Ok(Some(VocalCollisionMeasurements {
        collision_ppm,
        span_frames,
        start_frame,
        end_frame,
        outgoing_strength_ppm,
        incoming_strength_ppm,
    }))
}

#[derive(Clone, Debug)]
struct Island {
    start: i64,
    end: i64,
    outgoing_sum: i128,
    incoming_sum: i128,
    hops: i64,
}

impl Island {
    fn new(start: i64) -> Self {
        Self {
            start,
            end: start,
            outgoing_sum: 0,
            incoming_sum: 0,
            hops: 0,
        }
    }
}

fn evidence_at(trace: &VocalTraceV1, source_frame: i64) -> Result<i64> {
    let index = usize::try_from(source_frame.div_euclid(VOCAL_HOP_FRAMES)).map_err(|_| {
        Error::new(
            "VOCAL_TRACE_BOUNDS",
            "collision maps before vocal trace source",
        )
    })?;
    trace.evidence_ppm.get(index).copied().ok_or_else(|| {
        Error::new(
            "VOCAL_TRACE_BOUNDS",
            "collision maps outside vocal trace source",
        )
    })
}

fn validate_trace(trace: &VocalTraceV1) -> Result<()> {
    validate_body(&trace.body())?;
    if trace.trace_sha256 != trace_hash(&trace.body())? {
        return Err(Error::new(
            "VOCAL_TRACE_HASH_MISMATCH",
            "vocal trace hash does not match its body",
        ));
    }
    Ok(())
}

fn validate_body(body: &VocalTraceBodyV1) -> Result<()> {
    let expected_hops = body
        .source_frames
        .checked_add(VOCAL_HOP_FRAMES - 1)
        .map(|value| value / VOCAL_HOP_FRAMES);
    let valid = body.schema_version == VOCAL_TRACE_SCHEMA_VERSION
        && valid_hash(&body.source_pcm_sha256)
        && body.source_frames > 0
        && body.hop_frames == VOCAL_HOP_FRAMES
        && !body.analysis.extractor_id.is_empty()
        && !body.analysis.extractor_version.is_empty()
        && valid_hash(&body.analysis.algorithm_sha256)
        && (0..=1_000_000).contains(&body.analysis.positive_evidence_threshold_ppm)
        && i64::try_from(body.evidence_ppm.len()).ok() == expected_hops
        && body
            .evidence_ppm
            .iter()
            .all(|value| (0..=1_000_000).contains(value));
    if !valid {
        return Err(Error::new(
            "INVALID_VOCAL_TRACE",
            "vocal trace body violates the frozen v1 contract",
        ));
    }
    Ok(())
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn trace_hash(body: &VocalTraceBodyV1) -> Result<String> {
    let mut digest = Sha256::new();
    digest.update(b"vocal-evidence-trace/1\0");
    digest.update(canonical_json(body)?);
    Ok(format!("{:x}", digest.finalize()))
}

fn div_round_nearest_away_i128(numerator: i128, denominator: i64) -> Result<i64> {
    let denominator = i128::from(denominator);
    let rounded = (numerator + denominator / 2) / denominator;
    i64::try_from(rounded)
        .map_err(|_| Error::new("INTEGER_OVERFLOW", "vocal strength mean overflow"))
}
