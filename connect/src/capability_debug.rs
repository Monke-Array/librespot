use crate::protocol::{
    connect::{Capabilities, Cluster, DeviceInfo, PutStateRequest},
    player::PlayerState,
};
use protobuf::Message;
use std::collections::BTreeSet;

const PREFIX: &str = "[spotify-capability-debug]";

pub(crate) fn log_put_state(request: &PutStateRequest) {
    if !enabled() {
        return;
    }

    debug!(
        "{PREFIX} outgoing transport=spclient type=connect-state-put-state member-type={:?}({}) reason={:?}({}) active={} only-player-state={} message-id={} callback-url-present={} last-command-device-id-present={}",
        request.member_type.enum_value(),
        request.member_type.value(),
        request.put_state_reason.enum_value(),
        request.put_state_reason.value(),
        request.is_active,
        request.only_write_player_state,
        request.message_id,
        !request.callback_url.is_empty(),
        !request.last_command_sent_by_device_id.is_empty(),
    );
    log_unknown("put-state", request);

    let Some(device) = request.device.as_ref() else {
        debug!("{PREFIX} outgoing device-present=false");
        return;
    };
    log_unknown("device", device);
    if let Some(info) = device.device_info.as_ref() {
        log_device_info("outgoing", info, None);
    } else {
        debug!("{PREFIX} outgoing device-info-present=false");
    }
    if let Some(private) = device.private_device_info.as_ref() {
        debug!(
            "{PREFIX} outgoing private-device-info platform={}",
            safe_value("platform", &private.platform)
        );
        log_unknown("private-device-info", private);
    } else {
        debug!("{PREFIX} outgoing private-device-info-present=false platform=<unset>");
    }
    if let Some(player) = device.player_state.as_ref() {
        log_player_features("outgoing", player);
    } else {
        debug!("{PREFIX} outgoing player-state-present=false");
    }
}

/// A cluster includes the advertisements of the other Connect devices. Logging their shape makes
/// it possible to compare librespot with a first-party client without exposing device identifiers.
pub(crate) fn log_cluster_devices(source: &str, cluster: &Cluster) {
    if !enabled() {
        return;
    }

    let mut devices = cluster.device.iter().collect::<Vec<_>>();
    devices.sort_unstable_by_key(|(device_id, _)| *device_id);
    debug!(
        "{PREFIX} incoming source={source} cluster-device-count={}",
        devices.len()
    );
    for (index, (device_id, info)) in devices.into_iter().enumerate() {
        log_device_info(
            &format!("incoming source={source} device-index={index}"),
            info,
            Some(device_id.as_str() == cluster.active_device_id),
        );
    }
}

fn log_device_info(source: &str, info: &DeviceInfo, active: Option<bool>) {
    debug!(
        "{PREFIX} {source} device-info active={active:?} can-play={} type={:?}({}) software-version={} spirc-version={} client-id={} product-id={} brand={} model={} group={} dynamic={} social-connect={} private-session={} offline={}",
        info.can_play,
        info.device_type.enum_value(),
        info.device_type.value(),
        safe_value("software-version", &info.device_software_version),
        safe_value("spirc-version", &info.spirc_version),
        safe_value("client-id", &info.client_id),
        safe_value("product-id", &info.product_id),
        safe_value("brand", &info.brand),
        safe_value("model", &info.model),
        info.is_group,
        info.is_dynamic_device,
        info.is_social_connect,
        info.is_private_session,
        info.is_offline,
    );
    debug!(
        "{PREFIX} {source} device-info sensitive-or-user-fields name-present={} device-id-present={} deduplication-id-present={} public-ip-present={} license-present={} aliases={} selected-alias-id={} metadata-keys={:?} disallow-playback={:?} disallow-transfer={:?}",
        !info.name.is_empty(),
        !info.device_id.is_empty(),
        !info.deduplication_id.is_empty(),
        !info.public_ip.is_empty(),
        !info.license.is_empty(),
        info.device_aliases.len(),
        info.selected_alias_id,
        safe_keys(info.metadata_map.keys()),
        safe_values("disallow-playback", &info.disallow_playback_reasons),
        safe_values("disallow-transfer", &info.disallow_transfer_reasons),
    );
    if let Some(output) = info.audio_output_device_info.as_ref() {
        debug!(
            "{PREFIX} {source} audio-output type={:?}({:?}) name-present={}",
            output
                .audio_output_device_type
                .as_ref()
                .map(|output_type| output_type.enum_value()),
            output
                .audio_output_device_type
                .as_ref()
                .map(|output_type| output_type.value()),
            output
                .device_name
                .as_deref()
                .is_some_and(|name| !name.is_empty()),
        );
        log_unknown("audio-output-device-info", output);
    } else {
        debug!("{PREFIX} {source} audio-output-present=false");
    }
    log_unknown("device-info", info);

    if let Some(capabilities) = info.capabilities.as_ref() {
        log_capabilities(source, capabilities);
    } else {
        debug!("{PREFIX} {source} capabilities-present=false");
    }
}

