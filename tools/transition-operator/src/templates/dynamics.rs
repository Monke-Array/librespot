use super::gain::{draft, points, smooth_primary, time_map};
use super::{RecipeDuration, RecipeSpec, TemplateDraft, TemplateId, TemplateInputs, bind_recipe};
use crate::error::{Error, Result};
use crate::geometry::GeometryProposal;
use crate::model::{DuckEnvelope, DuckReason, Interpolation, Operation, RhythmicGate, Target};
use crate::scalar::div_round_nearest_away;

pub fn emit_dynamics_template(
    template_id: TemplateId,
    recipe: &RecipeSpec,
    geometry: &GeometryProposal,
    inputs: &TemplateInputs,
) -> Result<TemplateDraft> {
    bind_recipe(recipe, std::slice::from_ref(geometry))?;
    match template_id {
        TemplateId::DuckedOverlap => ducked(recipe, geometry, inputs),
        TemplateId::RhythmicHandoff => rhythmic(recipe, geometry, inputs),
        _ => Err(Error::new(
            "UNKNOWN_TEMPLATE",
            "dynamics emitter received an unsupported template",
        )),
    }
}

fn ducked(
    recipe: &RecipeSpec,
    geometry: &GeometryProposal,
    inputs: &TemplateInputs,
) -> Result<TemplateDraft> {
    let (depth, attack, release) = match recipe.id {
        "d6_fast_3s" | "d6_fast_2bar" => (501_187, 882, 3_528),
        "d9_fast_3s" | "d9_fast_2bar" => (354_813, 882, 3_528),
        "d6_smooth_5s" | "d6_smooth_4bar" => (501_187, 3_528, 8_820),
        "d9_smooth_5s" | "d9_smooth_4bar" => (354_813, 3_528, 8_820),
        _ => {
            return Err(Error::new(
                "UNKNOWN_RECIPE",
                "unknown ducked-overlap recipe",
            ));
        }
    };
    let two_beats = inputs.two_beats_frames.ok_or_else(|| {
        Error::new(
            "TEMPLATE_INAPPLICABLE",
            "duck resolution requires two-beat span",
        )
    })?;
    let vocal = localized(
        inputs.vocal_collision_ppm,
        inputs.vocal_collision_span_frames,
        two_beats,
    );
    let transient = localized(
        inputs.transient_collision_ppm,
        inputs.transient_collision_span_frames,
        two_beats,
    );
    if !(vocal ^ transient) {
        return Err(Error::new(
            "TEMPLATE_INAPPLICABLE",
            "duck requires exactly one localized collision",
        ));
    }
    let (reason, outgoing_activity, incoming_activity) = if vocal {
        (
            DuckReason::VocalCollision,
            inputs.outgoing_vocal_activity_ppm,
            inputs.incoming_vocal_activity_ppm,
        )
    } else {
        (
            DuckReason::TransientCollision,
            inputs.outgoing_transient_activity_ppm,
            inputs.incoming_transient_activity_ppm,
        )
    };
    let outgoing_activity = outgoing_activity
        .ok_or_else(|| Error::new("TEMPLATE_INAPPLICABLE", "duck target activity is missing"))?;
    let incoming_activity = incoming_activity
        .ok_or_else(|| Error::new("TEMPLATE_INAPPLICABLE", "duck target activity is missing"))?;
    let target = if outgoing_activity >= incoming_activity {
        Target::Outgoing
    } else {
        Target::Incoming
    };
    let start = -geometry.requested_dry_frames;
    let hold_start = inputs
        .collision_start_frame
        .ok_or_else(|| Error::new("TEMPLATE_INAPPLICABLE", "collision window is missing"))?
        .max(start);
    let hold_end = inputs
        .collision_end_frame
        .ok_or_else(|| Error::new("TEMPLATE_INAPPLICABLE", "collision window is missing"))?
        .min(0);
    let attack_start = hold_start
        .checked_sub(attack)
        .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "duck attack position overflow"))?;
    let release_end = hold_end
        .checked_add(release)
        .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "duck release position overflow"))?;
    if hold_start >= hold_end || attack_start < start || release_end > 0 {
        return Err(Error::new(
            "DUCK_WINDOW_OUT_OF_BOUNDS",
            "duck attack, hold, and release must fit the dry interval",
        ));
    }
    let curve = if attack == 882 {
        Interpolation::Linear
    } else {
        Interpolation::Smoothstep
    };
    let duck = DuckEnvelope {
        op_id: format!(
            "{}.duck",
            if target == Target::Outgoing {
                "outgoing"
            } else {
                "incoming"
            }
        ),
        target,
        reason,
        points: points(&[
            (attack_start, 1_000_000),
            (hold_start, depth),
            (hold_end, depth),
            (release_end, 1_000_000),
        ]),
        interpolations: vec![curve, Interpolation::Linear, curve],
    };
    let mut operations = if matches!(recipe.duration, RecipeDuration::Bars(_)) {
        time_map(geometry)
    } else {
        Vec::new()
    };
    operations.push(Operation::DuckEnvelope(duck));
    operations.extend(smooth_primary(start));
    Ok(draft(
        TemplateId::DuckedOverlap,
        recipe,
        geometry,
        start,
        0,
        operations,
    ))
}

