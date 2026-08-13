use std::{
    env, fs,
    ops::Deref,
    path::{Path, PathBuf},
};

fn out_dir() -> PathBuf {
    Path::new(&env::var("OUT_DIR").expect("env")).to_path_buf()
}

fn cleanup() {
    let _ = fs::remove_dir_all(out_dir());
}

fn compile() {
    let proto_dir = Path::new(&env::var("CARGO_MANIFEST_DIR").expect("env")).join("proto");

    let files = &[
        proto_dir.join("connect.proto"),
        proto_dir.join("media.proto"),
        proto_dir.join("connectivity.proto"),
        proto_dir.join("devices.proto"),
        proto_dir.join("entity_extension_data.proto"),
        proto_dir.join("extended_metadata.proto"),
        proto_dir.join("extension_kind.proto"),
        proto_dir.join("metadata.proto"),
        proto_dir.join("player.proto"),
        proto_dir.join("playlist_annotate3.proto"),
        proto_dir.join("playlist_permission.proto"),
        proto_dir.join("playlist4_external.proto"),
        proto_dir.join("playlist_get_request.proto"),
        proto_dir.join("playlist_query.proto"),
        proto_dir.join("playlist_request.proto"),
        proto_dir.join("playlist_playlist_state.proto"),
        proto_dir.join("playlist_track_state.proto"),
        proto_dir.join("playlist_user_state.proto"),
        proto_dir.join("response_status.proto"),
        proto_dir.join("collection/episode_collection_state.proto"),
        proto_dir.join("metadata/episode_metadata.proto"),
        proto_dir.join("metadata/extension.proto"),
        proto_dir.join("metadata/image_group.proto"),
        proto_dir.join("metadata/track_metadata.proto"),
        proto_dir.join("played_state/episode_played_state.proto"),
        proto_dir.join("played_state/playability_restriction.proto"),
        proto_dir.join("played_state/track_played_state.proto"),
        proto_dir.join("policy/album_decoration_policy.proto"),
        proto_dir.join("policy/artist_decoration_policy.proto"),
        proto_dir.join("policy/episode_decoration_policy.proto"),
        proto_dir.join("policy/playlist_album_decoration_policy.proto"),
        proto_dir.join("policy/playlist_decoration_policy.proto"),
        proto_dir.join("policy/playlist_episode_decoration_policy.proto"),
        proto_dir.join("policy/playlist_request_decoration_policy.proto"),
        proto_dir.join("policy/playlist_track_decoration_policy.proto"),
        proto_dir.join("policy/show_decoration_policy.proto"),
        proto_dir.join("policy/supported_link_types_in_playlists.proto"),
        proto_dir.join("policy/track_decoration_policy.proto"),
        proto_dir.join("policy/user_decoration_policy.proto"),
        proto_dir.join("sync/episode_sync_state.proto"),
        proto_dir.join("on_demand_in_free_reason.proto"),
        proto_dir.join("lens-model.proto"),
        proto_dir.join("signal-model.proto"),
        proto_dir.join("spotify/clienttoken/v0/clienttoken_http.proto"),
        proto_dir.join("spotify/login5/v3/challenges/code.proto"),
        proto_dir.join("spotify/login5/v3/challenges/hashcash.proto"),
        proto_dir.join("spotify/login5/v3/client_info.proto"),
        proto_dir.join("spotify/login5/v3/credentials/credentials.proto"),
        proto_dir.join("spotify/login5/v3/identifiers/identifiers.proto"),
        proto_dir.join("spotify/login5/v3/login5.proto"),
        proto_dir.join("spotify/login5/v3/user_info.proto"),
        proto_dir.join("storage-resolve.proto"),
        proto_dir.join("user_attributes.proto"),
        proto_dir.join("autoplay_context_request.proto"),
        proto_dir.join("automix_transition.proto"),
        proto_dir.join("social_connect_v2.proto"),
        proto_dir.join("transfer_state.proto"),
        proto_dir.join("context_player_options.proto"),
        proto_dir.join("playback.proto"),
        proto_dir.join("play_history.proto"),
        proto_dir.join("session.proto"),
        proto_dir.join("queue.proto"),
        proto_dir.join("context_track.proto"),
        proto_dir.join("context.proto"),
        proto_dir.join("restrictions.proto"),
        proto_dir.join("context_page.proto"),
        proto_dir.join("play_origin.proto"),
        proto_dir.join("suppressions.proto"),
        proto_dir.join("instrumentation_params.proto"),
        // TODO: remove these legacy protobufs when we are on the new API completely
        proto_dir.join("authentication.proto"),
        proto_dir.join("canvaz.proto"),
        proto_dir.join("canvaz-meta.proto"),
        proto_dir.join("explicit_content_pubsub.proto"),
        proto_dir.join("keyexchange.proto"),
        proto_dir.join("mercury.proto"),
        proto_dir.join("pubsub.proto"),
    ];

    let slices = files.iter().map(Deref::deref).collect::<Vec<_>>();

    let out_dir = out_dir();
    fs::create_dir(&out_dir).expect("create_dir");

    protobuf_codegen::Codegen::new()
        .pure()
        .out_dir(&out_dir)
        .inputs(&slices)
        .include(&proto_dir)
        .run()
        .expect("Codegen failed.");
}

fn main() {
    cleanup();
    compile();
}
