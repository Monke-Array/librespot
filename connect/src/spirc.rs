use crate::{
    LoadContextOptions, LoadRequestOptions, PlayContext, capability_debug,
    context_resolver::{ContextAction, ContextFailureKind, ContextResolver, ResolveContext},
    core::{
        Error, Session, SpotifyUri,
        authentication::Credentials,
        dealer::{
            manager::{BoxedStream, BoxedStreamResult, Reply, RequestReply},
            protocol::{Command, FallbackWrapper, Message, Request},
        },
        session::UserAttributes,
        spclient::TransferRequest,
    },
    mix_debug,
    model::{LoadRequest, PlayingTrack, SpircPlayStatus},
    playback::{
        mixer::Mixer,
        player::{Player, PlayerEvent, PlayerEventChannel},
    },
    protocol::{
        connect::{Cluster, ClusterUpdate, LogoutCommand, SetVolumeCommand},
        context::Context,
        explicit_content_pubsub::UserAttributesUpdate,
        playlist4_external::PlaylistModificationInfo,
        social_connect_v2::SessionUpdate,
        transfer_state::TransferState,
        user_attributes::UserAttributesMutation,
    },
    spotify_auto_mix_metadata::AutoTrackIdentity,
    spotify_auto_mix_selection::{
        LocalAutoError, LocalAutoPairKey, LocalAutoRequest, LocalAutoTransition,
        generate_local_auto,
    },
    spotify_mix_hydration::{
        CacheLookup, HydrationResult, TransitionDataClient, TransitionHydrationCache,
        TransitionHydrationKey, hydrate_transition_data, transition_uri,
    },
    state::{
        context::{ContextType, ResetContext},
        provider::IsProvider,
        {ConnectConfig, ConnectState},
    },
};
use futures_util::StreamExt;
use librespot_protocol::context_page::ContextPage;
use librespot_protocol::player::ProvidedTrack;
use protobuf::MessageField;
use std::{
    future::Future,
    sync::Arc,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinSet,
    time::sleep,
};

#[derive(Debug, Error)]
enum SpircError {
    #[error("response payload empty")]
    NoData,
    #[error("{0} had no uri")]
    NoUri(&'static str),
    #[error("message pushed for another URI")]
    InvalidUri(String),
    #[error("failed to put connect state for new device")]
    FailedDealerSetup,
    #[error("unknown endpoint: {0:#?}")]
    UnknownEndpoint(serde_json::Value),
}

struct LocalAutoTaskResult {
    key: LocalAutoPairKey,
    result: Result<LocalAutoTransition, LocalAutoError>,
}

#[derive(Clone)]
struct PreparedLocalAutoTransition {
    selected: LocalAutoTransition,
    plan: librespot_playback::TransitionPlan,
}

struct LocalAutoPairState {
    key: LocalAutoPairKey,
    higher_priority: HigherPriorityTransitionState,
    attempted: bool,
    incoming_identity: Option<AutoTrackIdentity>,
    prepared_transition: Option<PreparedLocalAutoTransition>,
    selected_local_auto: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HigherPriorityTransitionState {
    SavedSelected,
    Pending,
    Exhausted,
}

fn local_auto_is_eligible(state: HigherPriorityTransitionState) -> bool {
    !matches!(state, HigherPriorityTransitionState::SavedSelected)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CachedLocalAutoSelectionGate {
    Select,
    Suppressed,
    Stale,
    MissedWindow,
}

fn cached_local_auto_selection_gate(
    higher_priority: HigherPriorityTransitionState,
    selected_local_auto: bool,
    prepared_key: &LocalAutoPairKey,
    active_key: &LocalAutoPairKey,
    plan_can_still_apply: bool,
) -> CachedLocalAutoSelectionGate {
    if !local_auto_is_eligible(higher_priority) || selected_local_auto {
        return CachedLocalAutoSelectionGate::Suppressed;
    }
    if prepared_key != active_key {
        return CachedLocalAutoSelectionGate::Stale;
    }
    if !plan_can_still_apply {
        return CachedLocalAutoSelectionGate::MissedWindow;
    }
    CachedLocalAutoSelectionGate::Select
}

fn mixer_auto_edge_is_eligible(
    has_transition_plan: bool,
    has_transition_uri: bool,
    has_backend_auto_transition: bool,
    context_is_mixer: bool,
) -> bool {
    has_transition_plan || has_transition_uri || has_backend_auto_transition || context_is_mixer
}

impl From<SpircError> for Error {
    fn from(err: SpircError) -> Self {
        use SpircError::*;
        match err {
            NoData | NoUri(_) => Error::unavailable(err),
            InvalidUri(_) | FailedDealerSetup => Error::aborted(err),
            UnknownEndpoint(_) => Error::unimplemented(err),
        }
    }
}

struct SpircTask {
    player: Arc<Player>,
    mixer: Arc<dyn Mixer>,

    /// the state management object
    connect_state: ConnectState,
    connect_established: bool,

    play_request_id: Option<u64>,
    play_status: SpircPlayStatus,

    connection_id_update: BoxedStreamResult<String>,
    connect_state_update: BoxedStreamResult<ClusterUpdate>,
    connect_state_volume_update: BoxedStreamResult<SetVolumeCommand>,
    connect_state_logout_request: BoxedStreamResult<LogoutCommand>,
    playlist_update: BoxedStreamResult<PlaylistModificationInfo>,
    session_update: BoxedStreamResult<FallbackWrapper<SessionUpdate>>,
    connect_state_command: BoxedStream<RequestReply>,
    user_attributes_update: BoxedStreamResult<UserAttributesUpdate>,
    user_attributes_mutation: BoxedStreamResult<UserAttributesMutation>,

    commands: Option<mpsc::UnboundedReceiver<SpircCommand>>,
    player_events: Option<PlayerEventChannel>,

    context_resolver: ContextResolver,
    transition_hydration_cache: TransitionHydrationCache,
    transition_hydrations: JoinSet<HydrationResult>,
    local_auto_pair: Option<LocalAutoPairState>,
    local_auto_current_identity: Option<AutoTrackIdentity>,
    local_auto_tasks: JoinSet<LocalAutoTaskResult>,

    shutdown: bool,
    session: Session,
    credentials: Credentials,
    retain_on_session_failure: bool,

    /// is set when transferring, and used after resolving the contexts to finish the transfer
    pub transfer_state: Option<TransferState>,

    /// when set to true, it will update the volume after [VOLUME_UPDATE_DELAY],
    /// when no other future resolves, otherwise resets the delay
    update_volume: bool,

    /// when set to true, it will update the volume after [UPDATE_STATE_DELAY],
    /// when no other future resolves, otherwise resets the delay
    update_state: bool,

    spirc_id: usize,
}

static SPIRC_COUNTER: AtomicUsize = AtomicUsize::new(0);

enum SpircCommand {
    Play,
    PlayPause,
    Pause,
    Prev,
    Next,
    VolumeUp,
    VolumeDown,
    Shutdown,
    Shuffle(bool),
    Repeat(bool),
    RepeatTrack(bool),
    Disconnect {
        pause: bool,
    },
    SetPosition(u32),
    SetVolume(u16),
    Activate,
    Transfer(Option<TransferRequest>),
    Load(LoadRequest),
    ReplaceSession {
        session: Session,
        result: oneshot::Sender<Result<(), Error>>,
    },
}

impl SpircCommand {
    fn name(&self) -> &'static str {
        match self {
            Self::Play => "Play",
            Self::PlayPause => "PlayPause",
            Self::Pause => "Pause",
            Self::Prev => "Prev",
            Self::Next => "Next",
            Self::VolumeUp => "VolumeUp",
            Self::VolumeDown => "VolumeDown",
            Self::Shutdown => "Shutdown",
            Self::Shuffle(_) => "Shuffle",
            Self::Repeat(_) => "Repeat",
            Self::RepeatTrack(_) => "RepeatTrack",
            Self::Disconnect { .. } => "Disconnect",
            Self::SetPosition(_) => "SetPosition",
            Self::SetVolume(_) => "SetVolume",
            Self::Activate => "Activate",
            Self::Transfer(_) => "Transfer",
            Self::Load(_) => "Load",
            Self::ReplaceSession { .. } => "ReplaceSession",
        }
    }
}

struct SpircSessionBindings {
    connection_id_update: BoxedStreamResult<String>,
    connect_state_update: BoxedStreamResult<ClusterUpdate>,
    connect_state_volume_update: BoxedStreamResult<SetVolumeCommand>,
    connect_state_logout_request: BoxedStreamResult<LogoutCommand>,
    playlist_update: BoxedStreamResult<PlaylistModificationInfo>,
    session_update: BoxedStreamResult<FallbackWrapper<SessionUpdate>>,
    connect_state_command: BoxedStream<RequestReply>,
    user_attributes_update: BoxedStreamResult<UserAttributesUpdate>,
    user_attributes_mutation: BoxedStreamResult<UserAttributesMutation>,
}

const CONTEXT_FETCH_THRESHOLD: usize = 2;

#[derive(Debug, PartialEq, Eq)]
enum PlayerQueueAction {
    Ignore,
    Continue,
    Preserve,
    Advance,
    Unavailable { track_id: SpotifyUri, advance: bool },
}

fn player_queue_action(
    event: &PlayerEvent,
    play_request_id: &mut Option<u64>,
    current_uri: &str,
) -> Result<PlayerQueueAction, Error> {
    let is_current_request = matches! {
        (event.get_play_request_id(), *play_request_id),
        (Some(event_id), Some(current_id)) if event_id == current_id
    };

    if !is_current_request {
        return Ok(PlayerQueueAction::Ignore);
    }

    Ok(match event {
        PlayerEvent::EndOfTrack { .. } => {
            // Consume the request before advancing. A duplicate terminal event is then stale even
            // if the next Player command has not published its request id yet.
            *play_request_id = None;
            PlayerQueueAction::Advance
        }
        PlayerEvent::Unavailable { track_id, .. } => {
            let advance = track_id.to_uri()? == current_uri;
            if advance {
                *play_request_id = None;
            }
            PlayerQueueAction::Unavailable {
                track_id: track_id.clone(),
                advance,
            }
        }
        PlayerEvent::LoadFailed { .. } => PlayerQueueAction::Preserve,
        _ => PlayerQueueAction::Continue,
    })
}

// delay to update volume after a certain amount of time, instead on each update request
const VOLUME_UPDATE_DELAY: Duration = Duration::from_millis(500);
// to reduce updates to remote, we group some request by waiting for a set amount of time
const UPDATE_STATE_DELAY: Duration = Duration::from_millis(200);

/// The spotify connect handle
pub struct Spirc {
    commands: mpsc::UnboundedSender<SpircCommand>,
}

impl Spirc {
    /// Initializes a new spotify connect device
    ///
    /// The returned tuple consists out of a handle to the [`Spirc`] that
    /// can control the local connect device when active. And a [`Future`]
    /// which represents the [`Spirc`] event loop that processes the whole
    /// connect device logic.
    pub async fn new(
        config: ConnectConfig,
        session: Session,
        credentials: Credentials,
        player: Arc<Player>,
        mixer: Arc<dyn Mixer>,
    ) -> Result<(Spirc, impl Future<Output = ()>), Error> {
        Self::new_inner(config, session, credentials, player, mixer, false).await
    }

    /// Initializes a Connect device whose Player and Connect state can survive AP replacement.
    ///
    /// Unlike [`Spirc::new`], the returned task remains alive when its Session fails and waits for
    /// [`Spirc::replace_session`]. Consumers must monitor their current Session and provide a new,
    /// unconnected Session when it becomes invalid.
    pub async fn new_reconnectable(
        config: ConnectConfig,
        session: Session,
        credentials: Credentials,
        player: Arc<Player>,
        mixer: Arc<dyn Mixer>,
    ) -> Result<(Spirc, impl Future<Output = ()>), Error> {
        Self::new_inner(config, session, credentials, player, mixer, true).await
    }

