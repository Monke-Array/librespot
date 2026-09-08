use crate::canonical::canonical_json;
use crate::capability::{CapabilityRequirements, derive_capabilities};
use crate::error::{Error, Result};
use crate::model::*;
use crate::validation::{ValidatedPlanBody, ValidationReport};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt;

#[derive(Clone, Eq, PartialEq)]
pub struct PlanHash([u8; 32]);

#[derive(Clone, Eq, PartialEq)]
pub struct AudioSemanticsHash([u8; 32]);

macro_rules! hash_accessors {
    ($type:ty) => {
        impl $type {
            pub fn hex(&self) -> String {
                hex(&self.0)
            }
            pub fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }
        impl fmt::Debug for $type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.hex())
            }
        }
    };
}
hash_accessors!(PlanHash);
hash_accessors!(AudioSemanticsHash);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateId(String);

impl CandidateId {
    pub fn from_plan_hash(hash: &PlanHash) -> Self {
        candidate_id(hash)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct ValidatedPlan {
    plan: OperatorPlan,
    canonical_bytes: Vec<u8>,
    plan_hash: PlanHash,
    audio_semantics_hash: AudioSemanticsHash,
    capabilities: CapabilityRequirements,
    report: ValidationReport,
}

impl ValidatedPlan {
    pub fn plan(&self) -> &OperatorPlan {
        &self.plan
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }
    pub fn plan_hash(&self) -> &PlanHash {
        &self.plan_hash
    }
    pub fn audio_semantics_hash(&self) -> &AudioSemanticsHash {
        &self.audio_semantics_hash
    }
    pub fn capabilities(&self) -> &CapabilityRequirements {
        &self.capabilities
    }
    pub fn validation_report(&self) -> &ValidationReport {
        &self.report
    }
}

pub fn finalize_plan(validated: ValidatedPlanBody) -> Result<ValidatedPlan> {
    let (body, report) = validated.into_parts();
    let plan_hash = PlanHash(hash(&canonical_json(&body)?));
    let audio_semantics_hash = audio_semantics_hash(&body)?;
    let plan = OperatorPlan {
        schema_version: body.schema_version,
        plan_id: format!("op1-{}", plan_hash.hex()),
        template: body.template,
        format: body.format,
        sources: body.sources,
        timeline: body.timeline,
        operations: body.operations,
        output_safety: body.output_safety,
        feature_snapshot: body.feature_snapshot,
        provenance: body.provenance,
    };
    let canonical_bytes = canonical_json(&plan)?;
    let capabilities = derive_capabilities(&plan);
    Ok(ValidatedPlan {
        plan,
        canonical_bytes,
        plan_hash,
        audio_semantics_hash,
        capabilities,
        report,
    })
}

pub fn candidate_id(plan_hash: &PlanHash) -> CandidateId {
    let mut digest = Sha256::new();
    digest.update(b"transition-candidate/1\0");
    digest.update(plan_hash.as_bytes());
    CandidateId(format!("cand1-{}", hex(&digest.finalize())))
}

#[derive(Serialize)]
struct AudioSemanticsProjection<'a> {
    schema_version: &'a str,
    format: &'a AudioFormat,
    sources: &'a Sources,
    timeline: &'a Timeline,
    operations: &'a [Operation],
    output_safety: &'a OutputSafety,
}

fn audio_semantics_hash(body: &OperatorPlanBody) -> Result<AudioSemanticsHash> {
    let projection = AudioSemanticsProjection {
        schema_version: &body.schema_version,
        format: &body.format,
        sources: &body.sources,
        timeline: &body.timeline,
        operations: &body.operations,
        output_safety: &body.output_safety,
    };
    Ok(AudioSemanticsHash(hash(&canonical_json(&projection)?)))
}

fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    result
}

pub(crate) fn verify_plan_id(plan: &OperatorPlan, expected: &PlanHash) -> Result<()> {
    if plan.plan_id != format!("op1-{}", expected.hex()) {
        return Err(Error::new(
            "PLAN_ID_MISMATCH",
            "plan ID does not match canonical plan projection",
        ));
    }
    Ok(())
}
