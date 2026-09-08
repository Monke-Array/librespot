use crate::error::{Error, Result};
use crate::model::{Operation, OperatorPlanBody};

pub(super) fn validate_signature(body: &OperatorPlanBody) -> Result<()> {
    if body.template.version != 1 {
        return Err(Error::new(
            "UNSUPPORTED_TEMPLATE_VERSION",
            "template version must be one",
        ));
    }
    let gains = count(body, |op| matches!(op, Operation::GainEnvelope(_)));
    let time = count(body, |op| matches!(op, Operation::TimeMap(_)));
    let filters = count(body, |op| matches!(op, Operation::FilterEnvelope(_)));
    let crossovers = count(body, |op| matches!(op, Operation::CrossoverBandGain(_)));
    let ducks = count(body, |op| matches!(op, Operation::DuckEnvelope(_)));
    let tails = count(body, |op| matches!(op, Operation::FeedforwardDelayTail(_)));
    let gates = count(body, |op| matches!(op, Operation::RhythmicGate(_)));
    let exact = match body.template.id.as_str() {
        "safe_crossfade" => {
            gains == 2
                && time == 0
                && filters + crossovers + ducks + tails + gates == 0
                && safe_recipe(body)
        }
        "shaped_handoff" | "beat_cut" => {
            gains == 2 && time <= 1 && filters + crossovers + ducks + tails + gates == 0
        }
        "bass_handoff" => {
            gains == 2 && time <= 1 && filters == 0 && crossovers == 2 && ducks + tails + gates == 0
        }
        "spectral_handoff" => {
            gains == 2
                && time <= 1
                && ducks + tails + gates == 0
                && ((filters == 1 && crossovers == 0) || (filters == 0 && crossovers == 2))
        }
        "ducked_overlap" => {
            gains == 2 && time <= 1 && filters + crossovers + tails + gates == 0 && ducks == 1
        }
        "echo_tail_handoff" => {
            gains == 2 && time <= 1 && filters + crossovers + ducks + gates == 0 && tails == 1
        }
        "energy_ramp" => {
            gains == 2 && time <= 1 && crossovers + ducks + tails + gates == 0 && filters <= 1
        }
        "rhythmic_handoff" => {
            gains == 2 && time <= 1 && filters + crossovers + ducks + tails == 0 && gates == 1
        }
        _ => {
            return Err(Error::new(
                "UNKNOWN_TEMPLATE",
                "template is not in OperatorPlan/v1 vocabulary",
            ));
        }
    };
    if !exact {
        return Err(Error::new(
            "INVALID_TEMPLATE_SIGNATURE",
            "operation signature does not match template",
        ));
    }
    Ok(())
}

fn count(body: &OperatorPlanBody, predicate: impl Fn(&Operation) -> bool) -> usize {
    body.operations.iter().filter(|op| predicate(op)).count()
}

fn safe_recipe(body: &OperatorPlanBody) -> bool {
    body.template.recipe_id == "five_second_linear"
        && body.timeline.dry_start_frame == -220_500
        && body.timeline.effect_end_frame == 0
        && body.operations.iter().all(|operation| match operation {
            Operation::GainEnvelope(value) => {
                value.interpolations == [crate::model::Interpolation::Linear]
            }
            _ => false,
        })
}
