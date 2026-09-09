use super::gain::{draft, ensure_bounds, gain, time_map};
use super::{RecipeSpec, TemplateDraft, TemplateId, TemplateInputs, bind_recipe};
use crate::error::{Error, Result};
use crate::geometry::GeometryProposal;
use crate::model::{DelayTap, FeedforwardDelayTail, Interpolation, Operation, Target};
use crate::scalar::div_round_nearest_away;

pub fn emit_echo_tail_template(
    recipe: &RecipeSpec,
    geometry: &GeometryProposal,
    inputs: &TemplateInputs,
) -> Result<TemplateDraft> {
    bind_recipe(recipe, std::slice::from_ref(geometry))?;
    let two_beats = inputs.two_beats_frames.ok_or_else(|| {
        Error::new(
            "TEMPLATE_INAPPLICABLE",
            "echo tail requires a resolved beat duration",
        )
    })?;
    let beat = div_round_nearest_away(two_beats, 2)?;
    if beat <= 0 {
        return Err(Error::new(
            "INVALID_BEAT_DURATION",
            "echo tail beat duration must be positive",
        ));
    }
    let (capture_frames, fractions, gains): (i64, &[(i64, i64)], &[i64]) = match recipe.id {
        "quarter_3tap" => (
            div_round_nearest_away(beat, 2)?,
            &[(1, 4), (1, 2), (3, 4)],
            &[400_000, 240_000, 140_000],
        ),
        "half_3tap" => (
            beat,
            &[(1, 2), (1, 1), (3, 2)],
            &[400_000, 220_000, 120_000],
        ),
        "quarter_2tap" => (
            div_round_nearest_away(beat, 2)?,
            &[(1, 4), (1, 2)],
            &[360_000, 180_000],
        ),
        "half_2tap" => (beat, &[(1, 2), (1, 1)], &[360_000, 180_000]),
        _ => return Err(Error::new("UNKNOWN_RECIPE", "unknown echo-tail recipe")),
    };
    let mut taps = Vec::with_capacity(fractions.len());
    for ((numerator, denominator), gain_ppm) in fractions.iter().zip(gains) {
        let delay_frames = div_round_nearest_away(
            beat.checked_mul(*numerator)
                .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "echo delay overflow"))?,
            *denominator,
        )?;
        if !(882..=88_200).contains(&delay_frames) {
            return Err(Error::new(
                "DELAY_OUT_OF_RANGE",
                "resolved echo delay is outside v1 bounds",
            ));
        }
        taps.push(DelayTap {
            delay_frames,
            gain_ppm: *gain_ppm,
        });
    }
    let capture_start = capture_frames.checked_neg().ok_or_else(|| {
        Error::new(
            "INTEGER_OVERFLOW",
            "echo capture duration cannot be negated",
        )
    })?;
    let dry_start = capture_start.min(-882);
    ensure_bounds(geometry, dry_start)?;
    let effect_end = taps.last().unwrap().delay_frames;
    if effect_end > 264_600 {
        return Err(Error::new(
            "DELAY_OUT_OF_RANGE",
            "echo tail exceeds the v1 effect bound",
        ));
    }
    let tail = FeedforwardDelayTail {
        op_id: "outgoing.tail".into(),
        target: Target::Outgoing,
        capture_start_frame: capture_start,
        capture_end_frame: 0,
        taps,
    };
    let mut operations = time_map(geometry);
    operations.push(Operation::FeedforwardDelayTail(tail));
    operations.extend([
        Operation::GainEnvelope(gain(
            Target::Outgoing,
            &[(-882, 1_000_000), (0, 0)],
            &[Interpolation::Linear],
        )),
        Operation::GainEnvelope(gain(
            Target::Incoming,
            &[(-882, 0), (0, 1_000_000)],
            &[Interpolation::Linear],
        )),
    ]);
    Ok(draft(
        TemplateId::EchoTailHandoff,
        recipe,
        geometry,
        dry_start,
        effect_end,
        operations,
    ))
}
