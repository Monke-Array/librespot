use super::{RecipeId, RecipeSpec, TemplateDraft, TemplateId, TemplateInputs, bind_recipe};
use crate::error::{Error, Result};
use crate::geometry::GeometryProposal;
use crate::model::{
    CutoffPoint, EnvelopePoint, FilterEnvelope, FilterKind, GainEnvelope, Interpolation, Operation,
    Target, TimeMap, TimeMapProfile,
};
use crate::scalar::div_round_nearest_away;

pub fn emit_gain_template(
    template_id: TemplateId,
    recipe: &RecipeSpec,
    geometry: &GeometryProposal,
    inputs: &TemplateInputs,
) -> Result<TemplateDraft> {
    bind_recipe(recipe, std::slice::from_ref(geometry))?;
    match template_id {
        TemplateId::ShapedHandoff => shaped(recipe, geometry),
        TemplateId::BeatCut => beat_cut(recipe, geometry, inputs),
        TemplateId::EnergyRamp => energy(recipe, geometry, inputs),
        _ => Err(Error::new(
            "UNKNOWN_TEMPLATE",
            "gain emitter received a non-gain template",
        )),
    }
}

fn shaped(recipe: &RecipeSpec, geometry: &GeometryProposal) -> Result<TemplateDraft> {
    if recipe.id == "eq_4bar" && geometry.requested_dry_frames > 441_000 {
        return Err(Error::new(
            "GEOMETRY_TOO_LONG",
            "four-bar equal-power recipe is limited to ten seconds",
        ));
    }
    let start = -geometry.requested_dry_frames;
    let mut operations = time_map(geometry);
    let (outgoing, incoming) = if recipe.id.starts_with("eq_") {
        (
            gain(
                Target::Outgoing,
                &[(start, 1_000_000), (0, 0)],
                &[Interpolation::QuarterCosine],
            ),
            gain(
                Target::Incoming,
                &[(start, 0), (0, 1_000_000)],
                &[Interpolation::QuarterSine],
            ),
        )
    } else if recipe.id.starts_with("asym_early_") {
        let early = position(start, 3, 4)?;
        (
            gain(
                Target::Outgoing,
                &[(start, 1_000_000), (early, 0), (0, 0)],
                &[Interpolation::Smoothstep, Interpolation::Linear],
            ),
            gain(
                Target::Incoming,
                &[(start, 0), (0, 1_000_000)],
                &[Interpolation::Smoothstep],
            ),
        )
    } else {
        return Err(Error::new(
            "UNKNOWN_RECIPE",
            "unknown shaped-handoff recipe",
        ));
    };
    operations.extend([
        Operation::GainEnvelope(outgoing),
        Operation::GainEnvelope(incoming),
    ]);
    Ok(draft(
        TemplateId::ShapedHandoff,
        recipe,
        geometry,
        start,
        0,
        operations,
    ))
}

fn beat_cut(
    recipe: &RecipeSpec,
    geometry: &GeometryProposal,
    inputs: &TemplateInputs,
) -> Result<TemplateDraft> {
    let ramp = match recipe.id {
        "hard_0ms" => {
            if inputs.outgoing_hard_cut_safe != Some(true)
                || inputs.incoming_hard_cut_safe != Some(true)
            {
                return Err(Error::new(
                    "TEMPLATE_INAPPLICABLE",
                    "hard cut requires click-safe boundaries",
                ));
            }
            0
        }
        "soft_10ms" => 441,
        "soft_20ms" => 882,
        "soft_30ms" => 1_323,
        _ => return Err(Error::new("UNKNOWN_RECIPE", "unknown beat-cut recipe")),
    };
    let start = if ramp == 0 { -1 } else { -ramp };
    ensure_bounds(geometry, start)?;
    let interpolation = if ramp == 0 {
        Interpolation::Hold
    } else {
        Interpolation::Linear
    };
    let mut operations = time_map(geometry);
    operations.extend([
        Operation::GainEnvelope(gain(
            Target::Outgoing,
            &[(start, 1_000_000), (0, 0)],
            &[interpolation],
        )),
        Operation::GainEnvelope(gain(
            Target::Incoming,
            &[(start, 0), (0, 1_000_000)],
            &[interpolation],
        )),
    ]);
    Ok(draft(
        TemplateId::BeatCut,
        recipe,
        geometry,
        start,
        0,
        operations,
    ))
}