    async fn new_inner(
        config: ConnectConfig,
        session: Session,
        credentials: Credentials,
        player: Arc<Player>,
        mixer: Arc<dyn Mixer>,
        retain_on_session_failure: bool,
    ) -> Result<(Spirc, impl Future<Output = ()>), Error> {
        let spirc_id = SPIRC_COUNTER.fetch_add(1, Ordering::AcqRel);
        debug!("new Spirc[{spirc_id}]");

        let connect_state = ConnectState::new(config, &session);
        let bindings = SpircTask::connect_session(&session, credentials.clone()).await?;

        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

        let player_events = player.get_player_event_channel();

        let mut task = SpircTask {
            player,
            mixer,

            connect_state,
            connect_established: false,

            play_request_id: None,
            play_status: SpircPlayStatus::Stopped,

            connection_id_update: bindings.connection_id_update,
            connect_state_update: bindings.connect_state_update,
            connect_state_volume_update: bindings.connect_state_volume_update,
            connect_state_logout_request: bindings.connect_state_logout_request,
            playlist_update: bindings.playlist_update,
            session_update: bindings.session_update,
            connect_state_command: bindings.connect_state_command,
            user_attributes_update: bindings.user_attributes_update,
            user_attributes_mutation: bindings.user_attributes_mutation,
            commands: Some(cmd_rx),
            player_events: Some(player_events),

            context_resolver: ContextResolver::new(session.clone()),
            transition_hydration_cache: TransitionHydrationCache::default(),
            transition_hydrations: JoinSet::new(),
            local_auto_pair: None,
            local_auto_current_identity: None,
            local_auto_tasks: JoinSet::new(),

            shutdown: false,
            session,
            credentials,
            retain_on_session_failure,

            transfer_state: None,
            update_volume: false,
            update_state: false,

            spirc_id,
        };

        let spirc = Spirc { commands: cmd_tx };

        let initial_volume = task.connect_state.device_info().volume;
        task.connect_state.set_volume(0);

        match initial_volume.try_into() {
            Ok(volume) => {
                task.set_volume(volume);
                // we don't want to update the volume initially,
                // we just want to set the mixer to the correct volume
                task.update_volume = false;
            }
            Err(why) => error!("failed to update initial volume: {why}"),
        };

        Ok((spirc, Box::pin(task.run())))
    }

    /// Replace a failed AP session without reconstructing the player or Connect state.
    ///
    /// The Spirc task keeps its queue, context, play request, transfer, and playback bookkeeping.
    /// The replacement session is authenticated with the credentials used to create this Spirc,
    /// then installed into both Spirc and the retained [`Player`].
    pub async fn replace_session(&self, session: Session) -> Result<(), Error> {
        let (result_tx, result_rx) = oneshot::channel();
        self.commands.send(SpircCommand::ReplaceSession {
            session,
            result: result_tx,
        })?;
        result_rx.await?
    }

    /// Safely shutdowns the spirc.
    ///
    /// This pauses the playback, disconnects the connect device and
    /// bring the future initially returned to an end.
    pub fn shutdown(&self) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::Shutdown)?)
    }

    /// Resumes the playback
    ///
    /// Does nothing if we are not the active device, or it isn't paused.
    pub fn play(&self) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::Play)?)
    }

    /// Resumes or pauses the playback
    ///
    /// Does nothing if we are not the active device.
    pub fn play_pause(&self) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::PlayPause)?)
    }

    /// Pauses the playback
    ///
    /// Does nothing if we are not the active device, or if it isn't playing.
    pub fn pause(&self) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::Pause)?)
    }

    /// Seeks to the beginning or skips to the previous track.
    ///
    /// Seeks to the beginning when the current track position
    /// is greater than 3 seconds.
    ///
    /// Does nothing if we are not the active device.
    pub fn prev(&self) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::Prev)?)
    }

    /// Skips to the next track.
    ///
    /// Does nothing if we are not the active device.
    pub fn next(&self) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::Next)?)
    }

    /// Increases the volume by configured steps of [ConnectConfig].
    ///
    /// Does nothing if we are not the active device.
    pub fn volume_up(&self) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::VolumeUp)?)
    }

    /// Decreases the volume by configured steps of [ConnectConfig].
    ///
    /// Does nothing if we are not the active device.
    pub fn volume_down(&self) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::VolumeDown)?)
    }

    /// Shuffles the playback according to the value.
    ///
    /// If true shuffles/reshuffles the playback. Otherwise, does
    /// nothing (if not shuffled) or unshuffles the playback while
    /// resuming at the position of the current track.
    ///
    /// Does nothing if we are not the active device.
    pub fn shuffle(&self, shuffle: bool) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::Shuffle(shuffle))?)
    }

    /// Repeats the playback context according to the value.
    ///
    /// Does nothing if we are not the active device.
    pub fn repeat(&self, repeat: bool) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::Repeat(repeat))?)
    }

    /// Repeats the current track if true.
    ///
    /// Does nothing if we are not the active device.
    ///
    /// Skipping to the next track disables the repeating.
    pub fn repeat_track(&self, repeat: bool) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::RepeatTrack(repeat))?)
    }

    /// Update the volume to the given value.
    ///
    /// Does nothing if we are not the active device.
    pub fn set_volume(&self, volume: u16) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::SetVolume(volume))?)
    }

    /// Updates the position to the given value.
    ///
    /// Does nothing if we are not the active device.
    ///
    /// If value is greater than the track duration,
    /// the update is ignored.
    pub fn set_position_ms(&self, position_ms: u32) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::SetPosition(position_ms))?)
    }

    /// Load a new context and replace the current.
    ///
    /// Does nothing if we are not the active device.
    ///
    /// Does not overwrite the queue.
    pub fn load(&self, command: LoadRequest) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::Load(command))?)
    }

    /// Disconnects the current device and pauses the playback according the value.
    ///
    /// Does nothing if we are not the active device.
    pub fn disconnect(&self, pause: bool) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::Disconnect { pause })?)
    }

    /// Acquires the control as active connect device.
    ///
    /// Does not [Spirc::transfer] the playback. Does nothing if we are not the active device.
    pub fn activate(&self) -> Result<(), Error> {
        Ok(self.commands.send(SpircCommand::Activate)?)
    }

    /// Acquires the control as active connect device over the transfer flow.
    ///
    /// Does nothing if we are not the active device.
    pub fn transfer(&self, transfer_request: Option<TransferRequest>) -> Result<(), Error> {
        Ok(self
            .commands
            .send(SpircCommand::Transfer(transfer_request))?)
    }
}

fn extract_connection_id(msg: Message) -> Result<String, Error> {
    let connection_id = msg
        .headers
        .get("Spotify-Connection-Id")
        .ok_or_else(|| SpircError::InvalidUri(msg.uri.clone()))?;
    Ok(connection_id.to_owned())
}

fn preserve_session_identity(current: &Session, replacement: &Session) {
    replacement.set_session_id(&current.session_id());
}

impl SpircTask {
    async fn connect_session(
        session: &Session,
        credentials: Credentials,
    ) -> Result<SpircSessionBindings, Error> {
        let bindings = SpircSessionBindings {
            connection_id_update: session
                .dealer()
                .listen_for("hm://pusher/v1/connections/", extract_connection_id)?,
            connect_state_update: session
                .dealer()
                .listen_for("hm://connect-state/v1/cluster", Message::from_raw)?,
            connect_state_volume_update: session
                .dealer()
                .listen_for("hm://connect-state/v1/connect/volume", Message::from_raw)?,
            connect_state_logout_request: session
                .dealer()
                .listen_for("hm://connect-state/v1/connect/logout", Message::from_raw)?,
            playlist_update: session
                .dealer()
                .listen_for("hm://playlist/v2/playlist/", Message::from_raw)?,
            session_update: session
                .dealer()
                .listen_for("social-connect/v2/session_update", Message::try_from_json)?,
            connect_state_command: session
                .dealer()
                .handle_for("hm://connect-state/v1/player/command")?,
            user_attributes_update: session
                .dealer()
                .listen_for("spotify:user:attributes:update", Message::from_raw)?,
            user_attributes_mutation: session
                .dealer()
                .listen_for("spotify:user:attributes:mutated", Message::from_raw)?,
        };

        // Pre-acquire client_token, preventing multiple requests while running.
        let _ = session.spclient().client_token().await?;

        // Connect only after all Dealer message listeners are registered.
        session.connect(credentials, true).await?;

        // Pre-acquire access_token after authentication.
        let _ = session.login5().auth_token().await?;

        Ok(bindings)
    }

    async fn replace_session(&mut self, session: Session) -> Result<(), Error> {
        preserve_session_identity(&self.session, &session);

        let bindings = match Self::connect_session(&session, self.credentials.clone()).await {
            Ok(bindings) => bindings,
            Err(why) => {
                session.shutdown();
                return Err(why);
            }
        };
        if let Err(why) = session.dealer().start().await {
            session.shutdown();
            return Err(why);
        }

        if !self.session.is_invalid() {
            self.session.shutdown();
        }

        self.player.set_session(session.clone());
        self.context_resolver.set_session(session.clone());
        self.cancel_transition_hydrations();
        self.connection_id_update = bindings.connection_id_update;
        self.connect_state_update = bindings.connect_state_update;
        self.connect_state_volume_update = bindings.connect_state_volume_update;
        self.connect_state_logout_request = bindings.connect_state_logout_request;
        self.playlist_update = bindings.playlist_update;
        self.session_update = bindings.session_update;
        self.connect_state_command = bindings.connect_state_command;
        self.user_attributes_update = bindings.user_attributes_update;
        self.user_attributes_mutation = bindings.user_attributes_mutation;
        self.session = session;
        self.connect_established = false;

        debug!("Spirc session replaced in place; retained Connect state and player recovery state");
        Ok(())
    }