fn log_capabilities(source: &str, capabilities: &Capabilities) {
    debug!(
        "{PREFIX} {source} capability-booleans can-be-player={} restrict-to-local={} gaia-eq-connect-id={} supports-logout={} observable={} command-acks={} supports-rename={} hidden={} disable-volume={} connect-disabled={} supports-playlist-v2={} controllable={} supports-external-episodes={} supports-set-backend-metadata={} supports-transfer-command={} supports-command-request={} voice-enabled={} needs-full-player-state={} supports-gzip-pushes={} supports-set-options-command={} supports-rooms={} supports-dj={}",
        capabilities.can_be_player,
        capabilities.restrict_to_local,
        capabilities.gaia_eq_connect_id,
        capabilities.supports_logout,
        capabilities.is_observable,
        capabilities.command_acks,
        capabilities.supports_rename,
        capabilities.hidden,
        capabilities.disable_volume,
        capabilities.connect_disabled,
        capabilities.supports_playlist_v2,
        capabilities.is_controllable,
        capabilities.supports_external_episodes,
        capabilities.supports_set_backend_metadata,
        capabilities.supports_transfer_command,
        capabilities.supports_command_request,
        capabilities.is_voice_enabled,
        capabilities.needs_full_player_state,
        capabilities.supports_gzip_pushes,
        capabilities.supports_set_options_command,
        capabilities.supports_rooms,
        capabilities.supports_dj,
    );
    debug!(
        "{PREFIX} {source} capability-values volume-steps={} supported-media-types={:?} supported-audio-quality={:?}({}) connect-capabilities={} supported-codecs=[] codec-field-present-in-connect-schema=false",
        capabilities.volume_steps,
        safe_values("supported-media-type", &capabilities.supported_types),
        capabilities.supported_audio_quality.enum_value(),
        capabilities.supported_audio_quality.value(),
        safe_value("connect-capabilities", &capabilities.connect_capabilities),
    );
    debug!(
        "{PREFIX} {source} advertised-command-features command-acks={} transfer={} command-request={} set-options={} explicit-command-name-field-present=false",
        capabilities.command_acks,
        capabilities.supports_transfer_command,
        capabilities.supports_command_request,
        capabilities.supports_set_options_command,
    );
    debug!(
        "{PREFIX} {source} implementation-command-inventory player-endpoints=[transfer,play,pause,seek_to,set_shuffling_context,set_repeating_track,set_repeating_context,add_to_queue,set_queue,set_options,update_context,skip_next,skip_prev,resume] separate-endpoints=[volume] advertised-false-or-unimplemented=[logout,rename,set_backend_metadata] wire-command-name-list-present=false"
    );
    if let Some(hifi) = capabilities.supports_hifi.as_ref() {
        debug!(
            "{PREFIX} {source} supports-hifi present=true fully-supported={} user-eligible={} device-supported={}",
            hifi.fully_supported, hifi.user_eligible, hifi.device_supported,
        );
        log_unknown("supports-hifi", hifi);
    } else {
        debug!("{PREFIX} {source} supports-hifi present=false");
    }
    log_unknown("capabilities", capabilities);
}