fn energy(
    recipe: &RecipeSpec,
    geometry: &GeometryProposal,
    inputs: &TemplateInputs,
) -> Result<TemplateDraft> {
    let delta = inputs.energy_delta_mdb.ok_or_else(|| {
        Error::new(
            "TEMPLATE_INAPPLICABLE",
            "energy ramp requires a measured pair delta",
        )
    })?;
    if delta.checked_abs().is_none_or(|value| value < 3_000) {
        return Err(Error::new(
            "TEMPLATE_INAPPLICABLE",
            "energy mismatch is below the v1 threshold",
        ));
    }
    let filter_assisted = recipe.id.starts_with("filter_");
    if !filter_assisted && !recipe.id.starts_with("gain_") {
        return Err(Error::new("UNKNOWN_RECIPE", "unknown energy-ramp recipe"));
    }
    let start = -geometry.requested_dry_frames;
    let quarter = position(start, 1, 4)?;
    let three_quarters = position(start, 3, 4)?;
    let mut operations = Vec::new();
    if filter_assisted {
        operations.push(Operation::FilterEnvelope(if delta > 0 {
            filter(
                Target::Outgoing,
                start,
                FilterKind::LowpassBiquadV1,
                18_000_000,
                500_000,
                0,
                1_000_000,
            )
        } else {
            filter(
                Target::Incoming,
                start,
                FilterKind::HighpassBiquadV1,
                1_600_000,
                20_000,
                1_000_000,
                0,
            )
        }));
    }
    let (outgoing, incoming) = if delta > 0 {
        let attenuation = delta.min(6_000);
        let plateau = (1_000_000.0 * 10_f64.powf(-(attenuation as f64) / 20_000.0)).round() as i64;
        (
            gain(
                Target::Outgoing,
                &[(start, 1_000_000), (0, 0)],
                &[Interpolation::Smoothstep],
            ),
            gain(
                Target::Incoming,
                &[
                    (start, 0),
                    (quarter, plateau),
                    (three_quarters, plateau),
                    (0, 1_000_000),
                ],
                &[
                    Interpolation::Smoothstep,
                    Interpolation::Hold,
                    Interpolation::Smoothstep,
                ],
            ),
        )
    } else {
        (
            gain(
                Target::Outgoing,
                &[(start, 1_000_000), (three_quarters, 0), (0, 0)],
                &[Interpolation::Smoothstep, Interpolation::Linear],
            ),
            gain(
                Target::Incoming,
                &[(start, 0), (0, 1_000_000)],
                &[Interpolation::Smoothstep],
            ),
        )
    };
    operations.extend([
        Operation::GainEnvelope(outgoing),
        Operation::GainEnvelope(incoming),
    ]);
    Ok(draft(
        TemplateId::EnergyRamp,
        recipe,
        geometry,
        start,
        0,
        operations,
    ))
}

pub(super) fn filter(
    target: Target,
    start: i64,
    filter_kind: FilterKind,
    start_cutoff: i64,
    end_cutoff: i64,
    start_wet: i64,
    end_wet: i64,
) -> FilterEnvelope {
    FilterEnvelope {
        op_id: format!("{}.filter", target_name(target)),
        target,
        filter_kind,
        cutoff_points: vec![
            CutoffPoint {
                frame: start,
                cutoff_millihz: start_cutoff,
            },
            CutoffPoint {
                frame: 0,
                cutoff_millihz: end_cutoff,
            },
        ],
        cutoff_interpolations: vec![Interpolation::Smoothstep],
        q_milli: 707,
        wet_points: points(&[(start, start_wet), (0, end_wet)]),
        wet_interpolations: vec![Interpolation::Smoothstep],
        control_interval_frames: 64,
    }
}