    async fn run(mut self) {
        // simplify unwrapping of received item or parsed result
        macro_rules! unwrap {
            ( $next:expr, |$some:ident| $use_some:expr ) => {
                match $next {
                    Some($some) => $use_some,
                    None => {
                        error!("{} selected, but none received", stringify!($next));
                        break;
                    }
                }
            };
            ( $next:expr, match |$ok:ident| $use_ok:expr ) => {
                unwrap!($next, |$ok| match $ok {
                    Ok($ok) => $use_ok,
                    Err(why) => error!("could not parse {}: {}", stringify!($ok), why),
                })
            };
        }

        if let Err(why) = self.session.dealer().start().await {
            error!("starting dealer failed: {why}");
            self.session.shutdown();
        }

        'session: loop {
            while !self.session.is_invalid() && !self.shutdown {
                let commands = self.commands.as_mut();
                let player_events = self.player_events.as_mut();

                // when state and volume update have a higher priority than context resolving
                // because of that the context resolving has to wait, so that the other tasks can finish
                let allow_context_resolving = !self.update_state && !self.update_volume;

                tokio::select! {
                    // startup of the dealer requires a connection_id, which is retrieved at the very beginning
                    connection_id_update = self.connection_id_update.next() => unwrap! {
                        connection_id_update,
                        match |connection_id| if let Err(why) = self.handle_connection_id_update(connection_id).await {
                            error!("failed handling connection id update: {why}");
                            break;
                        }
                    },
                    // main dealer update of any remote device updates
                    cluster_update = self.connect_state_update.next() => unwrap! {
                        cluster_update,
                        match |cluster_update| if let Err(e) = self.handle_cluster_update(cluster_update).await {
                            error!("could not dispatch connect state update: {e}");
                        }
                    },
                    // main dealer request handling (dealer expects an answer)
                    request = self.connect_state_command.next() => unwrap! {
                        request,
                        |request| if let Err(e) = self.handle_connect_state_request(request).await {
                            error!("couldn't handle connect state command: {e}");
                        }
                    },
                    // volume request handling is send separately (it's more like a fire forget)
                    volume_update = self.connect_state_volume_update.next() => unwrap! {
                        volume_update,
                        match |volume_update| match volume_update.volume.try_into() {
                            Ok(volume) => self.set_volume(volume),
                            Err(why) => error!("can't update volume, failed to parse i32 to u16: {why}")
                        }
                    },
                    logout_request = self.connect_state_logout_request.next() => unwrap! {
                        logout_request,
                        |logout_request| {
                            error!("received logout request, currently not supported: {logout_request:#?}");
                            // todo: call logout handling
                        }
                    },
                    playlist_update = self.playlist_update.next() => unwrap! {
                        playlist_update,
                        match |playlist_update| if let Err(why) = self.handle_playlist_modification(playlist_update) {
                            error!("failed to handle playlist modification: {why}")
                        }
                    },
                    user_attributes_update = self.user_attributes_update.next() => unwrap! {
                        user_attributes_update,
                        match |attributes| self.handle_user_attributes_update(attributes)
                    },
                    user_attributes_mutation = self.user_attributes_mutation.next() => unwrap! {
                        user_attributes_mutation,
                        match |attributes| self.handle_user_attributes_mutation(attributes)
                    },
                    session_update = self.session_update.next() => unwrap! {
                        session_update,
                        match |session_update| self.handle_session_update(session_update)
                    },
                    cmd = async { commands?.recv().await }, if commands.is_some() => if let Some(cmd) = cmd {
                        if let Err(e) = self.handle_command(cmd).await {
                            debug!("could not dispatch command: {e}");
                        }
                    },
                    event = async { player_events?.recv().await }, if player_events.is_some() => if let Some(event) = event {
                        if let Err(e) = self.handle_player_event(event) {
                            error!("could not dispatch player event: {e}");
                        }
                    },
                    _ = async { sleep(UPDATE_STATE_DELAY).await }, if self.update_state => {
                        self.update_state = false;

                        if let Err(why) = self.notify().await {
                            error!("state update: {why}")
                        }
                    },
                    _ = async { sleep(VOLUME_UPDATE_DELAY).await }, if self.update_volume => {
                        self.update_volume = false;

                        info!("delayed volume update for all devices: volume is now {}", self.connect_state.device_info().volume);
                        if let Err(why) = self.connect_state.notify_volume_changed(&self.session).await {
                            error!("error updating connect state for volume update: {why}")
                        }

                        // for some reason the web-player does need two separate updates, so that the
                        // position of the current track is retained, other clients also send a state
                        // update before they send the volume update
                        if let Err(why) = self.notify().await {
                            error!("error updating connect state for volume update: {why}")
                        }
                    },
                    // context resolver handling, the idea/reason behind it the following:
                    //
                    // when we request a context that has multiple pages (for example an artist)
                    // resolving all pages at once can take around ~1-30sec, when we resolve
                    // everything at once that would block our main loop for that time
                    //
                    // to circumvent this behavior, we request each context separately here and
                    // finish after we received our last item of a type
                    next_context = async {
                        self.context_resolver.get_next_context(|| {
                            // Sending local file URIs to this endpoint results in a Bad Request status.
                            // It's likely appropriate to filter them out anyway; Spotify's backend
                            // has no knowledge about these tracks and so can't do anything with them.
                            self.connect_state.recent_track_uris()
                                .into_iter()
                                .filter(|t| !t.starts_with("spotify:local"))
                                .collect::<Vec<_>>()
                        }).await
                    }, if allow_context_resolving && self.context_resolver.has_next() => {
                        let update_state = self.handle_next_context(next_context);
                        if update_state {
                            if let Err(why) = self.notify().await {
                                error!("update after context resolving failed: {why}")
                            }
                        }
                    },
                    hydration = self.transition_hydrations.join_next(), if !self.transition_hydrations.is_empty() => {
                        match hydration {
                            Some(Ok(hydration)) => self.handle_transition_hydration(hydration),
                            Some(Err(error)) if !error.is_cancelled() => {
                                debug!("[spotify-mix] hydration task failed: {error}")
                            }
                            _ => {}
                        }
                    },
                    auto = self.local_auto_tasks.join_next(), if !self.local_auto_tasks.is_empty() => {
                        match auto {
                            Some(Ok(result)) => self.handle_local_auto_result(result),
                            Some(Err(error)) if !error.is_cancelled() => {
                                debug!("[spotify-auto] calculation task failed: {error}")
                            }
                            _ => {}
                        }
                    },
                    else => break
                }
            }

            if self.shutdown {
                break;
            }

            if !self.retain_on_session_failure {
                if self.connect_state.is_active() {
                    warn!("unexpected shutdown");
                    if let Err(why) = self.handle_disconnect().await {
                        error!("error during disconnecting: {why}")
                    }
                }
                break;
            }

            if !self.session.is_invalid() {
                warn!("Spirc session streams ended unexpectedly; invalidating the old session");
                self.session.shutdown();
            }
            self.connect_established = false;
            debug!("Spirc is retaining Connect state while waiting for a replacement Session");

            loop {
                let command = match self.commands.as_mut() {
                    Some(commands) => commands.recv().await,
                    None => None,
                };
                let Some(command) = command else {
                    self.shutdown = true;
                    break;
                };

                match command {
                    SpircCommand::ReplaceSession { session, result } => {
                        let replacement = self.replace_session(session).await;
                        let succeeded = replacement.is_ok();
                        let _ = result.send(replacement);
                        if succeeded {
                            continue 'session;
                        }
                    }
                    SpircCommand::Shutdown => {
                        self.handle_pause();
                        self.shutdown = true;
                        if let Some(commands) = self.commands.as_mut() {
                            commands.close();
                        }
                        break;
                    }
                    command => warn!(
                        "SpircCommand::{} ignored while waiting for a replacement Session",
                        command.name()
                    ),
                }
            }

            if self.shutdown {
                break;
            }
        }

        // this should clear the active session id, leaving an empty state
        if !self.session.is_invalid() {
            if let Err(why) = self.session.spclient().delete_connect_state_request().await {
                error!("error during connect state deletion: {why}")
            }
        }

