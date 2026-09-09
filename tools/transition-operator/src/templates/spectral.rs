use super::gain::{draft, equal_power, filter, points, position, time_map};
use super::{RecipeDuration, RecipeSpec, TemplateDraft, TemplateId, TemplateInputs, bind_recipe};
use crate::error::{Error, Result};
use crate::geometry::GeometryProposal;
use crate::model::{
    Band, BandGainEnvelope, CrossoverBandGain, CrossoverProfile, FilterKind, Interpolation,
    Operation, Target,
};

pub fn emit_spectral_template(
    template_id: TemplateId,
    recipe: &RecipeSpec,
    geometry: &GeometryProposal,
    _inputs: &TemplateInputs,
) -> Result<TemplateDraft> {
    bind_recipe(recipe, std::slice::from_ref(geometry))?;
    match template_id {
        TemplateId::BassHandoff => bass(recipe, geometry),
        TemplateId::SpectralHandoff => spectral(recipe, geometry),
        _ => Err(Error::new(
            "UNKNOWN_TEMPLATE",
            "spectral emitter received an unsupported template",
        )),
    }
}

fn bass(recipe: &RecipeSpec, geometry: &GeometryProposal) -> Result<TemplateDraft> {
    let (crossover, early) = match recipe.id {
        "b140_2_early" | "b140_4_early" => (140_000, true),
        "b180_2_early" | "b180_4_early" => (180_000, true),
        "b220_2_early" | "b220_4_early" => (220_000, true),
        "b180_2_center" | "b180_4_center" => (180_000, false),
        _ => return Err(Error::new("UNKNOWN_RECIPE", "unknown bass-handoff recipe")),
    };
    let start = -geometry.requested_dry_frames;
    if -start < 1_764 {
        return Err(Error::new(
            "GEOMETRY_TOO_SHORT",
            "crossover wet ramps do not fit geometry",
        ));
    }
    let (transfer_start, transfer_end) = if early {
        (position(start, 1, 4)?, position(start, 1, 2)?)
    } else {
        (position(start, 2, 5)?, position(start, 3, 5)?)
    };
    let mut operations = time_map(geometry);
    operations.extend([
        Operation::CrossoverBandGain(two_band(
            Target::Outgoing,
            start,
            crossover,
            transfer_start,
            transfer_end,
        )),
        Operation::CrossoverBandGain(two_band(
            Target::Incoming,
            start,
            crossover,
            transfer_start,
            transfer_end,
        )),
    ]);
    operations.extend(equal_power(start));
    Ok(draft(
        TemplateId::BassHandoff,
        recipe,
        geometry,
        start,
        0,
        operations,
    ))
}

fn spectral(recipe: &RecipeSpec, geometry: &GeometryProposal) -> Result<TemplateDraft> {
    let start = -geometry.requested_dry_frames;
    let mut operations = if matches!(recipe.duration, RecipeDuration::Bars(_)) {
        time_map(geometry)
    } else {
        Vec::new()
    };
    match recipe.id {
        "lp_out_3s_500" | "lp_out_2bar_500" => operations.push(Operation::FilterEnvelope(filter(
            Target::Outgoing,
            start,
            FilterKind::LowpassBiquadV1,
            18_000_000,
            500_000,
            0,
            1_000_000,
        ))),
        "lp_out_5s_250" => operations.push(Operation::FilterEnvelope(filter(
            Target::Outgoing,
            start,
            FilterKind::LowpassBiquadV1,
            18_000_000,
            250_000,
            0,
            1_000_000,
        ))),
        "hp_in_3s_2000" => operations.push(Operation::FilterEnvelope(filter(
            Target::Incoming,
            start,
            FilterKind::HighpassBiquadV1,
            2_000_000,
            20_000,
            1_000_000,
            0,
        ))),
        "hp_in_5s_1200" => operations.push(Operation::FilterEnvelope(filter(
            Target::Incoming,
            start,
            FilterKind::HighpassBiquadV1,
            1_200_000,
            20_000,
            1_000_000,
            0,
        ))),
        "hp_in_2bar_1600" => operations.push(Operation::FilterEnvelope(filter(
            Target::Incoming,
            start,
            FilterKind::HighpassBiquadV1,
            1_600_000,
            20_000,
            1_000_000,
            0,
        ))),
        "bands_2bar_high_then_low" | "bands_4bar_low_then_high" => {
            if -start < 1_764 {
                return Err(Error::new(
                    "GEOMETRY_TOO_SHORT",
                    "crossover wet ramps do not fit geometry",
                ));
            }
            let high_first = recipe.id == "bands_2bar_high_then_low";
            operations.extend([
                Operation::CrossoverBandGain(three_band(Target::Outgoing, start, high_first)?),
                Operation::CrossoverBandGain(three_band(Target::Incoming, start, high_first)?),
            ]);
        }
        _ => {
            return Err(Error::new(
                "UNKNOWN_RECIPE",
                "unknown spectral-handoff recipe",
            ));
        }
    }
    operations.extend(equal_power(start));
    Ok(draft(
        TemplateId::SpectralHandoff,
        recipe,
        geometry,
        start,
        0,
        operations,
    ))
}

