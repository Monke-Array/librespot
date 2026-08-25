use crate::protocol::{
    connect::{Cluster, ClusterUpdate},
    context::Context,
    context_track::ContextTrack,
    player::{PlayerState, ProvidedTrack},
    playlist4_external::PlaylistModificationInfo,
    transfer_state::TransferState,
};
use protobuf::Message;
use std::collections::{BTreeSet, HashMap};
use std::env;

const PREFIX: &str = "[spotify-mix-debug]";
const TRACK_LIMIT: usize = 8;
const RECIPE_ATTRIBUTE: &str = "automix.auto_transition_recipe";
const BACKEND_RECIPE_ATTRIBUTE: &str = "automix.backend_auto_transition";
const DEV_DUMP_MATERIALIZED_METADATA_ENV: &str = "LIBRESPOT_DEV_DUMP_MATERIALIZED_METADATA";

pub(crate) fn log_cluster(source: &str, cluster: &Cluster) {
    if !enabled() {
        return;
    }
    debug!(
        "{PREFIX} received message transport=dealer type=connect-state-cluster source={source} transfer-data-present={}",
        !cluster.transfer_data.is_empty()
    );
    unknown("connect-state cluster", cluster);
    if let Some(player) = cluster.player_state.as_ref() {
        log_player(source, player);
    }
}

pub(crate) fn log_cluster_update(update: &ClusterUpdate) {
    if !enabled() {
        return;
    }
    debug!(
        "{PREFIX} received message transport=dealer type=connect-state-cluster-update reason={:?}",
        update.update_reason.enum_value()
    );
    unknown("cluster update", update);
    if let Some(cluster) = update.cluster.as_ref() {
        log_cluster("cluster-update", cluster);
    }
}