        self.session.dealer().close().await;
    }

    fn handle_next_context(&mut self, next_context: Result<Context, Error>) -> bool {
        let next_context = match next_context {
            Err(why) => {
                let kind = self.context_resolver.classify_failure(&why);
                self.context_resolver.mark_next_unavailable(kind);
                if kind == ContextFailureKind::Permanent {
                    self.context_resolver.remove_used_and_invalid();
                }
                debug!("Context resolution failed as {kind:?}: {why}");
                return false;
            }
            Ok(ctx) => ctx,
        };

        self.context_resolver.mark_next_resolved();

        debug!("handling next context {:?}", next_context.uri);

        match self
            .context_resolver
            .apply_next_context(&mut self.connect_state, next_context)
        {
            Ok(remaining) => {
                if let Some(remaining) = remaining {
                    self.context_resolver.add_list(remaining)
                }
            }
            Err(why) => {
                error!("{why}")
            }
        }

        let update_state = if self
            .context_resolver
            .try_finish(&mut self.connect_state, &mut self.transfer_state)
        {
            self.add_autoplay_resolving_when_required();
            true
        } else {
            false
        };

        self.context_resolver.remove_used_and_invalid();
        update_state
    }

    // todo: is the time_delta still necessary?
    fn now_ms(&self) -> i64 {
        let dur = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|err| err.duration());

        dur.as_millis() as i64 + 1000 * self.session.time_delta()
    }

    async fn handle_command(&mut self, cmd: SpircCommand) -> Result<(), Error> {
        let command_name = cmd.name();
        trace!("Received SpircCommand::{command_name}");
        match cmd {
            SpircCommand::ReplaceSession { session, result } => {
                let replacement = self.replace_session(session).await;
                let _ = result.send(replacement);
                return Ok(());
            }
            SpircCommand::Shutdown => {
                trace!("Received SpircCommand::Shutdown");
                self.handle_pause();
                self.handle_disconnect().await?;
                self.shutdown = true;
                if let Some(rx) = self.commands.as_mut() {
                    rx.close()
                }
            }
            SpircCommand::Transfer(request) if !self.connect_state.is_active() => {
                let device_id = self.session.device_id();
                self.session
                    .spclient()
                    .transfer(device_id, device_id, request.as_ref())
                    .await?;
                return Ok(());
            }
            SpircCommand::Activate if !self.connect_state.is_active() => {
                trace!("Received SpircCommand::{command_name}");
                self.handle_activate();
                return self.notify().await;
            }
            SpircCommand::Transfer(..) | SpircCommand::Activate => {
                warn!("SpircCommand::{command_name} will be ignored while already active")
            }
            _ if !self.connect_state.is_active() => {
                warn!("SpircCommand::{command_name} will be ignored while Not Active")
            }
            SpircCommand::Disconnect { pause } => {
                if pause {
                    self.handle_pause()
                }
                return self.handle_disconnect().await;
            }
            SpircCommand::Play => self.handle_play(),
            SpircCommand::PlayPause => self.handle_play_pause(),
            SpircCommand::Pause => self.handle_pause(),
            SpircCommand::Prev => self.handle_prev()?,
            SpircCommand::Next => self.handle_next(None)?,
            SpircCommand::VolumeUp => self.handle_volume_up(),
            SpircCommand::VolumeDown => self.handle_volume_down(),
            SpircCommand::Shuffle(shuffle) => self.handle_shuffle(shuffle)?,
            SpircCommand::Repeat(repeat) => self.handle_repeat_context(repeat)?,
            SpircCommand::RepeatTrack(repeat) => self.handle_repeat_track(repeat),
            SpircCommand::SetPosition(position) => self.handle_seek(position),
            SpircCommand::SetVolume(volume) => self.set_volume(volume),
            SpircCommand::Load(command) => self.handle_load(command, None, None).await?,
        };

        self.notify().await
    }

    fn handle_player_event(&mut self, event: PlayerEvent) -> Result<(), Error> {
        if let PlayerEvent::TrackChanged {
            track_id,
            audio_item,
            canonical_duration_ms,
        } = event
        {
            if let Ok(canonical_uri) = track_id.to_uri() {
                self.local_auto_current_identity = Some(AutoTrackIdentity {
                    canonical_uri,
                    playable_uri: audio_item.uri.clone(),
                    canonical_duration_ms,
                });
            } else {
                self.local_auto_current_identity = None;
                debug!("[spotify-auto] current track had an invalid canonical URI");
            }
            self.maybe_start_local_auto();
            self.connect_state.update_duration(audio_item.duration_ms);
            self.update_state = true;
            return Ok(());
        }

        if let PlayerEvent::Preloading {
            track_id,
            playable_uri,
            canonical_duration_ms,
        } = event
        {
            if let Ok(canonical_uri) = track_id.to_uri() {
                self.handle_resolved_preload(AutoTrackIdentity {
                    canonical_uri,
                    playable_uri,
                    canonical_duration_ms,
                });
            } else {
                debug!("[spotify-auto] resolved preload had an invalid canonical URI");
            }
            return Ok(());
        }

        // update play_request_id
        if let PlayerEvent::PlayRequestIdChanged { play_request_id } = event {
            self.play_request_id = Some(play_request_id);
            return Ok(());
        }

        // we only process events if the play_request_id matches. If it doesn't, it is
        // an event that belongs to a previous track and only arrives now due to a race
        // condition. In this case we have updated the state already and don't want to
        // mess with it.
        let current_uri = self.connect_state.current_track(|track| track.uri.clone());
        match player_queue_action(&event, &mut self.play_request_id, &current_uri)? {
            PlayerQueueAction::Ignore => return Ok(()),
            PlayerQueueAction::Preserve => {
                if let PlayerEvent::LoadFailed {
                    track_id,
                    error,
                    is_preload,
                    ..
                } = event
                {
                    warn!(
                        "{} of <{track_id}> failed with {error:?}; preserving the queue",
                        if is_preload { "preload" } else { "load" }
                    );
                    self.cancel_transition_hydrations();
                }
                return Ok(());
            }
            PlayerQueueAction::Advance => {
                let next_track = self
                    .connect_state
                    .repeat_track()
                    .then(|| self.connect_state.current_track(|t| t.uri.clone()));

                self.handle_next(next_track)?;
                self.update_state = true;
                return Ok(());
            }
            PlayerQueueAction::Unavailable { track_id, advance } => {
                self.handle_unavailable(&track_id)?;
                if advance {
                    self.handle_next(None)?;
                }
                self.update_state = true;
                return Ok(());
            }
            PlayerQueueAction::Continue => (),
        }

        match event {
            PlayerEvent::Loading { .. } => match self.play_status {
                SpircPlayStatus::LoadingPlay { position_ms } => {
                    self.connect_state
                        .update_position(position_ms, self.now_ms());
                    trace!("==> LoadingPlay");
                }
                SpircPlayStatus::LoadingPause { position_ms } => {
                    self.connect_state
                        .update_position(position_ms, self.now_ms());
                    trace!("==> LoadingPause");
                }
                _ => {
                    self.connect_state.update_position(0, self.now_ms());
                    trace!("==> Loading");
                }
            },
            PlayerEvent::Seeked { position_ms, .. } => {
                trace!("==> Seeked");
                self.connect_state
                    .update_position(position_ms, self.now_ms())
            }
            PlayerEvent::Playing { position_ms, .. }
            | PlayerEvent::PositionCorrection { position_ms, .. } => {
                trace!("==> Playing");
                let new_nominal_start_time = self.now_ms() - position_ms as i64;
                match self.play_status {
                    SpircPlayStatus::Playing {
                        ref mut nominal_start_time,
                        ..
                    } => {
                        if (*nominal_start_time - new_nominal_start_time).abs() > 100 {
                            *nominal_start_time = new_nominal_start_time;
                            self.connect_state
                                .update_position(position_ms, self.now_ms());
                        } else {
                            return Ok(());
                        }
                    }
                    SpircPlayStatus::LoadingPlay { .. } | SpircPlayStatus::LoadingPause { .. } => {
                        self.connect_state
                            .update_position(position_ms, self.now_ms());
                        self.play_status = SpircPlayStatus::Playing {
                            nominal_start_time: new_nominal_start_time,
                            preloading_of_next_track_triggered: false,
                        };
                    }
                    _ => return Ok(()),
                }
            }
            PlayerEvent::Paused {
                position_ms: new_position_ms,
                ..
            } => {
                trace!("==> Paused");
                match self.play_status {
                    SpircPlayStatus::Paused { .. } | SpircPlayStatus::Playing { .. } => {
                        self.connect_state
                            .update_position(new_position_ms, self.now_ms());
                        self.play_status = SpircPlayStatus::Paused {
                            position_ms: new_position_ms,
                            preloading_of_next_track_triggered: false,
                        };
                    }
                    SpircPlayStatus::LoadingPlay { .. } | SpircPlayStatus::LoadingPause { .. } => {
                        self.connect_state
                            .update_position(new_position_ms, self.now_ms());
                        self.play_status = SpircPlayStatus::Paused {
                            position_ms: new_position_ms,
                            preloading_of_next_track_triggered: false,
                        };
                    }
                    _ => return Ok(()),
                }
            }
            PlayerEvent::Stopped { .. } => {
                trace!("==> Stopped");
                match self.play_status {
                    SpircPlayStatus::Stopped => return Ok(()),
                    _ => self.play_status = SpircPlayStatus::Stopped,
                }
            }
            PlayerEvent::TimeToPreloadNextTrack { .. } => {
                self.handle_preload_next_track();
                return Ok(());
            }
            _ => return Ok(()),
        }

        self.update_state = true;
        Ok(())
    }

    async fn handle_connection_id_update(&mut self, connection_id: String) -> Result<(), Error> {
        trace!("Received connection ID update: {connection_id:?}");
        self.session.set_connection_id(&connection_id);

        let cluster = match self
            .connect_state
            .notify_new_device_appeared(&self.session)
            .await
        {
            Ok(res) => Cluster::parse_from_bytes(&res).ok(),
            Err(why) => {
                error!("{why:?}");
                None
            }
        }
        .ok_or(SpircError::FailedDealerSetup)?;

        debug!(
            "successfully put connect state for {} with connection-id {connection_id}",
            self.session.device_id()
        );

        mix_debug::log_cluster("initial-put-state-response", &cluster);
        capability_debug::log_cluster_devices("initial-put-state-response", &cluster);

        self.connect_established = true;

        let same_session = cluster.player_state.session_id == self.session.session_id()
            || cluster.player_state.session_id.is_empty();
        if !cluster.active_device_id.is_empty() || !same_session {
            info!(
                "active device is <{}> with session <{}>",
                cluster.active_device_id, cluster.player_state.session_id
            );
            return Ok(());
        } else if cluster.transfer_data.is_empty() {
            debug!("got empty transfer state, do nothing");
            return Ok(());
        } else {
            info!(
                "trying to take over control automatically, session_id: {}",
                cluster.player_state.session_id
            )
        }

        use protobuf::Message;

        match TransferState::parse_from_bytes(&cluster.transfer_data) {
            Ok(transfer_state) => self.handle_transfer(transfer_state)?,
            Err(why) => error!("failed to take over control: {why}"),
        }

        Ok(())
    }

    fn handle_user_attributes_update(&mut self, update: UserAttributesUpdate) {
        trace!("Received attributes update: {update:#?}");
        let attributes: UserAttributes = update
            .pairs
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect();
        self.session.set_user_attributes(attributes)
    }

    fn handle_user_attributes_mutation(&mut self, mutation: UserAttributesMutation) {
        for attribute in mutation.fields.iter() {
            let key = &attribute.name;

            if key == "autoplay" && self.session.config().autoplay.is_some() {
                trace!("Autoplay override active. Ignoring mutation.");
                continue;
            }

            if let Some(old_value) = self.session.user_data().attributes.get(key) {
                let new_value = match old_value.as_ref() {
                    "0" => "1",
                    "1" => "0",
                    _ => old_value,
                };
                self.session.set_user_attribute(key, new_value);

                trace!("Received attribute mutation, {key} was {old_value} is now {new_value}");

                if key == "filter-explicit-content" && new_value == "1" {
                    self.player
                        .emit_filter_explicit_content_changed_event(matches!(new_value, "1"));
                }

                if key == "autoplay" && old_value != new_value {
                    self.player
                        .emit_auto_play_changed_event(matches!(new_value, "1"));

                    self.add_autoplay_resolving_when_required()
                }
            } else {
                trace!("Received attribute mutation for {key} but key was not found!");
            }
        }
    }

    async fn handle_cluster_update(
        &mut self,
        mut cluster_update: ClusterUpdate,
    ) -> Result<(), Error> {
        mix_debug::log_cluster_update(&cluster_update);
        if let Some(cluster) = cluster_update.cluster.as_ref() {
            capability_debug::log_cluster_devices("cluster-update", cluster);
        }
        let reason = cluster_update.update_reason.enum_value();

        let device_ids = cluster_update.devices_that_changed.join(", ");
        debug!(
            "cluster update: {reason:?} from {device_ids}, active device: {}",
            cluster_update.cluster.active_device_id
        );

        if let Some(cluster) = cluster_update.cluster.take() {
            let became_inactive = self.connect_state.is_active()
                && cluster.active_device_id != self.session.device_id();
            if became_inactive {
                info!("device became inactive");
                self.handle_disconnect().await?;
                self.handle_stop();
            } else if self.connect_state.is_active() {
                // fixme: workaround fix, because of missing information why it behaves like it does
                //  background: when another device sends a connect-state update, some player's position de-syncs
                //  tried: providing session_id, playback_id, track-metadata "track_player"
                self.update_state = true;
            }
        } else if self.connect_state.is_active() {
            self.connect_state.became_inactive(&self.session).await?;
        }

        Ok(())
    }

    async fn handle_connect_state_request(
        &mut self,
        (request, sender): RequestReply,
    ) -> Result<(), Error> {
        self.connect_state.set_last_command(request.clone());

        debug!(
            "handling: '{}' from {}",
            request.command, request.sent_by_device_id
        );

        let response = match self.handle_request(request).await {
            Ok(_) => Reply::Success,
            Err(why) => {
                error!("failed to handle request: {why}");
                Reply::Failure
            }
        };

        sender.send(response).map_err(Into::into)
    }

    async fn handle_request(&mut self, request: Request) -> Result<(), Error> {
        use Command::*;

        match request.command {
            // errors and unknown commands
            Transfer(transfer) if transfer.data.is_none() => {
                warn!("transfer endpoint didn't contain any data to transfer");
                Err(SpircError::NoData)?
            }
            Unknown(unknown) => Err(SpircError::UnknownEndpoint(unknown))?,
            // implicit update of the connect_state
            UpdateContext(update_context) => {
                if matches!(update_context.context.uri, Some(ref uri) if uri != self.connect_state.context_uri())
                {
                    debug!(
                        "ignoring context update for <{:?}>, because it isn't the current context <{}>",
                        update_context.context.uri,
                        self.connect_state.context_uri()
                    )
                } else {
                    self.context_resolver.add(ResolveContext::from_context(
                        update_context.context,
                        ContextType::Default,
                        ContextAction::Replace,
                    ))
                }
                return Ok(());
            }
            // modification and update of the connect_state
            Transfer(transfer) => {
                self.handle_transfer(transfer.data.expect("by condition checked"))?;
                return self.notify().await;
            }
            Play(mut play) => {
                if !self.connect_state.is_active() {
                    self.handle_activate()
                }

                let context = match play.context.uri {
                    Some(s) => PlayContext::Uri(s),
                    None if !play.context.pages.is_empty() => PlayContext::Tracks(
                        play.context
                            .pages
                            .iter()
                            .cloned()
                            .flat_map(|p| p.tracks)
                            .flat_map(|t| t.uri)
                            .collect(),
                    ),
                    None => Err(SpircError::NoUri("context"))?,
                };

                let context_options = play
                    .options
                    .player_options_override
                    .map(Into::into)
                    .map(LoadContextOptions::Options);

                let fallback_index = play
                    .options
                    .skip_to
                    .as_ref()
                    .and_then(|s| s.track_index)
                    .map(|i| i as usize);

                self.handle_load(
                    LoadRequest {
                        context,
                        options: LoadRequestOptions {
                            start_playing: true,
                            seek_to: play.options.seek_to.unwrap_or_default(),
                            playing_track: play.options.skip_to.and_then(|s| s.try_into().ok()),
                            context_options,
                        },
                    },
                    play.context.pages.pop(),
                    fallback_index,
                )
                .await?;

                self.connect_state.set_origin(play.play_origin)
            }
            Pause(_) => self.handle_pause(),
            SeekTo(seek_to) => {
                // for some reason the position is stored in value, not in position
                trace!("seek to {seek_to:?}");
                self.handle_seek(seek_to.value)
            }
            SetShufflingContext(shuffle) => self.handle_shuffle(shuffle.value)?,
            SetRepeatingContext(repeat_context) => {
                self.handle_repeat_context(repeat_context.value)?
            }
            SetRepeatingTrack(repeat_track) => self.handle_repeat_track(repeat_track.value),
            AddToQueue(add_to_queue) => {
                self.cancel_transition_hydrations();
                self.connect_state.add_to_queue(add_to_queue.track, true);
            }
            SetQueue(set_queue) => {
                self.cancel_transition_hydrations();
                self.connect_state.handle_set_queue(set_queue);
            }
            SetOptions(set_options) => {
                if let Some(repeat_context) = set_options.repeating_context {
                    self.handle_repeat_context(repeat_context)?
                }

                if let Some(repeat_track) = set_options.repeating_track {
                    self.handle_repeat_track(repeat_track)
                }

                let shuffle = set_options.shuffling_context;
                if let Some(shuffle) = shuffle {
                    self.handle_shuffle(shuffle)?;
                }
            }
            SkipNext(skip_next) => self.handle_next(skip_next.track.map(|t| t.uri))?,
            SkipPrev(_) => self.handle_prev()?,
            Resume(_) if matches!(self.play_status, SpircPlayStatus::Stopped) => {
                self.load_track(true, 0)?
            }
            Resume(_) => self.handle_play(),
        }

        self.update_state = true;
        Ok(())
    }

    fn handle_transfer(&mut self, mut transfer: TransferState) -> Result<(), Error> {
        mix_debug::log_transfer(&transfer);
        let mut ctx_uri = match transfer.current_session.context.uri {
            None => Err(SpircError::NoUri("transfer context"))?,
            // can apparently happen when a state is transferred and was started with "uris" via the api
            Some(ref uri) if uri == "-" || uri.is_empty() => None,
            Some(ref uri) => Some(uri.clone()),
        };

        if ctx_uri.as_deref() != Some(self.connect_state.context_uri().as_str()) {
            self.cancel_transition_hydrations();
        }

        self.connect_state.reset_context(
            ctx_uri
                .as_deref()
                .map(ResetContext::WhenDifferent)
                .unwrap_or(ResetContext::Completely),
        );

        match self.connect_state.current_track_from_transfer(&transfer) {
            Err(why) => warn!("didn't find initial track: {why}"),
            Ok(track) => {
                debug!("found initial track <{}>", track.uri);
                self.connect_state.set_track(track)
            }
        };

        let autoplay = self.connect_state.current_track(|t| t.is_autoplay());
        if autoplay {
            ctx_uri = ctx_uri.map(|c| c.replace("station:", ""));
        }

        let fallback = self.connect_state.current_track(|t| &t.uri).clone();
        let load_from_context_uri = ctx_uri.is_some();

        match ctx_uri {
            Some(ref uri) => {
                self.context_resolver.add(ResolveContext::from_uri(
                    uri.clone(),
                    &fallback,
                    ContextType::Default,
                    ContextAction::Replace,
                ));
            }
            None => {
                let all_tracks = transfer
                    .current_session
                    .context
                    .pages
                    .iter()
                    .cloned()
                    .flat_map(|p| p.tracks)
                    .collect::<Vec<_>>();

                if !all_tracks.is_empty() {
                    self.load_context_from_tracks(all_tracks)?;
                } else {
                    warn!(
                        "tried to transfer with an invalid state, using fallback as ctx_uri ({fallback})"
                    );
                    ctx_uri = Some(fallback.clone())
                }
            }
        };

        self.handle_activate();

        let timestamp = self.now_ms();
        let state = &mut self.connect_state;
        state.handle_initial_transfer(&mut transfer, ctx_uri.clone());

        // adjust active context, so resolve knows for which context it should set up the state
        state.active_context = if autoplay {
            ContextType::Autoplay
        } else {
            ContextType::Default
        };

        // update position if the track continued playing
        let transfer_timestamp = transfer.playback.timestamp.unwrap_or_default();
        let position = match transfer.playback.position_as_of_timestamp {
            Some(position) if transfer.playback.is_paused.unwrap_or_default() => position.into(),
            // update position if the track continued playing
            Some(position) if position > 0 => {
                let time_since_position_update = timestamp - transfer_timestamp;
                i64::from(position) + time_since_position_update
            }
            _ => 0,
        };

        let is_playing = !transfer.playback.is_paused();

        if self.connect_state.current_track(|t| t.is_autoplay()) || autoplay {
            if let Some(ctx_uri) = ctx_uri {
                debug!("currently in autoplay context, async resolving autoplay for {ctx_uri}");
                self.context_resolver.add(ResolveContext::from_uri(
                    ctx_uri,
                    fallback,
                    ContextType::Autoplay,
                    ContextAction::Replace,
                ))
            } else {
                warn!("couldn't resolve autoplay context without a context uri");
            }
        }

        if load_from_context_uri {
            self.transfer_state = Some(transfer);
        } else {
            match self.connect_state.get_context(ContextType::Default) {
                Err(why) => {
                    warn!("continuing transfer in an unknown state. {why}");
                    self.transfer_state = Some(transfer);
                }
                Ok(ctx) => {
                    let idx = ConnectState::find_index_in_context(ctx, |pt| {
                        self.connect_state.current_track(|t| pt.uri == t.uri)
                    })?;
                    self.connect_state.reset_playback_to_position(Some(idx))?;
                }
            }
        }

        self.load_track(is_playing, position.try_into()?)
    }

    async fn handle_disconnect(&mut self) -> Result<(), Error> {
        self.context_resolver.clear();

        self.play_status = SpircPlayStatus::Stopped {};
        self.connect_state
            .update_position_in_relation(self.now_ms());
        self.notify().await?;

        self.connect_state.became_inactive(&self.session).await?;

        self.player
            .emit_session_disconnected_event(self.session.connection_id(), self.session.username());

        Ok(())
    }

    fn handle_stop(&mut self) {
        self.cancel_transition_hydrations();
        self.player.stop();
        self.connect_state.update_position(0, self.now_ms());
        self.connect_state.clear_next_tracks();

        if let Err(why) = self.connect_state.reset_playback_to_position(None) {
            warn!("failed filling up next_track during stopping: {why}")
        }
    }

    fn handle_activate(&mut self) {
        self.connect_state.set_active(true);
        self.player
            .emit_session_connected_event(self.session.connection_id(), self.session.username());
        self.player.emit_session_client_changed_event(
            self.session.client_id(),
            self.session.client_name(),
            self.session.client_brand_name(),
            self.session.client_model_name(),
        );

        self.player
            .emit_volume_changed_event(self.connect_state.device_info().volume as u16);

        self.player
            .emit_auto_play_changed_event(self.session.autoplay());

        self.player
            .emit_filter_explicit_content_changed_event(self.session.filter_explicit_content());

        self.player
            .emit_shuffle_changed_event(self.connect_state.shuffling_context());

        self.player.emit_repeat_changed_event(
            self.connect_state.repeat_context(),
            self.connect_state.repeat_track(),
        );
    }

    async fn handle_load(
        &mut self,
        cmd: LoadRequest,
        page: Option<ContextPage>,
        fallback_index: Option<usize>,
    ) -> Result<(), Error> {
        self.cancel_transition_hydrations();
        self.connect_state
            .reset_context(if let PlayContext::Uri(ref uri) = cmd.context {
                ResetContext::WhenDifferent(uri)
            } else {
                ResetContext::Completely
            });

        self.connect_state.reset_options();

        let autoplay = matches!(cmd.context_options, Some(LoadContextOptions::Autoplay));
        match cmd.context {
            PlayContext::Uri(uri) => {
                self.load_context_from_uri(uri, page.as_ref(), autoplay)
                    .await?
            }
            PlayContext::Tracks(tracks) => self.load_context_from_tracks(tracks)?,
        }

        let cmd_options = cmd.options;

        self.connect_state.set_active_context(ContextType::Default);

        // for play commands with skip by uid, the context of the command contains
        // tracks with uri and uid, so we merge the new context with the resolved/existing context
        self.connect_state.merge_context(page);

        // load here, so that we clear the queue only after we definitely retrieved a new context
        self.connect_state.clear_next_tracks();
        self.connect_state.clear_restrictions();

        debug!("play track <{:?}>", cmd_options.playing_track);

        let index = match cmd_options.playing_track {
            None => None,
            Some(ref playing_track) => Some(match playing_track {
                PlayingTrack::Index(i) => Ok(*i as usize),
                PlayingTrack::Uri(uri) => {
                    let ctx = self.connect_state.get_context(ContextType::Default)?;
                    ConnectState::find_index_in_context(ctx, |t| &t.uri == uri)
                }
                PlayingTrack::Uid(uid) => {
                    let ctx = self.connect_state.get_context(ContextType::Default)?;
                    ConnectState::find_index_in_context(ctx, |t| &t.uid == uid)
                }
            }),
        }
        .map(|i| {
            i.unwrap_or_else(|why| {
                warn!(
                    "Failed to resolve index by {:?}, using fallback index: {:?} (Error: {why})",
                    cmd_options.playing_track, fallback_index
                );
                fallback_index.unwrap_or_default()
            })
        });

        if let Some(LoadContextOptions::Options(ref options)) = cmd_options.context_options {
            debug!(
                "loading with shuffle: <{}>, repeat track: <{}> context: <{}>",
                options.shuffle, options.repeat, options.repeat_track
            );

            self.connect_state.set_shuffle(options.shuffle);
            self.connect_state.set_repeat_context(options.repeat);
            self.connect_state.set_repeat_track(options.repeat_track);
        }

        if matches!(cmd_options.context_options, Some(LoadContextOptions::Options(ref o)) if o.shuffle)
        {
            if let Some(index) = index {
                self.connect_state.set_current_track(index)?;
            } else {
                self.connect_state.set_current_track_random()?;
            }

            if self.context_resolver.has_next() {
                self.connect_state.update_queue_revision()
            } else {
                self.connect_state.shuffle_new()?;
                self.add_autoplay_resolving_when_required();
            }
        } else {
            self.connect_state
                .set_current_track(index.unwrap_or_default())?;
            self.connect_state.reset_playback_to_position(index)?;
            self.add_autoplay_resolving_when_required();
        }

        if self.connect_state.current_track(MessageField::is_some) {
            self.load_track(cmd_options.start_playing, cmd_options.seek_to)?;
        } else {
            info!("No active track, stopping");
            self.handle_stop()
        }

        Ok(())
    }

    async fn load_context_from_uri(
        &mut self,
        context_uri: String,
        page: Option<&ContextPage>,
        autoplay: bool,
    ) -> Result<(), Error> {
        if !self.connect_state.is_active() {
            self.handle_activate();
        }

        let update_context = if autoplay {
            ContextType::Autoplay
        } else {
            ContextType::Default
        };

        self.connect_state.set_active_context(update_context);

        let fallback = match page {
            // check that the uri is valid or the page has a valid uri that can be used
            Some(page) => match ConnectState::find_valid_uri(Some(&context_uri), Some(page)) {
                Some(ctx_uri) => ctx_uri,
                None => return Err(SpircError::InvalidUri(context_uri).into()),
            },
            // when there is no page, the uri should be valid
            None => &context_uri,
        };

        let current_context_uri = self.connect_state.context_uri();

        if current_context_uri == &context_uri && fallback == context_uri {
            debug!("context <{current_context_uri}> didn't change, no resolving required")
        } else {
            debug!("resolving context for load command");
            self.context_resolver.clear();
            self.context_resolver.add(ResolveContext::from_uri(
                &context_uri,
                fallback,
                update_context,
                ContextAction::Replace,
            ));
            let context = self.context_resolver.get_next_context(Vec::new).await;
            self.handle_next_context(context);
        }

        Ok(())
    }

    fn load_context_from_tracks(&mut self, tracks: impl Into<ContextPage>) -> Result<(), Error> {
        const WEB_API_URI: &str = "spotify:web-api";
        let ctx = Context {
            // by providing values for uri/url the player in the official client's isn't frozen
            uri: Some(WEB_API_URI.into()),
            url: Some(format!("context://{WEB_API_URI}")),
            pages: vec![tracks.into()],
            ..Default::default()
        };

        let _ = self
            .connect_state
            .update_context(ctx, ContextType::Default)?;

        Ok(())
    }

    fn handle_play(&mut self) {
        match self.play_status {
            SpircPlayStatus::Paused {
                position_ms,
                preloading_of_next_track_triggered,
            } => {
                self.player.play();
                self.connect_state
                    .update_position(position_ms, self.now_ms());
                self.play_status = SpircPlayStatus::Playing {
                    nominal_start_time: self.now_ms() - position_ms as i64,
                    preloading_of_next_track_triggered,
                };
            }
            SpircPlayStatus::LoadingPause { position_ms } => {
                self.player.play();
                self.play_status = SpircPlayStatus::LoadingPlay { position_ms };
            }
            _ => return,
        }

        // Synchronize the volume from the mixer. This is useful on
        // systems that can switch sources from and back to librespot.
        let current_volume = self.mixer.volume();
        self.set_volume(current_volume);
    }

    fn handle_play_pause(&mut self) {
        match self.play_status {
            SpircPlayStatus::Paused { .. } | SpircPlayStatus::LoadingPause { .. } => {
                self.handle_play()
            }
            SpircPlayStatus::Playing { .. } | SpircPlayStatus::LoadingPlay { .. } => {
                self.handle_pause()
            }
            _ => (),
        }
    }

    fn handle_pause(&mut self) {
        match self.play_status {
            SpircPlayStatus::Playing {
                nominal_start_time,
                preloading_of_next_track_triggered,
            } => {
                self.player.pause();
                let position_ms = (self.now_ms() - nominal_start_time) as u32;
                self.connect_state
                    .update_position(position_ms, self.now_ms());
                self.play_status = SpircPlayStatus::Paused {
                    position_ms,
                    preloading_of_next_track_triggered,
                };
            }
            SpircPlayStatus::LoadingPlay { position_ms } => {
                self.player.pause();
                self.play_status = SpircPlayStatus::LoadingPause { position_ms };
            }
            _ => (),
        }
    }

    fn handle_seek(&mut self, position_ms: u32) {
        self.cancel_transition_hydrations();
        let duration = self.connect_state.player().duration;
        if i64::from(position_ms) > duration {
            warn!("tried to seek to {position_ms}ms of {duration}ms");
            return;
        }

        self.connect_state
            .update_position(position_ms, self.now_ms());
        self.player.seek(position_ms);
        let now = self.now_ms();
        match self.play_status {
            SpircPlayStatus::Stopped => (),
            SpircPlayStatus::LoadingPause {
                position_ms: ref mut position,
            }
            | SpircPlayStatus::LoadingPlay {
                position_ms: ref mut position,
            }
            | SpircPlayStatus::Paused {
                position_ms: ref mut position,
                ..
            } => *position = position_ms,
            SpircPlayStatus::Playing {
                ref mut nominal_start_time,
                ..
            } => *nominal_start_time = now - position_ms as i64,
        };
    }

    fn handle_shuffle(&mut self, shuffle: bool) -> Result<(), Error> {
        self.player.emit_shuffle_changed_event(shuffle);
        self.connect_state.handle_shuffle(shuffle)
    }

    fn handle_repeat_context(&mut self, repeat: bool) -> Result<(), Error> {
        self.player
            .emit_repeat_changed_event(repeat, self.connect_state.repeat_track());
        self.connect_state.handle_set_repeat_context(repeat)
    }

    fn handle_repeat_track(&mut self, repeat: bool) {
        self.player
            .emit_repeat_changed_event(self.connect_state.repeat_context(), repeat);
        self.connect_state.set_repeat_track(repeat);
    }

    fn handle_preload_next_track(&mut self) {
        // Requests the player thread to preload the next track
        match self.play_status {
            SpircPlayStatus::Paused {
                ref mut preloading_of_next_track_triggered,
                ..
            }
            | SpircPlayStatus::Playing {
                ref mut preloading_of_next_track_triggered,
                ..
            } => {
                *preloading_of_next_track_triggered = true;
            }
            _ => (),
        }

        if let (Some(track_id), Some(incoming)) = (
            self.connect_state.preview_next_track(),
            self.connect_state.preview_next_provided_track().cloned(),
        ) {
            let outgoing = self
                .connect_state
                .current_track(|track| track.as_ref().cloned());
            let Some(outgoing) = outgoing else {
                self.cancel_local_auto();
                debug!("[spotify-mix] hydration unavailable; using fallback");
                self.player.preload(track_id.clone());
                return;
            };

            let transition_plan =
                crate::spotify_mix::transition_plan_for_pair(&outgoing, &incoming);
            let transition_uri = transition_uri(&outgoing).map(str::to_owned);
            let mixer_edge_known = mixer_auto_edge_is_eligible(
                transition_plan.is_some(),
                transition_uri.is_some(),
                outgoing
                    .metadata
                    .contains_key(crate::spotify_mix::BACKEND_RECIPE_ATTRIBUTE),
                self.connect_state.is_mixer_context(),
            );
            if mixer_edge_known {
                self.prepare_local_auto_pair(&outgoing, &incoming);
            } else {
                self.cancel_local_auto();
            }
            self.player
                .preload_with_transition(track_id.clone(), transition_plan.clone());

            // Retain support for contexts that already carry the recipe inline.
            if transition_plan.is_some() {
                debug!(
                    "[spotify-auto] official transition selected edge={}->{}",
                    outgoing.uri, incoming.uri
                );
                self.suppress_local_auto(HigherPriorityTransitionState::SavedSelected);
                return;
            }

            let Some(transition_uri) = transition_uri else {
                debug!("[spotify-mix] hydration unavailable; considering local Auto");
                if mixer_edge_known {
                    self.enable_local_auto(&outgoing, &incoming);
                }
                return;
            };
            debug!("[spotify-mix] transition uri found uri={transition_uri}");
            let playlist_uri = self.connect_state.context_uri().clone();
            if !matches!(
                SpotifyUri::from_uri(&playlist_uri),
                Ok(SpotifyUri::Playlist { .. })
            ) {
                debug!("[spotify-mix] hydration unavailable; considering local Auto");
                self.enable_local_auto(&outgoing, &incoming);
                return;
            }
            let Some(row_uid) = self
                .connect_state
                .authentic_row_uid(&outgoing)
                .map(str::to_owned)
            else {
                debug!("[spotify-mix] hydration unavailable; considering local Auto");
                self.enable_local_auto(&outgoing, &incoming);
                return;
            };

            let key = TransitionHydrationKey {
                playlist_uri,
                row_uid,
                transition_uri,
                outgoing_uri: outgoing.uri.clone(),
                incoming_uri: incoming.uri.clone(),
            };
            match self.transition_hydration_cache.lookup_or_begin(key.clone()) {
                CacheLookup::Start(request_id) => {
                    self.suppress_local_auto(HigherPriorityTransitionState::Pending);
                    self.transition_hydrations.spawn(hydrate_transition_data(
                        TransitionDataClient::new(self.session.clone()),
                        key,
                        request_id,
                    ));
                    debug!("[spotify-mix] hydration started; local Auto running speculatively");
                }
                CacheLookup::Pending => {
                    self.suppress_local_auto(HigherPriorityTransitionState::Pending);
                    debug!("[spotify-mix] hydration pending; local Auto running speculatively");
                }
                CacheLookup::Unavailable => {
                    debug!("[spotify-mix] hydration unavailable; considering local Auto");
                    self.enable_local_auto(&outgoing, &incoming);
                }
                CacheLookup::Ready(transition) => {
                    if let Some(plan) = crate::spotify_mix::transition_plan_for_decoded_pair(
                        &outgoing,
                        &incoming,
                        &transition.recipe,
                    ) {
                        debug!("[spotify-mix] saved transition plan selected");
                        debug!(
                            "[spotify-auto] official transition selected edge={}->{}",
                            outgoing.uri, incoming.uri
                        );
                        self.suppress_local_auto(HigherPriorityTransitionState::SavedSelected);
                        self.player.preload_with_transition(track_id, Some(plan));
                    } else {
                        self.enable_local_auto(&outgoing, &incoming);
                    }
                }
            }
        }
    }

    fn handle_transition_hydration(&mut self, hydration: HydrationResult) {
        let HydrationResult {
            key,
            request_id,
            result,
        } = hydration;

        // A completed request can outlive a skip or context replacement. Never let it replace
        // the next-track preload or repopulate a cleared cache unless it still belongs to A -> B.
        let Some(outgoing) = self
            .connect_state
            .current_track(|track| track.as_ref().cloned())
        else {
            return;
        };
        let Some(incoming) = self.connect_state.preview_next_provided_track().cloned() else {
            return;
        };
        let Some(row_uid) = self.connect_state.authentic_row_uid(&outgoing) else {
            return;
        };
        let Some(active_transition_uri) = transition_uri(&outgoing) else {
            return;
        };
        if !key.matches_pair(
            self.connect_state.context_uri(),
            row_uid,
            active_transition_uri,
            &outgoing.uri,
            &incoming.uri,
        ) {
            debug!("[spotify-mix] discarded stale transition hydration result");
            return;
        }

        if !self
            .transition_hydration_cache
            .complete(key.clone(), request_id, result.clone())
        {
            debug!("[spotify-mix] discarded cancelled transition hydration result");
            return;
        }

        match &result {
            Ok(_) => debug!("[spotify-mix] transition data hydrated recipe=true"),
            Err(reason) => debug!("[spotify-mix] hydration failed: {reason}"),
        }

        let Ok(transition) = result else {
            debug!("[spotify-mix] hydration unavailable; considering local Auto");
            self.enable_local_auto(&outgoing, &incoming);
            return;
        };
        let Some(track_id) = self.connect_state.preview_next_track() else {
            return;
        };
        let Some(plan) = crate::spotify_mix::transition_plan_for_decoded_pair(
            &outgoing,
            &incoming,
            &transition.recipe,
        ) else {
            self.enable_local_auto(&outgoing, &incoming);
            return;
        };

        if !self.transition_plan_can_still_apply(&plan) {
            debug!("[spotify-mix] hydration unavailable; using fallback");
            return;
        }

        debug!("[spotify-mix] saved transition plan selected");
        debug!(
            "[spotify-auto] official transition selected edge={}->{}",
            outgoing.uri, incoming.uri
        );
        self.suppress_local_auto(HigherPriorityTransitionState::SavedSelected);
        self.player.preload_with_transition(track_id, Some(plan));
    }

    fn prepare_local_auto_pair(&mut self, outgoing: &ProvidedTrack, incoming: &ProvidedTrack) {
        let key = LocalAutoPairKey::from_edge(
            self.connect_state.context_uri().clone(),
            outgoing,
            incoming,
        );
        if self
            .local_auto_pair
            .as_ref()
            .is_some_and(|state| state.key == key)
        {
            return;
        }

        self.local_auto_tasks.abort_all();
        self.local_auto_pair = Some(LocalAutoPairState {
            key,
            higher_priority: HigherPriorityTransitionState::Pending,
            attempted: false,
            incoming_identity: None,
            prepared_transition: None,
            selected_local_auto: false,
        });
    }

    fn enable_local_auto(&mut self, outgoing: &ProvidedTrack, incoming: &ProvidedTrack) {
        self.prepare_local_auto_pair(outgoing, incoming);
        if outgoing
            .metadata
            .contains_key(crate::spotify_mix::BACKEND_RECIPE_ATTRIBUTE)
        {
            debug!(
                "[spotify-auto] backend_auto_transition is present but not enabled by this client; considering local Auto"
            );
        }
        if let Some(state) = self.local_auto_pair.as_mut() {
            state.higher_priority = HigherPriorityTransitionState::Exhausted;
        }
        self.maybe_start_local_auto();
        self.maybe_select_cached_local_auto();
    }

    fn suppress_local_auto(&mut self, higher_priority: HigherPriorityTransitionState) {
        if local_auto_is_eligible(higher_priority) {
            if let Some(state) = self.local_auto_pair.as_mut() {
                state.higher_priority = higher_priority;
            }
            return;
        }
        self.local_auto_tasks.abort_all();
        if let Some(state) = self.local_auto_pair.as_mut() {
            state.higher_priority = higher_priority;
        }
    }

    fn handle_resolved_preload(&mut self, identity: AutoTrackIdentity) {
        let Some(state) = self.local_auto_pair.as_mut() else {
            return;
        };
        if state.key.incoming_uri != identity.canonical_uri {
            return;
        }
        state.incoming_identity = Some(identity);
        self.maybe_start_local_auto();
    }

    fn maybe_start_local_auto(&mut self) {
        let Some(state) = self.local_auto_pair.as_ref() else {
            return;
        };
        if !local_auto_is_eligible(state.higher_priority) || state.attempted {
            return;
        }
        let key = state.key.clone();
        let Some(track_b) = state.incoming_identity.clone() else {
            return;
        };
        let Some(track_a) = self.local_auto_current_identity.clone() else {
            return;
        };
        if track_a.canonical_uri != key.outgoing_uri || track_b.canonical_uri != key.incoming_uri {
            return;
        }
        let Some(outgoing) = self
            .connect_state
            .current_track(|track| track.as_ref().cloned())
        else {
            return;
        };
        let Some(incoming) = self.connect_state.preview_next_provided_track().cloned() else {
            return;
        };
        if LocalAutoPairKey::from_edge(
            self.connect_state.context_uri().clone(),
            &outgoing,
            &incoming,
        ) != key
        {
            return;
        }

        let request = LocalAutoRequest::from_resolved_pair(
            self.connect_state.context_uri().clone(),
            outgoing,
            incoming,
            track_a,
            track_b,
        );
        if let Some(state) = self.local_auto_pair.as_mut() {
            state.attempted = true;
        }
        debug!(
            "[spotify-auto] speculative Auto started edge={}->{} canonicalA={} playableA={} canonicalB={} playableB={}",
            request.key.outgoing_uri,
            request.key.incoming_uri,
            request.track_a.canonical_uri,
            request.track_a.playable_uri,
            request.track_b.canonical_uri,
            request.track_b.playable_uri
        );
        let session = self.session.clone();
        self.local_auto_tasks.spawn(async move {
            let result = generate_local_auto(session, &request).await;
            LocalAutoTaskResult {
                key: request.key,
                result,
            }
        });
    }

    fn handle_local_auto_result(&mut self, result: LocalAutoTaskResult) {
        let Some(state) = self.local_auto_pair.as_ref() else {
            return;
        };
        if state.key != result.key {
            debug!(
                "[spotify-auto] stale Auto result discarded edge={}->{}",
                result.key.outgoing_uri, result.key.incoming_uri
            );
            return;
        }
        let Some(track_a) = self.local_auto_current_identity.as_ref() else {
            return;
        };
        let Some(track_b) = state.incoming_identity.as_ref() else {
            return;
        };
        let Some(outgoing) = self
            .connect_state
            .current_track(|track| track.as_ref().cloned())
        else {
            return;
        };
        let Some(incoming) = self.connect_state.preview_next_provided_track().cloned() else {
            return;
        };
        if outgoing.uri != result.key.outgoing_uri
            || incoming.uri != result.key.incoming_uri
            || LocalAutoPairKey::from_edge(
                self.connect_state.context_uri().clone(),
                &outgoing,
                &incoming,
            ) != result.key
            || track_a.canonical_uri != result.key.outgoing_uri
            || track_b.canonical_uri != result.key.incoming_uri
        {
            debug!(
                "[spotify-auto] stale Auto result discarded edge={}->{}",
                result.key.outgoing_uri, result.key.incoming_uri
            );
            return;
        }

        let selected = match result.result {
            Ok(selected) => selected,
            Err(error) => {
                debug!("[spotify-auto] local Auto unavailable: {error}; using fallback");
                return;
            }
        };
        let overlap = selected.transition.overlap;
        debug!(
            "[spotify-auto] selected local Auto canonicalA={} playableA={} canonicalB={} playableB={} startA={} startB={} duration={} durationBars={} speedA={} speedB={} beatmatched={} preset={} score={} presetScore={}",
            track_a.canonical_uri,
            track_a.playable_uri,
            track_b.canonical_uri,
            track_b.playable_uri,
            overlap.start_a_ms,
            overlap.start_b_ms,
            overlap.duration_ms,
            overlap.duration_bars,
            overlap.speed_a,
            overlap.speed_b,
            overlap.is_beatmatched,
            selected.preset.preset_id,
            selected.transition.computed_score,
            selected.preset.computed_score,
        );

        let plan = match crate::spotify_mix::transition_plan_for_local_auto_transition(
            &selected.transition,
        ) {
            Ok(plan) => plan,
            Err(error) => {
                debug!(
                    "[spotify-auto] selected result could not be materialized: {error}; using fallback"
                );
                return;
            }
        };
        debug!(
            "[spotify-auto] speculative Auto ready edge={}->{} startA={} startB={} duration={} speedA={} speedB={} preset={}",
            result.key.outgoing_uri,
            result.key.incoming_uri,
            overlap.start_a_ms,
            overlap.start_b_ms,
            overlap.duration_ms,
            overlap.speed_a,
            overlap.speed_b,
            selected.preset.preset_id
        );
        if let Some(state) = self.local_auto_pair.as_mut() {
            state.prepared_transition = Some(PreparedLocalAutoTransition { selected, plan });
        }
        self.maybe_select_cached_local_auto();
    }

    fn maybe_select_cached_local_auto(&mut self) {
        let Some(state) = self.local_auto_pair.as_ref() else {
            return;
        };
        if !local_auto_is_eligible(state.higher_priority) || state.selected_local_auto {
            return;
        }
        let Some(prepared) = state.prepared_transition.clone() else {
            return;
        };
        let key = state.key.clone();

        let Some(outgoing) = self
            .connect_state
            .current_track(|track| track.as_ref().cloned())
        else {
            return;
        };
        let Some(incoming) = self.connect_state.preview_next_provided_track().cloned() else {
            return;
        };
        let active_key = LocalAutoPairKey::from_edge(
            self.connect_state.context_uri().clone(),
            &outgoing,
            &incoming,
        );
        let Some(track_id) = self.connect_state.preview_next_track() else {
            return;
        };

        match cached_local_auto_selection_gate(
            state.higher_priority,
            state.selected_local_auto,
            &key,
            &active_key,
            self.transition_plan_can_still_apply(&prepared.plan),
        ) {
            CachedLocalAutoSelectionGate::Select => {}
            CachedLocalAutoSelectionGate::Suppressed => return,
            CachedLocalAutoSelectionGate::Stale => {
                debug!(
                    "[spotify-auto] stale Auto result discarded edge={}->{}",
                    key.outgoing_uri, key.incoming_uri
                );
                return;
            }
            CachedLocalAutoSelectionGate::MissedWindow => {
                debug!(
                    "[spotify-auto] cached local Auto transition missed transition window edge={}->{}",
                    key.outgoing_uri, key.incoming_uri
                );
                return;
            }
        }

        let overlap = prepared.selected.transition.overlap;
        debug!(
            "[spotify-auto] cached local Auto transition selected edge={}->{} startA={} startB={} duration={} speedA={} speedB={} preset={}",
            key.outgoing_uri,
            key.incoming_uri,
            overlap.start_a_ms,
            overlap.start_b_ms,
            overlap.duration_ms,
            overlap.speed_a,
            overlap.speed_b,
            prepared.selected.preset.preset_id
        );
        if let Some(state) = self.local_auto_pair.as_mut() {
            state.selected_local_auto = true;
        }
        self.player
            .preload_with_transition(track_id, Some(prepared.plan));
    }

    fn transition_plan_can_still_apply(&self, plan: &librespot_playback::TransitionPlan) -> bool {
        let current_position_ms = match self.play_status {
            SpircPlayStatus::Playing {
                nominal_start_time, ..
            } => self.now_ms().saturating_sub(nominal_start_time) as u64,
            SpircPlayStatus::Paused { position_ms, .. }
            | SpircPlayStatus::LoadingPause { position_ms }
            | SpircPlayStatus::LoadingPlay { position_ms } => u64::from(position_ms),
            SpircPlayStatus::Stopped => return false,
        };
        let transition_start_ms =
            u64::try_from(plan.current_start().as_millis()).unwrap_or(u64::MAX);
        current_position_ms < transition_start_ms.saturating_sub(200)
    }

    fn cancel_local_auto(&mut self) {
        self.local_auto_tasks.abort_all();
        self.local_auto_pair = None;
    }

    fn cancel_transition_hydrations(&mut self) {
        self.transition_hydrations.abort_all();
        self.transition_hydration_cache.clear();
        self.cancel_local_auto();
    }

    // Mark unavailable tracks so we can skip them later
    fn handle_unavailable(&mut self, track_id: &SpotifyUri) -> Result<(), Error> {
        self.connect_state.mark_unavailable(track_id)?;
        self.handle_preload_next_track();

        Ok(())
    }

    fn add_autoplay_resolving_when_required(&mut self) {
        let require_load_new = !self
            .connect_state
            .has_next_tracks(Some(CONTEXT_FETCH_THRESHOLD))
            && self.session.autoplay()
            && !self.connect_state.context_uri().is_empty();

        if !require_load_new {
            return;
        }

        let current_context = self.connect_state.context_uri();
        let fallback = self.connect_state.current_track(|t| &t.uri);

        let has_tracks = self
            .connect_state
            .get_context(ContextType::Autoplay)
            .map(|c| !c.tracks.is_empty())
            .unwrap_or_default();

        let resolve = ResolveContext::from_uri(
            current_context,
            fallback,
            ContextType::Autoplay,
            if has_tracks {
                ContextAction::Append
            } else {
                ContextAction::Replace
            },
        );

        self.context_resolver.add(resolve);
    }

    fn handle_next(&mut self, track_uri: Option<String>) -> Result<(), Error> {
        self.cancel_transition_hydrations();
        let continue_playing = self.connect_state.is_playing();

        let current_uri = self.connect_state.current_track(|t| &t.uri);
        let mut has_next_track =
            matches!(track_uri, Some(ref track_uri) if current_uri == track_uri);

        if !has_next_track {
            has_next_track = loop {
                let index = self.connect_state.next_track()?;

                let current_uri = self.connect_state.current_track(|t| &t.uri);
                if matches!(track_uri, Some(ref track_uri) if current_uri != track_uri) {
                    continue;
                } else {
                    break index.is_some();
                }
            };
        };

        if has_next_track {
            self.add_autoplay_resolving_when_required();
            self.load_track(continue_playing, 0)
        } else {
            info!("Not playing next track because there are no more tracks left in queue.");
            self.handle_stop();
            Ok(())
        }
    }

    fn handle_prev(&mut self) -> Result<(), Error> {
        self.cancel_transition_hydrations();
        // Previous behaves differently based on the position
        // Under 3s it goes to the previous song (starts playing)
        // Over 3s it seeks to zero (retains previous play status)
        if self.position() < 3000 {
            let repeat_context = self.connect_state.repeat_context();
            match self.connect_state.prev_track()? {
                None if repeat_context => self.connect_state.reset_playback_to_position(None)?,
                None => {
                    self.connect_state.reset_playback_to_position(None)?;
                    self.handle_stop()
                }
                Some(_) => self.load_track(self.connect_state.is_playing(), 0)?,
            }
        } else {
            self.handle_seek(0);
        }

        Ok(())
    }

    fn handle_volume_up(&mut self) {
        let volume = (self.connect_state.device_info().volume as u16)
            .saturating_add(self.connect_state.volume_step_size);

        self.set_volume(volume);
    }

    fn handle_volume_down(&mut self) {
        let volume = (self.connect_state.device_info().volume as u16)
            .saturating_sub(self.connect_state.volume_step_size);

        self.set_volume(volume);
    }

    fn handle_playlist_modification(
        &mut self,
        playlist_modification_info: PlaylistModificationInfo,
    ) -> Result<(), Error> {
        mix_debug::log_playlist_modification(&playlist_modification_info);
        let uri = playlist_modification_info
            .uri
            .ok_or(SpircError::NoUri("playlist modification"))?;
        let uri = String::from_utf8(uri)?;

        if self.connect_state.context_uri() != &uri {
            debug!(
                "ignoring playlist modification update for playlist <{uri}>, because it isn't the current context"
            );
            return Ok(());
        }

        debug!("playlist modification for current context: {uri}");
        self.cancel_transition_hydrations();
        self.context_resolver.add(ResolveContext::from_uri(
            uri,
            self.connect_state.current_track(|t| &t.uri),
            ContextType::Default,
            ContextAction::Replace,
        ));

        Ok(())
    }

    fn handle_session_update(&mut self, session_update: FallbackWrapper<SessionUpdate>) {
        // we know that this enum value isn't present in our current proto definitions, by that
        // the json parsing fails because the enum isn't known as proto representation
        const WBC: &str = "WIFI_BROADCAST_CHANGED";

        let mut session_update = match session_update {
            FallbackWrapper::Inner(update) => update,
            FallbackWrapper::Fallback(value) => {
                let fallback_inner = value.to_string();
                if fallback_inner.contains(WBC) {
                    log::debug!("Received SessionUpdate::{WBC}");
                } else {
                    log::warn!("SessionUpdate couldn't be parse correctly: {value:?}");
                }
                return;
            }
        };

        let reason = session_update.reason.enum_value();

        let mut session = match session_update.session.take() {
            None => return,
            Some(session) => session,
        };

        let active_device = session.host_active_device_id.take();
        if matches!(active_device, Some(ref device) if device == self.session.device_id()) {
            info!(
                "session update: <{:?}> for self, current session_id {}, new session_id {}",
                reason,
                self.session.session_id(),
                session.session_id
            );

            if self.session.session_id() != session.session_id {
                self.session.set_session_id(&session.session_id);
                self.connect_state.set_session_id(session.session_id);
            }
        } else {
            debug!("session update: <{reason:?}> from active session host: <{active_device:?}>");
        }

        // this seems to be used for jams or handling the current session_id
        //
        // handling this event was intended to keep the playback when other clients (primarily
        // mobile) connects, otherwise they would steel the current playback when there was no
        // session_id provided on the initial PutStateReason::NEW_DEVICE state update
        //
        // by generating an initial session_id from the get-go we prevent that behavior and
        // currently don't need to handle this event, might still be useful for later "jam" support
    }

    fn position(&mut self) -> u32 {
        match self.play_status {
            SpircPlayStatus::Stopped => 0,
            SpircPlayStatus::LoadingPlay { position_ms }
            | SpircPlayStatus::LoadingPause { position_ms }
            | SpircPlayStatus::Paused { position_ms, .. } => position_ms,
            SpircPlayStatus::Playing {
                nominal_start_time, ..
            } => (self.now_ms() - nominal_start_time) as u32,
        }
    }

    fn load_track(&mut self, start_playing: bool, position_ms: u32) -> Result<(), Error> {
        if self.connect_state.current_track(MessageField::is_none) {
            debug!("current track is none, stopping playback");
            self.handle_stop();
            return Ok(());
        }

        let current_uri = self.connect_state.current_track(|t| &t.uri);
        let id = SpotifyUri::from_uri(current_uri)?;
        self.player.load(id, start_playing, position_ms);

        self.connect_state
            .update_position(position_ms, self.now_ms());
        if start_playing {
            self.play_status = SpircPlayStatus::LoadingPlay { position_ms };
        } else {
            self.play_status = SpircPlayStatus::LoadingPause { position_ms };
        }
        self.connect_state.set_status(&self.play_status);

        Ok(())
    }

    async fn notify(&mut self) -> Result<(), Error> {
        self.connect_state.set_status(&self.play_status);

        if self.connect_state.is_playing() {
            self.connect_state
                .update_position_in_relation(self.now_ms());
        }

        self.connect_state.set_now(self.now_ms() as u64);

        self.connect_state
            .send_state(&self.session)
            .await
            .map(|_| ())
    }

    fn set_volume(&mut self, volume: u16) {
        debug!("SpircTask::set_volume({volume})");

        let old_volume = self.connect_state.device_info().volume;
        let new_volume = volume as u32;
        if old_volume != new_volume || self.mixer.volume() != volume {
            self.update_volume = true;

            self.connect_state.set_volume(new_volume);
            self.mixer.set_volume(volume);
            if let Some(cache) = self.session.cache() {
                cache.save_volume(volume)
            }
            if self.connect_state.is_active() {
                self.player.emit_volume_changed_event(volume);
            }
        }
    }
}

