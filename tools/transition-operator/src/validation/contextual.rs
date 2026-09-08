use crate::error::{Error, Result};
use crate::model::{Operation, OperatorPlanBody, SourceRef};
use crate::scalar::div_round_nearest_away;
use crate::validation::source_equals;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectedSource {
    pub pcm_sha256: String,
    pub pcm_frame_count: i64,
}

impl ExpectedSource {
    pub fn from_ref(source: &SourceRef) -> Self {
        Self {
            pcm_sha256: source.pcm_sha256.clone(),
            pcm_frame_count: source.pcm_frame_count,
        }
    }
}

pub trait TemplateFeatureView {
    fn snapshot_sha256(&self) -> &str;
    fn covers_plan_window(&self, body: &OperatorPlanBody) -> bool;
    fn template_is_applicable(&self, body: &OperatorPlanBody) -> bool;
    fn recipe_matches(&self, body: &OperatorPlanBody) -> bool;
}

pub struct ValidationContext<'a> {
    pub outgoing: ExpectedSource,
    pub incoming: ExpectedSource,
    pub features: &'a dyn TemplateFeatureView,
}

pub(super) fn validate_timeline_and_context(
    body: &OperatorPlanBody,
    context: &ValidationContext<'_>,
) -> Result<()> {
    let timeline = &body.timeline;
    if !(-705_600..=-1).contains(&timeline.dry_start_frame)
        || timeline.dry_handoff_frame != 0
        || !(0..=264_600).contains(&timeline.effect_end_frame)
    {
        return Err(Error::new(
            "INVALID_TIMELINE",
            "timeline is outside v1 bounds",
        ));
    }
    if !source_equals(&context.outgoing, &body.sources.outgoing)
        || !source_equals(&context.incoming, &body.sources.incoming)
    {
        return Err(Error::new(
            "SOURCE_CONTEXT_MISMATCH",
            "source identity differs from validation context",
        ));
    }

    let incoming_rate = body
        .operations
        .iter()
        .find_map(|operation| match operation {
            Operation::TimeMap(value) => Some(value.source_rate_ppm),
            _ => None,
        })
        .unwrap_or(1_000_000);
    check_source_window(
        &body.sources.outgoing,
        timeline.dry_start_frame,
        0,
        1_000_000,
    )?;
    check_source_window(
        &body.sources.incoming,
        timeline.dry_start_frame,
        timeline.effect_end_frame,
        incoming_rate,
    )?;

    if context.features.snapshot_sha256() != body.feature_snapshot.sha256 {
        return Err(Error::new(
            "FEATURE_SNAPSHOT_MISMATCH",
            "feature snapshot hash differs from context",
        ));
    }
    if !context.features.covers_plan_window(body) {
        return Err(Error::new(
            "FEATURE_WINDOW_NOT_COVERED",
            "feature windows do not cover plan interval",
        ));
    }
    Ok(())
}

fn check_source_window(source: &SourceRef, start: i64, end: i64, rate: i64) -> Result<()> {
    let start_offset = div_round_nearest_away(
        start
            .checked_mul(rate)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "source mapping overflow"))?,
        1_000_000,
    )?;
    let end_offset = div_round_nearest_away(
        end.checked_mul(rate)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "source mapping overflow"))?,
        1_000_000,
    )?;
    let first = source
        .cue_source_frame
        .checked_add(start_offset)
        .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "source start overflow"))?;
    let last = source
        .cue_source_frame
        .checked_add(end_offset)
        .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "source end overflow"))?;
    if first < 0
        || last > source.pcm_frame_count
        || source.cue_source_frame < 0
        || source.cue_source_frame >= source.pcm_frame_count
    {
        return Err(Error::new(
            "SOURCE_WINDOW_OUT_OF_BOUNDS",
            "mapped source window exceeds canonical PCM",
        ));
    }
    Ok(())
}

pub(super) fn validate_template(
    body: &OperatorPlanBody,
    context: &ValidationContext<'_>,
) -> Result<()> {
    if !context.features.template_is_applicable(body) {
        return Err(Error::new(
            "TEMPLATE_INAPPLICABLE",
            "template predicates are false",
        ));
    }
    if !context.features.recipe_matches(body) {
        return Err(Error::new(
            "TEMPLATE_RECIPE_MISMATCH",
            "resolved plan differs from named recipe",
        ));
    }
    Ok(())
}
