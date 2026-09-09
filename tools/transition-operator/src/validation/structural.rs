use crate::canonical::canonical_json;
use crate::error::{Error, Result};
use crate::model::*;
use crate::scalar::{JSON_SAFE_INTEGER_MAX, div_round_nearest_away};
use std::collections::HashSet;

pub(super) fn validate_schema_and_scalars(body: &OperatorPlanBody) -> Result<()> {
    if body.schema_version != "transition-operator-plan/1"
        || body.feature_snapshot.schema_version != "transition-feature-snapshot/2"
    {
        return Err(Error::new(
            "UNSUPPORTED_SCHEMA_VERSION",
            "exact v1 plan and feature reference versions are required",
        ));
    }
    if body.format.sample_rate_hz != 44_100
        || body.format.channels != 2
        || body.format.channel_order != ChannelOrder::StereoLr
    {
        return Err(Error::new(
            "INVALID_AUDIO_FORMAT",
            "canonical stereo 44100 Hz format is required",
        ));
    }
    let value = serde_json::to_value(body)
        .map_err(|_| Error::new("JSON_SERIALIZATION_FAILED", "plan serialization failed"))?;
    validate_value_scalars(&value)?;
    for hash in [
        &body.sources.outgoing.pcm_sha256,
        &body.sources.incoming.pcm_sha256,
        &body.feature_snapshot.sha256,
        &body.provenance.generator_config_sha256,
    ] {
        if !is_sha256(hash) {
            return Err(Error::new(
                "INVALID_SHA256",
                "hash must be 64 lowercase hexadecimal characters",
            ));
        }
    }
    if body.provenance.seed_hex_u64 != "0000000000000000" {
        return Err(Error::new(
            "INVALID_DSP_SEED",
            "v1 DSP seed must be all zero",
        ));
    }
    for source in [&body.sources.outgoing, &body.sources.incoming] {
        if source.pcm_profile != "pcm_s16le_stereo_44100_v1" {
            return Err(Error::new(
                "INVALID_PCM_PROFILE",
                "canonical PCM source profile is required",
            ));
        }
        if source.pcm_frame_count <= 0 {
            return Err(Error::new(
                "INVALID_SOURCE_LENGTH",
                "source frame count must be positive",
            ));
        }
    }
    validate_safety(&body.output_safety)?;
    let _ = canonical_json(body)?;
    Ok(())
}

