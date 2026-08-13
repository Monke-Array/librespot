use serde_json::{Map, Value};
use std::collections::BTreeSet;

const PREFIX: &str = "[spotify-mix-debug]";
const FIRST_TRACKS: usize = 8;
const MAX_KEYS: usize = 128;
const RECIPE_ATTRIBUTE: &str = "automix.auto_transition_recipe";
const BACKEND_RECIPE_ATTRIBUTE: &str = "automix.backend_auto_transition";

pub(crate) fn log_context_json(source: &str, json: &str) {
    if !enabled() {
        return;
    }
    match serde_json::from_str(json) {
        Ok(value) => log_context(source, &value),
        Err(why) => debug!("{PREFIX} context source={source} structural parse failed: {why}"),
    }
}

pub(crate) fn log_player_command_json(json: &str) {
    if !enabled() {
        return;
    }
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        debug!("{PREFIX} received message transport=dealer type=player-command invalid-json");
        return;
    };
    let Some(command) = root.get("command").and_then(Value::as_object) else {
        debug!("{PREFIX} received message transport=dealer type=player-command no-command-object");
        return;
    };
    let endpoint = command
        .get("endpoint")
        .and_then(Value::as_str)
        .and_then(|value| safe_scalar("endpoint", value))
        .unwrap_or_else(|| "unknown".into());
    debug!("{PREFIX} received message transport=dealer type=player-command endpoint={endpoint}");
    log_fields(&format!("command endpoint={endpoint}"), command, &[]);

    if let Some(context) = command.get("context") {
        log_context("dealer-player-command", context);
    }
    if let Some(origin) = get(command, &["play_origin", "playOrigin"]).and_then(Value::as_object) {
        log_fields("play origin", origin, &[]);
        debug!(
            "{PREFIX} play origin feature={} version={} classes={:?}",
            scalar(origin, &["feature_identifier", "featureIdentifier"]),
            scalar(origin, &["feature_version", "featureVersion"]),
            string_array(origin, &["feature_classes", "featureClasses"])
        );
    }
    if let Some(options) = command.get("options").and_then(Value::as_object) {
        log_fields("command options", options, &[]);
        if let Some(player_options) = get(
            options,
            &["player_options_override", "playerOptionsOverride"],
        )
        .and_then(Value::as_object)
        {
            log_fields("player options", player_options, &[]);
            log_map_member("player option modes", player_options, "modes", true);
        }
    }
    log_map_member("command modes", command, "modes", true);
    if let Some(track) = command.get("track") {
        log_track("command track", 0, track);
    }
    for name in ["next_tracks", "nextTracks", "prev_tracks", "prevTracks"] {
        if let Some(tracks) = command.get(name).and_then(Value::as_array) {
            log_tracks(&format!("command {name}"), tracks);
        }
    }
}

fn log_context(source: &str, value: &Value) {
    let Some(context) = value.as_object() else {
        return;
    };
    let (kind, id) = uri_identity(context.get("uri").and_then(Value::as_str));
    log_fields(
        &format!("context source={source} type={kind} id={id}"),
        context,
        &[
            "uri",
            "url",
            "metadata",
            "restrictions",
            "pages",
            "loading",
            "is_loading",
            "isLoading",
        ],
    );
    log_map_member("context metadata", context, "metadata", false);
    let Some(pages) = context.get("pages").and_then(Value::as_array) else {
        return;
    };
    for (page_index, page) in pages.iter().filter_map(Value::as_object).enumerate() {
        log_fields(
            &format!("page index={page_index}"),
            page,
            &[
                "page_url",
                "pageUrl",
                "next_page_url",
                "nextPageUrl",
                "metadata",
                "tracks",
                "loading",
                "is_loading",
                "isLoading",
            ],
        );
        log_map_member(
            &format!("page {page_index} metadata"),
            page,
            "metadata",
            false,
        );
        if let Some(tracks) = page.get("tracks").and_then(Value::as_array) {
            log_tracks(&format!("page {page_index} track"), tracks);
        }
    }
}