fn log_player_features(source: &str, player: &PlayerState) {
    debug!(
        "{PREFIX} {source} player-features signals={:?} context-metadata-keys={:?} page-metadata-keys={:?}",
        safe_values("signal", &player.signals),
        safe_keys(player.context_metadata.keys()),
        safe_keys(player.page_metadata.keys()),
    );
    if let Some(origin) = player.play_origin.as_ref() {
        debug!(
            "{PREFIX} {source} play-origin feature-identifier={} feature-version={} feature-classes={:?} restriction-identifier={} device-identifier-present={}",
            safe_value("feature-identifier", &origin.feature_identifier),
            safe_value("feature-version", &origin.feature_version),
            safe_values("feature-class", &origin.feature_classes),
            safe_value("restriction-identifier", &origin.restriction_identifier),
            !origin.device_identifier.is_empty(),
        );
        log_unknown("play-origin", origin);
    } else {
        debug!("{PREFIX} {source} play-origin-present=false");
    }
    if let Some(options) = player.options.as_ref() {
        debug!(
            "{PREFIX} {source} player-options shuffle={} repeat-context={} repeat-track={} playback-speed={:?} mode-keys={:?}",
            options.shuffling_context,
            options.repeating_context,
            options.repeating_track,
            options.playback_speed,
            safe_keys(options.modes.keys()),
        );
        log_unknown("player-options", options);
    } else {
        debug!("{PREFIX} {source} player-options-present=false");
    }
    if let Some(quality) = player.playback_quality.as_ref() {
        debug!(
            "{PREFIX} {source} playback-quality bitrate={:?}({}) strategy={:?}({}) target-bitrate={:?}({}) target-available={} hifi-status={:?}({})",
            quality.bitrate_level.enum_value(),
            quality.bitrate_level.value(),
            quality.strategy.enum_value(),
            quality.strategy.value(),
            quality.target_bitrate_level.enum_value(),
            quality.target_bitrate_level.value(),
            quality.target_bitrate_available,
            quality.hifi_status.enum_value(),
            quality.hifi_status.value(),
        );
        log_unknown("playback-quality", quality);
    } else {
        debug!("{PREFIX} {source} playback-quality-present=false");
    }
    log_unknown("player-state", player);
}

fn log_unknown<M: Message>(label: &str, message: &M) {
    let fields = unknown_field_shapes(message);
    debug!("{PREFIX} {label} unknown-protobuf-fields={fields:?}");
}

fn unknown_field_shapes<M: Message>(message: &M) -> Vec<String> {
    message
        .special_fields()
        .unknown_fields()
        .iter()
        .map(|(number, value)| format!("{number}:{:?}", value.wire_type()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn safe_keys<'a>(values: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut values = values
        .filter_map(|value| safe_non_secret("key", value))
        .collect::<Vec<_>>();
    values.sort();
    values.truncate(128);
    values
}

fn safe_values(field: &str, values: &[String]) -> Vec<String> {
    let mut values = values
        .iter()
        .filter_map(|value| safe_non_secret(field, value))
        .collect::<Vec<_>>();
    values.sort();
    values.truncate(128);
    values
}

fn safe_value(field: &str, value: &str) -> String {
    if value.is_empty() {
        "<unset>".into()
    } else {
        safe_non_secret(field, value).unwrap_or_else(|| "<redacted>".into())
    }
}

fn safe_non_secret(field: &str, value: &str) -> Option<String> {
    let field = field.to_ascii_lowercase();
    let secret = [
        "token",
        "auth",
        "cookie",
        "credential",
        "password",
        "secret",
        "signature",
        "session-key",
    ]
    .iter()
    .any(|term| field.contains(term));
    (!secret
        && !value.is_empty()
        && value.len() <= 256
        && !value.contains("://")
        && !value.contains('?')
        && !value.chars().any(char::is_control))
    .then(|| value.into())
}

fn enabled() -> bool {
    log::log_enabled!(log::Level::Debug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::connect::Capabilities;

    #[test]
    fn sanitizer_rejects_secrets_urls_and_control_characters() {
        assert_eq!(
            safe_value("client-id", "0123456789abcdef"),
            "0123456789abcdef"
        );
        assert_eq!(safe_value("access-token", "reusable"), "<redacted>");
        assert_eq!(
            safe_value("platform", "https://example.test/?sig=x"),
            "<redacted>"
        );
        assert_eq!(safe_value("feature", "mix\nsecret"), "<redacted>");
    }

    #[test]
    fn unknown_field_log_contains_only_number_and_wire_type() {
        let mut capabilities = Capabilities::new();
        capabilities
            .special_fields
            .mut_unknown_fields()
            .add_length_delimited(99, b"must-not-be-logged".to_vec());

        let description = unknown_field_shapes(&capabilities);
        assert_eq!(description, ["99:LengthDelimited"]);
        assert!(!description.join(" ").contains("must-not-be-logged"));
    }
}
