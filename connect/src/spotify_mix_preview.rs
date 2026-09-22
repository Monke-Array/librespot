use data_encoding::BASE64;
use librespot_core::{SpotifyUri, dealer::protocol::request::SignalCommand};
use librespot_protocol::{automix_preview::AutomixPreview, spotify_auto_mix_metadata::Cuepoints};
use protobuf::Message;
use sha1::{Digest, Sha1};

use librespot_playback::TransitionPlan;

use crate::{
    spotify_mix::{
        SpotifyRecipeMaterialization, SpotifyRecipePlanError, SpotifyTransitionError,
        SpotifyTransitionRecipe, item_speeds_match, materialize_spotify_recipe,
    },
    spotify_mix_style::{ResolvedSpotifyStyle, SpotifyStyleResolutionError},
};

pub(crate) const PREVIEW_WINDOW_MS: u32 = 3_000;
const MAX_PREVIEW_PARAMETERS_LEN: usize = 16 * 1024;

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum AutomixPreviewError {
    WrongSignalId,
    MissingParameters,
    EmptyParameters,
    OversizedParameters,
    InvalidBase64,
    InvalidProtobuf,
    MissingCanonicalA,
    MissingCanonicalB,
    MissingPlayableA,
    MissingPlayableB,
    InvalidTrackUri(&'static str),
    MissingMode,
    UnsupportedMode(String),
    MissingStartPosition,
    MissingRelativeFlag,
    AbsoluteStartUnsupported,
    UnsupportedWindow(i64),
    ExplicitStopUnsupported,
    MissingItemSpeed(&'static str),
    InvalidItemSpeed(&'static str),
    MissingRecipe,
    MissingRecipeOverlap,
    MissingRecipeTiming,
    InvalidRecipeDuration,
    InvalidRecipeAnalysis,
    CanonicalAMismatch,
    CanonicalBMismatch,
    PlayableAMismatch,
    PlayableBMismatch,
    ItemSpeedAMismatch,
    ItemSpeedBMismatch,
    OutgoingPrerollUnderflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PreviewFieldPresence {
    pub transition_uri: bool,
    pub cuepoints: bool,
    pub stop_position_ms: bool,
    pub context_uri: bool,
    pub arm_id: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreviewFingerprint([u8; 20]);

#[derive(Clone, Debug)]
pub(crate) struct AutomixPreviewRequest {
    pub automix_mode: String,
    pub canonical_a: SpotifyUri,
    pub canonical_b: SpotifyUri,
    pub playable_a: SpotifyUri,
    pub playable_b: SpotifyUri,
    pub context_uri: Option<String>,
    pub transition_uri: Option<String>,
    pub arm_id: Option<String>,
    pub cuepoints: Option<Cuepoints>,
    pub fields: PreviewFieldPresence,
    pub relative_start_position: bool,
    pub window_ms: u32,
    pub outgoing_load_position_ms: u32,
    pub incoming_load_position_ms: u32,
    pub item_speed_a_bits: u64,
    pub item_speed_b_bits: u64,
    pub recipe: SpotifyTransitionRecipe,
    pub fingerprint: PreviewFingerprint,
}

#[derive(Clone, Debug)]
pub(crate) enum PreviewResolution {
    NoTransition { preset_id: i32 },
    Playable(ResolvedAutomixPreview),
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedAutomixPreview {
    pub request: AutomixPreviewRequest,
    pub preset_id: i32,
    pub style: Box<ResolvedSpotifyStyle>,
    pub plan: TransitionPlan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AutomixPreviewResolutionError {
    MissingPreset,
    UnknownPreset(i32),
    UnsupportedRenderer { preset_id: i32 },
}

pub(crate) fn resolve_automix_preview(
    request: AutomixPreviewRequest,
) -> Result<PreviewResolution, AutomixPreviewResolutionError> {
    let preset_id = request.recipe.preset().map(|preset| preset.id());
    match materialize_spotify_recipe(&request.recipe) {
        Ok(SpotifyRecipeMaterialization::None { preset_id }) => {
            Ok(PreviewResolution::NoTransition { preset_id })
        }
        Ok(SpotifyRecipeMaterialization::Playable {
            preset_id,
            style,
            plan,
        }) => Ok(PreviewResolution::Playable(ResolvedAutomixPreview {
            request,
            preset_id,
            style,
            plan,
        })),
        Err(SpotifyRecipePlanError::MissingPreset) => {
            Err(AutomixPreviewResolutionError::MissingPreset)
        }
        Err(SpotifyRecipePlanError::Style(SpotifyStyleResolutionError::UnknownPreset(id))) => {
            Err(AutomixPreviewResolutionError::UnknownPreset(id))
        }
        Err(SpotifyRecipePlanError::Style(
            SpotifyStyleResolutionError::UnsupportedVolumeStyle(_),
        ))
        | Err(SpotifyRecipePlanError::UnsupportedRenderer) => {
            Err(AutomixPreviewResolutionError::UnsupportedRenderer {
                preset_id: preset_id.unwrap_or_default(),
            })
        }
    }
}

pub(crate) fn decode_automix_preview(
    command: &SignalCommand,
) -> Result<AutomixPreviewRequest, AutomixPreviewError> {
    if command.signal_id != "automix-preview" {
        return Err(AutomixPreviewError::WrongSignalId);
    }
    let encoded = command
        .parameters
        .as_deref()
        .ok_or(AutomixPreviewError::MissingParameters)?;
    if encoded.is_empty() {
        return Err(AutomixPreviewError::EmptyParameters);
    }
    if encoded.len() > MAX_PREVIEW_PARAMETERS_LEN {
        return Err(AutomixPreviewError::OversizedParameters);
    }
    let bytes = BASE64
        .decode(encoded.as_bytes())
        .map_err(|_| AutomixPreviewError::InvalidBase64)?;
    let wire = AutomixPreview::parse_from_bytes(&bytes)
        .map_err(|_| AutomixPreviewError::InvalidProtobuf)?;
    validate_preview(wire)
}

fn validate_preview(
    mut wire: AutomixPreview,
) -> Result<AutomixPreviewRequest, AutomixPreviewError> {
    let canonical_a_text = wire
        .track_uri_1
        .as_deref()
        .ok_or(AutomixPreviewError::MissingCanonicalA)?;
    let canonical_b_text = wire
        .track_uri_2
        .as_deref()
        .ok_or(AutomixPreviewError::MissingCanonicalB)?;
    let playable_a_text = wire
        .playable_track_uri_1
        .as_deref()
        .ok_or(AutomixPreviewError::MissingPlayableA)?;
    let playable_b_text = wire
        .playable_track_uri_2
        .as_deref()
        .ok_or(AutomixPreviewError::MissingPlayableB)?;

    let canonical_a = parse_track_uri(canonical_a_text, "canonical_a")?;
    let canonical_b = parse_track_uri(canonical_b_text, "canonical_b")?;
    let playable_a = parse_track_uri(playable_a_text, "playable_a")?;
    let playable_b = parse_track_uri(playable_b_text, "playable_b")?;

    let automix_mode = wire
        .automix_mode
        .as_deref()
        .ok_or(AutomixPreviewError::MissingMode)?;
    if automix_mode != "auto" {
        return Err(AutomixPreviewError::UnsupportedMode(
            automix_mode.to_owned(),
        ));
    }

    let window = wire
        .start_position_ms
        .ok_or(AutomixPreviewError::MissingStartPosition)?;
    let relative = wire
        .relative_start_position
        .ok_or(AutomixPreviewError::MissingRelativeFlag)?;
    if !relative {
        return Err(AutomixPreviewError::AbsoluteStartUnsupported);
    }
    if window != i64::from(PREVIEW_WINDOW_MS) {
        return Err(AutomixPreviewError::UnsupportedWindow(window));
    }
    if wire.stop_position_ms.is_some() {
        return Err(AutomixPreviewError::ExplicitStopUnsupported);
    }

    let item_speed_a = required_speed(wire.item_speed_a, "A")?;
    let item_speed_b = required_speed(wire.item_speed_b, "B")?;

    let transition = wire
        .transition_recipe
        .take()
        .ok_or(AutomixPreviewError::MissingRecipe)?;
    let overlap = transition
        .overlap
        .as_ref()
        .ok_or(AutomixPreviewError::MissingRecipeOverlap)?;
    let (Some(start_a), Some(start_b), Some(duration)) =
        (overlap.start_a_ms, overlap.start_b_ms, overlap.duration_ms)
    else {
        return Err(AutomixPreviewError::MissingRecipeTiming);
    };
    if start_a < 0 || start_b < 0 || duration <= 0 {
        return Err(AutomixPreviewError::InvalidRecipeDuration);
    }
    let Some(recipe_item_speed_a) = overlap.item_speed_a else {
        return Err(AutomixPreviewError::InvalidRecipeAnalysis);
    };
    let Some(recipe_item_speed_b) = overlap.item_speed_b else {
        return Err(AutomixPreviewError::InvalidRecipeAnalysis);
    };
    if !recipe_item_speed_a.is_finite()
        || recipe_item_speed_a <= 0.0
        || !recipe_item_speed_b.is_finite()
        || recipe_item_speed_b <= 0.0
    {
        return Err(AutomixPreviewError::InvalidRecipeAnalysis);
    }
    if overlap.track_a_uri() != canonical_a_text {
        return Err(AutomixPreviewError::CanonicalAMismatch);
    }
    if overlap.track_b_uri() != canonical_b_text {
        return Err(AutomixPreviewError::CanonicalBMismatch);
    }
    if overlap.track_a_playable_uri() != playable_a_text {
        return Err(AutomixPreviewError::PlayableAMismatch);
    }
    if overlap.track_b_playable_uri() != playable_b_text {
        return Err(AutomixPreviewError::PlayableBMismatch);
    }
    if !item_speeds_match(recipe_item_speed_a, item_speed_a) {
        return Err(AutomixPreviewError::ItemSpeedAMismatch);
    }
    if !item_speeds_match(recipe_item_speed_b, item_speed_b) {
        return Err(AutomixPreviewError::ItemSpeedBMismatch);
    }

    let outgoing_load_position_ms = u32::try_from(start_a)
        .ok()
        .and_then(|start| start.checked_sub(PREVIEW_WINDOW_MS))
        .ok_or(AutomixPreviewError::OutgoingPrerollUnderflow)?;
    let incoming_load_position_ms =
        u32::try_from(start_b).map_err(|_| AutomixPreviewError::InvalidRecipeDuration)?;

    let recipe_bytes = transition
        .write_to_bytes()
        .map_err(|_| AutomixPreviewError::InvalidProtobuf)?;
    let cuepoint_bytes = wire
        .cuepoints
        .as_ref()
        .map(Message::write_to_bytes)
        .transpose()
        .map_err(|_| AutomixPreviewError::InvalidProtobuf)?;
    let fields = PreviewFieldPresence {
        transition_uri: wire.transition_uri.is_some(),
        cuepoints: wire.cuepoints.is_some(),
        stop_position_ms: wire.stop_position_ms.is_some(),
        context_uri: wire.context_uri.is_some(),
        arm_id: wire.arm_id.is_some(),
    };
    let fingerprint = preview_fingerprint(
        &wire,
        fields,
        item_speed_a,
        item_speed_b,
        &recipe_bytes,
        cuepoint_bytes.as_deref(),
    );
    let recipe = SpotifyTransitionRecipe::from_transition(transition).map_err(map_recipe_error)?;

    Ok(AutomixPreviewRequest {
        automix_mode: automix_mode.to_owned(),
        canonical_a,
        canonical_b,
        playable_a,
        playable_b,
        context_uri: wire.context_uri,
        transition_uri: wire.transition_uri,
        arm_id: wire.arm_id,
        cuepoints: wire.cuepoints.into_option(),
        fields,
        relative_start_position: relative,
        window_ms: PREVIEW_WINDOW_MS,
        outgoing_load_position_ms,
        incoming_load_position_ms,
        item_speed_a_bits: item_speed_a.to_bits(),
        item_speed_b_bits: item_speed_b.to_bits(),
        recipe,
        fingerprint,
    })
}

fn parse_track_uri(value: &str, field: &'static str) -> Result<SpotifyUri, AutomixPreviewError> {
    let uri =
        SpotifyUri::from_uri(value).map_err(|_| AutomixPreviewError::InvalidTrackUri(field))?;
    if !uri.is_playable() {
        return Err(AutomixPreviewError::InvalidTrackUri(field));
    }
    Ok(uri)
}

fn required_speed(value: Option<f64>, side: &'static str) -> Result<f64, AutomixPreviewError> {
    let speed = value.ok_or(AutomixPreviewError::MissingItemSpeed(side))?;
    if !speed.is_finite() || speed <= 0.0 {
        return Err(AutomixPreviewError::InvalidItemSpeed(side));
    }
    Ok(speed)
}

fn map_recipe_error(error: SpotifyTransitionError) -> AutomixPreviewError {
    match error {
        SpotifyTransitionError::MissingOverlap => AutomixPreviewError::MissingRecipeOverlap,
        SpotifyTransitionError::MissingTiming => AutomixPreviewError::MissingRecipeTiming,
        SpotifyTransitionError::InvalidDuration => AutomixPreviewError::InvalidRecipeDuration,
        SpotifyTransitionError::InvalidAnalysisValue => AutomixPreviewError::InvalidRecipeAnalysis,
        _ => AutomixPreviewError::InvalidRecipeAnalysis,
    }
}

fn preview_fingerprint(
    wire: &AutomixPreview,
    fields: PreviewFieldPresence,
    item_speed_a: f64,
    item_speed_b: f64,
    recipe_bytes: &[u8],
    cuepoint_bytes: Option<&[u8]>,
) -> PreviewFingerprint {
    let mut hash = Sha1::new();
    hash_field(&mut hash, b"schema", b"automix-preview-v1");
    hash_optional_text(&mut hash, b"canonical-a", wire.track_uri_1.as_deref());
    hash_optional_text(&mut hash, b"canonical-b", wire.track_uri_2.as_deref());
    hash_optional_text(
        &mut hash,
        b"playable-a",
        wire.playable_track_uri_1.as_deref(),
    );
    hash_optional_text(
        &mut hash,
        b"playable-b",
        wire.playable_track_uri_2.as_deref(),
    );
    hash_optional_text(&mut hash, b"mode", wire.automix_mode.as_deref());
    hash_optional_text(&mut hash, b"transition-uri", wire.transition_uri.as_deref());
    hash_optional_text(&mut hash, b"context-uri", wire.context_uri.as_deref());
    hash_optional_text(&mut hash, b"arm-id", wire.arm_id.as_deref());
    hash_field(&mut hash, b"window-ms", &PREVIEW_WINDOW_MS.to_be_bytes());
    hash_field(
        &mut hash,
        b"relative",
        &[u8::from(wire.relative_start_position == Some(true))],
    );
    hash_field(
        &mut hash,
        b"item-speed-a",
        &item_speed_a.to_bits().to_be_bytes(),
    );
    hash_field(
        &mut hash,
        b"item-speed-b",
        &item_speed_b.to_bits().to_be_bytes(),
    );
    hash_field(&mut hash, b"recipe", recipe_bytes);
    hash_optional_bytes(&mut hash, b"cuepoints", cuepoint_bytes);
    hash_field(
        &mut hash,
        b"presence",
        &[
            u8::from(fields.transition_uri),
            u8::from(fields.cuepoints),
            u8::from(fields.stop_position_ms),
            u8::from(fields.context_uri),
            u8::from(fields.arm_id),
        ],
    );
    PreviewFingerprint(hash.finalize().into())
}

fn hash_optional_text(hash: &mut Sha1, tag: &[u8], value: Option<&str>) {
    hash_optional_bytes(hash, tag, value.map(str::as_bytes));
}

fn hash_optional_bytes(hash: &mut Sha1, tag: &[u8], value: Option<&[u8]>) {
    hash_field(hash, tag, &[u8::from(value.is_some())]);
    if let Some(value) = value {
        hash_field(hash, tag, value);
    }
}

fn hash_field(hash: &mut Sha1, tag: &[u8], value: &[u8]) {
    hash.update((tag.len() as u32).to_be_bytes());
    hash.update(tag);
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use data_encoding::BASE64;
    use librespot_core::dealer::protocol::request::{LoggingParams, SignalCommand};
    use librespot_protocol::{
        automix_preview::AutomixPreview,
        automix_transition::{
            EqCurveOverrides, EqStyle, FilterFxStyle, Overlap, Preset, Transition,
        },
    };
    use protobuf::{Message, MessageField};

    const TRACK_A: &str = "spotify:track:2TpxZ7JUBn3uw46aR7qd6V";
    const TRACK_B: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
    const TRACK_C: &str = "spotify:track:0VjIjW4GlUZAMYd2vXMi3b";

    fn logging(command_id: &str) -> LoggingParams {
        LoggingParams {
            interaction_ids: None,
            device_identifier: None,
            command_initiated_time: None,
            page_instance_ids: None,
            command_id: Some(command_id.to_owned()),
        }
    }

    fn signal(wire: AutomixPreview) -> SignalCommand {
        SignalCommand {
            signal_id: "automix-preview".to_owned(),
            parameters: Some(BASE64.encode(&wire.write_to_bytes().unwrap())),
            logging_params: logging("preview-1"),
        }
    }

    fn decode_wire(wire: AutomixPreview) -> Result<AutomixPreviewRequest, AutomixPreviewError> {
        decode_automix_preview(&signal(wire))
    }

    fn decode_parameters(value: &str) -> Result<AutomixPreviewRequest, AutomixPreviewError> {
        decode_automix_preview(&SignalCommand {
            signal_id: "automix-preview".to_owned(),
            parameters: Some(value.to_owned()),
            logging_params: logging("preview-1"),
        })
    }

    fn valid_preview_proto() -> AutomixPreview {
        AutomixPreview {
            track_uri_1: Some(TRACK_A.to_owned()),
            track_uri_2: Some(TRACK_B.to_owned()),
            automix_mode: Some("auto".to_owned()),
            transition_uri: Some("spotify:transition:captured".to_owned()),
            start_position_ms: Some(3_000),
            relative_start_position: Some(true),
            stop_position_ms: None,
            transition_recipe: MessageField::some(Transition {
                overlap: MessageField::some(Overlap {
                    start_a_ms: Some(184_812),
                    start_b_ms: Some(944),
                    duration_ms: Some(7_385),
                    track_a_uri: Some(TRACK_A.to_owned()),
                    track_b_uri: Some(TRACK_B.to_owned()),
                    track_a_playable_uri: Some(TRACK_A.to_owned()),
                    track_b_playable_uri: Some(TRACK_B.to_owned()),
                    item_speed_a: Some(1.0),
                    item_speed_b: Some(1.0),
                    ..Default::default()
                }),
                preset: MessageField::some(Preset {
                    id: Some(10),
                    eq_style_override: MessageField::some(EqStyle {
                        id: Some(0),
                        ..Default::default()
                    }),
                    filter_fx_style_override: MessageField::some(FilterFxStyle {
                        id: Some(0),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            playable_track_uri_1: Some(TRACK_A.to_owned()),
            playable_track_uri_2: Some(TRACK_B.to_owned()),
            context_uri: Some("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M".to_owned()),
            arm_id: Some("captured-arm".to_owned()),
            item_speed_a: Some(1.0),
            item_speed_b: Some(1.0),
            ..Default::default()
        }
    }

    fn with_recipe_canonical_a(mut wire: AutomixPreview, uri: &str) -> AutomixPreview {
        wire.transition_recipe
            .mut_or_insert_default()
            .overlap
            .mut_or_insert_default()
            .track_a_uri = Some(uri.to_owned());
        wire
    }

    fn with_recipe_playable_b(mut wire: AutomixPreview, uri: &str) -> AutomixPreview {
        wire.transition_recipe
            .mut_or_insert_default()
            .overlap
            .mut_or_insert_default()
            .track_b_playable_uri = Some(uri.to_owned());
        wire
    }

    fn with_recipe_item_speed_b(mut wire: AutomixPreview, speed: f64) -> AutomixPreview {
        wire.transition_recipe
            .mut_or_insert_default()
            .overlap
            .mut_or_insert_default()
            .item_speed_b = Some(speed);
        wire
    }

    fn with_preset(preset_id: i32) -> AutomixPreview {
        let mut wire = valid_preview_proto();
        wire.transition_recipe
            .mut_or_insert_default()
            .preset
            .mut_or_insert_default()
            .id = Some(preset_id);
        wire
    }

    fn with_custom_curve() -> AutomixPreview {
        let mut wire = valid_preview_proto();
        wire.transition_recipe
            .mut_or_insert_default()
            .preset
            .mut_or_insert_default()
            .eq_out_curve_overrides = MessageField::some(EqCurveOverrides::default());
        wire
    }

    fn resolve_wire(
        wire: AutomixPreview,
    ) -> Result<PreviewResolution, AutomixPreviewResolutionError> {
        resolve_automix_preview(decode_wire(wire).unwrap())
    }

    #[test]
    fn valid_relative_preview_decodes_with_exact_positions_and_presence() {
        let request = decode_automix_preview(&signal(valid_preview_proto())).unwrap();
        assert_eq!(request.window_ms, 3_000);
        assert_eq!(request.outgoing_load_position_ms, 181_812);
        assert_eq!(request.incoming_load_position_ms, 944);
        assert!(request.fields.transition_uri);
        assert!(!request.fields.stop_position_ms);
    }

    #[test]
    fn supported_preview_uses_shared_style_and_plan() {
        let request = decode_wire(valid_preview_proto()).unwrap();
        let resolved = resolve_automix_preview(request).unwrap();
        let PreviewResolution::Playable(preview) = resolved else {
            panic!("must render")
        };
        assert_eq!(preview.preset_id, 10);
        assert_eq!(preview.plan.current_start(), Duration::from_millis(184_812));
        assert_eq!(preview.plan.next_start(), Duration::from_millis(944));
    }

    #[test]
    fn none_is_successful_no_preview_but_unknown_and_custom_are_rejected() {
        assert!(matches!(
            resolve_wire(with_preset(0)).unwrap(),
            PreviewResolution::NoTransition { preset_id: 0 }
        ));
        assert_eq!(
            resolve_wire(with_preset(999)).unwrap_err(),
            AutomixPreviewResolutionError::UnknownPreset(999)
        );
        assert_eq!(
            resolve_wire(with_custom_curve()).unwrap_err(),
            AutomixPreviewResolutionError::UnsupportedRenderer { preset_id: 10 }
        );
    }

    #[test]
    fn absent_required_field_is_not_defaulted() {
        let mut wire = valid_preview_proto();
        wire.start_position_ms = None;
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::MissingStartPosition
        );
    }

    #[test]
    fn non_evidenced_window_absolute_start_and_explicit_stop_are_rejected() {
        let mut wrong_window = valid_preview_proto();
        wrong_window.start_position_ms = Some(2_999);
        assert_eq!(
            decode_wire(wrong_window).unwrap_err(),
            AutomixPreviewError::UnsupportedWindow(2_999)
        );

        let mut absolute = valid_preview_proto();
        absolute.relative_start_position = Some(false);
        assert_eq!(
            decode_wire(absolute).unwrap_err(),
            AutomixPreviewError::AbsoluteStartUnsupported
        );

        let mut stopped = valid_preview_proto();
        stopped.stop_position_ms = Some(195_000);
        assert_eq!(
            decode_wire(stopped).unwrap_err(),
            AutomixPreviewError::ExplicitStopUnsupported
        );
    }

    #[test]
    fn oversized_invalid_base64_and_invalid_protobuf_are_bounded_failures() {
        assert_eq!(
            decode_parameters(&"A".repeat(16 * 1024 + 1)).unwrap_err(),
            AutomixPreviewError::OversizedParameters
        );
        assert_eq!(
            decode_parameters("not base64!").unwrap_err(),
            AutomixPreviewError::InvalidBase64
        );
        assert_eq!(
            decode_parameters("/w==").unwrap_err(),
            AutomixPreviewError::InvalidProtobuf
        );
    }

    #[test]
    fn canonical_playable_and_item_speed_mismatches_are_rejected() {
        assert_eq!(
            decode_wire(with_recipe_canonical_a(valid_preview_proto(), TRACK_C)).unwrap_err(),
            AutomixPreviewError::CanonicalAMismatch
        );
        assert_eq!(
            decode_wire(with_recipe_playable_b(valid_preview_proto(), TRACK_C)).unwrap_err(),
            AutomixPreviewError::PlayableBMismatch
        );
        assert_eq!(
            decode_wire(with_recipe_item_speed_b(valid_preview_proto(), 1.25)).unwrap_err(),
            AutomixPreviewError::ItemSpeedBMismatch
        );
    }

    #[test]
    fn signal_and_parameter_presence_are_strict() {
        let mut command = signal(valid_preview_proto());
        command.signal_id = "other".to_owned();
        assert_eq!(
            decode_automix_preview(&command).unwrap_err(),
            AutomixPreviewError::WrongSignalId
        );
        command.signal_id = "automix-preview".to_owned();
        command.parameters = None;
        assert_eq!(
            decode_automix_preview(&command).unwrap_err(),
            AutomixPreviewError::MissingParameters
        );
        assert_eq!(
            decode_parameters("").unwrap_err(),
            AutomixPreviewError::EmptyParameters
        );
    }

    #[test]
    fn identities_mode_and_item_speeds_are_validated() {
        let mut wire = valid_preview_proto();
        wire.track_uri_1 = None;
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::MissingCanonicalA
        );

        let mut wire = valid_preview_proto();
        wire.playable_track_uri_2 = None;
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::MissingPlayableB
        );

        let mut wire = valid_preview_proto();
        wire.track_uri_1 = Some("not-a-uri".to_owned());
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::InvalidTrackUri("canonical_a")
        );

        let mut wire = valid_preview_proto();
        wire.automix_mode = None;
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::MissingMode
        );

        let mut wire = valid_preview_proto();
        wire.automix_mode = Some("manual".to_owned());
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::UnsupportedMode("manual".to_owned())
        );

        let mut wire = valid_preview_proto();
        wire.relative_start_position = None;
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::MissingRelativeFlag
        );

        let mut wire = valid_preview_proto();
        wire.item_speed_a = None;
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::MissingItemSpeed("A")
        );

        let mut wire = valid_preview_proto();
        wire.item_speed_a = Some(0.0);
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::InvalidItemSpeed("A")
        );

        let mut wire = valid_preview_proto();
        wire.item_speed_b = Some(f64::NAN);
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::InvalidItemSpeed("B")
        );
    }

    #[test]
    fn recipe_presence_geometry_and_analysis_are_validated() {
        let mut wire = valid_preview_proto();
        wire.transition_recipe = MessageField::none();
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::MissingRecipe
        );

        let mut wire = valid_preview_proto();
        wire.transition_recipe.mut_or_insert_default().overlap = MessageField::none();
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::MissingRecipeOverlap
        );

        let mut wire = valid_preview_proto();
        wire.transition_recipe
            .mut_or_insert_default()
            .overlap
            .mut_or_insert_default()
            .duration_ms = None;
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::MissingRecipeTiming
        );

        let mut wire = valid_preview_proto();
        wire.transition_recipe
            .mut_or_insert_default()
            .overlap
            .mut_or_insert_default()
            .duration_ms = Some(0);
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::InvalidRecipeDuration
        );

        let mut wire = valid_preview_proto();
        wire.transition_recipe
            .mut_or_insert_default()
            .overlap
            .mut_or_insert_default()
            .item_speed_a = None;
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::InvalidRecipeAnalysis
        );

        let mut wire = valid_preview_proto();
        wire.transition_recipe
            .mut_or_insert_default()
            .overlap
            .mut_or_insert_default()
            .start_a_ms = Some(2_999);
        assert_eq!(
            decode_wire(wire).unwrap_err(),
            AutomixPreviewError::OutgoingPrerollUnderflow
        );
    }

    #[test]
    fn fingerprint_ignores_logging_but_retains_optional_presence() {
        let wire = valid_preview_proto();
        let first = decode_automix_preview(&signal(wire.clone())).unwrap();
        let mut duplicate = signal(wire.clone());
        duplicate.logging_params.command_id = Some("other-command".to_owned());
        let duplicate = decode_automix_preview(&duplicate).unwrap();
        assert_eq!(first.fingerprint, duplicate.fingerprint);

        let mut absent_context = wire;
        absent_context.context_uri = None;
        let absent_context = decode_wire(absent_context).unwrap();
        assert_ne!(first.fingerprint, absent_context.fingerprint);
    }
}