impl Drop for SpircTask {
    fn drop(&mut self) {
        debug!("drop Spirc[{}]", self.spirc_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{core::config::SessionConfig, playback::player::PlayerLoadErrorKind};

    const CURRENT_URI: &str = "spotify:track:2TpxZ7JUBn3uw46aR7qd6V";
    const NEXT_URI: &str = "spotify:track:4cOdK2wGLETKBW3PvgPWqT";
    const LATER_URI: &str = "spotify:track:7ouMYWpwJ422jRcDASZB7P";

    #[test]
    fn speculative_local_auto_starts_while_hydration_is_pending_but_not_after_official_selection() {
        assert!(!local_auto_is_eligible(
            HigherPriorityTransitionState::SavedSelected
        ));
        assert!(local_auto_is_eligible(
            HigherPriorityTransitionState::Pending
        ));
        assert!(local_auto_is_eligible(
            HigherPriorityTransitionState::Exhausted
        ));
    }

    fn local_auto_key(
        context_uri: &str,
        outgoing_uri: &str,
        incoming_uri: &str,
        outgoing_uid: &str,
        incoming_uid: &str,
    ) -> LocalAutoPairKey {
        LocalAutoPairKey {
            context_uri: context_uri.to_owned(),
            outgoing_uri: outgoing_uri.to_owned(),
            incoming_uri: incoming_uri.to_owned(),
            outgoing_uid: outgoing_uid.to_owned(),
            incoming_uid: incoming_uid.to_owned(),
        }
    }

    #[test]
    fn official_transition_selection_suppresses_ready_speculative_local_auto() {
        let key = local_auto_key(
            "spotify:playlist:mixer",
            CURRENT_URI,
            NEXT_URI,
            "current-row",
            "next-row",
        );

        assert_eq!(
            cached_local_auto_selection_gate(
                HigherPriorityTransitionState::SavedSelected,
                false,
                &key,
                &key,
                true,
            ),
            CachedLocalAutoSelectionGate::Suppressed
        );
    }

    #[test]
    fn stale_cached_auto_result_is_not_selected_for_replaced_next_track() {
        let prepared = local_auto_key(
            "spotify:playlist:mixer",
            CURRENT_URI,
            NEXT_URI,
            "current-row",
            "next-row",
        );
        let active = local_auto_key(
            "spotify:playlist:mixer",
            CURRENT_URI,
            LATER_URI,
            "current-row",
            "later-row",
        );

        assert_eq!(
            cached_local_auto_selection_gate(
                HigherPriorityTransitionState::Exhausted,
                false,
                &prepared,
                &active,
                true,
            ),
            CachedLocalAutoSelectionGate::Stale
        );
    }

    #[test]
    fn mixer_context_without_per_edge_metadata_is_local_auto_eligible() {
        assert!(mixer_auto_edge_is_eligible(false, false, false, true));
        assert!(!mixer_auto_edge_is_eligible(false, false, false, false));
    }

    #[test]
    fn replace_session_api_delivers_replacement_to_retained_task() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        runtime.block_on(async {
            let (commands, mut command_rx) = mpsc::unbounded_channel();
            let spirc = Spirc { commands };
            let replacement = Session::new(SessionConfig::default(), None);
            let replacement_id = replacement.session_id();

            let acknowledge = async {
                let command = command_rx.recv().await.expect("replacement command");
                let SpircCommand::ReplaceSession { session, result } = command else {
                    panic!("expected replacement command")
                };
                assert_eq!(session.session_id(), replacement_id);
                result.send(Ok(())).expect("receiver should remain alive");
            };

            let (result, ()) = tokio::join!(spirc.replace_session(replacement), acknowledge);
            result.expect("replacement acknowledgement should propagate");
        });
    }

    #[test]
    fn replacement_session_inherits_existing_connect_session_identity() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let _guard = runtime.enter();
        let current = Session::new(SessionConfig::default(), None);
        let replacement = Session::new(SessionConfig::default(), None);
        assert_ne!(current.session_id(), replacement.session_id());

        preserve_session_identity(&current, &replacement);

        assert_eq!(current.session_id(), replacement.session_id());
    }