fn log_tracks(label: &str, tracks: &[Value]) {
    let mut shown = 0;
    for (index, track) in tracks.iter().enumerate() {
        if index < FIRST_TRACKS || diagnostic_track(track) {
            log_track(label, index, track);
            shown += 1;
        }
    }
    if shown < tracks.len() {
        debug!(
            "{PREFIX} {label}s summarized shown={shown} total={}",
            tracks.len()
        );
    }
}

fn log_track(label: &str, index: usize, value: &Value) {
    let Some(track) = value.as_object() else {
        return;
    };
    let (kind, id) = uri_identity(track.get("uri").and_then(Value::as_str));
    log_fields(
        &format!("{label} index={index} type={kind} id={id}"),
        track,
        &[
            "uri",
            "uid",
            "gid",
            "metadata",
            "removed",
            "blocked",
            "provider",
            "restrictions",
            "album_uri",
            "albumUri",
            "disallow_reasons",
            "disallowReasons",
            "artist_uri",
            "artistUri",
        ],
    );
    let metadata_keys = get(track, &["metadata"])
        .and_then(Value::as_object)
        .map(keys)
        .unwrap_or_default();
    let format_attribute_keys = format_attribute_keys(track);
    debug!(
        "{PREFIX} {label} index={index} uri={} uid={}",
        scalar(track, &["uri"]),
        scalar(track, &["uid"])
    );
    debug!(
        "{PREFIX} {label} {index} metadata_keys={metadata_keys:?} format_attribute_keys={format_attribute_keys:?}"
    );
    debug!(
        "{PREFIX} {label} {index} recipe_present={} backend_recipe_present={}",
        has_track_attribute(track, RECIPE_ATTRIBUTE),
        has_track_attribute(track, BACKEND_RECIPE_ATTRIBUTE)
    );
}

fn format_attribute_keys(track: &Map<String, Value>) -> Vec<String> {
    let mut keys = BTreeSet::new();
    for name in [
        "format_list_attributes",
        "formatListAttributes",
        "format_attributes",
        "formatAttributes",
    ] {
        match track.get(name) {
            Some(Value::Object(attributes)) => {
                keys.extend(attributes.keys().cloned());
            }
            Some(Value::Array(attributes)) => {
                keys.extend(attributes.iter().filter_map(|attribute| {
                    attribute
                        .get("key")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                }));
            }
            _ => {}
        }
    }
    keys.into_iter().take(MAX_KEYS).collect()
}

fn has_track_attribute(track: &Map<String, Value>, name: &str) -> bool {
    track
        .get("metadata")
        .and_then(Value::as_object)
        .is_some_and(|metadata| metadata.contains_key(name))
        || format_attribute_keys(track).iter().any(|key| key == name)
}

fn log_fields(label: &str, object: &Map<String, Value>, known: &[&str]) {
    let keys = keys(object);
    let unknown = keys
        .iter()
        .filter(|key| !known.is_empty() && !known.contains(&key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    debug!("{PREFIX} {label} field keys={keys:?} unknown fields present={unknown:?}");
}

fn log_map_member(label: &str, object: &Map<String, Value>, name: &str, all_values: bool) {
    let Some(map) = object.get(name).and_then(Value::as_object) else {
        return;
    };
    debug!("{PREFIX} {label} keys={:?}", keys(map));
    let mut values = map
        .iter()
        .filter(|(key, _)| all_values || mix_key(key))
        .filter_map(|(key, value)| {
            safe_metadata_scalar(key, value.as_str()?).map(|value| format!("{key}={value}"))
        })
        .collect::<Vec<_>>();
    values.sort();
    values.truncate(MAX_KEYS);
    if !values.is_empty() {
        debug!("{PREFIX} {label} relevant scalar values={values:?}");
    }
}

fn diagnostic_track(value: &Value) -> bool {
    let metadata_is_diagnostic = value
        .get("metadata")
        .and_then(Value::as_object)
        .is_some_and(|map| map.keys().any(|key| mix_key(key)));
    let format_attributes_are_present = value
        .as_object()
        .is_some_and(|track| !format_attribute_keys(track).is_empty());

    metadata_is_diagnostic || format_attributes_are_present
}

fn keys(object: &Map<String, Value>) -> Vec<String> {
    let mut keys = object
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
    keys.truncate(MAX_KEYS);
    keys
}

fn get<'a>(object: &'a Map<String, Value>, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|name| object.get(*name))
}