fn rhythmic(
    recipe: &RecipeSpec,
    geometry: &GeometryProposal,
    inputs: &TemplateInputs,
) -> Result<TemplateDraft> {
    let (subdivision, requested_bars, final_peak): (i64, i64, i64) = match recipe.id {
        "quarter_1bar_even" => (1, 1, 1_000_000),
        "eighth_1bar_even" => (2, 1, 1_000_000),
        "quarter_2bar_decay" => (1, 2, 700_000),
        "eighth_1bar_decay" => (2, 1, 600_000),
        _ => {
            return Err(Error::new(
                "UNKNOWN_RECIPE",
                "unknown rhythmic-handoff recipe",
            ));
        }
    };
    let meter = inputs
        .meter_beats
        .ok_or_else(|| Error::new("TEMPLATE_INAPPLICABLE", "rhythmic recipe requires meter"))?;
    let needed_beats = meter
        .checked_mul(requested_bars)
        .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "rhythmic beat count overflow"))?;
    let start = -geometry.requested_dry_frames;
    let mut resolved_beats = inputs.beat_frames.clone();
    resolved_beats.sort_unstable();
    resolved_beats.dedup();
    let window_beats: Vec<_> = resolved_beats
        .into_iter()
        .filter(|frame| (start..=0).contains(frame))
        .collect();
    if window_beats.len() != needed_beats as usize + 1
        || window_beats.first() != Some(&start)
        || window_beats.last() != Some(&0)
    {
        return Err(Error::new(
            "RHYTHMIC_GRID_MISMATCH",
            "beat grid must exactly cover the requested bars",
        ));
    }
    let mut cells = Vec::new();
    for beat in window_beats.windows(2) {
        if subdivision == 1 {
            cells.push((beat[0], beat[1]));
        } else {
            let midpoint = div_round_nearest_away(
                beat[0]
                    .checked_add(beat[1])
                    .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "beat midpoint overflow"))?,
                2,
            )?;
            cells.extend([(beat[0], midpoint), (midpoint, beat[1])]);
        }
    }
    let cell_count = cells.len();
    let mut values = Vec::<(i64, i64)>::new();
    for (index, (cell_start, cell_end)) in cells.into_iter().enumerate() {
        let midpoint = div_round_nearest_away(
            cell_start
                .checked_add(cell_end)
                .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "cell midpoint overflow"))?,
            2,
        )?;
        if midpoint - cell_start < 442 {
            return Err(Error::new(
                "RHYTHMIC_CELL_TOO_SHORT",
                "rhythmic cell cannot contain both click-safe ramps",
            ));
        }
        let peak = if cell_count == 1 {
            final_peak
        } else {
            1_000_000
                + div_round_nearest_away(
                    (final_peak - 1_000_000)
                        .checked_mul(index as i64)
                        .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "gate decay overflow"))?,
                    cell_count as i64 - 1,
                )?
        };
        append_point(&mut values, (cell_start, 0));
        append_point(&mut values, (cell_start + 221, peak));
        append_point(&mut values, (midpoint - 221, peak));
        append_point(&mut values, (midpoint, 0));
        append_point(&mut values, (cell_end, 0));
    }
    let gate = RhythmicGate {
        op_id: "outgoing.gate".into(),
        target: Target::Outgoing,
        interpolations: vec![Interpolation::Linear; values.len() - 1],
        points: points(&values),
    };
    let mut operations = time_map(geometry);
    operations.push(Operation::RhythmicGate(gate));
    operations.extend(smooth_primary(start));
    Ok(draft(
        TemplateId::RhythmicHandoff,
        recipe,
        geometry,
        start,
        0,
        operations,
    ))
}

fn localized(value: Option<i64>, span: Option<i64>, two_beats: i64) -> bool {
    matches!(value, Some(300_000..=700_000)) && span.is_some_and(|span| span <= two_beats)
}

fn append_point(points: &mut Vec<(i64, i64)>, point: (i64, i64)) {
    if points.last().is_some_and(|previous| *previous == point) {
        return;
    }
    points.push(point);
}