fn validate_value_scalars(value: &serde_json::Value) -> Result<()> {
    match value {
        serde_json::Value::String(value)
            if !value.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) =>
        {
            Err(Error::new(
                "NON_ASCII_PLAN_STRING",
                "plan strings must be printable ASCII",
            ))
        }
        serde_json::Value::Number(number) => {
            let value = number.as_i64().ok_or_else(|| {
                Error::new(
                    "INTEGER_OUT_OF_RANGE",
                    "plan numbers must be signed integers",
                )
            })?;
            if !(-JSON_SAFE_INTEGER_MAX..=JSON_SAFE_INTEGER_MAX).contains(&value) {
                return Err(Error::new(
                    "INTEGER_OUT_OF_RANGE",
                    "integer exceeds interoperable range",
                ));
            }
            Ok(())
        }
        serde_json::Value::Array(values) => values.iter().try_for_each(validate_value_scalars),
        serde_json::Value::Object(values) => values.values().try_for_each(validate_value_scalars),
        _ => Ok(()),
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_safety(value: &OutputSafety) -> Result<()> {
    if !(-24_000..=0).contains(&value.pair_output_gain_mdb) {
        return Err(Error::new(
            "INVALID_PAIR_OUTPUT_GAIN",
            "pair output gain must be -24000..=0 mdb",
        ));
    }
    if value.profile != OutputSafetyProfile::TransitionOutputSafetyV1
        || value.sample_peak_ceiling_mdbfs != -1_200
        || value.true_peak_target_mdbtp != -1_000
        || value.limiter_profile != LimiterProfile::LookaheadPeakLimiterV1
        || value.lookahead_frames != 221
        || value.release_frames != 4_410
        || value.maximum_gain_reduction_mdb != 3_000
        || value.maximum_active_fraction_ppm != 50_000
        || value.activity_threshold_mdb != 100
        || value.true_peak_measurement_profile != TruePeakProfile::Bs1770_4xV1
    {
        return Err(Error::new(
            "INVALID_OUTPUT_SAFETY_PROFILE",
            "output safety constants must exactly match v1",
        ));
    }
    Ok(())
}

pub(super) fn validate_operations(body: &OperatorPlanBody) -> Result<()> {
    if body.operations.len() > 12 {
        return Err(Error::new(
            "TOO_MANY_OPERATIONS",
            "v1 permits at most 12 operations",
        ));
    }
    let mut ids = HashSet::new();
    for operation in &body.operations {
        if !ids.insert(operation.op_id()) {
            return Err(Error::new(
                "DUPLICATE_OPERATION_ID",
                "operation IDs must be unique",
            ));
        }
    }
    let keys: Vec<_> = body.operations.iter().map(order_key).collect();
    if keys.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(Error::new(
            "NON_CANONICAL_OPERATION_ORDER",
            "operation array is not in normative stage order",
        ));
    }

    let mut total_points = 0usize;
    let mut outgoing_primary = 0;
    let mut incoming_primary = 0;
    for operation in &body.operations {
        match operation {
            Operation::TimeMap(value) => validate_time_map(value)?,
            Operation::GainEnvelope(value) => {
                validate_envelope(
                    &value.points,
                    &value.interpolations,
                    8,
                    0,
                    1_000_000,
                    body,
                    value.op_id.as_str(),
                )?;
                total_points += value.points.len();
                if value.op_id == "outgoing.primary_gain" && value.target == Target::Outgoing {
                    outgoing_primary += 1;
                    validate_primary(value, body, true)?;
                }
                if value.op_id == "incoming.primary_gain" && value.target == Target::Incoming {
                    incoming_primary += 1;
                    validate_primary(value, body, false)?;
                }
            }
            Operation::FilterEnvelope(value) => {
                total_points += validate_filter(value, body)?;
            }
            Operation::CrossoverBandGain(value) => {
                total_points += validate_crossover(value, body)?;
            }
            Operation::DuckEnvelope(value) => {
                total_points += validate_duck(value, body)?;
            }
            Operation::FeedforwardDelayTail(value) => validate_delay(value, body)?,
            Operation::RhythmicGate(value) => {
                total_points += validate_gate(value, body)?;
            }
        }
    }
    if outgoing_primary != 1 || incoming_primary != 1 {
        return Err(Error::new(
            "MISSING_PRIMARY_GAIN",
            "exactly one primary gain per source is required",
        ));
    }
    if total_points > 512 {
        return Err(Error::new(
            "TOO_MANY_ENVELOPE_POINTS",
            "plan exceeds 512 total envelope points",
        ));
    }
    validate_combinations(body)?;
    Ok(())
}

fn order_key(operation: &Operation) -> (u8, u8, &str) {
    let stage = match operation {
        Operation::TimeMap(_) => 1,
        Operation::FilterEnvelope(_) | Operation::CrossoverBandGain(_) => 2,
        Operation::DuckEnvelope(_) | Operation::RhythmicGate(_) => 3,
        Operation::FeedforwardDelayTail(_) => 4,
        Operation::GainEnvelope(_) => 5,
    };
    let target = match operation.target() {
        Target::Outgoing => 0,
        Target::Incoming => 1,
    };
    (stage, target, operation.op_id())
}

