use crate::identity::{CandidateId, ValidatedPlan};
use crate::templates::TemplateId;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct GeneratedCandidate {
    pub candidate_id: CandidateId,
    pub template_id: TemplateId,
    pub recipe_id: String,
    pub plan: ValidatedPlan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuplicateRecord {
    pub duplicate_candidate_id: CandidateId,
    pub earlier_candidate_id: CandidateId,
}

pub fn deduplicate_validated_plans(
    candidates: Vec<GeneratedCandidate>,
) -> (Vec<GeneratedCandidate>, Vec<DuplicateRecord>) {
    let mut seen = BTreeMap::<String, CandidateId>::new();
    let mut retained = Vec::new();
    let mut duplicates = Vec::new();
    for candidate in candidates {
        let semantics = candidate.plan.audio_semantics_hash().hex();
        if let Some(earlier) = seen.get(&semantics) {
            duplicates.push(DuplicateRecord {
                duplicate_candidate_id: candidate.candidate_id.clone(),
                earlier_candidate_id: earlier.clone(),
            });
        } else {
            seen.insert(semantics, candidate.candidate_id.clone());
            retained.push(candidate);
        }
    }
    (retained, duplicates)
}
