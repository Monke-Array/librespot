use super::{RecipeId, TemplateDraft, TemplateId};
use crate::error::Result;
use crate::geometry::{GeometryProposal, validate_fallback_geometry};
use crate::model::{EnvelopePoint, GainEnvelope, Interpolation, Operation, Target};

pub fn emit_safe_fallback(geometry: &GeometryProposal) -> Result<TemplateDraft> {
    validate_fallback_geometry(geometry)?;
    Ok(TemplateDraft {
        template_id: TemplateId::SafeCrossfade,
        recipe_id: RecipeId::new("five_second_linear"),
        geometry_id: geometry.geometry_id.clone(),
        dry_start_frame: -220_500,
        effect_end_frame: 0,
        operations: vec![
            Operation::GainEnvelope(primary_gain(Target::Outgoing)),
            Operation::GainEnvelope(primary_gain(Target::Incoming)),
        ],
    })
}

fn primary_gain(target: Target) -> GainEnvelope {
    let (first, last, op_id) = match target {
        Target::Outgoing => (1_000_000, 0, "outgoing.primary_gain"),
        Target::Incoming => (0, 1_000_000, "incoming.primary_gain"),
    };
    GainEnvelope {
        op_id: op_id.into(),
        target,
        points: vec![
            EnvelopePoint {
                frame: -220_500,
                value_ppm: first,
            },
            EnvelopePoint {
                frame: 0,
                value_ppm: last,
            },
        ],
        interpolations: vec![Interpolation::Linear],
    }
}