fn validate_envelope(
    points: &[EnvelopePoint],
    interpolations: &[Interpolation],
    max: usize,
    min_value: i64,
    max_value: i64,
    body: &OperatorPlanBody,
    op_id: &str,
) -> Result<()> {
    if points.len() < 2 || points.len() > max || interpolations.len() + 1 != points.len() {
        return Err(Error::new(
            "INVALID_ENVELOPE_SHAPE",
            "envelope point/interpolation count is invalid",
        ));
    }
    if points.windows(2).any(|pair| pair[0].frame >= pair[1].frame) {
        return Err(Error::new(
            "NON_INCREASING_ENVELOPE_POINTS",
            "envelope frames must strictly increase",
        ));
    }
    if points
        .iter()
        .any(|point| !(min_value..=max_value).contains(&point.value_ppm))
    {
        return Err(Error::new(
            "GAIN_OUT_OF_RANGE",
            "envelope gain is outside its nonboosting range",
        ));
    }
    if points.iter().any(|point| {
        point.frame < body.timeline.dry_start_frame || point.frame > body.timeline.effect_end_frame
    }) {
        return Err(Error::new(
            "OPERATION_OUTSIDE_TIMELINE",
            "envelope point is outside transition timeline",
        ));
    }
    for (segment, interpolation) in points.windows(2).zip(interpolations) {
        if *interpolation == Interpolation::Hold
            && segment[0].value_ppm != segment[1].value_ppm
            && !(body.template.id == "beat_cut"
                && body.template.recipe_id == "hard_0ms"
                && op_id.ends_with("primary_gain"))
        {
            return Err(Error::new(
                "INVALID_HOLD_STEP",
                "hold with unequal endpoints is allowed only for hard beat cut",
            ));
        }
    }
    Ok(())
}

fn validate_primary(value: &GainEnvelope, body: &OperatorPlanBody, outgoing: bool) -> Result<()> {
    let first = value.points.first().unwrap();
    let last = value.points.last().unwrap();
    let valid = first.frame >= body.timeline.dry_start_frame
        && last.frame == 0
        && if outgoing {
            first.value_ppm == 1_000_000 && last.value_ppm == 0
        } else {
            first.value_ppm == 0 && last.value_ppm == 1_000_000
        };
    if !valid {
        return Err(Error::new(
            "INVALID_PRIMARY_GAIN_ENDPOINT",
            "primary gains must have exact dry-start and handoff endpoints",
        ));
    }
    Ok(())
}

fn validate_time_map(value: &TimeMap) -> Result<()> {
    if value.op_id != "incoming.time_map"
        || value.target != Target::Incoming
        || !(920_000..=1_080_000).contains(&value.source_rate_ppm)
    {
        return Err(Error::new(
            "INVALID_TIME_MAP",
            "incoming time map violates v1 identity, target, or rate",
        ));
    }
    Ok(())
}

fn validate_cutoffs(
    points: &[CutoffPoint],
    interpolations: &[Interpolation],
    body: &OperatorPlanBody,
) -> Result<()> {
    if points.len() < 2
        || points.len() > 8
        || interpolations.len() + 1 != points.len()
        || points.windows(2).any(|p| p[0].frame >= p[1].frame)
    {
        return Err(Error::new(
            "INVALID_FILTER_ENVELOPE",
            "cutoff envelope shape is invalid",
        ));
    }
    if points.iter().any(|p| {
        !(20_000..=18_000_000).contains(&p.cutoff_millihz)
            || p.frame < body.timeline.dry_start_frame
            || p.frame > body.timeline.effect_end_frame
    }) {
        return Err(Error::new(
            "FILTER_PARAMETER_OUT_OF_RANGE",
            "filter cutoff or frame is outside v1 bounds",
        ));
    }
    if interpolations
        .iter()
        .any(|i| !matches!(i, Interpolation::Linear | Interpolation::Smoothstep))
    {
        return Err(Error::new(
            "INVALID_FILTER_INTERPOLATION",
            "filter interpolation must be linear or smoothstep",
        ));
    }
    Ok(())
}