    fn uri(value: &str) -> SpotifyUri {
        SpotifyUri::from_uri(value).expect("test URI should be valid")
    }

    fn transient_event(track_id: SpotifyUri, is_preload: bool) -> PlayerEvent {
        PlayerEvent::LoadFailed {
            play_request_id: 7,
            track_id,
            error: PlayerLoadErrorKind::TransientNetwork,
            is_preload,
        }
    }

    #[test]
    fn transient_current_load_failure_does_not_advance_queue() {
        let mut play_request_id = Some(7);
        let queue = vec![CURRENT_URI, NEXT_URI, LATER_URI];

        let action = player_queue_action(
            &transient_event(uri(CURRENT_URI), false),
            &mut play_request_id,
            CURRENT_URI,
        )
        .expect("event classification should succeed");

        assert_eq!(action, PlayerQueueAction::Preserve);
        assert_eq!(play_request_id, Some(7));
        assert_eq!(queue, vec![CURRENT_URI, NEXT_URI, LATER_URI]);
    }

    #[test]
    fn transient_preload_failure_does_not_poison_upcoming_tracks() {
        let mut play_request_id = Some(7);
        let upcoming = vec![NEXT_URI, LATER_URI];

        let action = player_queue_action(
            &transient_event(uri(NEXT_URI), true),
            &mut play_request_id,
            CURRENT_URI,
        )
        .expect("event classification should succeed");

        assert_eq!(action, PlayerQueueAction::Preserve);
        assert_eq!(play_request_id, Some(7));
        assert_eq!(upcoming, vec![NEXT_URI, LATER_URI]);
    }