fn scalar(object: &Map<String, Value>, names: &[&str]) -> String {
    get(object, names)
        .and_then(Value::as_str)
        .and_then(|value| safe_scalar(names[0], value))
        .unwrap_or_else(|| "<unset-or-redacted>".into())
}

fn string_array(object: &Map<String, Value>, names: &[&str]) -> Vec<String> {
    get(object, names)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_KEYS)
        .filter_map(Value::as_str)
        .filter_map(|value| safe_scalar("feature_class", value))
        .collect()
}

fn uri_identity(uri: Option<&str>) -> (&'static str, String) {
    let Some(rest) = uri.and_then(|uri| uri.strip_prefix("spotify:")) else {
        return ("non-spotify-or-unset", "<redacted>".into());
    };
    let parts = rest.split(':').collect::<Vec<_>>();
    let kind = [
        "playlist",
        "track",
        "episode",
        "album",
        "artist",
        "show",
        "station",
        "collection",
    ]
    .into_iter()
    .find(|kind| parts.contains(kind))
    .unwrap_or("other");
    let id = parts.last().copied().unwrap_or_default();
    let id = if id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
        id.into()
    } else {
        "<redacted>".into()
    };
    (kind, id)
}

fn mix_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "mix",
        "transition",
        "crossfade",
        "automix",
        "bpm",
        "tempo",
        "key",
        "beat",
        "cue",
        "fade",
        "curve",
        "overlap",
        "offset",
        "analysis",
        "loudness",
        "energy",
        "waveform",
        "filter",
        "equalizer",
    ]
    .iter()
    .any(|term| key.contains(term))
        || key == "eq"
}

fn safe_scalar(key: &str, value: &str) -> Option<String> {
    let key = key.to_ascii_lowercase();
    let secret = [
        "token",
        "auth",
        "cookie",
        "credential",
        "password",
        "secret",
        "signature",
    ]
    .iter()
    .any(|term| key.contains(term));
    (!secret
        && !value.is_empty()
        && value.len() <= 96
        && !value.contains("://")
        && !value.contains('?')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._:/+-".contains(c)))
    .then(|| value.into())
}

fn safe_metadata_scalar(key: &str, value: &str) -> Option<String> {
    let value = safe_scalar(key, value)?;
    let normalized = value.to_ascii_lowercase();
    let known_enum = [
        "true",
        "false",
        "none",
        "default",
        "regular",
        "airbag",
        "radio_airbag",
        "sleep",
        "mixed",
        "custom",
        "heuristic",
        "backend",
        "cuepoints",
        "crossfade",
        "gapless",
        "heuristic_transition",
        "backend_transition",
    ]
    .contains(&normalized.as_str());
    (known_enum || value.parse::<f64>().is_ok()).then_some(value)
}

fn enabled() -> bool {
    log::log_enabled!(log::Level::Debug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_sanitizer_rejects_secrets_and_urls() {
        assert_eq!(safe_metadata_scalar("tempo", "128.0"), Some("128.0".into()));
        assert_eq!(
            safe_metadata_scalar("automix", "mixed"),
            Some("mixed".into())
        );
        assert_eq!(safe_metadata_scalar("automix", "opaqueReusableBlob"), None);
        assert_eq!(safe_scalar("access_token", "reusable-secret"), None);
        assert_eq!(safe_scalar("mix_url", "https://example.test/?sig=x"), None);
    }

    #[test]
    fn format_attribute_diagnostics_log_keys_not_values() {
        let track = serde_json::json!({
            "metadata": {"ordinary": "metadata-value"},
            "formatListAttributes": {
                RECIPE_ATTRIBUTE: "must-not-be-logged",
                "item.speed": "1.05"
            }
        });
        let track = track.as_object().expect("object");

        assert_eq!(
            format_attribute_keys(track),
            [RECIPE_ATTRIBUTE.to_owned(), "item.speed".to_owned()]
        );
        assert!(has_track_attribute(track, RECIPE_ATTRIBUTE));
        assert!(!format!("{:?}", format_attribute_keys(track)).contains("must-not-be-logged"));
    }
}
