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

const PREFIX: &str = "[spotify-mix-debug]";
const TRACK_LIMIT: usize = 8;

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

fn log_context_track(label: &str, index: usize, track: &ContextTrack) {
    debug!(
        "{PREFIX} {label} index={index} id={} metadata keys={:?}",
        spotify_id(track.uri.as_deref()),
        keys(&track.metadata)
    );
    unknown("context track", track);
}

fn log_provided_track(label: &str, index: usize, track: &ProvidedTrack) {
    debug!(
        "{PREFIX} {label} index={index} id={} metadata keys={:?}",
        spotify_id(Some(&track.uri)),
        keys(&track.metadata)
    );
    unknown("provided track", track);
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
        .map(|key| {
            if key.len() <= 96 && !key.chars().any(char::is_control) {
                key.clone()
            } else {
                "<invalid-key>".into()
            }
        })
        .collect::<Vec<_>>();
    keys.sort();
    keys.truncate(128);
    keys
}

fn spotify_id(uri: Option<&str>) -> String {
    let Some(uri) = uri.filter(|uri| uri.starts_with("spotify:")) else {
        return "<redacted>".into();
    };
    let id = uri.rsplit(':').next().unwrap_or_default();
    if id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
        id.into()
    } else {
        "<redacted>".into()
    }
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
}