    #[test]
    fn repeated_preload_failures_do_not_walk_the_queue() {
        let mut play_request_id = Some(7);
        let upcoming = vec![NEXT_URI, LATER_URI];

        for _ in 0..5 {
            let action = player_queue_action(
                &transient_event(uri(NEXT_URI), true),
                &mut play_request_id,
                CURRENT_URI,
            )
            .expect("event classification should succeed");
            assert_eq!(action, PlayerQueueAction::Preserve);
        }

        assert_eq!(play_request_id, Some(7));
        assert_eq!(upcoming, vec![NEXT_URI, LATER_URI]);
    }

    #[test]
    fn permanent_unavailable_current_track_advances_once() {
        let mut play_request_id = Some(7);
        let event = PlayerEvent::Unavailable {
            play_request_id: 7,
            track_id: uri(CURRENT_URI),
        };

        let action = player_queue_action(&event, &mut play_request_id, CURRENT_URI)
            .expect("event classification should succeed");

        assert!(matches!(
            action,
            PlayerQueueAction::Unavailable { advance: true, .. }
        ));
        assert_eq!(play_request_id, None);
    }

    #[test]
    fn duplicate_terminal_event_does_not_advance_twice() {
        let mut play_request_id = Some(7);
        let event = PlayerEvent::Unavailable {
            play_request_id: 7,
            track_id: uri(CURRENT_URI),
        };
        let mut advances = 0;

        for _ in 0..2 {
            let action = player_queue_action(&event, &mut play_request_id, CURRENT_URI)
                .expect("event classification should succeed");
            if matches!(action, PlayerQueueAction::Unavailable { advance: true, .. }) {
                advances += 1;
            }
        }

        assert_eq!(advances, 1);
    }