fn validate_filter(value: &FilterEnvelope, body: &OperatorPlanBody) -> Result<usize> {
    if !(500..=1_000).contains(&value.q_milli) || value.control_interval_frames != 64 {
        return Err(Error::new(
            "FILTER_PARAMETER_OUT_OF_RANGE",
            "filter Q/control interval violates v1",
        ));
    }
    validate_cutoffs(&value.cutoff_points, &value.cutoff_interpolations, body)?;
    validate_envelope(
        &value.wet_points,
        &value.wet_interpolations,
        8,
        0,
        1_000_000,
        body,
        &value.op_id,
    )?;
    if value
        .wet_interpolations
        .iter()
        .any(|i| !matches!(i, Interpolation::Linear | Interpolation::Smoothstep))
    {
        return Err(Error::new(
            "INVALID_FILTER_INTERPOLATION",
            "wet interpolation must be linear or smoothstep",
        ));
    }
    Ok(value.cutoff_points.len() + value.wet_points.len())
}

fn validate_crossover(value: &CrossoverBandGain, body: &OperatorPlanBody) -> Result<usize> {
    if value.control_interval_frames != 64
        || !(1..=2).contains(&value.crossover_millihz.len())
        || value.crossover_millihz.windows(2).any(|p| p[0] >= p[1])
        || value
            .crossover_millihz
            .iter()
            .any(|f| !(80_000..=5_000_000).contains(f))
    {
        return Err(Error::new(
            "INVALID_CROSSOVER",
            "crossover frequency/count/order violates v1",
        ));
    }
    let expected_bands: &[Band] = if value.crossover_millihz.len() == 1 {
        &[Band::Low, Band::High]
    } else {
        &[Band::Low, Band::Mid, Band::High]
    };
    if value.bands != expected_bands || value.band_gain_envelopes.len() != expected_bands.len() {
        return Err(Error::new(
            "INVALID_CROSSOVER_BANDS",
            "band topology must exactly match crossover count",
        ));
    }
    let mut count = 0;
    for (index, envelope) in value.band_gain_envelopes.iter().enumerate() {
        if envelope.band != expected_bands[index] {
            return Err(Error::new(
                "INVALID_CROSSOVER_BANDS",
                "band envelopes must follow canonical band order",
            ));
        }
        validate_envelope(
            &envelope.points,
            &envelope.interpolations,
            8,
            0,
            1_000_000,
            body,
            &value.op_id,
        )?;
        if envelope
            .interpolations
            .iter()
            .any(|interpolation| *interpolation != Interpolation::Linear)
        {
            return Err(Error::new(
                "INVALID_CROSSOVER_INTERPOLATION",
                "crossover band ownership must use linear interpolation",
            ));
        }
        count += envelope.points.len();
    }
    validate_envelope(
        &value.wet_points,
        &value.wet_interpolations,
        8,
        0,
        1_000_000,
        body,
        &value.op_id,
    )?;
    if value.wet_interpolations.iter().any(|interpolation| {
        !matches!(
            interpolation,
            Interpolation::Linear | Interpolation::Smoothstep
        )
    }) {
        return Err(Error::new(
            "INVALID_CROSSOVER_INTERPOLATION",
            "crossover wet interpolation must be linear or smoothstep",
        ));
    }
    count += value.wet_points.len();
    Ok(count)
}

fn validate_duck(value: &DuckEnvelope, body: &OperatorPlanBody) -> Result<usize> {
    validate_envelope(
        &value.points,
        &value.interpolations,
        6,
        251_189,
        1_000_000,
        body,
        &value.op_id,
    )?;
    if value
        .interpolations
        .iter()
        .any(|i| !matches!(i, Interpolation::Linear | Interpolation::Smoothstep))
    {
        return Err(Error::new(
            "INVALID_DUCK",
            "duck curves must be linear or smoothstep",
        ));
    }
    let attack = value.points[1].frame - value.points[0].frame;
    let release =
        value.points[value.points.len() - 1].frame - value.points[value.points.len() - 2].frame;
    if !(882..=8_820).contains(&attack) || !(882..=8_820).contains(&release) {
        return Err(Error::new(
            "INVALID_DUCK_RAMP",
            "duck attack/release must be 20-200 ms",
        ));
    }
    Ok(value.points.len())
}

