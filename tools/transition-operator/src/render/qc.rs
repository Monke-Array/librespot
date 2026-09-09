use crate::candidate::GeneratedCandidate;
use crate::error::Result;
use crate::templates::TemplateId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairRenderStatus {
    Accepted,
    FallbackRenderFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderRejection {
    pub candidate_id: Option<String>,
    pub code: String,
    pub cause_code: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PairRenderResult<R> {
    pub status: PairRenderStatus,
    pub fallback: Option<R>,
    pub rich: Vec<(String, R)>,
    pub rejections: Vec<RenderRejection>,
}

pub fn render_candidates_with_fallback<R>(
    candidates: &[GeneratedCandidate],
    mut render: impl FnMut(&GeneratedCandidate) -> Result<R>,
) -> PairRenderResult<R> {
    let Some(fallback) = candidates
        .iter()
        .find(|candidate| candidate.template_id == TemplateId::SafeCrossfade)
    else {
        return PairRenderResult {
            status: PairRenderStatus::FallbackRenderFailed,
            fallback: None,
            rich: Vec::new(),
            rejections: vec![RenderRejection {
                candidate_id: None,
                code: "FALLBACK_RENDER_FAILED".to_owned(),
                cause_code: Some("MISSING_FALLBACK_CANDIDATE".to_owned()),
            }],
        };
    };

    let fallback_render = match render(fallback) {
        Ok(record) => record,
        Err(error) => {
            return PairRenderResult {
                status: PairRenderStatus::FallbackRenderFailed,
                fallback: None,
                rich: Vec::new(),
                rejections: vec![RenderRejection {
                    candidate_id: Some(fallback.candidate_id.as_str().to_owned()),
                    code: "FALLBACK_RENDER_FAILED".to_owned(),
                    cause_code: Some(error.code().to_owned()),
                }],
            };
        }
    };

    let mut rich = Vec::new();
    let mut rejections = Vec::new();
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate.template_id != TemplateId::SafeCrossfade)
    {
        match render(candidate) {
            Ok(record) => rich.push((candidate.candidate_id.as_str().to_owned(), record)),
            Err(error) => rejections.push(RenderRejection {
                candidate_id: Some(candidate.candidate_id.as_str().to_owned()),
                code: error.code().to_owned(),
                cause_code: None,
            }),
        }
    }
    PairRenderResult {
        status: PairRenderStatus::Accepted,
        fallback: Some(fallback_render),
        rich,
        rejections,
    }
}