fn two_band(
    target: Target,
    start: i64,
    crossover_millihz: i64,
    transfer_start: i64,
    transfer_end: i64,
) -> CrossoverBandGain {
    let (before, after) = if target == Target::Outgoing {
        (1_000_000, 0)
    } else {
        (0, 1_000_000)
    };
    CrossoverBandGain {
        op_id: format!("{}.crossover", target_name(target)),
        target,
        profile: CrossoverProfile::LinkwitzRiley4V1,
        crossover_millihz: vec![crossover_millihz],
        bands: vec![Band::Low, Band::High],
        band_gain_envelopes: vec![
            BandGainEnvelope {
                band: Band::Low,
                points: points(&[
                    (start, before),
                    (transfer_start, before),
                    (transfer_end, after),
                    (0, after),
                ]),
                interpolations: vec![Interpolation::Linear; 3],
            },
            BandGainEnvelope {
                band: Band::High,
                points: points(&[(start, 1_000_000), (0, 1_000_000)]),
                interpolations: vec![Interpolation::Linear],
            },
        ],
        wet_points: wet_points(start),
        wet_interpolations: vec![Interpolation::Smoothstep; 3],
        control_interval_frames: 64,
    }
}

fn three_band(target: Target, start: i64, high_first: bool) -> Result<CrossoverBandGain> {
    let third = position(start, 1, 3)?;
    let two_thirds = position(start, 2, 3)?;
    let windows = if high_first {
        [(two_thirds, 0), (third, two_thirds), (start, third)]
    } else {
        [(start, third), (third, two_thirds), (two_thirds, 0)]
    };
    let bands = [Band::Low, Band::Mid, Band::High];
    let envelopes = bands
        .into_iter()
        .zip(windows)
        .map(|(band, (transfer_start, transfer_end))| BandGainEnvelope {
            band,
            points: transfer_points(target, start, transfer_start, transfer_end),
            interpolations: if transfer_start == start || transfer_end == 0 {
                vec![Interpolation::Linear; 2]
            } else {
                vec![Interpolation::Linear; 3]
            },
        })
        .collect();
    Ok(CrossoverBandGain {
        op_id: format!("{}.crossover", target_name(target)),
        target,
        profile: CrossoverProfile::LinkwitzRiley4V1,
        crossover_millihz: vec![180_000, 2_500_000],
        bands: bands.to_vec(),
        band_gain_envelopes: envelopes,
        wet_points: wet_points(start),
        wet_interpolations: vec![Interpolation::Smoothstep; 3],
        control_interval_frames: 64,
    })
}

fn transfer_points(
    target: Target,
    start: i64,
    transfer_start: i64,
    transfer_end: i64,
) -> Vec<crate::model::EnvelopePoint> {
    let (before, after) = if target == Target::Outgoing {
        (1_000_000, 0)
    } else {
        (0, 1_000_000)
    };
    let mut values = vec![(start, before)];
    if transfer_start != start {
        values.push((transfer_start, before));
    }
    values.push((transfer_end, after));
    if transfer_end != 0 {
        values.push((0, after));
    }
    points(&values)
}

fn wet_points(start: i64) -> Vec<crate::model::EnvelopePoint> {
    points(&[
        (start, 0),
        (start + 882, 1_000_000),
        (-882, 1_000_000),
        (0, 0),
    ])
}

fn target_name(target: Target) -> &'static str {
    match target {
        Target::Outgoing => "outgoing",
        Target::Incoming => "incoming",
    }
}