pub(crate) fn log_transfer(transfer: &TransferState) {
    if !enabled() {
        return;
    }
    debug!(
        "{PREFIX} received message transport=dealer type=transfer-state optional fields present={:?}",
        [
            transfer.options.is_some().then_some("options"),
            transfer.playback.is_some().then_some("playback"),
            transfer
                .current_session
                .is_some()
                .then_some("current_session"),
            transfer.queue.is_some().then_some("queue"),
            transfer.play_history.is_some().then_some("play_history"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
    );
    unknown("transfer state", transfer);
    if let Some(options) = transfer.options.as_ref() {
        debug!(
            "{PREFIX} transfer option modes keys={:?}",
            keys(&options.modes)
        );
        unknown("transfer options", options);
    }
    if let Some(playback) = transfer.playback.as_ref() {
        unknown("transfer playback", playback);
        if let Some(track) = playback.current_track.as_ref() {
            log_context_track("transfer current track", 0, track);
        }
        if let Some(track) = playback.associated_current_track.as_ref() {
            log_context_track("transfer associated track", 0, track);
        }
    }
    if let Some(session) = transfer.current_session.as_ref() {
        unknown("transfer current session", session);
        if let Some(origin) = session.play_origin.as_ref() {
            debug!(
                "{PREFIX} play origin source=transfer feature={} version={} classes={:?}",
                safe(origin.feature_identifier.as_deref()),
                safe(origin.feature_version.as_deref()),
                origin
                    .feature_classes
                    .iter()
                    .map(|value| safe(Some(value)))
                    .collect::<Vec<_>>()
            );
            unknown("transfer play origin", origin);
        }
        if let Some(context) = session.context.as_ref() {
            log_context("transfer context", context);
        }
        if let Some(context) = session.main_context.as_ref() {
            log_context("transfer main context", context);
        }
        if let Some(options) = session.option_overrides.as_ref() {
            debug!(
                "{PREFIX} transfer option override modes keys={:?}",
                keys(&options.modes)
            );
            unknown("transfer option overrides", options);
        }
    }
    if let Some(queue) = transfer.queue.as_ref() {
        unknown("transfer queue", queue);
        for (index, track) in queue.tracks.iter().take(TRACK_LIMIT).enumerate() {
            log_context_track("transfer queue track", index, track);
        }
    }
}

pub(crate) fn log_playlist_modification(update: &PlaylistModificationInfo) {
    if !enabled() {
        return;
    }
    debug!(
        "{PREFIX} received message transport=dealer type=playlist-modification operation-count={} optional fields present={:?}",
        update.ops.len(),
        [
            update.uri.is_some().then_some("uri"),
            update.new_revision.is_some().then_some("new_revision"),
            update
                .parent_revision
                .is_some()
                .then_some("parent_revision"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
    );
    unknown("playlist modification", update);
}

fn log_player(source: &str, player: &PlayerState) {
    debug!(
        "{PREFIX} player state source={source} context metadata keys={:?} page metadata keys={:?}",
        keys(&player.context_metadata),
        keys(&player.page_metadata)
    );
    unknown("player state", player);
    if let Some(origin) = player.play_origin.as_ref() {
        debug!(
            "{PREFIX} play origin feature={} version={} classes={:?}",
            safe(Some(&origin.feature_identifier)),
            safe(Some(&origin.feature_version)),
            origin
                .feature_classes
                .iter()
                .map(|value| safe(Some(value)))
                .collect::<Vec<_>>()
        );
        unknown("play origin", origin);
    }
    if let Some(options) = player.options.as_ref() {
        debug!(
            "{PREFIX} player option modes keys={:?}",
            keys(&options.modes)
        );
        unknown("player options", options);
    }
    for (label, restrictions) in [
        ("player restrictions", player.restrictions.as_ref()),
        ("context restrictions", player.context_restrictions.as_ref()),
    ] {
        if let Some(restrictions) = restrictions {
            debug!(
                "{PREFIX} {label} disallow-setting-modes keys={:?} disallow-signals keys={:?}",
                keys(&restrictions.disallow_setting_modes),
                keys(&restrictions.disallow_signals)
            );
            unknown(label, restrictions);
        }
    }
    if let Some(track) = player.track.as_ref() {
        log_provided_track("player current track", 0, track);
    }
    for (index, track) in player.next_tracks.iter().take(TRACK_LIMIT).enumerate() {
        log_provided_track("player next track", index, track);
    }
}

fn log_context(source: &str, context: &Context) {
    debug!(
        "{PREFIX} context source={source} metadata keys={:?} page-count={}",
        keys(&context.metadata),
        context.pages.len()
    );
    unknown("context", context);
    for (page_index, page) in context.pages.iter().enumerate() {
        debug!(
            "{PREFIX} page index={page_index} metadata keys={:?} track-count={}",
            keys(&page.metadata),
            page.tracks.len()
        );
        unknown("context page", page);
        for (track_index, track) in page.tracks.iter().take(TRACK_LIMIT).enumerate() {
            log_context_track(&format!("page {page_index} track"), track_index, track);
        }
    }
}

pub(crate) fn log_context_track_before_conversion(track: &ContextTrack) {
    if !enabled() {
        return;
    }

    let mut fields = [
        track.uri.is_some().then_some("uri"),
        track.uid.is_some().then_some("uid"),
        track.gid.is_some().then_some("gid"),
        (!track.metadata.is_empty()).then_some("metadata"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    fields.sort_unstable();

    debug!(
        "[spotify-mix] raw ContextTrack uri={} uid={}",
        safe(track.uri.as_deref()),
        safe(track.uid.as_deref())
    );
    debug!(
        "[spotify-mix] raw ContextTrack field_names={fields:?} metadata_keys={:?}",
        keys(&track.metadata)
    );
    log_materialized_metadata_values("raw ContextTrack", &track.metadata);
    // ContextTrack has no format-list-attribute field in the current schema.
    debug!("[spotify-mix] raw ContextTrack format_attribute_keys=[]");
    debug!(
        "[spotify-mix] raw ContextTrack recipe_present={} backend_recipe_present={}",
        track.metadata.contains_key(RECIPE_ATTRIBUTE),
        track.metadata.contains_key(BACKEND_RECIPE_ATTRIBUTE)
    );
    unknown("raw ContextTrack before conversion", track);
}

fn log_context_track(label: &str, index: usize, track: &ContextTrack) {
    debug!(
        "{PREFIX} {label} index={index} uri={} uid={} metadata_keys={:?} format_attribute_keys=[] recipe_present={} backend_recipe_present={}",
        safe(track.uri.as_deref()),
        safe(track.uid.as_deref()),
        keys(&track.metadata),
        track.metadata.contains_key(RECIPE_ATTRIBUTE),
        track.metadata.contains_key(BACKEND_RECIPE_ATTRIBUTE)
    );
    log_materialized_metadata_values(label, &track.metadata);
    unknown("context track", track);
}

fn log_provided_track(label: &str, index: usize, track: &ProvidedTrack) {
    debug!(
        "{PREFIX} {label} index={index} uri={} uid={} metadata_keys={:?} format_attribute_keys=[] recipe_present={} backend_recipe_present={}",
        safe(Some(&track.uri)),
        safe(Some(&track.uid)),
        keys(&track.metadata),
        track.metadata.contains_key(RECIPE_ATTRIBUTE),
        track.metadata.contains_key(BACKEND_RECIPE_ATTRIBUTE)
    );
    log_materialized_metadata_values(label, &track.metadata);
    unknown("provided track", track);
}

fn log_materialized_metadata_values(label: &str, metadata: &HashMap<String, String>) {
    if !dev_dump_materialized_metadata_enabled() {
        return;
    }

    let mut values = metadata
        .iter()
        .filter(|(key, _)| is_materialized_metadata_key(key))
        .map(|(key, value)| format!("{}={}", safe_metadata_key(key), safe_metadata_value(value)))
        .collect::<Vec<_>>();
    values.sort();
    values.truncate(128);
    if !values.is_empty() {
        debug!("{PREFIX} {label} materialized_metadata_values={values:?}");
    }
}

fn is_materialized_metadata_key(key: &str) -> bool {
    key.starts_with("audio.")
        || matches!(
            key,
            "automix.auto_preset_id"
                | "automix.mode"
                | "automix.transition_uri"
                | "automix.fade_in_cuepoint.origin"
                | "automix.fade_in_cuepoint.position"
                | "automix.fade_in_cuepoint.tempo"
                | "automix.fade_out_cuepoint.origin"
                | "automix.fade_out_cuepoint.position"
                | "automix.fade_out_cuepoint.tempo"
        )
}

fn unknown<M: Message>(label: &str, message: &M) {
    let fields = message
        .special_fields()
        .unknown_fields()
        .iter()
        .map(|(number, value)| format!("{number}:{:?}", value.wire_type()))
        .collect::<BTreeSet<_>>();
    if !fields.is_empty() {
        debug!(
            "{PREFIX} {label} unknown protobuf fields present={:?}",
            fields.into_iter().collect::<Vec<_>>()
        );
    }
}

fn keys<V>(map: &HashMap<String, V>) -> Vec<String> {
    let mut keys = map
        .keys()
        .map(|key| safe_metadata_key(key).into_owned())
        .collect::<Vec<_>>();
    keys.sort();
    keys.truncate(128);
    keys
}

fn safe_metadata_key(value: &str) -> std::borrow::Cow<'_, str> {
    if value.len() <= 96 && !value.chars().any(char::is_control) {
        value.into()
    } else {
        "<invalid-key>".into()
    }
}

fn safe_metadata_value(value: &str) -> String {
    let mut sanitized = value
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>();
    if sanitized.len() > 2048 {
        sanitized.truncate(2048);
        sanitized.push_str("<truncated>");
    }
    sanitized
}

fn safe(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "<unset-or-redacted>".into();
    };
    if !value.is_empty()
        && value.len() <= 96
        && !value.contains("://")
        && !value.contains('?')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._:/+-".contains(c))
    {
        value.into()
    } else {
        "<unset-or-redacted>".into()
    }
}

fn enabled() -> bool {
    log::log_enabled!(log::Level::Debug)
}

fn dev_dump_materialized_metadata_enabled() -> bool {
    env::var(DEV_DUMP_MATERIALIZED_METADATA_ENV).as_deref() == Ok("1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_field_description_excludes_opaque_data() {
        let mut transfer = TransferState::new();
        transfer
            .special_fields
            .mut_unknown_fields()
            .add_length_delimited(99, b"must-not-be-logged".to_vec());
        let description = transfer
            .special_fields()
            .unknown_fields()
            .iter()
            .map(|(number, value)| format!("{number}:{:?}", value.wire_type()))
            .collect::<Vec<_>>();
        assert_eq!(description, ["99:LengthDelimited"]);
        assert!(!description.join(" ").contains("must-not-be-logged"));
    }

    #[test]
    fn materialized_metadata_key_filter_is_narrow() {
        assert!(is_materialized_metadata_key("audio.fade_overlap"));
        assert!(is_materialized_metadata_key("automix.transition_uri"));
        assert!(is_materialized_metadata_key("automix.auto_preset_id"));
        assert!(!is_materialized_metadata_key("added_by_username"));
        assert!(!is_materialized_metadata_key("context_uri"));
    }

    #[test]
    fn metadata_value_sanitizer_removes_controls_and_truncates() {
        let clean = safe_metadata_value("spotify:core-auto-transition\nsecret");
        assert_eq!(clean, "spotify:core-auto-transition secret");

        let long = safe_metadata_value(&"a".repeat(3000));
        assert!(long.ends_with("<truncated>"));
        assert!(long.len() < 2100);
    }
}
