use crate::error::{Error, Result};
use crate::model::{Interpolation, Operation, OperatorPlan};
use crate::scalar::div_round_nearest_away;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CurrentLoweringStatus {
    Lowerable,
    NotLowerable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurrentCompatibility {
    pub musical_core: CurrentLoweringStatus,
    pub full_plan: CurrentLoweringStatus,
}

pub fn classify_current_transition_plan(plan: &OperatorPlan) -> CurrentCompatibility {
    let musical_core = if plan.operations.iter().all(operation_is_core_lowerable) {
        CurrentLoweringStatus::Lowerable
    } else {
        CurrentLoweringStatus::NotLowerable
    };
    CurrentCompatibility {
        musical_core,
        full_plan: CurrentLoweringStatus::NotLowerable,
    }
}

fn operation_is_core_lowerable(operation: &Operation) -> bool {
    match operation {
        Operation::TimeMap(_) => true,
        Operation::GainEnvelope(value) => {
            value
                .interpolations
                .iter()
                .all(|value| matches!(value, Interpolation::Linear | Interpolation::Smoothstep))
                && value
                    .points
                    .iter()
                    .all(|point| frame_to_exact_nanoseconds(point.frame).is_ok())
        }
        _ => false,
    }
}

pub fn frame_to_exact_nanoseconds(frame: i64) -> Result<i64> {
    let numerator = frame.checked_mul(1_000_000_000).ok_or_else(|| {
        Error::new(
            "INTEGER_OVERFLOW",
            "frame-to-nanosecond conversion overflow",
        )
    })?;
    let nanoseconds = div_round_nearest_away(numerator, 44_100)?;
    let reverse_numerator = nanoseconds.checked_mul(44_100).ok_or_else(|| {
        Error::new(
            "INTEGER_OVERFLOW",
            "nanosecond-to-frame conversion overflow",
        )
    })?;
    if div_round_nearest_away(reverse_numerator, 1_000_000_000)? != frame {
        return Err(Error::new(
            "INEXACT_TIME_CONVERSION",
            "frame does not round-trip through nanoseconds",
        ));
    }
    Ok(nanoseconds)
}