    #[test]
    fn duplicate_end_of_track_does_not_advance_twice() {
        let mut play_request_id = Some(7);
        let event = PlayerEvent::EndOfTrack {
            play_request_id: 7,
            track_id: uri(CURRENT_URI),
        };
        let mut advances = 0;

        for _ in 0..2 {
            if player_queue_action(&event, &mut play_request_id, CURRENT_URI)
                .expect("event classification should succeed")
                == PlayerQueueAction::Advance
            {
                advances += 1;
            }
        }

        assert_eq!(advances, 1);
    }

    #[test]
    fn session_invalid_preserves_queue_and_current_request() {
        let mut play_request_id = Some(7);
        let queue = vec![CURRENT_URI, NEXT_URI, LATER_URI];
        let event = PlayerEvent::LoadFailed {
            play_request_id: 7,
            track_id: uri(CURRENT_URI),
            error: PlayerLoadErrorKind::SessionInvalid,
            is_preload: false,
        };

        assert_eq!(
            player_queue_action(&event, &mut play_request_id, CURRENT_URI)
                .expect("event classification should succeed"),
            PlayerQueueAction::Preserve
        );
        assert_eq!(play_request_id, Some(7));
        assert_eq!(queue, vec![CURRENT_URI, NEXT_URI, LATER_URI]);
    }

    #[test]
    fn transient_event_preserves_current_uri_and_queue_position() {
        let mut play_request_id = Some(7);
        let current_uri = CURRENT_URI.to_owned();
        let queue_position = 11;

        let action = player_queue_action(
            &transient_event(uri(CURRENT_URI), false),
            &mut play_request_id,
            &current_uri,
        )
        .expect("event classification should succeed");

        assert_eq!(action, PlayerQueueAction::Preserve);
        assert_eq!(current_uri, CURRENT_URI);
        assert_eq!(queue_position, 11);
        assert_eq!(play_request_id, Some(7));
    }
}