fn validate_delay(value: &FeedforwardDelayTail, body: &OperatorPlanBody) -> Result<()> {
    if value.target != Target::Outgoing
        || value.capture_start_frame < body.timeline.dry_start_frame
        || value.capture_start_frame >= value.capture_end_frame
        || value.capture_end_frame > 0
        || !(1..=3).contains(&value.taps.len())
    {
        return Err(Error::new(
            "INVALID_DELAY_TAIL",
            "delay capture/target/tap count violates v1",
        ));
    }
    let mut sum = 0;
    let mut previous_delay = 0;
    let mut previous_gain = i64::MAX;
    for tap in &value.taps {
        if !(882..=88_200).contains(&tap.delay_frames)
            || tap.delay_frames <= previous_delay
            || !(1..=500_000).contains(&tap.gain_ppm)
            || tap.gain_ppm >= previous_gain
        {
            return Err(Error::new(
                "INVALID_DELAY_TAP",
                "delay taps must be bounded, increasing, and nonincreasing in gain",
            ));
        }
        sum += tap.gain_ppm;
        previous_delay = tap.delay_frames;
        previous_gain = tap.gain_ppm;
    }
    if sum > 800_000 || value.capture_end_frame + previous_delay != body.timeline.effect_end_frame {
        return Err(Error::new(
            "INVALID_DELAY_TAIL",
            "delay sum or effect end violates v1",
        ));
    }
    Ok(())
}

fn validate_gate(value: &RhythmicGate, body: &OperatorPlanBody) -> Result<usize> {
    validate_envelope(
        &value.points,
        &value.interpolations,
        256,
        0,
        1_000_000,
        body,
        &value.op_id,
    )?;
    if value.target != Target::Outgoing
        || value.points.last().unwrap().frame != 0
        || value.points.last().unwrap().value_ppm != 0
    {
        return Err(Error::new(
            "INVALID_RHYTHMIC_GATE",
            "v1 gate must target outgoing and end at zero at handoff",
        ));
    }
    for pair in value.points.windows(2) {
        if pair[0].value_ppm != pair[1].value_ppm && pair[1].frame - pair[0].frame < 221 {
            return Err(Error::new(
                "INVALID_RHYTHMIC_GATE",
                "gate changes require 221 frame edges",
            ));
        }
    }
    let transition_starts: Vec<_> = value
        .points
        .windows(2)
        .filter(|pair| pair[0].value_ppm != pair[1].value_ppm)
        .map(|pair| pair[0].frame)
        .collect();
    if transition_starts
        .windows(9)
        .any(|events| events[8] - events[0] < 44_100)
    {
        return Err(Error::new(
            "INVALID_RHYTHMIC_GATE",
            "gate has more than eight transitions in one second",
        ));
    }
    if value.points.last().unwrap().frame - value.points.first().unwrap().frame > 529_200 {
        return Err(Error::new(
            "INVALID_RHYTHMIC_GATE",
            "gate exceeds two-bar absolute safety bound",
        ));
    }
    Ok(value.points.len())
}

