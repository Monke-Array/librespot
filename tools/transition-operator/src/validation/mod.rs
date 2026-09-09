mod contextual;
mod structural;
mod template;

use crate::canonical::canonical_json;
use crate::error::{Error, Result};
use crate::model::{OperatorPlan, OperatorPlanBody, SourceRef};
use crate::{ValidatedPlan, finalize_plan, require_canonical_json, verify_plan_id};

pub use contextual::{ExpectedSource, TemplateFeatureView, ValidationContext};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationReport {
    stages_completed: u8,
}

impl ValidationReport {
    pub fn stages_completed(&self) -> u8 {
        self.stages_completed
    }

    pub(crate) fn advanced_to(mut self, stages_completed: u8) -> Self {
        self.stages_completed = stages_completed;
        self
    }
}

#[derive(Clone, Debug)]
pub struct ValidatedPlanBody {
    body: OperatorPlanBody,
    report: ValidationReport,
}

impl ValidatedPlanBody {
    pub fn body(&self) -> &OperatorPlanBody {
        &self.body
    }
    pub fn report(&self) -> &ValidationReport {
        &self.report
    }
    pub(crate) fn into_parts(self) -> (OperatorPlanBody, ValidationReport) {
        (self.body, self.report)
    }
}

#[derive(Clone, Debug)]
pub struct StructurallyValidatedPlan {
    plan: OperatorPlan,
    report: ValidationReport,
}

impl StructurallyValidatedPlan {
    pub fn plan(&self) -> &OperatorPlan {
        &self.plan
    }
    pub fn report(&self) -> &ValidationReport {
        &self.report
    }
    pub(crate) fn into_parts(self) -> (OperatorPlan, ValidationReport) {
        (self.plan, self.report)
    }
}

pub struct PlanValidator;

impl PlanValidator {
    pub fn validate_body(
        body: OperatorPlanBody,
        context: &ValidationContext<'_>,
    ) -> Result<ValidatedPlanBody> {
        if canonical_json(&body)?.len() > 65_536 {
            return Err(Error::new(
                "PLAN_TOO_LARGE",
                "canonical plan exceeds 65536 bytes",
            ));
        }
        structural::validate_schema_and_scalars(&body)?;
        contextual::validate_timeline_and_context(&body, context)?;
        structural::validate_operations(&body)?;
        template::validate_signature(&body)?;
        contextual::validate_template(&body, context)?;
        Ok(ValidatedPlanBody {
            body,
            report: ValidationReport {
                stages_completed: 8,
            },
        })
    }

    pub fn validate_structure(
        plan: OperatorPlan,
        context: &ValidationContext<'_>,
    ) -> Result<StructurallyValidatedPlan> {
        if canonical_json(&plan)?.len() > 65_536 {
            return Err(Error::new(
                "PLAN_TOO_LARGE",
                "canonical plan exceeds 65536 bytes",
            ));
        }
        let validated = Self::validate_body(plan.body(), context)?;
        let (_, report) = validated.into_parts();
        Ok(StructurallyValidatedPlan { plan, report })
    }

    pub fn validate_bytes(bytes: &[u8], context: &ValidationContext<'_>) -> Result<ValidatedPlan> {
        if bytes.len() > 65_536 {
            return Err(Error::new(
                "PLAN_TOO_LARGE",
                "canonical plan exceeds 65536 bytes",
            ));
        }
        let plan: OperatorPlan = require_canonical_json(bytes)?;
        let structurally_validated = Self::validate_structure(plan, context)?;
        let (plan, _) = structurally_validated.into_parts();
        let validated_body = Self::validate_body(plan.body(), context)?;
        let finalized = finalize_plan(validated_body)?;
        verify_plan_id(&plan, finalized.plan_hash())?;
        if finalized.canonical_bytes() != bytes {
            return Err(Error::new(
                "PLAN_ID_MISMATCH",
                "accepted bytes differ after identity finalization",
            ));
        }
        Ok(finalized)
    }
}

pub(crate) fn source_equals(expected: &ExpectedSource, actual: &SourceRef) -> bool {
    expected.pcm_sha256 == actual.pcm_sha256 && expected.pcm_frame_count == actual.pcm_frame_count
}
