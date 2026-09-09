mod common;
mod safe;

pub use common::*;
pub use safe::emit_safe_fallback;

const SAFE: [RecipeSpec; 1] = [RecipeSpec {
    id: "five_second_linear",
    index: 0,
    duration: RecipeDuration::Seconds(220_500),
}];

const SHAPED: [RecipeSpec; 8] = [
    recipe("eq_3s", 0, RecipeDuration::Seconds(132_300)),
    recipe("eq_5s", 1, RecipeDuration::Seconds(220_500)),
    recipe("eq_2bar", 2, RecipeDuration::Bars(2)),
    recipe("eq_4bar", 3, RecipeDuration::Bars(4)),
    recipe("asym_early_3s", 4, RecipeDuration::Seconds(132_300)),
    recipe("asym_early_5s", 5, RecipeDuration::Seconds(220_500)),
    recipe("asym_early_2bar", 6, RecipeDuration::Bars(2)),
    recipe("asym_early_4bar", 7, RecipeDuration::Bars(4)),
];

const BEAT: [RecipeSpec; 4] = [
    recipe("hard_0ms", 0, RecipeDuration::Cut),
    recipe("soft_10ms", 1, RecipeDuration::Cut),
    recipe("soft_20ms", 2, RecipeDuration::Cut),
    recipe("soft_30ms", 3, RecipeDuration::Cut),
];

const BASS: [RecipeSpec; 8] = [
    recipe("b140_2_early", 0, RecipeDuration::Bars(2)),
    recipe("b180_2_early", 1, RecipeDuration::Bars(2)),
    recipe("b220_2_early", 2, RecipeDuration::Bars(2)),
    recipe("b180_2_center", 3, RecipeDuration::Bars(2)),
    recipe("b140_4_early", 4, RecipeDuration::Bars(4)),
    recipe("b180_4_early", 5, RecipeDuration::Bars(4)),
    recipe("b220_4_early", 6, RecipeDuration::Bars(4)),
    recipe("b180_4_center", 7, RecipeDuration::Bars(4)),
];

const SPECTRAL: [RecipeSpec; 8] = [
    recipe("lp_out_3s_500", 0, RecipeDuration::Seconds(132_300)),
    recipe("lp_out_5s_250", 1, RecipeDuration::Seconds(220_500)),
    recipe("lp_out_2bar_500", 2, RecipeDuration::Bars(2)),
    recipe("hp_in_3s_2000", 3, RecipeDuration::Seconds(132_300)),
    recipe("hp_in_5s_1200", 4, RecipeDuration::Seconds(220_500)),
    recipe("hp_in_2bar_1600", 5, RecipeDuration::Bars(2)),
    recipe("bands_2bar_high_then_low", 6, RecipeDuration::Bars(2)),
    recipe("bands_4bar_low_then_high", 7, RecipeDuration::Bars(4)),
];

const DUCKED: [RecipeSpec; 8] = [
    recipe("d6_fast_3s", 0, RecipeDuration::Seconds(132_300)),
    recipe("d9_fast_3s", 1, RecipeDuration::Seconds(132_300)),
    recipe("d6_smooth_5s", 2, RecipeDuration::Seconds(220_500)),
    recipe("d9_smooth_5s", 3, RecipeDuration::Seconds(220_500)),
    recipe("d6_fast_2bar", 4, RecipeDuration::Bars(2)),
    recipe("d9_fast_2bar", 5, RecipeDuration::Bars(2)),
    recipe("d6_smooth_4bar", 6, RecipeDuration::Bars(4)),
    recipe("d9_smooth_4bar", 7, RecipeDuration::Bars(4)),
];

const ECHO: [RecipeSpec; 4] = [
    recipe("quarter_3tap", 0, RecipeDuration::Cut),
    recipe("half_3tap", 1, RecipeDuration::Cut),
    recipe("quarter_2tap", 2, RecipeDuration::Cut),
    recipe("half_2tap", 3, RecipeDuration::Cut),
];

const ENERGY: [RecipeSpec; 6] = [
    recipe("gain_3s", 0, RecipeDuration::Seconds(132_300)),
    recipe("gain_5s", 1, RecipeDuration::Seconds(220_500)),
    recipe("gain_2bar", 2, RecipeDuration::Bars(2)),
    recipe("gain_4bar", 3, RecipeDuration::Bars(4)),
    recipe("filter_2bar", 4, RecipeDuration::Bars(2)),
    recipe("filter_4bar", 5, RecipeDuration::Bars(4)),
];

const RHYTHMIC: [RecipeSpec; 4] = [
    recipe("quarter_1bar_even", 0, RecipeDuration::Bars(1)),
    recipe("eighth_1bar_even", 1, RecipeDuration::Bars(1)),
    recipe("quarter_2bar_decay", 2, RecipeDuration::Bars(2)),
    recipe("eighth_1bar_decay", 3, RecipeDuration::Bars(1)),
];

const REGISTRY: [TemplateFamily; 9] = [
    family(TemplateId::SafeCrossfade, &SAFE),
    family(TemplateId::ShapedHandoff, &SHAPED),
    family(TemplateId::BeatCut, &BEAT),
    family(TemplateId::BassHandoff, &BASS),
    family(TemplateId::SpectralHandoff, &SPECTRAL),
    family(TemplateId::DuckedOverlap, &DUCKED),
    family(TemplateId::EchoTailHandoff, &ECHO),
    family(TemplateId::EnergyRamp, &ENERGY),
    family(TemplateId::RhythmicHandoff, &RHYTHMIC),
];

const fn recipe(id: &'static str, index: usize, duration: RecipeDuration) -> RecipeSpec {
    RecipeSpec {
        id,
        index,
        duration,
    }
}

const fn family(id: TemplateId, recipes: &'static [RecipeSpec]) -> TemplateFamily {
    TemplateFamily {
        id,
        quota: recipes.len(),
        recipes,
    }
}

pub fn template_registry() -> &'static [TemplateFamily] {
    &REGISTRY
}

pub fn rich_template_families(inputs: &TemplateInputs) -> Vec<FamilyApplicability> {
    REGISTRY[1..]
        .iter()
        .map(|family| family.applicability(inputs))
        .collect()
}