pub(super) fn gain(
    target: Target,
    values: &[(i64, i64)],
    interpolations: &[Interpolation],
) -> GainEnvelope {
    GainEnvelope {
        op_id: format!("{}.primary_gain", target_name(target)),
        target,
        points: points(values),
        interpolations: interpolations.to_vec(),
    }
}

pub(super) fn equal_power(start: i64) -> [Operation; 2] {
    [
        Operation::GainEnvelope(gain(
            Target::Outgoing,
            &[(start, 1_000_000), (0, 0)],
            &[Interpolation::QuarterCosine],
        )),
        Operation::GainEnvelope(gain(
            Target::Incoming,
            &[(start, 0), (0, 1_000_000)],
            &[Interpolation::QuarterSine],
        )),
    ]
}

pub(super) fn smooth_primary(start: i64) -> [Operation; 2] {
    [
        Operation::GainEnvelope(gain(
            Target::Outgoing,
            &[(start, 1_000_000), (0, 0)],
            &[Interpolation::Smoothstep],
        )),
        Operation::GainEnvelope(gain(
            Target::Incoming,
            &[(start, 0), (0, 1_000_000)],
            &[Interpolation::Smoothstep],
        )),
    ]
}

pub(super) fn time_map(geometry: &GeometryProposal) -> Vec<Operation> {
    if geometry.incoming_source_rate_ppm == 1_000_000 {
        Vec::new()
    } else {
        vec![Operation::TimeMap(TimeMap {
            op_id: "incoming.time_map".into(),
            target: Target::Incoming,
            source_rate_ppm: geometry.incoming_source_rate_ppm,
            profile: TimeMapProfile::PitchPreservingBalancedTransientsV1,
        })]
    }
}

pub(super) fn points(values: &[(i64, i64)]) -> Vec<EnvelopePoint> {
    values
        .iter()
        .map(|&(frame, value_ppm)| EnvelopePoint { frame, value_ppm })
        .collect()
}

pub(super) fn draft(
    template_id: TemplateId,
    recipe: &RecipeSpec,
    geometry: &GeometryProposal,
    dry_start_frame: i64,
    effect_end_frame: i64,
    operations: Vec<Operation>,
) -> TemplateDraft {
    TemplateDraft {
        template_id,
        recipe_id: RecipeId::new(recipe.id),
        geometry_id: geometry.geometry_id.clone(),
        dry_start_frame,
        effect_end_frame,
        operations,
    }
}

pub(super) fn position(start: i64, numerator: i64, denominator: i64) -> Result<i64> {
    let duration = start
        .checked_neg()
        .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "template duration cannot be negated"))?;
    let offset = div_round_nearest_away(
        duration
            .checked_mul(numerator)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "template position overflow"))?,
        denominator,
    )?;
    start
        .checked_add(offset)
        .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "template position overflow"))
}

pub(super) fn ensure_bounds(geometry: &GeometryProposal, start: i64) -> Result<()> {
    let incoming_offset = div_round_nearest_away(
        start
            .checked_mul(geometry.incoming_source_rate_ppm)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "template source mapping overflow"))?,
        1_000_000,
    )?;
    if geometry
        .outgoing_cue_source_frame
        .checked_add(start)
        .is_none_or(|frame| frame < 0)
        || geometry
            .incoming_cue_source_frame
            .checked_add(incoming_offset)
            .is_none_or(|frame| frame < 0)
    {
        return Err(Error::new(
            "GEOMETRY_SOURCE_OUT_OF_BOUNDS",
            "template interval exceeds source bounds",
        ));
    }
    Ok(())
}

fn target_name(target: Target) -> &'static str {
    match target {
        Target::Outgoing => "outgoing",
        Target::Incoming => "incoming",
    }
}