fn validate_combinations(body: &OperatorPlanBody) -> Result<()> {
    let count = |predicate: fn(&Operation) -> bool| {
        body.operations.iter().filter(|op| predicate(op)).count()
    };
    let tail_count = count(|op| matches!(op, Operation::FeedforwardDelayTail(_)));
    if count(|op| matches!(op, Operation::TimeMap(_))) > 1
        || tail_count > 1
        || (tail_count == 0 && body.timeline.effect_end_frame != 0)
    {
        return Err(Error::new(
            "INVALID_OPERATION_COMBINATION",
            "time map and tail are singular",
        ));
    }
    for target in [Target::Outgoing, Target::Incoming] {
        let filter = body
            .operations
            .iter()
            .any(|op| op.target() == target && matches!(op, Operation::FilterEnvelope(_)));
        let crossover = body
            .operations
            .iter()
            .any(|op| op.target() == target && matches!(op, Operation::CrossoverBandGain(_)));
        let duck = body
            .operations
            .iter()
            .any(|op| op.target() == target && matches!(op, Operation::DuckEnvelope(_)));
        let gate = body
            .operations
            .iter()
            .any(|op| op.target() == target && matches!(op, Operation::RhythmicGate(_)));
        if (filter && crossover) || (duck && gate) {
            return Err(Error::new(
                "INVALID_OPERATION_COMBINATION",
                "source has mutually exclusive operations",
            ));
        }
    }
    if body.template.id == "bass_handoff" {
        validate_complementary_bass(body)?;
    }
    Ok(())
}

fn validate_complementary_bass(body: &OperatorPlanBody) -> Result<()> {
    let crossovers: Vec<_> = body
        .operations
        .iter()
        .filter_map(|operation| match operation {
            Operation::CrossoverBandGain(value) => Some(value),
            _ => None,
        })
        .collect();
    if crossovers.len() != 2 {
        return Ok(());
    }
    let outgoing = crossovers
        .iter()
        .find(|value| value.target == Target::Outgoing)
        .ok_or_else(|| Error::new("INVALID_BASS_HANDOFF", "missing outgoing crossover"))?;
    let incoming = crossovers
        .iter()
        .find(|value| value.target == Target::Incoming)
        .ok_or_else(|| Error::new("INVALID_BASS_HANDOFF", "missing incoming crossover"))?;
    if low_band(outgoing)
        .interpolations
        .iter()
        .chain(&low_band(incoming).interpolations)
        .any(|value| *value != Interpolation::Linear)
    {
        return Err(Error::new(
            "INVALID_BASS_HANDOFF",
            "complementary bass transfer must use linear interpolation",
        ));
    }
    let mut frames = Vec::new();
    for point in low_band(outgoing)
        .points
        .iter()
        .chain(&low_band(incoming).points)
    {
        frames.push(point.frame);
    }
    frames.sort_unstable();
    frames.dedup();
    for pair in frames.clone().windows(2) {
        frames.push(div_round_nearest_away(pair[0] + pair[1], 2)?);
    }
    for frame in frames {
        let (out_numerator, out_denominator) = linear_envelope_ratio(low_band(outgoing), frame);
        let (in_numerator, in_denominator) = linear_envelope_ratio(low_band(incoming), frame);
        if out_numerator * in_denominator + in_numerator * out_denominator
            > 1_000_000_i128 * out_denominator * in_denominator
        {
            return Err(Error::new(
                "BASS_OWNERSHIP_OVERLAP",
                "complementary low-band gains exceed unity",
            ));
        }
    }
    Ok(())
}

fn low_band(value: &CrossoverBandGain) -> &BandGainEnvelope {
    value
        .band_gain_envelopes
        .iter()
        .find(|envelope| envelope.band == Band::Low)
        .unwrap()
}

fn linear_envelope_ratio(envelope: &BandGainEnvelope, frame: i64) -> (i128, i128) {
    if frame <= envelope.points[0].frame {
        return (i128::from(envelope.points[0].value_ppm), 1);
    }
    for pair in envelope.points.windows(2) {
        if frame < pair[1].frame {
            let denominator = i128::from(pair[1].frame - pair[0].frame);
            let offset = i128::from(frame - pair[0].frame);
            let numerator = i128::from(pair[0].value_ppm) * denominator
                + i128::from(pair[1].value_ppm - pair[0].value_ppm) * offset;
            return (numerator, denominator);
        }
    }
    (i128::from(envelope.points.last().unwrap().value_ppm), 1)
}
