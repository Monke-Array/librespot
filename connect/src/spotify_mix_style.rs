use librespot_protocol::automix_transition::{
    Curve, CurvePoint, CurveSet, EqCurveOverrides, FilterCurveOverrides, FxCurveOverrides, Overlap,
    Preset,
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SpotifyStyleIds {
    pub volume: i32,
    pub eq: i32,
    pub filter_fx: i32,
    pub fx: i32,
    pub jogwheel: i32,
    pub looping: i32,
}

const PRESET_STYLES: [SpotifyStyleIds; 23] = [
    SpotifyStyleIds {
        volume: 0,
        eq: 0,
        filter_fx: 0,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 6,
        eq: 4,
        filter_fx: 0,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 7,
        eq: 1,
        filter_fx: 0,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 7,
        eq: 10,
        filter_fx: 13,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 5,
        eq: 4,
        filter_fx: 7,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 9,
        eq: 4,
        filter_fx: 6,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 7,
        eq: 10,
        filter_fx: 13,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 5,
        eq: 7,
        filter_fx: 0,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 9,
        eq: 4,
        filter_fx: 6,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 10,
        eq: 4,
        filter_fx: 5,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 7,
        eq: 4,
        filter_fx: 7,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 1,
        eq: 0,
        filter_fx: 0,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 1,
        eq: 0,
        filter_fx: 0,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 1,
        eq: 0,
        filter_fx: 0,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 1,
        eq: 0,
        filter_fx: 0,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 1,
        eq: 0,
        filter_fx: 0,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 9,
        eq: 10,
        filter_fx: 0,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 7,
        eq: 4,
        filter_fx: 11,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 9,
        eq: 4,
        filter_fx: 10,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 7,
        eq: 4,
        filter_fx: 11,
        fx: 0,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 9,
        eq: 10,
        filter_fx: 14,
        fx: 5,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 9,
        eq: 10,
        filter_fx: 14,
        fx: 3,
        jogwheel: 0,
        looping: 0,
    },
    SpotifyStyleIds {
        volume: 6,
        eq: 4,
        filter_fx: 0,
        fx: 1,
        jogwheel: 0,
        looping: 0,
    },
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SpotifyRenderCapability {
    EqPhysicalMapping { style_id: i32 },
    FilterPhysicalMapping { style_id: i32 },
    FxPhysicalMapping { style_id: i32 },
    Jogwheel { style_id: i32 },
    Looping { style_id: i32 },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedSpotifyStyle {
    pub preset_id: i32,
    pub styles: SpotifyStyleIds,
    pub effective_num_bars: i32,
    pub effective_bpm_a: f32,
    pub effective_bpm_b: f32,
    pub outgoing_volume: CurveSet,
    pub incoming_volume: CurveSet,
    pub eq_out_overrides: Option<EqCurveOverrides>,
    pub eq_in_overrides: Option<EqCurveOverrides>,
    pub filter_out_overrides: Option<FilterCurveOverrides>,
    pub filter_in_overrides: Option<FilterCurveOverrides>,
    pub fx_out_overrides: Option<FxCurveOverrides>,
    pub fx_in_overrides: Option<FxCurveOverrides>,
    pub volume_override_ignored: bool,
    pub optional_curve_overrides_ignored: bool,
    pub unsupported: Vec<SpotifyRenderCapability>,
}

#[derive(Clone, Debug, Error, PartialEq)]
pub(crate) enum SpotifyStyleResolutionError {
    #[error("unknown Spotify preset {0}")]
    UnknownPreset(i32),
    #[error("unsupported Spotify volume style {0}")]
    UnsupportedVolumeStyle(i32),
}

pub(crate) fn resolve_spotify_style(
    preset: &Preset,
    overlap: &Overlap,
    use_curve_overrides: bool,
) -> Result<ResolvedSpotifyStyle, SpotifyStyleResolutionError> {
    let preset_id = preset.id();
    let mut styles = *PRESET_STYLES
        .get(usize::try_from(preset_id).unwrap_or(usize::MAX))
        .ok_or(SpotifyStyleResolutionError::UnknownPreset(preset_id))?;
    if let Some(style) = preset.volume_style_override.as_ref() {
        styles.volume = style.id();
    }
    if let Some(style) = preset.eq_style_override.as_ref() {
        styles.eq = style.id();
    }
    if let Some(style) = preset.filter_fx_style_override.as_ref() {
        styles.filter_fx = style.id();
    }
    if let Some(style) = preset.fx_style_override.as_ref() {
        styles.fx = style.id();
    }
    if let Some(style) = preset.jogwheel_style_override.as_ref() {
        styles.jogwheel = style.id();
    }
    if let Some(style) = preset.looping_style_override.as_ref() {
        styles.looping = style.id();
    }

    let effective_num_bars = overlap.duration_bars.unwrap_or_default().clamp(2, 32);
    let (mut outgoing_volume, mut incoming_volume) =
        resolve_volume_style(styles.volume, effective_num_bars)?;
    let has_volume_override =
        preset.volume_out_curve_override.is_some() || preset.volume_in_curve_override.is_some();
    let has_optional_curve_override = preset.eq_out_curve_overrides.is_some()
        || preset.eq_in_curve_overrides.is_some()
        || preset.filter_out_curve_overrides.is_some()
        || preset.filter_in_curve_overrides.is_some()
        || preset.fx_out_curve_overrides.is_some()
        || preset.fx_in_curve_overrides.is_some();
    if use_curve_overrides {
        if let Some(curves) = preset.volume_out_curve_override.as_ref() {
            outgoing_volume = curves.clone();
        }
        if let Some(curves) = preset.volume_in_curve_override.as_ref() {
            incoming_volume = curves.clone();
        }
    }

    let mut unsupported = Vec::new();
    if styles.eq != 0
        || (use_curve_overrides
            && (preset.eq_out_curve_overrides.is_some() || preset.eq_in_curve_overrides.is_some()))
    {
        unsupported.push(SpotifyRenderCapability::EqPhysicalMapping {
            style_id: styles.eq,
        });
    }
    if styles.filter_fx != 0
        || (use_curve_overrides
            && (preset.filter_out_curve_overrides.is_some()
                || preset.filter_in_curve_overrides.is_some()))
    {
        unsupported.push(SpotifyRenderCapability::FilterPhysicalMapping {
            style_id: styles.filter_fx,
        });
    }
    if styles.fx != 0
        || (use_curve_overrides
            && (preset.fx_out_curve_overrides.is_some() || preset.fx_in_curve_overrides.is_some()))
    {
        unsupported.push(SpotifyRenderCapability::FxPhysicalMapping {
            style_id: styles.fx,
        });
    }
    if styles.jogwheel != 0 {
        unsupported.push(SpotifyRenderCapability::Jogwheel {
            style_id: styles.jogwheel,
        });
    }
    if styles.looping != 0 {
        unsupported.push(SpotifyRenderCapability::Looping {
            style_id: styles.looping,
        });
    }

    let effective_bpm_a = effective_bpm(
        overlap.bpm_a,
        overlap.duration_ms(),
        effective_num_bars,
        overlap.item_speed_a,
    );
    let effective_bpm_b = effective_bpm(
        overlap.bpm_b,
        overlap.duration_ms(),
        effective_num_bars,
        overlap.item_speed_b,
    );
    Ok(ResolvedSpotifyStyle {
        preset_id,
        styles,
        effective_num_bars,
        effective_bpm_a,
        effective_bpm_b,
        outgoing_volume,
        incoming_volume,
        eq_out_overrides: use_curve_overrides
            .then(|| preset.eq_out_curve_overrides.as_ref().cloned())
            .flatten(),
        eq_in_overrides: use_curve_overrides
            .then(|| preset.eq_in_curve_overrides.as_ref().cloned())
            .flatten(),
        filter_out_overrides: use_curve_overrides
            .then(|| preset.filter_out_curve_overrides.as_ref().cloned())
            .flatten(),
        filter_in_overrides: use_curve_overrides
            .then(|| preset.filter_in_curve_overrides.as_ref().cloned())
            .flatten(),
        fx_out_overrides: use_curve_overrides
            .then(|| preset.fx_out_curve_overrides.as_ref().cloned())
            .flatten(),
        fx_in_overrides: use_curve_overrides
            .then(|| preset.fx_in_curve_overrides.as_ref().cloned())
            .flatten(),
        volume_override_ignored: has_volume_override && !use_curve_overrides,
        optional_curve_overrides_ignored: has_optional_curve_override && !use_curve_overrides,
        unsupported,
    })
}

fn effective_bpm(
    supplied_bpm: Option<f32>,
    outgoing_duration_ms: i32,
    bars: i32,
    item_speed: Option<f64>,
) -> f32 {
    let base = supplied_bpm
        .filter(|bpm| bpm.is_finite() && *bpm > 0.0)
        .unwrap_or_else(|| {
            if outgoing_duration_ms > 0 {
                bars as f32 * 240_000.0 / outgoing_duration_ms as f32
            } else {
                120.0
            }
        });
    let speed = item_speed
        .filter(|speed| speed.is_finite() && *speed > 0.0)
        .unwrap_or(1.0) as f32;
    base * speed
}

fn resolve_volume_style(
    style_id: i32,
    bars: i32,
) -> Result<(CurveSet, CurveSet), SpotifyStyleResolutionError> {
    let cut_width = 1.0 / (16.0 * f64::from(bars));
    let edge_width = 1.0 / (256.0 * f64::from(bars));
    let pair = match style_id {
        0 | 6 => (cross_shape(1.0, 0.0, 0.6), cross_shape(0.0, 1.0, 0.4)),
        1 => (cut_out(cut_width), cut_in(cut_width)),
        2 => (single_line(1.0, 0.0), single_line(0.0, 1.0)),
        3 => (cut_out(cut_width), slow_in()),
        4 => (slow_out(0.5), cut_in(cut_width)),
        5 => (slow_out(0.5), slow_in()),
        7 => (edge_out(edge_width), edge_in(edge_width)),
        8 => return Err(SpotifyStyleResolutionError::UnsupportedVolumeStyle(8)),
        9 => (edge_out(edge_width), slow_in()),
        10 => (slow_out(0.5), edge_in(edge_width)),
        11 => (slow_out(1.0 - 1.0 / f64::from(bars)), slow_in()),
        _ => {
            return Err(SpotifyStyleResolutionError::UnsupportedVolumeStyle(
                style_id,
            ));
        }
    };
    Ok(pair)
}

fn point(x: f64, y: f64) -> CurvePoint {
    CurvePoint {
        x: Some(x),
        y: Some(y),
        ..Default::default()
    }
}

fn segment(start: f64, end: f64, from: f64, to: f64) -> Curve {
    Curve {
        points: vec![point(0.0, from), point(1.0, to)],
        start: Some(start),
        end: Some(end),
        ..Default::default()
    }
}

fn cubic_segment(start: f64, end: f64, from: f64, to: f64, edge: f64) -> Curve {
    Curve {
        points: vec![
            point(0.0, from),
            point(edge, from),
            point(edge, to),
            point(1.0, to),
        ],
        start: Some(start),
        end: Some(end),
        ..Default::default()
    }
}

fn curves(curves: Vec<Curve>) -> CurveSet {
    CurveSet {
        curves,
        minimum: Some(0.0),
        maximum: Some(0.0),
        ..Default::default()
    }
}

fn single_line(from: f64, to: f64) -> CurveSet {
    curves(vec![segment(0.0, 1.0, from, to)])
}

fn cross_shape(from: f64, to: f64, edge: f64) -> CurveSet {
    curves(vec![cubic_segment(0.0, 1.0, from, to, edge)])
}

fn cut_out(width: f64) -> CurveSet {
    curves(vec![
        segment(0.0, 0.5, 1.0, 1.0),
        segment(0.5, 0.5 + width, 1.0, 0.0),
        segment(0.5 + width, 1.0, 0.0, 0.0),
    ])
}

fn cut_in(width: f64) -> CurveSet {
    curves(vec![
        segment(0.0, 0.5 - width, 0.0, 0.0),
        segment(0.5 - width, 0.5, 0.0, 1.0),
        segment(0.5, 1.0, 1.0, 1.0),
    ])
}

fn slow_out(start: f64) -> CurveSet {
    curves(vec![
        segment(0.0, start, 1.0, 1.0),
        segment(start, 1.0, 1.0, 0.0),
    ])
}

fn slow_in() -> CurveSet {
    curves(vec![
        segment(0.0, 0.5, 0.0, 1.0),
        segment(0.5, 1.0, 1.0, 1.0),
    ])
}

fn edge_out(width: f64) -> CurveSet {
    curves(vec![
        segment(0.0, 1.0 - width, 1.0, 1.0),
        cubic_segment(1.0 - width, 1.0, 1.0, 0.0, 0.5),
    ])
}

fn edge_in(width: f64) -> CurveSet {
    curves(vec![
        cubic_segment(0.0, width, 0.0, 1.0, 0.5),
        segment(width, 1.0, 1.0, 1.0),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use librespot_protocol::automix_transition::{
        Curve, CurvePoint, CurveSet, EqStyle, JogwheelStyle, LoopingStyle, Overlap, Preset,
        VolumeStyle,
    };
    use protobuf::MessageField;
    use serde_json::Value;

    fn overlap(bars: i32) -> Overlap {
        Overlap {
            duration_ms: Some(8_000),
            duration_bars: Some(bars),
            item_speed_a: Some(1.0),
            item_speed_b: Some(1.0),
            ..Default::default()
        }
    }

    fn preset(id: i32) -> Preset {
        Preset {
            id: Some(id),
            ..Default::default()
        }
    }

    fn line(from: f64, to: f64) -> CurveSet {
        CurveSet {
            curves: vec![Curve {
                points: vec![
                    CurvePoint {
                        x: Some(0.0),
                        y: Some(from),
                        ..Default::default()
                    },
                    CurvePoint {
                        x: Some(1.0),
                        y: Some(to),
                        ..Default::default()
                    },
                ],
                start: Some(0.0),
                end: Some(1.0),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn preset_two_expands_to_documented_style_ids() {
        let resolved = resolve_spotify_style(&preset(2), &overlap(4), false).unwrap();
        assert_eq!(
            resolved.styles,
            SpotifyStyleIds {
                volume: 7,
                eq: 1,
                filter_fx: 0,
                fx: 0,
                jogwheel: 0,
                looping: 0,
            }
        );
        assert_eq!(
            resolved.unsupported,
            [SpotifyRenderCapability::EqPhysicalMapping { style_id: 1 }]
        );
    }

    #[test]
    fn preset_eleven_is_a_renderable_volume_only_cut() {
        let resolved = resolve_spotify_style(&preset(11), &overlap(4), false).unwrap();
        assert_eq!(resolved.styles.volume, 1);
        assert!(resolved.unsupported.is_empty());
        assert_eq!(resolved.outgoing_volume.curves[1].start(), 0.5);
        assert_eq!(resolved.outgoing_volume.curves[1].end(), 0.515625);
        assert_eq!(resolved.incoming_volume.curves[1].start(), 0.484375);
        assert_eq!(resolved.incoming_volume.curves[1].end(), 0.5);
    }

    #[test]
    fn bars_are_clamped_and_three_bar_cut_geometry_is_exact() {
        let low = resolve_spotify_style(&preset(11), &overlap(1), false).unwrap();
        assert_eq!(low.effective_num_bars, 2);
        assert_eq!(low.outgoing_volume.curves[1].end(), 0.53125);

        let three = resolve_spotify_style(&preset(11), &overlap(3), false).unwrap();
        assert_eq!(three.effective_num_bars, 3);
        assert_eq!(three.outgoing_volume.curves[1].end(), 0.5208333333333334);

        let high = resolve_spotify_style(&preset(11), &overlap(99), false).unwrap();
        assert_eq!(high.effective_num_bars, 32);
        assert_eq!(high.outgoing_volume.curves[1].end(), 0.501953125);
    }

    #[test]
    fn supplied_derived_and_fallback_bpm_include_item_speed_once() {
        let mut supplied = overlap(4);
        supplied.bpm_a = Some(100.0);
        supplied.bpm_b = Some(110.0);
        supplied.item_speed_a = Some(1.1);
        supplied.item_speed_b = Some(0.9);
        let resolved = resolve_spotify_style(&preset(11), &supplied, false).unwrap();
        assert!((resolved.effective_bpm_a - 110.0).abs() < 1e-5);
        assert!((resolved.effective_bpm_b - 99.0).abs() < 1e-5);

        let mut derived = overlap(4);
        derived.duration_ms = Some(16_000);
        derived.item_speed_b = Some(1.25);
        let resolved = resolve_spotify_style(&preset(11), &derived, false).unwrap();
        assert_eq!(resolved.effective_bpm_a, 60.0);
        assert_eq!(resolved.effective_bpm_b, 75.0);

        let mut fallback = overlap(4);
        fallback.duration_ms = Some(0);
        let resolved = resolve_spotify_style(&preset(11), &fallback, false).unwrap();
        assert_eq!(resolved.effective_bpm_a, 120.0);
        assert_eq!(resolved.effective_bpm_b, 120.0);
    }

    #[test]
    fn explicit_style_zero_overrides_preset_mapping() {
        let mut value = preset(2);
        value.volume_style_override = MessageField::some(VolumeStyle {
            id: Some(0),
            ..Default::default()
        });
        value.eq_style_override = MessageField::some(EqStyle {
            id: Some(0),
            ..Default::default()
        });
        let resolved = resolve_spotify_style(&value, &overlap(4), false).unwrap();
        assert_eq!(resolved.styles.volume, 0);
        assert_eq!(resolved.styles.eq, 0);
        assert!(resolved.unsupported.is_empty());
    }

    #[test]
    fn enabled_volume_curve_overrides_replace_style_curves_with_presence() {
        let mut value = preset(11);
        value.volume_out_curve_override = MessageField::some(line(0.8, 0.1));
        value.volume_in_curve_override = MessageField::some(line(0.2, 0.9));

        let disabled = resolve_spotify_style(&value, &overlap(4), false).unwrap();
        assert_eq!(disabled.outgoing_volume.curves.len(), 3);
        assert!(disabled.volume_override_ignored);

        let enabled = resolve_spotify_style(&value, &overlap(4), true).unwrap();
        assert_eq!(enabled.outgoing_volume, line(0.8, 0.1));
        assert_eq!(enabled.incoming_volume, line(0.2, 0.9));
        assert!(!enabled.volume_override_ignored);
    }

    #[test]
    fn disabled_optional_curve_overrides_remain_explicitly_unresolved() {
        let mut value = preset(11);
        value.eq_out_curve_overrides = MessageField::some(EqCurveOverrides::default());

        let disabled = resolve_spotify_style(&value, &overlap(4), false).unwrap();
        assert!(disabled.optional_curve_overrides_ignored);

        let enabled = resolve_spotify_style(&value, &overlap(4), true).unwrap();
        assert!(!enabled.optional_curve_overrides_ignored);
        assert!(matches!(
            enabled.unsupported.as_slice(),
            [SpotifyRenderCapability::EqPhysicalMapping { style_id: 0 }]
        ));
    }

    #[test]
    fn unknown_preset_and_nonzero_block_styles_fail_explicitly() {
        assert_eq!(
            resolve_spotify_style(&preset(999), &overlap(4), false),
            Err(SpotifyStyleResolutionError::UnknownPreset(999))
        );

        let mut value = preset(11);
        value.jogwheel_style_override = MessageField::some(JogwheelStyle {
            id: Some(3),
            ..Default::default()
        });
        value.looping_style_override = MessageField::some(LoopingStyle {
            id: Some(5),
            ..Default::default()
        });
        let resolved = resolve_spotify_style(&value, &overlap(4), false).unwrap();
        assert_eq!(
            resolved.unsupported,
            [
                SpotifyRenderCapability::Jogwheel { style_id: 3 },
                SpotifyRenderCapability::Looping { style_id: 5 },
            ]
        );
    }

    fn style_fixture() -> Value {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../tools/runtime-diagnostics/evidence/2026-09-19-style-responses.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn fixture_number(value: &Value, key: &str) -> f64 {
        value[key]
            .as_f64()
            .unwrap_or_else(|| panic!("missing numeric {key} in {value}"))
    }

    fn assert_curve_set_matches_json(actual: &CurveSet, expected: &Value) {
        let expected_curves = expected["curves"].as_array().unwrap();
        assert_eq!(actual.curves.len(), expected_curves.len());
        for (actual, expected) in actual.curves.iter().zip(expected_curves) {
            assert_eq!(
                actual.start().to_bits(),
                fixture_number(expected, "start").to_bits()
            );
            assert_eq!(
                actual.end().to_bits(),
                fixture_number(expected, "end").to_bits()
            );
            let points = expected["points"].as_array().unwrap();
            assert_eq!(actual.points.len(), points.len());
            for (actual, expected) in actual.points.iter().zip(points) {
                assert_eq!(
                    actual.x().to_bits(),
                    fixture_number(expected, "x").to_bits()
                );
                assert_eq!(
                    actual.y().to_bits(),
                    fixture_number(expected, "y").to_bits()
                );
            }
        }
    }

    #[test]
    fn preset_table_matches_all_sanitized_numeric_responses() {
        let fixture = style_fixture();
        for record in fixture["records"].as_array().unwrap() {
            if record["method"] != "getStylesForPresetId" {
                continue;
            }
            for row in record["response"]["presetStyles"].as_array().unwrap() {
                let id = row["presetId"].as_i64().unwrap();
                if !(0..=22).contains(&id) {
                    continue;
                }
                let actual = PRESET_STYLES[id as usize];
                assert_eq!(
                    actual.volume,
                    row["volumeStyle"]["id"].as_i64().unwrap() as i32
                );
                assert_eq!(actual.eq, row["eqStyle"]["id"].as_i64().unwrap() as i32);
                assert_eq!(
                    actual.filter_fx,
                    row["filterFxStyle"]["id"].as_i64().unwrap() as i32
                );
                assert_eq!(actual.fx, row["fxStyle"]["id"].as_i64().unwrap() as i32);
                assert_eq!(
                    actual.jogwheel,
                    row["jogwheelStyle"]["id"].as_i64().unwrap() as i32
                );
                assert_eq!(
                    actual.looping,
                    row["loopingStyle"]["id"].as_i64().unwrap() as i32
                );
            }
        }
    }

    #[test]
    fn supported_volume_curves_match_sanitized_numeric_responses() {
        let fixture = style_fixture();
        for record in fixture["records"].as_array().unwrap() {
            if record["method"] != "getVolumeStylesForVolumeStyleId" {
                continue;
            }
            let Some(bars) = record["request"]["numBars"].as_i64() else {
                continue;
            };
            if !(2..=32).contains(&bars) {
                continue;
            }
            for row in record["response"]["volumeStyles"].as_array().unwrap() {
                let style_id = row["style"]["id"].as_i64().unwrap() as i32;
                if style_id == 8 || !(0..=11).contains(&style_id) {
                    continue;
                }
                let (outgoing, incoming) = resolve_volume_style(style_id, bars as i32).unwrap();
                assert_curve_set_matches_json(&outgoing, &row["fadeOutCurves"]);
                assert_curve_set_matches_json(&incoming, &row["fadeInCurves"]);
            }
        }
    }
}
