use std::{
    collections::HashMap,
    fmt, fs,
    fs::File,
    future::Future,
    io::{self, Read, Seek, SeekFrom},
    mem,
    pin::Pin,
    process::exit,
    sync::Mutex,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    thread,
    time::{Duration, Instant},
};

#[cfg(feature = "passthrough-decoder")]
use crate::decoder::PassthroughDecoder;
use crate::{
    audio::{
        AudioDecrypt, AudioFetchParams, AudioFile, AudioFileErrorKind, StreamLoaderController,
    },
    audio_backend::Sink,
    config::{Bitrate, NormalisationMethod, NormalisationType, PlayerConfig},
    convert::Converter,
    core::{
        Error, Session, SpotifyId, SpotifyUri, audio_key::AudioKeyError, cdn_url::CdnUrlError,
        error::ErrorKind, http_client::HttpClientError, mercury::MercuryError,
        session::SessionError, util::SeqGenerator,
    },
    decoder::{AudioDecoder, AudioPacket, AudioPacketPosition, DecoderError, SymphoniaDecoder},
    local_file::{LocalFileLookup, create_local_file_lookup},
    metadata::audio::{AudioFileFormat, AudioFiles, AudioItem},
    mixer::VolumeGetter,
    secondary::{Decoder, SecondaryDecodeEvent, SourceDecoder},
    transition::{NoTransitionPolicy, TransitionEngine, TransitionPolicy},
};
use futures_util::{StreamExt, future::FusedFuture, stream::futures_unordered::FuturesUnordered};
use librespot_metadata::{audio::UniqueFields, track::Tracks};

use symphonia::core::io::MediaSource;
use symphonia::core::probe::Hint;
use tokio::{
    sync::{mpsc, oneshot},
    time::Sleep,
};

use crate::{NUM_CHANNELS, SAMPLE_RATE, SAMPLES_PER_SECOND};

const PRELOAD_NEXT_TRACK_BEFORE_END_DURATION_MS: u32 = 30000;
pub const DB_VOLTAGE_RATIO: f64 = 20.0;
pub const PCM_AT_0DBFS: f64 = 1.0;

// Spotify inserts a custom Ogg packet at the start with custom metadata values, that you would
// otherwise expect in Vorbis comments. This packet isn't well-formed and players may balk at it.
const SPOTIFY_OGG_HEADER_END: u64 = 0xa7;

const LOAD_HANDLES_POISON_MSG: &str = "load handles mutex should not be poisoned";
const MAX_TRACK_RECOVERY_ATTEMPTS: usize = 8;
const MAX_TRACK_RECOVERY_DELAY: Duration = Duration::from_secs(30);
const MIN_SLOW_RECOVERY_DELAY: Duration = Duration::from_secs(30);
const MAX_SLOW_RECOVERY_DELAY: Duration = Duration::from_secs(60);

pub type PlayerResult = Result<(), Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerLoadErrorKind {
    PermanentTrack,
    TransientNetwork,
    TransientService,
    SessionInvalid,
    Cancelled,
}

#[derive(Debug, thiserror::Error)]
#[error("{kind:?}: {source}")]
struct PlayerLoadError {
    kind: PlayerLoadErrorKind,
    #[source]
    source: Error,
}

impl PlayerLoadError {
    fn new(kind: PlayerLoadErrorKind, source: Error) -> Self {
        Self { kind, source }
    }

    fn permanent<E>(source: E) -> Self
    where
        E: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        Self::new(
            PlayerLoadErrorKind::PermanentTrack,
            Error::failed_precondition(source),
        )
    }

    fn from_error(session: &Session, source: Error) -> Self {
        use PlayerLoadErrorKind::*;

        let kind = if session.is_invalid()
            || source.kind == ErrorKind::Unauthenticated
            || matches!(
                source.error.downcast_ref::<SessionError>(),
                Some(SessionError::NotConnected)
            ) {
            SessionInvalid
        } else if let Some(error) = source
            .error
            .downcast_ref::<librespot_metadata::MetadataError>()
        {
            match error {
                librespot_metadata::MetadataError::NonPlayable
                | librespot_metadata::MetadataError::InvalidDuration(_)
                | librespot_metadata::MetadataError::ExplicitContentFiltered => PermanentTrack,
                librespot_metadata::MetadataError::Empty => TransientService,
            }
        } else if source.error.downcast_ref::<HttpClientError>().is_some()
            || source.error.downcast_ref::<MercuryError>().is_some()
            || source.error.downcast_ref::<CdnUrlError>().is_some()
            || source.error.downcast_ref::<AudioKeyError>().is_some()
        {
            TransientService
        } else {
            match source.kind {
                ErrorKind::Cancelled => Cancelled,
                ErrorKind::DeadlineExceeded
                | ErrorKind::Aborted
                | ErrorKind::DataLoss
                | ErrorKind::Unavailable => TransientNetwork,
                _ => TransientService,
            }
        };

        Self::new(kind, source)
    }

    fn message(kind: PlayerLoadErrorKind, message: impl Into<String>) -> Self {
        Self::new(
            kind,
            Error::failed_precondition(io::Error::new(io::ErrorKind::InvalidData, message.into())),
        )
    }

    fn from_decoder_error(session: &Session, source: DecoderError) -> Self {
        use PlayerLoadErrorKind::*;

        let kind = if session.is_invalid() {
            SessionInvalid
        } else {
            match &source {
                DecoderError::AudioFile(failure) => match failure.kind {
                    AudioFileErrorKind::TransientNetwork => TransientNetwork,
                    AudioFileErrorKind::TransientService => TransientService,
                    AudioFileErrorKind::SessionInvalid => SessionInvalid,
                    AudioFileErrorKind::Cancelled => Cancelled,
                    AudioFileErrorKind::PermanentMedia => PermanentTrack,
                },
                DecoderError::Io(error) => match error.kind() {
                    io::ErrorKind::TimedOut
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::NotConnected
                    | io::ErrorKind::UnexpectedEof => TransientNetwork,
                    io::ErrorKind::Interrupted => Cancelled,
                    io::ErrorKind::InvalidData => PermanentTrack,
                    _ => TransientService,
                },
                DecoderError::PassthroughDecoder(_) | DecoderError::SymphoniaDecoder(_) => {
                    PermanentTrack
                }
            }
        };

        Self::new(kind, source.into())
    }
}

pub struct Player {
    commands: Option<mpsc::UnboundedSender<PlayerCommand>>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum SinkStatus {
    Running,
    Closed,
    TemporarilyClosed,
}

pub type SinkEventCallback = Box<dyn Fn(SinkStatus) + Send>;

struct PlayerInternal {
    session: Session,
    config: PlayerConfig,
    commands: mpsc::UnboundedReceiver<PlayerCommand>,
    load_handles: Arc<Mutex<HashMap<thread::ThreadId, thread::JoinHandle<()>>>>,

    state: PlayerState,
    preload: PlayerPreload,
    secondary_generation: u64,
    recovery: Option<PlayerRecovery>,
    recovery_generation: u64,
    network_health: RecoveryHealth,
    #[cfg(test)]
    recovery_load_script: std::collections::VecDeque<Result<PlaybackSource, PlayerLoadError>>,
    sink: Box<dyn Sink>,
    sink_status: SinkStatus,
    sink_event_callback: Option<SinkEventCallback>,
    volume_getter: Box<dyn VolumeGetter + Send>,
    event_senders: Vec<mpsc::UnboundedSender<PlayerEvent>>,
    converter: Converter,
    transition_policy: NoTransitionPolicy,
    transition: TransitionEngine,

    // The existing dynamic limiter is an output-stage processor. Keeping its history global
    // preserves single-source behavior; a future audible-mixing pass should place it after the
    // transition engine rather than duplicate it per decoder.
    normalisation_integrators: [f64; 2],
    normalisation_peaks: [f64; 2],
    normalisation_channel: usize,
    normalisation_knee_factor: f64,

    auto_normalise_as_album: bool,

    player_id: usize,
    play_request_id_generator: SeqGenerator<u64>,
    last_progress_update: Instant,

    local_file_lookup: Arc<LocalFileLookup>,
}

static PLAYER_COUNTER: AtomicUsize = AtomicUsize::new(0);

enum PlayerCommand {
    Load {
        track_id: SpotifyUri,
        play: bool,
        position_ms: u32,
    },
    Preload {
        track_id: SpotifyUri,
    },
    Play,
    Pause,
    Stop,
    Seek(u32),
    SetSession(Session),
    AddEventSender(mpsc::UnboundedSender<PlayerEvent>),
    SetSinkEventCallback(Option<SinkEventCallback>),
    EmitVolumeChangedEvent(u16),
    SetAutoNormaliseAsAlbum(bool),
    EmitSessionDisconnectedEvent {
        connection_id: String,
        user_name: String,
    },
    EmitSessionConnectedEvent {
        connection_id: String,
        user_name: String,
    },
    EmitSessionClientChangedEvent {
        client_id: String,
        client_name: String,
        client_brand_name: String,
        client_model_name: String,
    },
    EmitFilterExplicitContentChangedEvent(bool),
    EmitShuffleChangedEvent(bool),
    EmitRepeatChangedEvent {
        context: bool,
        track: bool,
    },
    EmitAutoPlayChangedEvent(bool),
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    // Play request id changed
    PlayRequestIdChanged {
        play_request_id: u64,
    },
    // Fired when the player is stopped (e.g. by issuing a "stop" command to the player).
    Stopped {
        play_request_id: u64,
        track_id: SpotifyUri,
    },
    // The player is delayed by loading a track.
    Loading {
        play_request_id: u64,
        track_id: SpotifyUri,
        position_ms: u32,
    },
    // The player is preloading a track.
    Preloading {
        track_id: SpotifyUri,
    },
    // The player is playing a track.
    // This event is issued at the start of playback of whenever the position must be communicated
    // because it is out of sync. This includes:
    // start of a track
    // un-pausing
    // after a seek
    // after a buffer-underrun
    Playing {
        play_request_id: u64,
        track_id: SpotifyUri,
        position_ms: u32,
    },
    // The player entered a paused state.
    Paused {
        play_request_id: u64,
        track_id: SpotifyUri,
        position_ms: u32,
    },
    // The player thinks it's a good idea to issue a preload command for the next track now.
    // This event is intended for use within spirc.
    TimeToPreloadNextTrack {
        play_request_id: u64,
        track_id: SpotifyUri,
    },
    // The player reached the end of a track.
    // This event is intended for use within spirc. Spirc will respond by issuing another command.
    EndOfTrack {
        play_request_id: u64,
        track_id: SpotifyUri,
    },
    // The player was unable to load the requested track.
    Unavailable {
        play_request_id: u64,
        track_id: SpotifyUri,
    },
    // Loading was interrupted by a failure that does not prove the track is unavailable.
    LoadFailed {
        play_request_id: u64,
        track_id: SpotifyUri,
        error: PlayerLoadErrorKind,
        is_preload: bool,
    },
    // The mixer volume was set to a new level.
    VolumeChanged {
        volume: u16,
    },
    PositionCorrection {
        play_request_id: u64,
        track_id: SpotifyUri,
        position_ms: u32,
    },
    /// Requires `PlayerConfig::position_update_interval` to be set to Some.
    /// Once set this event will be sent periodically while playing the track to inform about the
    /// current playback position
    PositionChanged {
        play_request_id: u64,
        track_id: SpotifyUri,
        position_ms: u32,
    },
    Seeked {
        play_request_id: u64,
        track_id: SpotifyUri,
        position_ms: u32,
    },
    TrackChanged {
        audio_item: Box<AudioItem>,
    },
    SessionConnected {
        connection_id: String,
        user_name: String,
    },
    SessionDisconnected {
        connection_id: String,
        user_name: String,
    },
    SessionClientChanged {
        client_id: String,
        client_name: String,
        client_brand_name: String,
        client_model_name: String,
    },
    ShuffleChanged {
        shuffle: bool,
    },
    RepeatChanged {
        context: bool,
        track: bool,
    },
    AutoPlayChanged {
        auto_play: bool,
    },
    FilterExplicitContentChanged {
        filter: bool,
    },
}

impl PlayerEvent {
    pub fn get_play_request_id(&self) -> Option<u64> {
        use PlayerEvent::*;
        match self {
            Loading {
                play_request_id, ..
            }
            | Unavailable {
                play_request_id, ..
            }
            | LoadFailed {
                play_request_id, ..
            }
            | Playing {
                play_request_id, ..
            }
            | TimeToPreloadNextTrack {
                play_request_id, ..
            }
            | EndOfTrack {
                play_request_id, ..
            }
            | Paused {
                play_request_id, ..
            }
            | Stopped {
                play_request_id, ..
            }
            | PositionCorrection {
                play_request_id, ..
            }
            | Seeked {
                play_request_id, ..
            } => Some(*play_request_id),
            _ => None,
        }
    }
}

fn load_error_event(
    track_id: SpotifyUri,
    play_request_id: u64,
    error: PlayerLoadErrorKind,
    is_preload: bool,
) -> PlayerEvent {
    if error == PlayerLoadErrorKind::PermanentTrack {
        PlayerEvent::Unavailable {
            track_id,
            play_request_id,
        }
    } else {
        PlayerEvent::LoadFailed {
            track_id,
            play_request_id,
            error,
            is_preload,
        }
    }
}

fn natural_end_of_track_event(track_id: SpotifyUri, play_request_id: u64) -> PlayerEvent {
    PlayerEvent::EndOfTrack {
        track_id,
        play_request_id,
    }
}

pub type PlayerEventChannel = mpsc::UnboundedReceiver<PlayerEvent>;

#[inline]
pub fn db_to_ratio(db: f64) -> f64 {
    f64::powf(10.0, db / DB_VOLTAGE_RATIO)
}

#[inline]
pub fn ratio_to_db(ratio: f64) -> f64 {
    ratio.log10() * DB_VOLTAGE_RATIO
}

pub fn duration_to_coefficient(duration: Duration) -> f64 {
    f64::exp(-1.0 / (duration.as_secs_f64() * SAMPLES_PER_SECOND as f64))
}

pub fn coefficient_to_duration(coefficient: f64) -> Duration {
    Duration::from_secs_f64(-1.0 / f64::ln(coefficient) / SAMPLES_PER_SECOND as f64)
}

#[derive(Clone, Copy, Debug)]
pub struct NormalisationData {
    // Spotify provides these as `f32`, but audio metadata can contain up to `f64`.
    // Also, this negates the need for casting during sample processing.
    pub track_gain_db: f64,
    pub track_peak: f64,
    pub album_gain_db: f64,
    pub album_peak: f64,
}

impl Default for NormalisationData {
    fn default() -> Self {
        Self {
            track_gain_db: 0.0,
            track_peak: 1.0,
            album_gain_db: 0.0,
            album_peak: 1.0,
        }
    }
}

impl NormalisationData {
    fn parse_from_ogg<T: Read + Seek>(mut file: T) -> io::Result<NormalisationData> {
        const SPOTIFY_NORMALIZATION_HEADER_START_OFFSET: u64 = 144;
        const NORMALISATION_DATA_SIZE: usize = 16;

        let newpos = file.seek(SeekFrom::Start(SPOTIFY_NORMALIZATION_HEADER_START_OFFSET))?;
        if newpos != SPOTIFY_NORMALIZATION_HEADER_START_OFFSET {
            error!(
                "NormalisationData::parse_from_file seeking to {SPOTIFY_NORMALIZATION_HEADER_START_OFFSET} but position is now {newpos}"
            );

            error!("Falling back to default (non-track and non-album) normalisation data.");

            return Ok(NormalisationData::default());
        }

        let mut buf = [0u8; NORMALISATION_DATA_SIZE];

        file.read_exact(&mut buf)?;

        let track_gain_db = f32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as f64;
        let track_peak = f32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as f64;
        let album_gain_db = f32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as f64;
        let album_peak = f32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]) as f64;

        Ok(Self {
            track_gain_db,
            track_peak,
            album_gain_db,
            album_peak,
        })
    }

    fn get_factor(config: &PlayerConfig, data: NormalisationData) -> f64 {
        if !config.normalisation {
            return 1.0;
        }

        let (gain_db, gain_peak) = if config.normalisation_type == NormalisationType::Album {
            (data.album_gain_db, data.album_peak)
        } else {
            (data.track_gain_db, data.track_peak)
        };

        // As per the ReplayGain 1.0 & 2.0 (proposed) spec:
        // https://wiki.hydrogenaud.io/index.php?title=ReplayGain_1.0_specification#Clipping_prevention
        // https://wiki.hydrogenaud.io/index.php?title=ReplayGain_2.0_specification#Clipping_prevention
        let normalisation_factor = if config.normalisation_method == NormalisationMethod::Basic {
            // For Basic Normalisation, factor = min(ratio of (ReplayGain + PreGain), 1.0 / peak level).
            // https://wiki.hydrogenaud.io/index.php?title=ReplayGain_1.0_specification#Peak_amplitude
            // https://wiki.hydrogenaud.io/index.php?title=ReplayGain_2.0_specification#Peak_amplitude
            // We then limit that to 1.0 as not to exceed dBFS (0.0 dB).
            let factor = f64::min(
                db_to_ratio(gain_db + config.normalisation_pregain_db),
                PCM_AT_0DBFS / gain_peak,
            );

            if factor > PCM_AT_0DBFS {
                info!(
                    "Lowering gain by {:.2} dB for the duration of this track to avoid potentially exceeding dBFS.",
                    ratio_to_db(factor)
                );

                PCM_AT_0DBFS
            } else {
                factor
            }
        } else {
            // For Dynamic Normalisation it's up to the player to decide,
            // factor = ratio of (ReplayGain + PreGain).
            // We then let the dynamic limiter handle gain reduction.
            let factor = db_to_ratio(gain_db + config.normalisation_pregain_db);
            let threshold_ratio = db_to_ratio(config.normalisation_threshold_dbfs);

            if factor > PCM_AT_0DBFS {
                let factor_db = gain_db + config.normalisation_pregain_db;
                let limiting_db = factor_db + config.normalisation_threshold_dbfs.abs();

                warn!(
                    "This track may exceed dBFS by {factor_db:.2} dB and be subject to {limiting_db:.2} dB of dynamic limiting at its peak."
                );
            } else if factor > threshold_ratio {
                let limiting_db = gain_db
                    + config.normalisation_pregain_db
                    + config.normalisation_threshold_dbfs.abs();

                info!(
                    "This track may be subject to {limiting_db:.2} dB of dynamic limiting at its peak."
                );
            }

            factor
        };

        debug!("Normalisation Data: {data:?}");
        debug!(
            "Calculated Normalisation Factor for {:?}: {:.2}%",
            config.normalisation_type,
            normalisation_factor * 100.0
        );

        normalisation_factor
    }
}

impl Player {
    pub fn new<F>(
        config: PlayerConfig,
        session: Session,
        volume_getter: Box<dyn VolumeGetter + Send>,
        sink_builder: F,
    ) -> Arc<Self>
    where
        F: FnOnce() -> Box<dyn Sink> + Send + 'static,
    {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

        if config.normalisation {
            debug!("Normalisation Type: {:?}", config.normalisation_type);
            debug!(
                "Normalisation Pregain: {:.1} dB",
                config.normalisation_pregain_db
            );
            debug!(
                "Normalisation Threshold: {:.1} dBFS",
                config.normalisation_threshold_dbfs
            );
            debug!("Normalisation Method: {:?}", config.normalisation_method);

            if config.normalisation_method == NormalisationMethod::Dynamic {
                // as_millis() has rounding errors (truncates)
                debug!(
                    "Normalisation Attack: {:.0} ms",
                    coefficient_to_duration(config.normalisation_attack_cf).as_secs_f64() * 1000.
                );
                debug!(
                    "Normalisation Release: {:.0} ms",
                    coefficient_to_duration(config.normalisation_release_cf).as_secs_f64() * 1000.
                );
                debug!("Normalisation Knee: {} dB", config.normalisation_knee_db);
            }
        }

        let handle = thread::spawn(move || {
            let player_id = PLAYER_COUNTER.fetch_add(1, Ordering::AcqRel);
            debug!("new Player [{player_id}]");

            let converter = Converter::new(config.ditherer);
            let normalisation_knee_factor = 1.0 / (8.0 * config.normalisation_knee_db);

            // TODO: it would be neat if we could watch for added or modified files in the
            // specified directories, and dynamically update the lookup. Currently, a new player
            // must be created for any new local files to be playable.
            let local_file_lookup =
                create_local_file_lookup(config.local_file_directories.as_slice());

            let internal = PlayerInternal {
                session,
                config,
                commands: cmd_rx,
                load_handles: Arc::new(Mutex::new(HashMap::new())),

                state: PlayerState::Stopped,
                preload: PlayerPreload::None,
                secondary_generation: 0,
                recovery: None,
                recovery_generation: 0,
                network_health: RecoveryHealth::Healthy,
                #[cfg(test)]
                recovery_load_script: std::collections::VecDeque::new(),
                sink: sink_builder(),
                sink_status: SinkStatus::Closed,
                sink_event_callback: None,
                volume_getter,
                event_senders: vec![],
                converter,
                transition_policy: NoTransitionPolicy,
                transition: TransitionEngine::new(SAMPLE_RATE, NUM_CHANNELS as usize),

                normalisation_peaks: [0.0; 2],
                normalisation_integrators: [0.0; 2],
                normalisation_channel: 0,
                normalisation_knee_factor,

                auto_normalise_as_album: false,

                player_id,
                play_request_id_generator: SeqGenerator::new(0),
                last_progress_update: Instant::now(),

                local_file_lookup: Arc::new(local_file_lookup),
            };

            // While PlayerInternal is written as a future, it still contains blocking code.
            // It must be run by using block_on() in a dedicated thread.
            let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
            runtime.block_on(internal);

            debug!("PlayerInternal thread finished.");
        });

        Arc::new(Self {
            commands: Some(cmd_tx),
            thread_handle: Some(handle),
        })
    }

    pub fn is_invalid(&self) -> bool {
        if let Some(handle) = self.thread_handle.as_ref() {
            return handle.is_finished();
        }
        true
    }

    fn command(&self, cmd: PlayerCommand) {
        if let Some(commands) = self.commands.as_ref() {
            if let Err(e) = commands.send(cmd) {
                error!("Player Commands Error: {e}");
            }
        }
    }

    pub fn load(&self, track_id: SpotifyUri, start_playing: bool, position_ms: u32) {
        self.command(PlayerCommand::Load {
            track_id,
            play: start_playing,
            position_ms,
        });
    }

    pub fn preload(&self, track_id: SpotifyUri) {
        self.command(PlayerCommand::Preload { track_id });
    }

    pub fn play(&self) {
        self.command(PlayerCommand::Play)
    }

    pub fn pause(&self) {
        self.command(PlayerCommand::Pause)
    }

    pub fn stop(&self) {
        self.command(PlayerCommand::Stop)
    }

    pub fn seek(&self, position_ms: u32) {
        self.command(PlayerCommand::Seek(position_ms));
    }

    /// Replace the session used for new media operations. Any latched same-track recovery is
    /// restarted against the replacement while retaining its URI, position, and play intent.
    pub fn set_session(&self, session: Session) {
        // A recovery waiting on an invalid session is intentionally owned by the player. Replacing
        // the session restarts that same URI/position under a newer generation without requiring
        // the caller to reconstruct the Player.
        self.command(PlayerCommand::SetSession(session));
    }

    pub fn get_player_event_channel(&self) -> PlayerEventChannel {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        self.command(PlayerCommand::AddEventSender(event_sender));
        event_receiver
    }

    pub async fn await_end_of_track(&self) {
        let mut channel = self.get_player_event_channel();
        while let Some(event) = channel.recv().await {
            if matches!(
                event,
                PlayerEvent::EndOfTrack { .. } | PlayerEvent::Stopped { .. }
            ) {
                return;
            }
        }
    }

    pub fn set_sink_event_callback(&self, callback: Option<SinkEventCallback>) {
        self.command(PlayerCommand::SetSinkEventCallback(callback));
    }

    pub fn emit_volume_changed_event(&self, volume: u16) {
        self.command(PlayerCommand::EmitVolumeChangedEvent(volume));
    }

    pub fn set_auto_normalise_as_album(&self, setting: bool) {
        self.command(PlayerCommand::SetAutoNormaliseAsAlbum(setting));
    }

    pub fn emit_filter_explicit_content_changed_event(&self, filter: bool) {
        self.command(PlayerCommand::EmitFilterExplicitContentChangedEvent(filter));
    }

    pub fn emit_session_connected_event(&self, connection_id: String, user_name: String) {
        self.command(PlayerCommand::EmitSessionConnectedEvent {
            connection_id,
            user_name,
        });
    }

    pub fn emit_session_disconnected_event(&self, connection_id: String, user_name: String) {
        self.command(PlayerCommand::EmitSessionDisconnectedEvent {
            connection_id,
            user_name,
        });
    }

    pub fn emit_session_client_changed_event(
        &self,
        client_id: String,
        client_name: String,
        client_brand_name: String,
        client_model_name: String,
    ) {
        self.command(PlayerCommand::EmitSessionClientChangedEvent {
            client_id,
            client_name,
            client_brand_name,
            client_model_name,
        });
    }

    pub fn emit_shuffle_changed_event(&self, shuffle: bool) {
        self.command(PlayerCommand::EmitShuffleChangedEvent(shuffle));
    }

    pub fn emit_repeat_changed_event(&self, context: bool, track: bool) {
        self.command(PlayerCommand::EmitRepeatChangedEvent { context, track });
    }

    pub fn emit_auto_play_changed_event(&self, auto_play: bool) {
        self.command(PlayerCommand::EmitAutoPlayChangedEvent(auto_play));
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        debug!("Shutting down player thread ...");
        self.commands = None;
        if let Some(handle) = self.thread_handle.take() {
            if let Err(e) = handle.join() {
                error!("Player thread Error: {e:?}");
            }
        }
    }
}

/// Decoder-local runtime owned independently for each loaded audio source.
///
/// This deliberately excludes player intent, request identity, recovery, events, and sink state.
/// A ready preload and the externally current track therefore have the same ownership shape and
/// can be promoted by moving this value without rebuilding its decoder.
struct PlaybackSource {
    decoder: SourceDecoder,
    normalisation_data: NormalisationData,
    normalisation_factor: f64,
    stream_loader_controller: StreamLoaderController,
    audio_item: AudioItem,
    bytes_per_second: usize,
    duration_ms: u32,
    stream_position_ms: u32,
    reported_nominal_start_time: Option<Instant>,
    suggested_to_preload_next_track: bool,
    is_explicit: bool,
}

type TrackLoaderFuture =
    Pin<Box<dyn FusedFuture<Output = Result<PlaybackSource, PlayerLoadError>> + Send>>;

struct CancellableTrackLoader {
    receiver: oneshot::Receiver<Result<PlaybackSource, PlayerLoadError>>,
    cancelled: Arc<AtomicBool>,
    active_controller: Arc<Mutex<Option<StreamLoaderController>>>,
    terminated: bool,
}

impl Future for CancellableTrackLoader {
    type Output = Result<PlaybackSource, PlayerLoadError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.receiver).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                self.terminated = true;
                self.active_controller
                    .lock()
                    .expect(LOAD_HANDLES_POISON_MSG)
                    .take();
                Poll::Ready(result.unwrap_or_else(|error| {
                    Err(PlayerLoadError::new(
                        PlayerLoadErrorKind::Cancelled,
                        Error::cancelled(error),
                    ))
                }))
            }
        }
    }
}

impl FusedFuture for CancellableTrackLoader {
    fn is_terminated(&self) -> bool {
        self.terminated
    }
}

impl Drop for CancellableTrackLoader {
    fn drop(&mut self) {
        if self.terminated {
            return;
        }

        self.cancelled.store(true, Ordering::Release);
        if let Some(controller) = self
            .active_controller
            .lock()
            .expect(LOAD_HANDLES_POISON_MSG)
            .take()
        {
            controller.close();
        }
    }
}

enum PlayerPreload {
    None,
    Loading {
        track_id: SpotifyUri,
        loader: TrackLoaderFuture,
    },
    Ready {
        track_id: SpotifyUri,
        // Intentionally dormant: decoder reads can block, so only ownership is prepared here.
        // A bounded worker/buffer must be introduced before this source may advance concurrently.
        source: Box<PlaybackSource>,
    },
}

impl PlayerPreload {
    fn track_id(&self) -> Option<&SpotifyUri> {
        match self {
            Self::Loading { track_id, .. } | Self::Ready { track_id, .. } => Some(track_id),
            Self::None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveryHealth {
    Healthy,
    Degraded,
    Recovering,
}

enum RecoveryPhase {
    Waiting(Pin<Box<Sleep>>),
    Loading(TrackLoaderFuture),
    WaitingForSession,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveryMode {
    Fast,
    Slow,
}

struct PlayerRecovery {
    track_id: SpotifyUri,
    play_request_id: u64,
    position_ms: u32,
    start_playback: bool,
    generation: u64,
    attempt: usize,
    mode: RecoveryMode,
    phase: RecoveryPhase,
}

struct RecoveryRequest {
    track_id: SpotifyUri,
    play_request_id: u64,
    position_ms: u32,
    start_playback: bool,
    kind: PlayerLoadErrorKind,
    failed_attempts: usize,
    buffer_starved: bool,
}

impl PlayerRecovery {
    fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }

    fn set_play_intent(&mut self, start_playback: bool) {
        self.start_playback = start_playback;
    }

    fn restart(&mut self, generation: u64, position_ms: u32, session_invalid: bool) {
        self.generation = generation;
        self.position_ms = position_ms;
        self.attempt = 0;
        self.mode = RecoveryMode::Fast;
        self.phase = if session_invalid {
            RecoveryPhase::WaitingForSession
        } else {
            RecoveryPhase::Waiting(Box::pin(tokio::time::sleep(Duration::ZERO)))
        };
    }
}

enum PlayerState {
    Stopped,
    Loading {
        track_id: SpotifyUri,
        play_request_id: u64,
        start_playback: bool,
        position_ms: u32,
        loader: TrackLoaderFuture,
    },
    Paused {
        track_id: SpotifyUri,
        play_request_id: u64,
        source: PlaybackSource,
    },
    Playing {
        track_id: SpotifyUri,
        play_request_id: u64,
        source: PlaybackSource,
    },
    EndOfTrack {
        track_id: SpotifyUri,
        play_request_id: u64,
        source: PlaybackSource,
    },
    Invalid,
}

impl PlayerState {
    fn is_playing(&self) -> bool {
        use self::PlayerState::*;
        match *self {
            Stopped | EndOfTrack { .. } | Paused { .. } | Loading { .. } => false,
            Playing { .. } => true,
            Invalid => {
                error!("PlayerState::is_playing in invalid state");
                exit(1);
            }
        }
    }

    #[allow(dead_code)]
    fn is_stopped(&self) -> bool {
        use self::PlayerState::*;
        matches!(self, Stopped)
    }

    #[allow(dead_code)]
    fn is_loading(&self) -> bool {
        use self::PlayerState::*;
        matches!(self, Loading { .. })
    }

    fn source_mut(&mut self) -> Option<&mut PlaybackSource> {
        use self::PlayerState::*;
        match self {
            Paused { source, .. } | Playing { source, .. } => Some(source),
            Stopped | Loading { .. } | EndOfTrack { .. } => None,
            Invalid => {
                error!("PlayerState::source_mut in invalid state");
                exit(1);
            }
        }
    }

    fn playing_to_end_of_track(&mut self) {
        use self::PlayerState::*;
        let new_state = mem::replace(self, Invalid);
        match new_state {
            Playing {
                track_id,
                play_request_id,
                source,
            } => {
                *self = EndOfTrack {
                    track_id,
                    play_request_id,
                    source,
                };
            }
            _ => {
                error!("Called playing_to_end_of_track in non-playing state: {new_state:?}");
                exit(1);
            }
        }
    }

    fn paused_to_playing(&mut self) {
        use self::PlayerState::*;
        let new_state = mem::replace(self, Invalid);
        match new_state {
            Paused {
                track_id,
                play_request_id,
                mut source,
            } => {
                source.reported_nominal_start_time = Instant::now()
                    .checked_sub(Duration::from_millis(u64::from(source.stream_position_ms)));
                *self = Playing {
                    track_id,
                    play_request_id,
                    source,
                };
            }
            _ => {
                error!("PlayerState::paused_to_playing in invalid state: {new_state:?}");
                exit(1);
            }
        }
    }

    fn playing_to_paused(&mut self) {
        use self::PlayerState::*;
        let new_state = mem::replace(self, Invalid);
        match new_state {
            Playing {
                track_id,
                play_request_id,
                mut source,
            } => {
                source.reported_nominal_start_time = None;
                *self = Paused {
                    track_id,
                    play_request_id,
                    source,
                };
            }
            _ => {
                error!("PlayerState::playing_to_paused in invalid state: {new_state:?}");
                exit(1);
            }
        }
    }
}

struct PlayerTrackLoader {
    session: Session,
    config: PlayerConfig,
    local_file_lookup: Arc<LocalFileLookup>,
}

impl PlayerTrackLoader {
    async fn find_available_alternative(
        &self,
        audio_item: AudioItem,
    ) -> Result<AudioItem, PlayerLoadError> {
        if let Err(e) = audio_item.availability {
            error!("Track is unavailable: {e}");
            Err(PlayerLoadError::permanent(e))
        } else if !audio_item.files.is_empty() {
            Ok(audio_item)
        } else if let Some(alternatives) = audio_item.alternatives {
            let Tracks(alternatives_vec) = alternatives; // required to make `into_iter` able to move

            let mut alternatives: FuturesUnordered<_> = alternatives_vec
                .into_iter()
                .map(|alt_id| AudioItem::get_file(&self.session, alt_id))
                .collect();

            let mut failure: Option<PlayerLoadError> = None;
            while let Some(alternative) = alternatives.next().await {
                match alternative {
                    Ok(alternative)
                        if alternative.availability.is_ok() && !alternative.files.is_empty() =>
                    {
                        return Ok(alternative);
                    }
                    Ok(_) => (),
                    Err(error) => {
                        let error = PlayerLoadError::from_error(&self.session, error);
                        if failure.as_ref().is_none_or(|previous| {
                            previous.kind == PlayerLoadErrorKind::PermanentTrack
                        }) {
                            // Any transient alternative lookup failure prevents us from proving
                            // that the media item itself is permanently unavailable.
                            failure = Some(error);
                        }
                    }
                }
            }

            Err(failure.unwrap_or_else(|| {
                PlayerLoadError::message(
                    PlayerLoadErrorKind::PermanentTrack,
                    "track and all alternatives have no playable files",
                )
            }))
        } else {
            error!("Track should be available, but no alternatives found.");
            Err(PlayerLoadError::message(
                PlayerLoadErrorKind::PermanentTrack,
                "track has no playable files or alternatives",
            ))
        }
    }

    fn stream_data_rate(&self, format: AudioFileFormat) -> Option<usize> {
        let kbps = match format {
            AudioFileFormat::OGG_VORBIS_96 => 12.,
            AudioFileFormat::OGG_VORBIS_160 => 20.,
            AudioFileFormat::OGG_VORBIS_320 => 40.,
            AudioFileFormat::MP3_256 => 32.,
            AudioFileFormat::MP3_320 => 40.,
            AudioFileFormat::MP3_160 => 20.,
            AudioFileFormat::MP3_96 => 12.,
            AudioFileFormat::MP3_160_ENC => 20.,
            AudioFileFormat::AAC_24 => 3.,
            AudioFileFormat::AAC_48 => 6.,
            AudioFileFormat::AAC_160 => 20.,
            AudioFileFormat::AAC_320 => 40.,
            AudioFileFormat::MP4_128 => 16.,
            AudioFileFormat::OTHER5 => 40.,
            AudioFileFormat::FLAC_FLAC => 112., // assume 900 kbit/s on average
            AudioFileFormat::XHE_AAC_12 => 1.5,
            AudioFileFormat::XHE_AAC_16 => 2.,
            AudioFileFormat::XHE_AAC_24 => 3.,
            AudioFileFormat::FLAC_FLAC_24BIT => 3.,
        };
        let data_rate: f32 = kbps * 1024.;
        Some(data_rate.ceil() as usize)
    }

    async fn load_track(
        &self,
        track_uri: SpotifyUri,
        position_ms: u32,
    ) -> Result<PlaybackSource, PlayerLoadError> {
        match track_uri {
            SpotifyUri::Track { .. } | SpotifyUri::Episode { .. } => {
                self.load_remote_track(track_uri, position_ms).await
            }
            SpotifyUri::Local { .. } => self.load_local_track(track_uri, position_ms).await,
            _ => {
                error!("Cannot handle load of track with URI: <{track_uri}>",);
                Err(PlayerLoadError::message(
                    PlayerLoadErrorKind::PermanentTrack,
                    format!("unsupported media URI: {track_uri}"),
                ))
            }
        }
    }

    async fn load_remote_track(
        &self,
        track_uri: SpotifyUri,
        position_ms: u32,
    ) -> Result<PlaybackSource, PlayerLoadError> {
        let track_id: SpotifyId = match (&track_uri).try_into() {
            Ok(id) => id,
            Err(error) => {
                warn!("<{track_uri}> could not be converted to a base62 ID");
                return Err(PlayerLoadError::permanent(error));
            }
        };

        let audio_item = match AudioItem::get_file(&self.session, track_uri).await {
            Ok(audio) => self.find_available_alternative(audio).await?,
            Err(e) => {
                error!("Unable to load audio item: {e:?}");
                return Err(PlayerLoadError::from_error(&self.session, e));
            }
        };

        info!(
            "Loading <{}> with Spotify URI <{}>",
            audio_item.name, audio_item.uri
        );

        // (Most) podcasts seem to support only 96 kbps Ogg Vorbis, so fall back to it
        let formats = match self.config.bitrate {
            Bitrate::Bitrate96 => [
                AudioFileFormat::OGG_VORBIS_96,
                AudioFileFormat::MP3_96,
                AudioFileFormat::OGG_VORBIS_160,
                AudioFileFormat::MP3_160,
                AudioFileFormat::MP3_256,
                AudioFileFormat::OGG_VORBIS_320,
                AudioFileFormat::MP3_320,
            ],
            Bitrate::Bitrate160 => [
                AudioFileFormat::OGG_VORBIS_160,
                AudioFileFormat::MP3_160,
                AudioFileFormat::OGG_VORBIS_96,
                AudioFileFormat::MP3_96,
                AudioFileFormat::MP3_256,
                AudioFileFormat::OGG_VORBIS_320,
                AudioFileFormat::MP3_320,
            ],
            Bitrate::Bitrate320 => [
                AudioFileFormat::OGG_VORBIS_320,
                AudioFileFormat::MP3_320,
                AudioFileFormat::MP3_256,
                AudioFileFormat::OGG_VORBIS_160,
                AudioFileFormat::MP3_160,
                AudioFileFormat::OGG_VORBIS_96,
                AudioFileFormat::MP3_96,
            ],
        };

        let (format, file_id) =
            match formats
                .iter()
                .find_map(|format| match audio_item.files.get(format) {
                    Some(&file_id) => Some((*format, file_id)),
                    _ => None,
                }) {
                Some(t) => t,
                None => {
                    warn!(
                        "<{}> is not available in any supported format",
                        audio_item.name
                    );
                    return Err(PlayerLoadError::message(
                        PlayerLoadErrorKind::PermanentTrack,
                        format!(
                            "track <{}> is not available in a supported format",
                            audio_item.name
                        ),
                    ));
                }
            };

        let bytes_per_second = self.stream_data_rate(format).ok_or_else(|| {
            PlayerLoadError::message(
                PlayerLoadErrorKind::PermanentTrack,
                format!("unsupported audio format: {format:?}"),
            )
        })?;

        // This is only a loop to be able to reload the file if an error occurred
        // while opening a cached file.
        loop {
            let encrypted_file = AudioFile::open(&self.session, file_id, bytes_per_second);

            let encrypted_file = match encrypted_file.await {
                Ok(encrypted_file) => encrypted_file,
                Err(e) => {
                    error!("Unable to load encrypted file: {e:?}");
                    return Err(PlayerLoadError::from_error(&self.session, e));
                }
            };

            let is_cached = encrypted_file.is_cached();

            let stream_loader_controller = encrypted_file
                .get_stream_loader_controller()
                .map_err(|e| PlayerLoadError::from_error(&self.session, e))?;

            // Not all audio files are encrypted. If we can't get a key, try loading the track
            // without decryption. If the file was encrypted after all, the decoder will fail
            // parsing and bail out, so we should be safe from outputting ear-piercing noise.
            let (key, key_error) = match self.session.audio_key().request(track_id, file_id).await {
                Ok(key) => (Some(key), None),
                Err(e) => {
                    warn!("Unable to load key, continuing without decryption: {e}");
                    (None, Some(e))
                }
            };

            let mut decrypted_file = AudioDecrypt::new(key, encrypted_file);

            let is_ogg_vorbis = AudioFiles::is_ogg_vorbis(format);
            let (offset, mut normalisation_data) = if is_ogg_vorbis {
                // Spotify stores normalisation data in a custom Ogg packet instead of Vorbis comments.
                let normalisation_data =
                    NormalisationData::parse_from_ogg(&mut decrypted_file).ok();
                (SPOTIFY_OGG_HEADER_END, normalisation_data)
            } else {
                (0, None)
            };

            let audio_file = match Subfile::new(
                decrypted_file,
                offset,
                stream_loader_controller.len() as u64,
            ) {
                Ok(audio_file) => audio_file,
                Err(e) => {
                    error!("PlayerTrackLoader::load_track error opening subfile: {e}");
                    return Err(PlayerLoadError::from_error(&self.session, e.into()));
                }
            };

            let mut symphonia_decoder = |audio_file, format| {
                SymphoniaDecoder::new(audio_file, format).map(|mut decoder| {
                    // For formats other that Vorbis, we'll try getting normalisation data from
                    // ReplayGain metadata fields, if present.
                    if normalisation_data.is_none() {
                        normalisation_data = decoder.normalisation_data();
                    }
                    Box::new(decoder) as Decoder
                })
            };

            let mut hint = Hint::new();
            if let Some(mime_type) = AudioFiles::mime_type(format) {
                hint.mime_type(mime_type);
            }

            #[cfg(feature = "passthrough-decoder")]
            let decoder_type = if self.config.passthrough {
                PassthroughDecoder::new(audio_file, format).map(|x| Box::new(x) as Decoder)
            } else {
                symphonia_decoder(audio_file, hint)
            };

            #[cfg(not(feature = "passthrough-decoder"))]
            let decoder_type = { symphonia_decoder(audio_file, hint) };

            let normalisation_data = normalisation_data.unwrap_or_else(|| {
                warn!("Unable to get normalisation data, continuing with defaults.");
                NormalisationData::default()
            });

            let mut decoder = match decoder_type {
                Ok(decoder) => decoder,
                Err(e) if is_cached => {
                    warn!("Unable to read cached audio file: {e}. Trying to download it.");

                    match self.session.cache() {
                        Some(cache) => {
                            if cache.remove_file(file_id).is_err() {
                                error!("Error removing file from cache");
                                return Err(PlayerLoadError::message(
                                    PlayerLoadErrorKind::TransientService,
                                    "failed to remove unreadable cached audio file",
                                ));
                            }
                        }
                        None => {
                            error!("If the audio file is cached, a cache should exist");
                            return Err(PlayerLoadError::message(
                                PlayerLoadErrorKind::TransientService,
                                "cached audio file has no configured cache",
                            ));
                        }
                    }

                    // Just try it again
                    continue;
                }
                Err(e) => {
                    error!("Unable to read audio file: {e}");
                    return Err(match key_error {
                        Some(error) => PlayerLoadError::from_error(&self.session, error),
                        None => PlayerLoadError::from_decoder_error(&self.session, e),
                    });
                }
            };

            let duration_ms = audio_item.duration_ms;
            // Don't try to seek past the track's duration.
            // If the position is invalid just start from
            // the beginning of the track.
            let position_ms = if position_ms > duration_ms {
                warn!(
                    "Invalid start position of {position_ms} ms exceeds track's duration of {duration_ms} ms, starting track from the beginning"
                );
                0
            } else {
                position_ms
            };

            // Ensure the starting position. Even when we want to play from the beginning,
            // the cursor may have been moved by parsing normalisation data. This may not
            // matter for playback (but won't hurt either), but may be useful for the
            // passthrough decoder.
            let stream_position_ms = match decoder.seek(position_ms) {
                Ok(new_position_ms) => new_position_ms,
                Err(e) => {
                    error!(
                        "PlayerTrackLoader::load_track error seeking to starting position {position_ms}: {e}"
                    );
                    return Err(PlayerLoadError::from_decoder_error(&self.session, e));
                }
            };

            // Ensure streaming mode now that we are ready to play from the requested position.
            stream_loader_controller.set_stream_mode();

            let is_explicit = audio_item.is_explicit;

            info!("<{}> ({} ms) loaded", audio_item.name, duration_ms);

            return Ok(PlaybackSource {
                decoder: SourceDecoder::direct(decoder),
                normalisation_data,
                normalisation_factor: 1.0,
                stream_loader_controller,
                audio_item,
                bytes_per_second,
                duration_ms,
                stream_position_ms,
                reported_nominal_start_time: None,
                suggested_to_preload_next_track: false,
                is_explicit,
            });
        }
    }

    async fn load_local_track(
        &self,
        track_uri: SpotifyUri,
        position_ms: u32,
    ) -> Result<PlaybackSource, PlayerLoadError> {
        info!("Loading local file with Spotify URI <{}>", track_uri);

        let SpotifyUri::Local { duration, .. } = track_uri else {
            error!("Unable to determine track duration for local file: not a local file URI");
            return Err(PlayerLoadError::message(
                PlayerLoadErrorKind::PermanentTrack,
                "local media URI has no duration",
            ));
        };

        let entry = self.local_file_lookup.get(&track_uri);

        let Some(path) = entry else {
            error!("Unable to find file path for local file <{track_uri}>");
            return Err(PlayerLoadError::message(
                PlayerLoadErrorKind::PermanentTrack,
                format!("local media file is not indexed: {track_uri}"),
            ));
        };

        let src = match File::open(path) {
            Ok(src) => src,
            Err(e) => {
                error!("Failed to open local file: {e}");
                return Err(PlayerLoadError::from_error(&self.session, e.into()));
            }
        };

        let mut hint = Hint::new();
        if let Some(file_extension) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(file_extension);
        }

        let decoder = match SymphoniaDecoder::new(src, hint) {
            Ok(decoder) => decoder,
            Err(e) => {
                error!("Error decoding local file: {e}");
                return Err(PlayerLoadError::permanent(e));
            }
        };

        let mut decoder = Box::new(decoder);
        let normalisation_data = decoder.normalisation_data().unwrap_or_else(|| {
            warn!("Unable to get normalisation data, continuing with defaults.");
            NormalisationData::default()
        });

        let local_file_metadata = decoder.local_file_metadata().unwrap_or_default();

        let stream_position_ms = match decoder.seek(position_ms) {
            Ok(new_position_ms) => new_position_ms,
            Err(e) => {
                error!(
                    "PlayerTrackLoader::load_local_track error seeking to starting position {position_ms}: {e}"
                );
                return Err(PlayerLoadError::permanent(e));
            }
        };

        let file_size = fs::metadata(path)
            .map_err(|e| PlayerLoadError::from_error(&self.session, e.into()))?
            .len();
        let bytes_per_second = (file_size / duration.as_secs()) as usize;

        let stream_loader_controller = StreamLoaderController::from_local_file(file_size);

        let name = local_file_metadata.name.unwrap_or_default();

        info!("Loaded <{name}> from path <{}>", path.display());

        Ok(PlaybackSource {
            decoder: SourceDecoder::direct(decoder),
            normalisation_data,
            normalisation_factor: 1.0,
            stream_loader_controller,
            bytes_per_second,
            duration_ms: duration.as_millis() as u32,
            stream_position_ms,
            reported_nominal_start_time: None,
            suggested_to_preload_next_track: false,
            is_explicit: false,
            audio_item: AudioItem {
                duration_ms: duration.as_millis() as u32,
                uri: track_uri.to_uri().unwrap_or_default(),
                track_id: track_uri,
                files: Default::default(),
                name,
                // We can't get a CoverImage.URL for the track image, applications will have to parse the file metadata themselves using unique_fields.path
                covers: vec![],
                language: local_file_metadata
                    .language
                    .map(|val| vec![val])
                    .unwrap_or_default(),
                is_explicit: false,
                availability: Ok(()),
                alternatives: None,
                unique_fields: UniqueFields::Local {
                    artists: local_file_metadata.artists,
                    album: local_file_metadata.album,
                    album_artists: local_file_metadata.album_artists,
                    number: local_file_metadata.number,
                    disc_number: local_file_metadata.disc_number,
                    path: path.to_path_buf(),
                },
            },
        })
    }
}

impl PlayerInternal {
    fn next_recovery_generation(&mut self) -> u64 {
        self.recovery_generation = self.recovery_generation.wrapping_add(1);
        self.recovery_generation
    }

    fn recovery_delay(generation: u64, next_attempt: usize) -> Duration {
        let exponent = next_attempt.saturating_sub(1).min(7) as u32;
        let base_ms = 250_u64.saturating_mul(1_u64 << exponent);
        let jitter_ms =
            (generation.wrapping_mul(67) ^ (next_attempt as u64).wrapping_mul(131)) % 250;
        Duration::from_millis(base_ms + jitter_ms).min(MAX_TRACK_RECOVERY_DELAY)
    }

    fn slow_recovery_delay(generation: u64, next_attempt: usize) -> Duration {
        let span = MAX_SLOW_RECOVERY_DELAY - MIN_SLOW_RECOVERY_DELAY;
        let jitter = (generation.wrapping_mul(71) ^ (next_attempt as u64).wrapping_mul(149))
            % (span.as_secs() + 1);
        MIN_SLOW_RECOVERY_DELAY + Duration::from_secs(jitter)
    }

    fn recovery_track_loader(
        &mut self,
        track_id: SpotifyUri,
        position_ms: u32,
    ) -> TrackLoaderFuture {
        #[cfg(test)]
        if let Some(outcome) = self.recovery_load_script.pop_front() {
            return Box::pin(futures_util::future::ready(outcome));
        }

        Box::pin(self.load_track(track_id, position_ms, true))
    }

    fn cancel_recovery(&mut self, reason: &str) {
        if let Some(recovery) = self.recovery.take() {
            debug!(
                "Cancelling recovery generation {} for <{}> at {} ms because {reason}",
                recovery.generation, recovery.track_id, recovery.position_ms
            );
            self.next_recovery_generation();
            self.network_health = RecoveryHealth::Healthy;
        }
    }

    fn begin_recovery(&mut self, request: RecoveryRequest) {
        let RecoveryRequest {
            track_id,
            play_request_id,
            position_ms,
            start_playback,
            kind,
            failed_attempts,
            buffer_starved,
        } = request;
        self.cancel_secondary_source("current-track recovery started");
        self.cancel_recovery("a newer recovery superseded it");
        let generation = self.next_recovery_generation();

        if buffer_starved {
            debug!(
                "Compressed buffer starved for <{track_id}> at {position_ms} ms; entering latched recovery generation {generation}"
            );
            self.ensure_sink_stopped(true);
        } else {
            debug!(
                "Entering recovery generation {generation} for <{track_id}> at {position_ms} ms after {kind:?}"
            );
        }

        self.state = PlayerState::Stopped;
        self.network_health = RecoveryHealth::Recovering;

        let phase = if kind == PlayerLoadErrorKind::SessionInvalid {
            debug!(
                "Session invalid while recovering <{track_id}>; waiting for Player::set_session"
            );
            RecoveryPhase::WaitingForSession
        } else {
            let delay = Self::recovery_delay(generation, failed_attempts + 1);
            debug!(
                "Scheduling same-track recovery for <{track_id}> attempt {} in {delay:?}",
                failed_attempts + 1
            );
            RecoveryPhase::Waiting(Box::pin(tokio::time::sleep(delay)))
        };

        self.recovery = Some(PlayerRecovery {
            track_id: track_id.clone(),
            play_request_id,
            position_ms,
            start_playback,
            generation,
            attempt: failed_attempts,
            mode: RecoveryMode::Fast,
            phase,
        });
        self.send_event(load_error_event(track_id, play_request_id, kind, false));
    }

    fn poll_recovery(&mut self, cx: &mut Context<'_>) -> bool {
        let Some(mut recovery) = self.recovery.take() else {
            return false;
        };

        match &mut recovery.phase {
            RecoveryPhase::Waiting(delay) => match delay.as_mut().poll(cx) {
                Poll::Pending => {
                    self.recovery = Some(recovery);
                    false
                }
                Poll::Ready(()) => {
                    if !recovery.is_current(self.recovery_generation) {
                        debug!(
                            "Discarding stale recovery generation {} for <{}>",
                            recovery.generation, recovery.track_id
                        );
                        return true;
                    }

                    debug!(
                        "Retrying <{}> from {} ms (attempt {}, generation {})",
                        recovery.track_id,
                        recovery.position_ms,
                        recovery.attempt.saturating_add(1),
                        recovery.generation
                    );
                    let loader =
                        self.recovery_track_loader(recovery.track_id.clone(), recovery.position_ms);
                    recovery.phase = RecoveryPhase::Loading(loader);
                    self.recovery = Some(recovery);
                    true
                }
            },
            RecoveryPhase::Loading(loader) => match loader.as_mut().poll(cx) {
                Poll::Pending => {
                    self.recovery = Some(recovery);
                    false
                }
                Poll::Ready(Ok(loaded_track)) => {
                    if !recovery.is_current(self.recovery_generation) {
                        debug!(
                            "Ignoring successful stale recovery generation {} for <{}>",
                            recovery.generation, recovery.track_id
                        );
                        return true;
                    }

                    debug!(
                        "Recovery generation {} succeeded for <{}> at {} ms; resuming once",
                        recovery.generation, recovery.track_id, loaded_track.stream_position_ms
                    );
                    self.network_health = RecoveryHealth::Healthy;
                    self.start_playback(
                        recovery.track_id,
                        recovery.play_request_id,
                        loaded_track,
                        recovery.start_playback,
                    );
                    true
                }
                Poll::Ready(Err(error)) => {
                    if !recovery.is_current(self.recovery_generation) {
                        debug!(
                            "Ignoring failed stale recovery generation {} for <{}>",
                            recovery.generation, recovery.track_id
                        );
                        return true;
                    }

                    recovery.attempt = recovery.attempt.saturating_add(1);
                    debug!(
                        "Recovery attempt {} for <{}> at {} ms failed as {:?}",
                        recovery.attempt, recovery.track_id, recovery.position_ms, error.kind
                    );

                    match error.kind {
                        PlayerLoadErrorKind::PermanentTrack => {
                            self.network_health = RecoveryHealth::Healthy;
                            self.send_event(PlayerEvent::Unavailable {
                                track_id: recovery.track_id,
                                play_request_id: recovery.play_request_id,
                            });
                        }
                        PlayerLoadErrorKind::Cancelled => {
                            debug!(
                                "Recovery generation {} for <{}> was cancelled",
                                recovery.generation, recovery.track_id
                            );
                            self.network_health = RecoveryHealth::Healthy;
                        }
                        PlayerLoadErrorKind::SessionInvalid => {
                            debug!(
                                "Recovery generation {} is waiting for a replacement session",
                                recovery.generation
                            );
                            recovery.phase = RecoveryPhase::WaitingForSession;
                            self.recovery = Some(recovery);
                        }
                        _ if recovery.mode == RecoveryMode::Fast
                            && recovery.attempt < MAX_TRACK_RECOVERY_ATTEMPTS =>
                        {
                            let next_attempt = recovery.attempt.saturating_add(1);
                            let delay = Self::recovery_delay(recovery.generation, next_attempt);
                            debug!(
                                "Recovery generation {} remains latched; attempt {} in {delay:?}",
                                recovery.generation, next_attempt
                            );
                            recovery.phase =
                                RecoveryPhase::Waiting(Box::pin(tokio::time::sleep(delay)));
                            self.recovery = Some(recovery);
                        }
                        _ => {
                            if recovery.mode == RecoveryMode::Fast {
                                debug!(
                                    "Recovery generation {} exhausted {} fast attempts for <{}>; entering low-frequency latched recovery",
                                    recovery.generation, recovery.attempt, recovery.track_id
                                );
                                recovery.mode = RecoveryMode::Slow;
                            }
                            let next_attempt = recovery.attempt.saturating_add(1);
                            let delay =
                                Self::slow_recovery_delay(recovery.generation, next_attempt);
                            debug!(
                                "Slow recovery generation {} remains latched; retry {} for <{}> in {delay:?}",
                                recovery.generation, next_attempt, recovery.track_id
                            );
                            recovery.phase =
                                RecoveryPhase::Waiting(Box::pin(tokio::time::sleep(delay)));
                            self.recovery = Some(recovery);
                        }
                    }
                    true
                }
            },
            RecoveryPhase::WaitingForSession => {
                self.recovery = Some(recovery);
                false
            }
        }
    }
}

impl Future for PlayerInternal {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // While this is written as a future, it still contains blocking code.
        // It must be run on its own thread.
        let passthrough = self.config.passthrough;

        loop {
            let mut all_futures_completed_or_not_ready = true;

            // process commands that were sent to us
            let cmd = match self.commands.poll_recv(cx) {
                Poll::Ready(None) => return Poll::Ready(()), // client has disconnected - shut down.
                Poll::Ready(Some(cmd)) => {
                    all_futures_completed_or_not_ready = false;
                    Some(cmd)
                }
                _ => None,
            };

            if let Some(cmd) = cmd {
                if let Err(e) = self.handle_command(cmd) {
                    error!("Error handling command: {e}");
                }
            }

            // Handle loading of a new track to play.
            let mut current_load_failure = None;
            if let PlayerState::Loading {
                ref mut loader,
                ref track_id,
                start_playback,
                play_request_id,
                position_ms,
            } = self.state
            {
                // The loader may be terminated if we are trying to load the same track
                // as before, and that track failed to open before.
                let track_id = track_id.clone();

                if !loader.as_mut().is_terminated() {
                    match loader.as_mut().poll(cx) {
                        Poll::Ready(Ok(loaded_track)) => {
                            self.start_playback(
                                track_id,
                                play_request_id,
                                loaded_track,
                                start_playback,
                            );
                            if let PlayerState::Loading { .. } = self.state {
                                error!("The state wasn't changed by start_playback()");
                                exit(1);
                            }
                        }
                        Poll::Ready(Err(e)) => {
                            debug!(
                                "Current load failed for <{track_id}> at {position_ms} ms as {:?}",
                                e.kind
                            );
                            current_load_failure = Some((
                                track_id,
                                play_request_id,
                                position_ms,
                                start_playback,
                                e.kind,
                            ));
                        }
                        Poll::Pending => (),
                    }
                }
            }

            if let Some((track_id, play_request_id, position_ms, start_playback, kind)) =
                current_load_failure
            {
                match kind {
                    PlayerLoadErrorKind::PermanentTrack => {
                        self.state = PlayerState::Stopped;
                        self.send_event(PlayerEvent::Unavailable {
                            track_id,
                            play_request_id,
                        });
                    }
                    PlayerLoadErrorKind::Cancelled => {
                        self.state = PlayerState::Stopped;
                        self.send_event(PlayerEvent::LoadFailed {
                            track_id,
                            play_request_id,
                            error: kind,
                            is_preload: false,
                        });
                    }
                    _ => self.begin_recovery(RecoveryRequest {
                        track_id,
                        play_request_id,
                        position_ms,
                        start_playback,
                        kind,
                        failed_attempts: 1,
                        buffer_starved: false,
                    }),
                }
            }

            if self.poll_recovery(cx) {
                all_futures_completed_or_not_ready = false;
            }

            // handle pending preload requests.
            if let PlayerPreload::Loading {
                ref mut loader,
                ref track_id,
            } = self.preload
            {
                let track_id = track_id.clone();
                match loader.as_mut().poll(cx) {
                    Poll::Ready(Ok(source)) => {
                        self.send_event(PlayerEvent::Preloading {
                            track_id: track_id.clone(),
                        });
                        debug!("Secondary playback source ready for <{track_id}>");
                        self.preload = PlayerPreload::Ready {
                            track_id,
                            source: Box::new(source),
                        };
                        self.arm_transition_if_selected();
                    }
                    Poll::Ready(Err(e)) => {
                        debug!("Unable to preload {track_id:?}: {e}");
                        self.cancel_secondary_source("secondary source load failed");
                        if let PlayerState::Playing {
                            play_request_id, ..
                        }
                        | PlayerState::Paused {
                            play_request_id, ..
                        } = self.state
                        {
                            self.send_event(load_error_event(
                                track_id,
                                play_request_id,
                                e.kind,
                                true,
                            ));
                        }
                    }
                    Poll::Pending => (),
                }
            }

            if self.network_health != RecoveryHealth::Recovering {
                let stream_health = match &self.state {
                    PlayerState::Playing {
                        track_id, source, ..
                    }
                    | PlayerState::Paused {
                        track_id, source, ..
                    } => Some((
                        track_id.clone(),
                        source.stream_position_ms,
                        source.stream_loader_controller.is_degraded(),
                    )),
                    _ => None,
                };
                if let Some((track_id, position_ms, degraded)) = stream_health {
                    match (self.network_health, degraded) {
                        (RecoveryHealth::Healthy, true) => {
                            self.network_health = RecoveryHealth::Degraded;
                            if !matches!(self.preload, PlayerPreload::None) {
                                debug!(
                                    "Cancelling next-track preload to prioritize degraded current playback"
                                );
                                self.cancel_secondary_source("current playback became degraded");
                            }
                            debug!(
                                "Network degraded for <{track_id}> at {position_ms} ms; buffered audio remains available"
                            );
                        }
                        (RecoveryHealth::Degraded, false) => {
                            self.network_health = RecoveryHealth::Healthy;
                            debug!(
                                "Network recovered for <{track_id}> at {position_ms} ms without interrupting playback"
                            );
                        }
                        _ => {}
                    }
                }
            }

            let mut decoder_failure = None;
            if self.state.is_playing() {
                self.ensure_sink_running();

                if let PlayerState::Playing {
                    ref track_id,
                    play_request_id,
                    ref mut source,
                } = self.state
                {
                    let track_id = track_id.clone();
                    let normalisation_factor = source.normalisation_factor;
                    match source.decoder.next_packet() {
                        Ok(result) => {
                            if let Some((ref packet_position, ref packet)) = result {
                                let new_stream_position_ms = packet_position.position_ms;
                                let expected_position_ms = std::mem::replace(
                                    &mut source.stream_position_ms,
                                    new_stream_position_ms,
                                );

                                if !passthrough {
                                    match packet.samples() {
                                        Ok(_) => {
                                            let new_stream_position = Duration::from_millis(
                                                new_stream_position_ms as u64,
                                            );

                                            let now = Instant::now();

                                            // Only notify if we're skipped some packets *or* we are behind.
                                            // If we're ahead it's probably due to a buffer of the backend
                                            // and we're actually in time.
                                            let notify_about_position = match source
                                                .reported_nominal_start_time
                                            {
                                                None => true,
                                                Some(reported_nominal_start_time) => {
                                                    let mut notify = false;

                                                    if packet_position.skipped {
                                                        if let Some(ahead) = new_stream_position
                                                            .checked_sub(Duration::from_millis(
                                                                expected_position_ms as u64,
                                                            ))
                                                        {
                                                            notify |=
                                                                ahead >= Duration::from_secs(1)
                                                        }
                                                    }

                                                    if let Some(lag) = now.checked_duration_since(
                                                        reported_nominal_start_time,
                                                    ) {
                                                        if let Some(lag) =
                                                            lag.checked_sub(new_stream_position)
                                                        {
                                                            notify |= lag >= Duration::from_secs(1)
                                                        }
                                                    }

                                                    notify
                                                }
                                            };

                                            if notify_about_position {
                                                source.reported_nominal_start_time =
                                                    now.checked_sub(new_stream_position);
                                                self.send_event(PlayerEvent::PositionCorrection {
                                                    play_request_id,
                                                    track_id: track_id.clone(),
                                                    position_ms: new_stream_position_ms,
                                                });
                                            }

                                            if let Some(interval) =
                                                self.config.position_update_interval
                                            {
                                                let last_progress_update_since_ms =
                                                    now.duration_since(self.last_progress_update);

                                                if last_progress_update_since_ms > interval {
                                                    self.last_progress_update = now;
                                                    self.send_event(PlayerEvent::PositionChanged {
                                                        play_request_id,
                                                        track_id,
                                                        position_ms: new_stream_position_ms,
                                                    });
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            decoder_failure = Some((
                                                track_id.clone(),
                                                play_request_id,
                                                source.stream_position_ms,
                                                DecoderError::SymphoniaDecoder(format!(
                                                    "decoded packet type mismatch: {e}"
                                                )),
                                            ));
                                        }
                                    }
                                }
                            }

                            if decoder_failure.is_none() {
                                self.handle_packet(result, normalisation_factor);
                            }
                        }
                        Err(e) => {
                            decoder_failure =
                                Some((track_id, play_request_id, source.stream_position_ms, e));
                        }
                    }
                } else {
                    error!("PlayerInternal poll: Invalid PlayerState");
                    exit(1);
                };
            }

            if let Some((track_id, play_request_id, position_ms, source)) = decoder_failure {
                let error = PlayerLoadError::from_decoder_error(&self.session, source);
                debug!(
                    "Decoder stopped for <{track_id}> at {position_ms} ms with classification {:?}",
                    error.kind
                );
                match error.kind {
                    PlayerLoadErrorKind::TransientNetwork
                    | PlayerLoadErrorKind::TransientService
                    | PlayerLoadErrorKind::SessionInvalid => self.begin_recovery(RecoveryRequest {
                        track_id,
                        play_request_id,
                        position_ms,
                        start_playback: true,
                        kind: error.kind,
                        failed_attempts: 0,
                        buffer_starved: true,
                    }),
                    PlayerLoadErrorKind::PermanentTrack => {
                        self.state.playing_to_end_of_track();
                        self.network_health = RecoveryHealth::Healthy;
                        self.send_event(PlayerEvent::Unavailable {
                            track_id,
                            play_request_id,
                        });
                    }
                    PlayerLoadErrorKind::Cancelled => {
                        self.state = PlayerState::Stopped;
                        self.ensure_sink_stopped(true);
                    }
                }
            }

            let allow_preload =
                self.network_health == RecoveryHealth::Healthy && self.recovery.is_none();
            if let PlayerState::Playing {
                ref track_id,
                play_request_id,
                ref mut source,
            }
            | PlayerState::Paused {
                ref track_id,
                play_request_id,
                ref mut source,
            } = self.state
            {
                let track_id = track_id.clone();

                if (!source.suggested_to_preload_next_track)
                    && allow_preload
                    && ((source.duration_ms as i64 - source.stream_position_ms as i64)
                        < PRELOAD_NEXT_TRACK_BEFORE_END_DURATION_MS as i64)
                    && source.stream_loader_controller.range_to_end_available()
                {
                    source.suggested_to_preload_next_track = true;
                    self.send_event(PlayerEvent::TimeToPreloadNextTrack {
                        track_id,
                        play_request_id,
                    });
                }
            }

            if (!self.state.is_playing()) && all_futures_completed_or_not_ready {
                return Poll::Pending;
            }
        }
    }
}

impl PlayerInternal {
    fn next_secondary_generation(&mut self) -> u64 {
        self.secondary_generation = self.secondary_generation.wrapping_add(1);
        self.secondary_generation
    }

    fn cancel_secondary_source(&mut self, reason: &str) {
        self.transition.cancel(reason);
        let preload = mem::replace(&mut self.preload, PlayerPreload::None);
        if !matches!(preload, PlayerPreload::None) {
            self.next_secondary_generation();
        }
        match preload {
            PlayerPreload::None => {}
            PlayerPreload::Loading { track_id, .. } => {
                debug!("Secondary source load for <{track_id}> cancelled: {reason}");
            }
            PlayerPreload::Ready { track_id, .. } => {
                debug!("Secondary playback source for <{track_id}> cancelled: {reason}");
            }
        }
    }

    fn start_secondary_decode(&mut self) -> Result<bool, DecoderError> {
        if self.config.passthrough {
            return Ok(false);
        }

        let should_start = matches!(
            &self.preload,
            PlayerPreload::Ready { source, .. } if !source.decoder.is_worker()
        );
        if !should_start {
            return Ok(false);
        }

        let generation = self.next_secondary_generation();
        let PlayerPreload::Ready { track_id, source } = &mut self.preload else {
            unreachable!("ready secondary changed while starting decoder worker");
        };

        source.decoder.start_secondary(
            source.stream_loader_controller.clone(),
            generation,
            track_id.to_string(),
        )
    }

    /// Non-blocking handoff used by the future transition scheduler. Terminal secondary events
    /// are isolated here and never enter the current-track decoder/error path.
    #[cfg_attr(not(test), allow(dead_code))]
    fn try_take_secondary_packet(&mut self) -> Option<(AudioPacketPosition, AudioPacket)> {
        let (track_id, message) = {
            let PlayerPreload::Ready { track_id, source } = &self.preload else {
                return None;
            };
            let message = match source.decoder.try_recv_secondary() {
                Ok(message) => message,
                Err(std::sync::mpsc::TryRecvError::Empty) => return None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    debug!("Secondary decode failed for <{track_id}>: worker disconnected");
                    self.cancel_secondary_source("secondary decode worker disconnected");
                    return None;
                }
            };
            (track_id.clone(), message)
        };

        if message.generation != self.secondary_generation {
            debug!(
                "Discarding stale secondary PCM generation {} for <{track_id}>; current generation is {}",
                message.generation, self.secondary_generation
            );
            self.cancel_secondary_source("stale secondary decode generation");
            return None;
        }

        match message.event {
            SecondaryDecodeEvent::Packet(position, packet) => Some((position, packet)),
            SecondaryDecodeEvent::Eof => {
                self.cancel_secondary_source("secondary reached EOF");
                None
            }
            SecondaryDecodeEvent::Failed(error) => {
                let error = PlayerLoadError::from_decoder_error(&self.session, error);
                if let PlayerState::Playing {
                    play_request_id, ..
                }
                | PlayerState::Paused {
                    play_request_id, ..
                } = self.state
                {
                    self.send_event(load_error_event(
                        track_id,
                        play_request_id,
                        error.kind,
                        true,
                    ));
                }
                self.cancel_secondary_source("secondary decode failed");
                None
            }
        }
    }

    fn cancel_secondary_source_unless(&mut self, retained_track_id: &SpotifyUri, reason: &str) {
        if self.preload.track_id() == Some(retained_track_id) {
            self.transition.cancel(reason);
        } else {
            self.cancel_secondary_source(reason);
        }
    }

    fn promote_preloaded_source(
        &mut self,
        requested_track_id: &SpotifyUri,
        play_request_id: u64,
        play: bool,
        position_ms: u32,
    ) -> Result<bool, Error> {
        let PlayerPreload::Ready { track_id, .. } = &self.preload else {
            return Ok(false);
        };
        if track_id != requested_track_id {
            return Ok(false);
        }

        let PlayerPreload::Ready {
            track_id,
            mut source,
        } = mem::replace(&mut self.preload, PlayerPreload::None)
        else {
            unreachable!("ready preload changed while being promoted");
        };

        if position_ms != source.stream_position_ms {
            // This may be blocking, exactly as in the existing preloaded-track path.
            source.stream_position_ms = source.decoder.seek(position_ms)?;
        }

        debug!("Secondary promoted for <{track_id}>");
        self.start_playback(track_id, play_request_id, *source, play);
        Ok(true)
    }

    fn arm_transition_if_selected(&mut self) {
        let (current_position, current_duration) = match &self.state {
            PlayerState::Playing { source, .. } | PlayerState::Paused { source, .. } => (
                Duration::from_millis(u64::from(source.stream_position_ms)),
                Duration::from_millis(u64::from(source.duration_ms)),
            ),
            _ => return,
        };

        let Some(spec) = self
            .transition_policy
            .plan(current_position, current_duration)
        else {
            return;
        };

        if self.config.passthrough {
            debug!("Transition policy selected PCM mixing while encoded passthrough is active");
            return;
        }

        match self.transition.arm(spec) {
            Ok(()) => {
                if let Err(e) = self.start_secondary_decode() {
                    warn!("Unable to start secondary decoder: {e}");
                    self.cancel_secondary_source("secondary decoder could not start");
                }
            }
            Err(e) => warn!("Unable to arm transition: {e}"),
        }
    }

    fn ensure_sink_running(&mut self) {
        if self.sink_status != SinkStatus::Running {
            trace!("== Starting sink ==");
            if let Some(callback) = &mut self.sink_event_callback {
                callback(SinkStatus::Running);
            }
            match self.sink.start() {
                Ok(()) => self.sink_status = SinkStatus::Running,
                Err(e) => {
                    error!("{e}");
                    self.handle_pause();
                }
            }
        }
    }

    fn ensure_sink_stopped(&mut self, temporarily: bool) {
        match self.sink_status {
            SinkStatus::Running => {
                trace!("== Stopping sink ==");
                match self.sink.stop() {
                    Ok(()) => {
                        self.sink_status = if temporarily {
                            SinkStatus::TemporarilyClosed
                        } else {
                            SinkStatus::Closed
                        };
                        if let Some(callback) = &mut self.sink_event_callback {
                            callback(self.sink_status);
                        }
                    }
                    Err(e) => {
                        error!("{e}");
                        exit(1);
                    }
                }
            }
            SinkStatus::TemporarilyClosed => {
                if !temporarily {
                    self.sink_status = SinkStatus::Closed;
                    if let Some(callback) = &mut self.sink_event_callback {
                        callback(SinkStatus::Closed);
                    }
                }
            }
            SinkStatus::Closed => (),
        }
    }

    fn handle_player_stop(&mut self) {
        self.cancel_secondary_source("player stopped");
        if let Some(recovery) = self.recovery.take() {
            debug!(
                "Stop cancelled recovery generation {} for <{}> at {} ms",
                recovery.generation, recovery.track_id, recovery.position_ms
            );
            self.next_recovery_generation();
            self.network_health = RecoveryHealth::Healthy;
            self.ensure_sink_stopped(false);
            self.state = PlayerState::Stopped;
            self.send_event(PlayerEvent::Stopped {
                track_id: recovery.track_id,
                play_request_id: recovery.play_request_id,
            });
            return;
        }

        self.next_recovery_generation();
        match self.state {
            PlayerState::Playing {
                ref track_id,
                play_request_id,
                ..
            }
            | PlayerState::Paused {
                ref track_id,
                play_request_id,
                ..
            }
            | PlayerState::EndOfTrack {
                ref track_id,
                play_request_id,
                ..
            }
            | PlayerState::Loading {
                ref track_id,
                play_request_id,
                ..
            } => {
                let track_id = track_id.clone();

                self.ensure_sink_stopped(false);
                self.send_event(PlayerEvent::Stopped {
                    track_id,
                    play_request_id,
                });
                self.state = PlayerState::Stopped;
            }
            PlayerState::Stopped => (),
            PlayerState::Invalid => {
                error!("PlayerInternal::handle_player_stop in invalid state");
                exit(1);
            }
        }
    }

    fn handle_play(&mut self) {
        if let Some(recovery) = self.recovery.as_mut() {
            debug!(
                "Updating play intent during recovery generation {} for <{}>",
                recovery.generation, recovery.track_id
            );
            recovery.set_play_intent(true);
            return;
        }

        match self.state {
            PlayerState::Paused {
                ref track_id,
                play_request_id,
                ref source,
                ..
            } => {
                let track_id = track_id.clone();
                let position_ms = source.stream_position_ms;

                self.state.paused_to_playing();
                self.send_event(PlayerEvent::Playing {
                    track_id,
                    play_request_id,
                    position_ms,
                });
                self.ensure_sink_running();
            }
            PlayerState::Loading {
                ref mut start_playback,
                ..
            } => {
                *start_playback = true;
            }
            _ => error!("Player::play called from invalid state: {:?}", self.state),
        }
    }

    fn handle_pause(&mut self) {
        if let Some(recovery) = self.recovery.as_mut() {
            debug!(
                "Updating pause intent during recovery generation {} for <{}>",
                recovery.generation, recovery.track_id
            );
            recovery.set_play_intent(false);
            self.ensure_sink_stopped(true);
            return;
        }

        match self.state {
            PlayerState::Paused { .. } => self.ensure_sink_stopped(false),
            PlayerState::Playing {
                ref track_id,
                play_request_id,
                ref source,
                ..
            } => {
                let track_id = track_id.clone();
                let position_ms = source.stream_position_ms;

                self.state.playing_to_paused();

                self.ensure_sink_stopped(false);
                self.send_event(PlayerEvent::Paused {
                    track_id,
                    play_request_id,
                    position_ms,
                });
            }
            PlayerState::Loading {
                ref mut start_playback,
                ..
            } => {
                *start_playback = false;
            }
            _ => error!("Player::pause called from invalid state: {:?}", self.state),
        }
    }

    fn handle_packet(
        &mut self,
        packet: Option<(AudioPacketPosition, AudioPacket)>,
        normalisation_factor: f64,
    ) {
        match packet {
            Some((_, mut packet)) => {
                if !packet.is_empty() {
                    if let AudioPacket::Samples(ref mut data) = packet {
                        // Get the volume for the packet. In the case of hardware volume control
                        // this will always be 1.0 (no change).
                        let volume = self.volume_getter.attenuation_factor();

                        // For the basic normalisation method, a normalisation factor of 1.0
                        // indicates that there is nothing to normalise (all samples should pass
                        // unaltered). For the dynamic method, there may still be peaks that we
                        // want to shave off.
                        //
                        // No matter the case we apply volume attenuation last if there is any.
                        match (self.config.normalisation, self.config.normalisation_method) {
                            (false, _) => {
                                if volume < 1.0 {
                                    for sample in data.iter_mut() {
                                        *sample *= volume;
                                    }
                                }
                            }
                            (true, NormalisationMethod::Dynamic) => {
                                // zero-cost shorthands
                                let threshold_db = self.config.normalisation_threshold_dbfs;
                                let knee_db = self.config.normalisation_knee_db;
                                let attack_cf = self.config.normalisation_attack_cf;
                                let release_cf = self.config.normalisation_release_cf;

                                for sample in data.iter_mut() {
                                    // Feedforward limiter in the log domain
                                    // After: Giannoulis, D., Massberg, M., & Reiss, J.D. (2012).
                                    // Digital Dynamic Range Compressor Design—A Tutorial and
                                    // Analysis. Journal of The Audio Engineering Society, 60,
                                    // 399-408.

                                    // This implementation assumes audio is stereo.

                                    // step 0: apply gain stage
                                    *sample *= normalisation_factor;

                                    // step 1-4: half-wave rectification and conversion into dB, and
                                    // gain computer with soft knee and subtractor
                                    let limiter_db = {
                                        // Add slight DC offset. Some samples are silence, which is
                                        // -inf dB and gets the limiter stuck. Adding a small
                                        // positive offset prevents this.
                                        *sample += f64::MIN_POSITIVE;

                                        let bias_db = ratio_to_db(sample.abs()) - threshold_db;
                                        let knee_boundary_db = bias_db * 2.0;
                                        if knee_boundary_db < -knee_db {
                                            0.0
                                        } else if knee_boundary_db.abs() <= knee_db {
                                            let term = knee_boundary_db + knee_db;
                                            term * term * self.normalisation_knee_factor
                                        } else {
                                            bias_db
                                        }
                                    };

                                    // track left/right channel
                                    let channel = self.normalisation_channel;
                                    self.normalisation_channel ^= 1;

                                    // step 5: smooth, decoupled peak detector for each channel
                                    // Use direct references to reduce repeated array indexing
                                    let integrator = &mut self.normalisation_integrators[channel];
                                    let peak = &mut self.normalisation_peaks[channel];

                                    *integrator = f64::max(
                                        limiter_db,
                                        release_cf * *integrator + (1.0 - release_cf) * limiter_db,
                                    );
                                    *peak = attack_cf * *peak + (1.0 - attack_cf) * *integrator;

                                    // steps 6-8: conversion into level and multiplication into gain
                                    // stage. Find maximum peak across both channels to couple the
                                    // gain and maintain stereo imaging.
                                    let max_peak = f64::max(
                                        self.normalisation_peaks[0],
                                        self.normalisation_peaks[1],
                                    );
                                    *sample *= db_to_ratio(-max_peak) * volume;
                                }
                            }
                            (true, NormalisationMethod::Basic) => {
                                if normalisation_factor < 1.0 || volume < 1.0 {
                                    for sample in data.iter_mut() {
                                        *sample *= normalisation_factor * volume;
                                    }
                                }
                            }
                        }
                    }

                    let packet = match self.transition.render(packet, None) {
                        Ok(packet) => packet,
                        Err(e) => {
                            error!("Transition engine rejected current-source packet: {e}");
                            self.transition.cancel("rendering failed");
                            self.handle_pause();
                            return;
                        }
                    };

                    if let Err(e) = self.sink.write(packet, &mut self.converter) {
                        error!("{e}");
                        self.handle_pause();
                    }
                }
            }

            None => {
                self.state.playing_to_end_of_track();
                if let PlayerState::EndOfTrack {
                    ref track_id,
                    play_request_id,
                    ..
                } = self.state
                {
                    self.send_event(natural_end_of_track_event(
                        track_id.clone(),
                        play_request_id,
                    ))
                } else {
                    error!("PlayerInternal handle_packet: Invalid PlayerState");
                    exit(1);
                }
            }
        }
    }

    fn start_playback(
        &mut self,
        track_id: SpotifyUri,
        play_request_id: u64,
        mut source: PlaybackSource,
        start_playback: bool,
    ) {
        self.network_health = RecoveryHealth::Healthy;
        let audio_item = Box::new(source.audio_item.clone());

        self.send_event(PlayerEvent::TrackChanged { audio_item });

        let position_ms = source.stream_position_ms;

        let mut config = self.config.clone();
        if config.normalisation_type == NormalisationType::Auto {
            if self.auto_normalise_as_album {
                config.normalisation_type = NormalisationType::Album;
            } else {
                config.normalisation_type = NormalisationType::Track;
            }
        };
        source.normalisation_factor =
            NormalisationData::get_factor(&config, source.normalisation_data);
        source.suggested_to_preload_next_track = false;

        if start_playback {
            source.reported_nominal_start_time =
                Instant::now().checked_sub(Duration::from_millis(u64::from(position_ms)));
            self.ensure_sink_running();
            self.send_event(PlayerEvent::Playing {
                track_id: track_id.clone(),
                play_request_id,
                position_ms,
            });

            self.state = PlayerState::Playing {
                track_id,
                play_request_id,
                source,
            };
        } else {
            source.reported_nominal_start_time = None;
            self.ensure_sink_stopped(false);

            self.state = PlayerState::Paused {
                track_id: track_id.clone(),
                play_request_id,
                source,
            };

            self.send_event(PlayerEvent::Paused {
                track_id,
                play_request_id,
                position_ms,
            });
        }
    }

    fn handle_command_load(
        &mut self,
        track_id: SpotifyUri,
        play_request_id_option: Option<u64>,
        play: bool,
        position_ms: u32,
    ) -> PlayerResult {
        self.cancel_secondary_source_unless(&track_id, "new track load");
        self.cancel_recovery("a newer Load command arrived");
        self.next_recovery_generation();
        let play_request_id =
            play_request_id_option.unwrap_or(self.play_request_id_generator.get());

        self.send_event(PlayerEvent::PlayRequestIdChanged { play_request_id });

        if !self.config.gapless {
            self.ensure_sink_stopped(play);
        }

        if matches!(self.state, PlayerState::Invalid) {
            return Err(Error::internal(format!(
                "Player::handle_command_load called from invalid state: {:?}",
                self.state
            )));
        }

        // Now we check at different positions whether we already have a pre-loaded version
        // of this track somewhere. If so, use it and return.

        // Check if there's a matching loaded track in the EndOfTrack player state.
        // This is the case if we're repeating the same track again.
        if let PlayerState::EndOfTrack {
            track_id: previous_track_id,
            ..
        } = &self.state
        {
            if *previous_track_id == track_id {
                let mut source = match mem::replace(&mut self.state, PlayerState::Invalid) {
                    PlayerState::EndOfTrack { source, .. } => source,
                    _ => {
                        return Err(Error::internal(format!(
                            "PlayerInternal::handle_command_load repeating the same track: invalid state: {:?}",
                            self.state
                        )));
                    }
                };

                if position_ms != source.stream_position_ms {
                    // This may be blocking.
                    source.stream_position_ms = source.decoder.seek(position_ms)?;
                }
                self.cancel_secondary_source("same-track replay");
                self.start_playback(track_id, play_request_id, source, play);
                if let PlayerState::Invalid = self.state {
                    return Err(Error::internal(format!(
                        "PlayerInternal::handle_command_load repeating the same track: start_playback() did not transition to valid player state: {:?}",
                        self.state
                    )));
                }
                return Ok(());
            }
        }

        // Check if we are already playing the track. If so, just do a seek and update our info.
        if let PlayerState::Playing {
            track_id: ref current_track_id,
            ref mut source,
            ..
        }
        | PlayerState::Paused {
            track_id: ref current_track_id,
            ref mut source,
            ..
        } = self.state
        {
            if *current_track_id == track_id {
                // we can use the current decoder. Ensure it's at the correct position.
                if position_ms != source.stream_position_ms {
                    // This may be blocking.
                    source.stream_position_ms = source.decoder.seek(position_ms)?;
                }

                // Move the current source through the usual activation path.
                let old_state = mem::replace(&mut self.state, PlayerState::Invalid);

                if let PlayerState::Playing { source, .. } | PlayerState::Paused { source, .. } =
                    old_state
                {
                    self.cancel_secondary_source("same-track load");
                    self.start_playback(track_id, play_request_id, source, play);

                    if let PlayerState::Invalid = self.state {
                        return Err(Error::internal(format!(
                            "PlayerInternal::handle_command_load already playing this track: start_playback() did not transition to valid player state: {:?}",
                            self.state
                        )));
                    }

                    return Ok(());
                } else {
                    return Err(Error::internal(format!(
                        "PlayerInternal::handle_command_load already playing this track: invalid state: {:?}",
                        self.state
                    )));
                }
            }
        }

        // Promote a matching ready source by ownership transfer; no decoder is reconstructed and
        // no track is loaded a second time.
        if self.promote_preloaded_source(&track_id, play_request_id, play, position_ms)? {
            return Ok(());
        }

        self.send_event(PlayerEvent::Loading {
            track_id: track_id.clone(),
            play_request_id,
            position_ms,
        });

        // Try to extract a pending loader from the preloading mechanism
        let loader = if let PlayerPreload::Loading {
            track_id: loaded_track_id,
            ..
        } = &self.preload
        {
            if (track_id == *loaded_track_id) && (position_ms == 0) {
                let mut preload = PlayerPreload::None;
                std::mem::swap(&mut preload, &mut self.preload);
                if let PlayerPreload::Loading { loader, .. } = preload {
                    Some(loader)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        self.cancel_secondary_source("preload moved or superseded by current load");

        // If we don't have a loader yet, create one from scratch.
        let loader = loader
            .unwrap_or_else(|| Box::pin(self.load_track(track_id.clone(), position_ms, true)));

        // Set ourselves to a loading state.
        self.state = PlayerState::Loading {
            track_id,
            play_request_id,
            start_playback: play,
            position_ms,
            loader,
        };

        Ok(())
    }

    fn handle_command_preload(&mut self, track_id: SpotifyUri) {
        if self.recovery.is_some() || self.network_health != RecoveryHealth::Healthy {
            debug!(
                "Deferring preload of <{track_id}> while current-track networking is degraded or recovering"
            );
            return;
        }

        debug!("Preloading track");
        let mut preload_track = true;
        // check whether the track is already loaded somewhere or being loaded.
        if let Some(currently_loading) = self.preload.track_id() {
            if *currently_loading == track_id {
                // we're already preloading the requested track.
                preload_track = false;
            } else {
                // we're preloading something else - cancel it.
                self.cancel_secondary_source("next-track preload was replaced");
            }
        }

        if let PlayerState::Playing {
            track_id: current_track_id,
            ..
        }
        | PlayerState::Paused {
            track_id: current_track_id,
            ..
        }
        | PlayerState::EndOfTrack {
            track_id: current_track_id,
            ..
        } = &self.state
        {
            if *current_track_id == track_id {
                // we already have the requested track loaded.
                preload_track = false;
            }
        }

        // schedule the preload of the current track if desired.
        if preload_track {
            let loader = self.load_track(track_id.clone(), 0, true);
            self.preload = PlayerPreload::Loading {
                track_id,
                loader: Box::pin(loader),
            }
        }
    }

    fn handle_command_seek(&mut self, position_ms: u32) -> PlayerResult {
        self.cancel_secondary_source("player seeked");
        if let Some(mut recovery) = self.recovery.take() {
            let old_generation = recovery.generation;
            let generation = self.next_recovery_generation();
            recovery.restart(generation, position_ms, self.session.is_invalid());
            debug!(
                "Seek invalidated recovery generation {old_generation}; generation {generation} will reopen <{}> at {position_ms} ms",
                recovery.track_id
            );
            self.send_event(PlayerEvent::Seeked {
                play_request_id: recovery.play_request_id,
                track_id: recovery.track_id.clone(),
                position_ms,
            });
            self.recovery = Some(recovery);
            return Ok(());
        }

        self.next_recovery_generation();
        // When we are still loading, the user may immediately ask to
        // seek to another position yet the decoder won't be ready for
        // that. In this case just restart the loading process but
        // with the requested position.
        if let PlayerState::Loading {
            ref track_id,
            play_request_id,
            start_playback,
            ..
        } = self.state
        {
            return self.handle_command_load(
                track_id.clone(),
                Some(play_request_id),
                start_playback,
                position_ms,
            );
        }

        let seek_request = match &self.state {
            PlayerState::Playing {
                track_id,
                play_request_id,
                ..
            } => Some((track_id.clone(), *play_request_id, true)),
            PlayerState::Paused {
                track_id,
                play_request_id,
                ..
            } => Some((track_id.clone(), *play_request_id, false)),
            _ => None,
        };

        let mut seek_error = None;

        if let Some(source) = self.state.source_mut() {
            match source.decoder.seek(position_ms) {
                Ok(new_position_ms) => {
                    source.stream_position_ms = new_position_ms;

                    if let PlayerState::Playing {
                        ref track_id,
                        play_request_id,
                        ..
                    }
                    | PlayerState::Paused {
                        ref track_id,
                        play_request_id,
                        ..
                    } = self.state
                    {
                        self.send_event(PlayerEvent::Seeked {
                            play_request_id,
                            track_id: track_id.clone(),
                            position_ms: new_position_ms,
                        });
                    }
                }
                Err(error) => {
                    seek_error = Some(PlayerLoadError::from_decoder_error(&self.session, error))
                }
            }
        } else {
            error!("Player::seek called from invalid state: {:?}", self.state);
        }

        if let (Some((track_id, play_request_id, start_playback)), Some(error)) =
            (seek_request.clone(), seek_error)
        {
            self.handle_seek_recovery(
                track_id,
                play_request_id,
                start_playback,
                position_ms,
                error,
            );
            return Ok(());
        }

        // ensure we have a bit of a buffer of downloaded data
        if let Err(source) = self.preload_data_before_playback() {
            if let Some((track_id, play_request_id, start_playback)) = seek_request {
                let error = PlayerLoadError::from_error(&self.session, source);
                self.handle_seek_recovery(
                    track_id,
                    play_request_id,
                    start_playback,
                    position_ms,
                    error,
                );
                return Ok(());
            }
        }

        if let PlayerState::Playing { ref mut source, .. } = self.state {
            source.reported_nominal_start_time =
                Instant::now().checked_sub(Duration::from_millis(u64::from(position_ms)));
        }

        Ok(())
    }

    fn handle_seek_recovery(
        &mut self,
        track_id: SpotifyUri,
        play_request_id: u64,
        start_playback: bool,
        position_ms: u32,
        error: PlayerLoadError,
    ) {
        debug!(
            "Seek for <{track_id}> at {position_ms} ms failed as {:?}",
            error.kind
        );
        match error.kind {
            PlayerLoadErrorKind::TransientNetwork
            | PlayerLoadErrorKind::TransientService
            | PlayerLoadErrorKind::SessionInvalid => self.begin_recovery(RecoveryRequest {
                track_id,
                play_request_id,
                position_ms,
                start_playback,
                kind: error.kind,
                failed_attempts: 0,
                buffer_starved: start_playback,
            }),
            PlayerLoadErrorKind::PermanentTrack => {
                self.state = PlayerState::Stopped;
                self.ensure_sink_stopped(true);
                self.send_event(PlayerEvent::Unavailable {
                    track_id,
                    play_request_id,
                });
            }
            PlayerLoadErrorKind::Cancelled => {
                self.state = PlayerState::Stopped;
                self.ensure_sink_stopped(true);
            }
        }
    }

    fn handle_set_session(&mut self, session: Session) {
        let old_session_invalid = self.session.is_invalid();
        self.session = session;

        if let Some(mut recovery) = self.recovery.take() {
            let old_generation = recovery.generation;
            let generation = self.next_recovery_generation();
            let position_ms = recovery.position_ms;
            recovery.restart(generation, position_ms, self.session.is_invalid());
            debug!(
                "Replacement session invalidated recovery generation {old_generation}; restarting generation {generation} for <{}> at {} ms",
                recovery.track_id, recovery.position_ms
            );
            self.recovery = Some(recovery);
            return;
        }

        if old_session_invalid {
            if let PlayerState::Loading {
                track_id,
                play_request_id,
                start_playback,
                position_ms,
                ..
            } = &self.state
            {
                let track_id = track_id.clone();
                let play_request_id = *play_request_id;
                let start_playback = *start_playback;
                let position_ms = *position_ms;
                self.next_recovery_generation();
                debug!(
                    "Restarting in-flight load for <{track_id}> at {position_ms} ms on replacement session"
                );
                let loader = Box::pin(self.load_track(track_id.clone(), position_ms, true));
                self.state = PlayerState::Loading {
                    track_id,
                    play_request_id,
                    start_playback,
                    position_ms,
                    loader,
                };
            }
        }
    }

    fn handle_command(&mut self, cmd: PlayerCommand) -> PlayerResult {
        debug!("command={cmd:?}");
        match cmd {
            PlayerCommand::Load {
                track_id,
                play,
                position_ms,
            } => self.handle_command_load(track_id, None, play, position_ms)?,

            PlayerCommand::Preload { track_id } => self.handle_command_preload(track_id),

            PlayerCommand::Seek(position_ms) => self.handle_command_seek(position_ms)?,

            PlayerCommand::Play => self.handle_play(),

            PlayerCommand::Pause => self.handle_pause(),

            PlayerCommand::Stop => self.handle_player_stop(),

            PlayerCommand::SetSession(session) => self.handle_set_session(session),

            PlayerCommand::AddEventSender(sender) => self.event_senders.push(sender),

            PlayerCommand::SetSinkEventCallback(callback) => self.sink_event_callback = callback,

            PlayerCommand::EmitVolumeChangedEvent(volume) => {
                self.send_event(PlayerEvent::VolumeChanged { volume })
            }

            PlayerCommand::EmitRepeatChangedEvent { context, track } => {
                self.send_event(PlayerEvent::RepeatChanged { context, track })
            }

            PlayerCommand::EmitShuffleChangedEvent(shuffle) => {
                self.send_event(PlayerEvent::ShuffleChanged { shuffle })
            }

            PlayerCommand::EmitAutoPlayChangedEvent(auto_play) => {
                self.send_event(PlayerEvent::AutoPlayChanged { auto_play })
            }

            PlayerCommand::EmitSessionClientChangedEvent {
                client_id,
                client_name,
                client_brand_name,
                client_model_name,
            } => self.send_event(PlayerEvent::SessionClientChanged {
                client_id,
                client_name,
                client_brand_name,
                client_model_name,
            }),

            PlayerCommand::EmitSessionConnectedEvent {
                connection_id,
                user_name,
            } => self.send_event(PlayerEvent::SessionConnected {
                connection_id,
                user_name,
            }),

            PlayerCommand::EmitSessionDisconnectedEvent {
                connection_id,
                user_name,
            } => self.send_event(PlayerEvent::SessionDisconnected {
                connection_id,
                user_name,
            }),

            PlayerCommand::SetAutoNormaliseAsAlbum(setting) => {
                self.auto_normalise_as_album = setting
            }

            PlayerCommand::EmitFilterExplicitContentChangedEvent(filter) => {
                self.send_event(PlayerEvent::FilterExplicitContentChanged { filter });

                if filter {
                    if let PlayerState::Playing {
                        ref track_id,
                        play_request_id,
                        ref source,
                    }
                    | PlayerState::Paused {
                        ref track_id,
                        play_request_id,
                        ref source,
                    } = self.state
                    {
                        let track_id = track_id.clone();

                        if source.is_explicit {
                            warn!(
                                "Currently loaded track is explicit, which client setting forbids -- skipping to next track."
                            );
                            self.send_event(PlayerEvent::EndOfTrack {
                                track_id,
                                play_request_id,
                            })
                        }
                    }
                }
            }
        };

        Ok(())
    }

    fn send_event(&mut self, event: PlayerEvent) {
        self.event_senders
            .retain(|sender| sender.send(event.clone()).is_ok());
    }

    fn load_track(
        &mut self,
        spotify_uri: SpotifyUri,
        position_ms: u32,
        prebuffer: bool,
    ) -> impl FusedFuture<Output = Result<PlaybackSource, PlayerLoadError>> + Send + 'static {
        // This method creates a future that returns the loaded stream and associated info.
        // Ideally all work should be done using asynchronous code. However, seek() on the
        // audio stream is implemented in a blocking fashion. Thus, we can't turn it into future
        // easily. Instead we spawn a thread to do the work and return a one-shot channel as the
        // future to work with.

        let loader = PlayerTrackLoader {
            session: self.session.clone(),
            config: self.config.clone(),
            local_file_lookup: self.local_file_lookup.clone(),
        };

        let (result_tx, result_rx) = oneshot::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let active_controller = Arc::new(Mutex::new(None));
        let worker_cancelled = cancelled.clone();
        let worker_controller = active_controller.clone();

        let load_handles_clone = self.load_handles.clone();
        let handle = tokio::runtime::Handle::current();

        let load_handle = thread::spawn(move || {
            let session = loader.session.clone();
            let mut data = handle.block_on(loader.load_track(spotify_uri, position_ms));

            if worker_cancelled.load(Ordering::Acquire) {
                if let Ok(loaded_track) = data.as_ref() {
                    loaded_track.stream_loader_controller.close();
                }
                data = Err(PlayerLoadError::new(
                    PlayerLoadErrorKind::Cancelled,
                    Error::cancelled("track load was superseded"),
                ));
            } else if prebuffer {
                let mut cancelled_before_prebuffer = false;
                if let Ok(loaded_track) = data.as_mut() {
                    *worker_controller.lock().expect(LOAD_HANDLES_POISON_MSG) =
                        Some(loaded_track.stream_loader_controller.clone());

                    // Close the narrow race where cancellation happens after the first check but
                    // before the controller becomes visible to `CancellableTrackLoader::drop`.
                    if worker_cancelled.load(Ordering::Acquire) {
                        loaded_track.stream_loader_controller.close();
                        cancelled_before_prebuffer = true;
                    } else {
                        let params = AudioFetchParams::get();
                        let time_target = (params.read_ahead_before_playback.as_secs_f32()
                            * loaded_track.bytes_per_second as f32)
                            as usize;
                        let target = time_target.max(params.minimum_read_ahead_bytes);
                        debug!(
                            "Prebuffering up to {target} compressed bytes before playback at {} ms",
                            loaded_track.stream_position_ms
                        );
                        if let Err(error) = loaded_track
                            .stream_loader_controller
                            .fetch_next_and_wait(target, target)
                        {
                            data = Err(PlayerLoadError::from_error(&session, error));
                        }
                    }
                }
                if cancelled_before_prebuffer {
                    data = Err(PlayerLoadError::new(
                        PlayerLoadErrorKind::Cancelled,
                        Error::cancelled("track load was superseded"),
                    ));
                }
            }
            worker_controller
                .lock()
                .expect(LOAD_HANDLES_POISON_MSG)
                .take();
            let _ = result_tx.send(data);

            let mut load_handles = load_handles_clone.lock().expect(LOAD_HANDLES_POISON_MSG);
            load_handles.remove(&thread::current().id());
        });

        let mut load_handles = self.load_handles.lock().expect(LOAD_HANDLES_POISON_MSG);
        load_handles.insert(load_handle.thread().id(), load_handle);

        CancellableTrackLoader {
            receiver: result_rx,
            cancelled,
            active_controller,
            terminated: false,
        }
    }

    fn preload_data_before_playback(&mut self) -> PlayerResult {
        if let PlayerState::Playing { ref mut source, .. }
        | PlayerState::Paused { ref mut source, .. } = self.state
        {
            let params = AudioFetchParams::get();
            let read_ahead_during_playback = params.read_ahead_during_playback;
            // Request our read ahead range
            let request_data_length = (read_ahead_during_playback.as_secs_f32()
                * source.bytes_per_second as f32) as usize;
            let request_data_length = request_data_length.max(params.minimum_read_ahead_bytes);

            // Request the part we want to wait for blocking. This effectively means we wait for the previous request to partially complete.
            let wait_for_data_length = request_data_length;

            source
                .stream_loader_controller
                .fetch_next_and_wait(request_data_length, wait_for_data_length)
        } else {
            Ok(())
        }
    }
}

impl Drop for PlayerInternal {
    fn drop(&mut self) {
        debug!("drop PlayerInternal[{}]", self.player_id);

        let handles: Vec<thread::JoinHandle<()>> = {
            // waiting for the thread while holding the mutex would result in a deadlock
            let mut load_handles = self.load_handles.lock().expect(LOAD_HANDLES_POISON_MSG);

            load_handles
                .drain()
                .map(|(_thread_id, handle)| handle)
                .collect()
        };

        for handle in handles {
            let _ = handle.join();
        }
    }
}

impl fmt::Debug for PlayerCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlayerCommand::Load {
                track_id,
                play,
                position_ms,
                ..
            } => f
                .debug_tuple("Load")
                .field(&track_id)
                .field(&play)
                .field(&position_ms)
                .finish(),
            PlayerCommand::Preload { track_id } => {
                f.debug_tuple("Preload").field(&track_id).finish()
            }
            PlayerCommand::Play => f.debug_tuple("Play").finish(),
            PlayerCommand::Pause => f.debug_tuple("Pause").finish(),
            PlayerCommand::Stop => f.debug_tuple("Stop").finish(),
            PlayerCommand::Seek(position) => f.debug_tuple("Seek").field(&position).finish(),
            PlayerCommand::SetSession(_) => f.debug_tuple("SetSession").finish(),
            PlayerCommand::AddEventSender(_) => f.debug_tuple("AddEventSender").finish(),
            PlayerCommand::SetSinkEventCallback(_) => {
                f.debug_tuple("SetSinkEventCallback").finish()
            }
            PlayerCommand::EmitVolumeChangedEvent(volume) => f
                .debug_tuple("EmitVolumeChangedEvent")
                .field(&volume)
                .finish(),
            PlayerCommand::SetAutoNormaliseAsAlbum(setting) => f
                .debug_tuple("SetAutoNormaliseAsAlbum")
                .field(&setting)
                .finish(),
            PlayerCommand::EmitFilterExplicitContentChangedEvent(filter) => f
                .debug_tuple("EmitFilterExplicitContentChangedEvent")
                .field(&filter)
                .finish(),
            PlayerCommand::EmitSessionConnectedEvent {
                connection_id,
                user_name,
            } => f
                .debug_tuple("EmitSessionConnectedEvent")
                .field(&connection_id)
                .field(&user_name)
                .finish(),
            PlayerCommand::EmitSessionDisconnectedEvent {
                connection_id,
                user_name,
            } => f
                .debug_tuple("EmitSessionDisconnectedEvent")
                .field(&connection_id)
                .field(&user_name)
                .finish(),
            PlayerCommand::EmitSessionClientChangedEvent {
                client_id,
                client_name,
                client_brand_name,
                client_model_name,
            } => f
                .debug_tuple("EmitSessionClientChangedEvent")
                .field(&client_id)
                .field(&client_name)
                .field(&client_brand_name)
                .field(&client_model_name)
                .finish(),
            PlayerCommand::EmitShuffleChangedEvent(shuffle) => f
                .debug_tuple("EmitShuffleChangedEvent")
                .field(&shuffle)
                .finish(),
            PlayerCommand::EmitRepeatChangedEvent { context, track } => f
                .debug_tuple("EmitRepeatChangedEvent")
                .field(&context)
                .field(&track)
                .finish(),
            PlayerCommand::EmitAutoPlayChangedEvent(auto_play) => f
                .debug_tuple("EmitAutoPlayChangedEvent")
                .field(&auto_play)
                .finish(),
        }
    }
}

impl fmt::Debug for PlayerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use PlayerState::*;
        match self {
            Stopped => f.debug_struct("Stopped").finish(),
            Loading {
                track_id,
                play_request_id,
                ..
            } => f
                .debug_struct("Loading")
                .field("track_id", &track_id)
                .field("play_request_id", &play_request_id)
                .finish(),
            Paused {
                track_id,
                play_request_id,
                ..
            } => f
                .debug_struct("Paused")
                .field("track_id", &track_id)
                .field("play_request_id", &play_request_id)
                .finish(),
            Playing {
                track_id,
                play_request_id,
                ..
            } => f
                .debug_struct("Playing")
                .field("track_id", &track_id)
                .field("play_request_id", &play_request_id)
                .finish(),
            EndOfTrack {
                track_id,
                play_request_id,
                ..
            } => f
                .debug_struct("EndOfTrack")
                .field("track_id", &track_id)
                .field("play_request_id", &play_request_id)
                .finish(),
            Invalid => f.debug_struct("Invalid").finish(),
        }
    }
}

struct Subfile<T: Read + Seek> {
    stream: T,
    offset: u64,
    length: u64,
}

impl<T: Read + Seek> Subfile<T> {
    pub fn new(mut stream: T, offset: u64, length: u64) -> Result<Subfile<T>, io::Error> {
        let target = SeekFrom::Start(offset);
        stream.seek(target)?;

        Ok(Subfile {
            stream,
            offset,
            length,
        })
    }
}

impl<T: Read + Seek> Read for Subfile<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buf)
    }
}

impl<T: Read + Seek> Seek for Subfile<T> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let pos = match pos {
            SeekFrom::Start(offset) => SeekFrom::Start(offset + self.offset),
            SeekFrom::End(offset) => {
                if (self.length as i64 - offset) < self.offset as i64 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "newpos would be < self.offset",
                    ));
                }
                pos
            }
            _ => pos,
        };

        let newpos = self.stream.seek(pos)?;
        Ok(newpos - self.offset)
    }
}

impl<R> MediaSource for Subfile<R>
where
    R: Read + Seek + Send + Sync,
{
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.length)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        audio_backend::SinkResult,
        core::config::SessionConfig,
        local_file::LocalFileLookup,
        mixer::NoOpVolume,
        secondary::{SECONDARY_PCM_CHANNEL_CAPACITY, SECONDARY_PCM_CHUNK_FRAMES},
        transition::{TransitionCurve, TransitionSpec, TransitionState},
    };
    use std::sync::mpsc as std_mpsc;

    fn track_uri() -> SpotifyUri {
        SpotifyUri::from_uri("spotify:track:2TpxZ7JUBn3uw46aR7qd6V")
            .expect("test URI should be valid")
    }

    fn next_track_uri() -> SpotifyUri {
        SpotifyUri::from_uri("spotify:track:4uLU6hMCjMI75M1A2tKUQC")
            .expect("test URI should be valid")
    }

    fn recovery() -> PlayerRecovery {
        PlayerRecovery {
            track_id: track_uri(),
            play_request_id: 7,
            position_ms: 42_123,
            start_playback: true,
            generation: 11,
            attempt: 1,
            mode: RecoveryMode::Fast,
            phase: RecoveryPhase::WaitingForSession,
        }
    }

    #[derive(Clone, Copy)]
    enum ScriptedLoadOutcome {
        TransientFailure,
        Success,
    }

    fn run_scripted_recovery(
        recovery: PlayerRecovery,
        outcomes: &[ScriptedLoadOutcome],
    ) -> (Vec<(SpotifyUri, u32, bool)>, usize) {
        let generation = recovery.generation;
        let mut pending = Some(recovery);
        let mut attempts = Vec::new();
        let mut resumes = 0;

        for outcome in outcomes {
            let Some(current) = pending.as_mut() else {
                continue;
            };
            if !current.is_current(generation) {
                continue;
            }
            attempts.push((
                current.track_id.clone(),
                current.position_ms,
                current.start_playback,
            ));
            match outcome {
                ScriptedLoadOutcome::TransientFailure => current.attempt += 1,
                ScriptedLoadOutcome::Success => {
                    pending.take();
                    resumes += 1;
                }
            }
        }

        (attempts, resumes)
    }

    fn session(runtime: &tokio::runtime::Runtime) -> Session {
        let _guard = runtime.enter();
        Session::new(SessionConfig::default(), None)
    }

    struct CountingSink {
        starts: Arc<AtomicUsize>,
        stops: Arc<AtomicUsize>,
    }

    impl Sink for CountingSink {
        fn start(&mut self) -> SinkResult<()> {
            self.starts.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        fn stop(&mut self) -> SinkResult<()> {
            self.stops.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }

        fn write(&mut self, _: AudioPacket, _: &mut Converter) -> SinkResult<()> {
            Ok(())
        }
    }

    struct RecordingSink {
        packets: Arc<Mutex<Vec<Vec<f64>>>>,
    }

    impl Sink for RecordingSink {
        fn write(&mut self, packet: AudioPacket, _: &mut Converter) -> SinkResult<()> {
            self.packets
                .lock()
                .expect("recorded packets mutex should not be poisoned")
                .push(
                    packet
                        .samples()
                        .expect("recording sink expects PCM")
                        .to_vec(),
                );
            Ok(())
        }
    }

    struct ScriptedDecoder;

    impl AudioDecoder for ScriptedDecoder {
        fn seek(&mut self, position_ms: u32) -> Result<u32, DecoderError> {
            Ok(position_ms)
        }

        fn next_packet(
            &mut self,
        ) -> Result<Option<(AudioPacketPosition, AudioPacket)>, DecoderError> {
            Ok(None)
        }
    }

    struct PanicDecoder;

    impl AudioDecoder for PanicDecoder {
        fn seek(&mut self, _: u32) -> Result<u32, DecoderError> {
            panic!("secondary decoder must remain dormant")
        }

        fn next_packet(
            &mut self,
        ) -> Result<Option<(AudioPacketPosition, AudioPacket)>, DecoderError> {
            panic!("secondary decoder must remain dormant")
        }
    }

    struct CountingPcmDecoder {
        calls: Arc<AtomicUsize>,
        packets_remaining: Option<usize>,
    }

    impl AudioDecoder for CountingPcmDecoder {
        fn seek(&mut self, position_ms: u32) -> Result<u32, DecoderError> {
            Ok(position_ms)
        }

        fn next_packet(
            &mut self,
        ) -> Result<Option<(AudioPacketPosition, AudioPacket)>, DecoderError> {
            let call = self.calls.fetch_add(1, Ordering::AcqRel);
            if matches!(self.packets_remaining, Some(0)) {
                return Ok(None);
            }
            if let Some(remaining) = self.packets_remaining.as_mut() {
                *remaining -= 1;
            }

            Ok(Some((
                AudioPacketPosition {
                    position_ms: (call as u32).saturating_mul(23),
                    skipped: false,
                },
                AudioPacket::Samples(vec![
                    0.25;
                    SECONDARY_PCM_CHUNK_FRAMES * NUM_CHANNELS as usize
                ]),
            )))
        }
    }

    struct BackpressureDecoder {
        calls: std_mpsc::Sender<usize>,
        call: usize,
    }

    impl AudioDecoder for BackpressureDecoder {
        fn seek(&mut self, position_ms: u32) -> Result<u32, DecoderError> {
            Ok(position_ms)
        }

        fn next_packet(
            &mut self,
        ) -> Result<Option<(AudioPacketPosition, AudioPacket)>, DecoderError> {
            self.call += 1;
            let _ = self.calls.send(self.call);
            Ok(Some((
                AudioPacketPosition {
                    position_ms: (self.call as u32).saturating_mul(23),
                    skipped: false,
                },
                AudioPacket::Samples(vec![
                    0.5;
                    SECONDARY_PCM_CHUNK_FRAMES * NUM_CHANNELS as usize
                ]),
            )))
        }
    }

    struct CancellationBlockedDecoder {
        cancelled: Arc<AtomicBool>,
        entered: Option<std_mpsc::Sender<()>>,
    }

    impl AudioDecoder for CancellationBlockedDecoder {
        fn seek(&mut self, position_ms: u32) -> Result<u32, DecoderError> {
            Ok(position_ms)
        }

        fn next_packet(
            &mut self,
        ) -> Result<Option<(AudioPacketPosition, AudioPacket)>, DecoderError> {
            if let Some(entered) = self.entered.take() {
                let _ = entered.send(());
            }
            while !self.cancelled.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(2));
            }
            Err(DecoderError::Io(io::Error::new(
                io::ErrorKind::Interrupted,
                "scripted blocked decode cancelled",
            )))
        }
    }

    struct ErrorDecoder {
        error: Option<DecoderError>,
    }

    impl AudioDecoder for ErrorDecoder {
        fn seek(&mut self, position_ms: u32) -> Result<u32, DecoderError> {
            Ok(position_ms)
        }

        fn next_packet(
            &mut self,
        ) -> Result<Option<(AudioPacketPosition, AudioPacket)>, DecoderError> {
            Err(self.error.take().expect("scripted error should run once"))
        }
    }

    fn scripted_source(track_id: SpotifyUri, position_ms: u32, decoder: Decoder) -> PlaybackSource {
        PlaybackSource {
            decoder: SourceDecoder::direct(decoder),
            normalisation_data: NormalisationData::default(),
            normalisation_factor: 1.0,
            stream_loader_controller: StreamLoaderController::from_local_file(512 * 1024),
            audio_item: AudioItem {
                track_id: track_id.clone(),
                uri: track_id.to_uri().expect("test URI should serialize"),
                files: Default::default(),
                name: "scripted recovery".into(),
                covers: vec![],
                language: vec![],
                duration_ms: 180_000,
                is_explicit: false,
                availability: Ok(()),
                alternatives: None,
                unique_fields: UniqueFields::Track {
                    artists: Default::default(),
                    album: String::new(),
                    album_artists: vec![],
                    popularity: 0,
                    number: 1,
                    disc_number: 1,
                },
            },
            bytes_per_second: 20 * 1024,
            duration_ms: 180_000,
            stream_position_ms: position_ms,
            reported_nominal_start_time: None,
            suggested_to_preload_next_track: false,
            is_explicit: false,
        }
    }

    fn scripted_loaded_track(position_ms: u32) -> PlaybackSource {
        scripted_source(track_uri(), position_ms, Box::new(ScriptedDecoder))
    }

    fn set_playing_source(
        player: &mut PlayerInternal,
        track_id: SpotifyUri,
        source: PlaybackSource,
    ) {
        player.state = PlayerState::Playing {
            track_id,
            play_request_id: 7,
            source,
        };
    }

    fn set_ready_secondary(
        player: &mut PlayerInternal,
        track_id: SpotifyUri,
        source: PlaybackSource,
    ) {
        player.preload = PlayerPreload::Ready {
            track_id,
            source: Box::new(source),
        };
    }

    fn arm_test_transition(player: &mut PlayerInternal) {
        player
            .transition
            .arm(TransitionSpec {
                duration: Duration::from_secs(1),
                curve: TransitionCurve::Linear,
                current_gain: 1.0,
                next_gain: 1.0,
            })
            .expect("test transition should arm");
    }

    fn start_secondary_with_cancellation(player: &mut PlayerInternal, cancelled: Arc<AtomicBool>) {
        let generation = player.next_secondary_generation();
        let PlayerPreload::Ready { track_id, source } = &mut player.preload else {
            panic!("test requires a ready secondary source");
        };
        source
            .decoder
            .start_secondary_with_cancellation(
                source.stream_loader_controller.clone(),
                generation,
                track_id.to_string(),
                cancelled,
            )
            .expect("secondary worker should start");
    }

    fn secondary_worker_finished(player: &PlayerInternal) -> Arc<AtomicBool> {
        let PlayerPreload::Ready { source, .. } = &player.preload else {
            panic!("test requires a ready secondary source");
        };
        source
            .decoder
            .worker_finished()
            .expect("test requires a worker-backed secondary")
    }

    fn wait_until(message: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while !condition() {
            assert!(Instant::now() < deadline, "{message}");
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn wait_for_secondary_packet(
        player: &mut PlayerInternal,
    ) -> (AudioPacketPosition, AudioPacket) {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Some(packet) = player.try_take_secondary_packet() {
                return packet;
            }
            assert!(
                Instant::now() < deadline,
                "secondary packet was not produced"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn poll_recovery_once(player: &mut PlayerInternal) -> bool {
        let mut context = Context::from_waker(std::task::Waker::noop());
        player.poll_recovery(&mut context)
    }

    fn player_internal_with_sink(
        runtime: &tokio::runtime::Runtime,
        starts: Arc<AtomicUsize>,
        stops: Arc<AtomicUsize>,
    ) -> PlayerInternal {
        player_internal_with_session(session(runtime), starts, stops)
    }

    fn player_internal_with_session(
        session: Session,
        starts: Arc<AtomicUsize>,
        stops: Arc<AtomicUsize>,
    ) -> PlayerInternal {
        let (_cmd_tx, commands) = mpsc::unbounded_channel();
        PlayerInternal {
            session,
            config: PlayerConfig::default(),
            commands,
            load_handles: Arc::new(Mutex::new(HashMap::new())),
            state: PlayerState::Stopped,
            preload: PlayerPreload::None,
            secondary_generation: 0,
            recovery: None,
            recovery_generation: 0,
            network_health: RecoveryHealth::Healthy,
            recovery_load_script: std::collections::VecDeque::new(),
            sink: Box::new(CountingSink { starts, stops }),
            sink_status: SinkStatus::Running,
            sink_event_callback: None,
            volume_getter: Box::new(NoOpVolume),
            event_senders: vec![],
            converter: Converter::new(None),
            transition_policy: NoTransitionPolicy,
            transition: TransitionEngine::new(SAMPLE_RATE, NUM_CHANNELS as usize),
            normalisation_peaks: [0.0; 2],
            normalisation_integrators: [0.0; 2],
            normalisation_channel: 0,
            normalisation_knee_factor: 1.0,
            auto_normalise_as_album: false,
            player_id: 0,
            play_request_id_generator: SeqGenerator::new(0),
            last_progress_update: Instant::now(),
            local_file_lookup: Arc::new(LocalFileLookup::default()),
        }
    }

    #[test]
    fn current_and_secondary_sources_coexist_without_polling_secondary() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        let (command_tx, commands) = mpsc::unbounded_channel();
        player.commands = commands;

        set_playing_source(&mut player, track_uri(), scripted_loaded_track(0));
        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(next_track_uri(), 0, Box::new(PanicDecoder)),
        );

        let mut context = Context::from_waker(std::task::Waker::noop());
        assert!(Pin::new(&mut player).poll(&mut context).is_pending());
        drop(command_tx);

        assert!(matches!(
            player.state,
            PlayerState::EndOfTrack { ref track_id, .. } if track_id == &track_uri()
        ));
        assert!(matches!(
            player.preload,
            PlayerPreload::Ready { ref track_id, .. } if track_id == &next_track_uri()
        ));
    }

    #[test]
    fn ready_secondary_is_promoted_without_reloading_or_rebuilding_decoder() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        set_playing_source(&mut player, track_uri(), scripted_loaded_track(10_000));

        let next_track_id = next_track_uri();
        let next_source = scripted_source(next_track_id.clone(), 0, Box::new(ScriptedDecoder));
        let decoder_address = next_source
            .decoder
            .direct_decoder_address()
            .expect("new source should own its direct decoder");
        set_ready_secondary(&mut player, next_track_id.clone(), next_source);
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        player.event_senders.push(event_tx);

        player
            .handle_command_load(next_track_id.clone(), None, true, 0)
            .expect("ready secondary should promote");

        let PlayerState::Playing {
            track_id, source, ..
        } = &player.state
        else {
            panic!("promoted source should be playing");
        };
        assert_eq!(track_id, &next_track_id);
        assert_eq!(
            source
                .decoder
                .direct_decoder_address()
                .expect("unstarted promoted source should remain direct"),
            decoder_address,
        );
        assert!(matches!(player.preload, PlayerPreload::None));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(PlayerEvent::PlayRequestIdChanged { .. })
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(PlayerEvent::TrackChanged { .. })
        ));
        assert!(matches!(
            event_rx.try_recv(),
            Ok(PlayerEvent::Playing { ref track_id, .. }) if track_id == &next_track_id
        ));
        assert!(event_rx.try_recv().is_err());
    }

    #[test]
    fn default_policy_does_not_start_secondary_pcm_decode() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        let decode_calls = Arc::new(AtomicUsize::new(0));
        set_playing_source(&mut player, track_uri(), scripted_loaded_track(0));
        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(
                next_track_uri(),
                0,
                Box::new(CountingPcmDecoder {
                    calls: decode_calls.clone(),
                    packets_remaining: None,
                }),
            ),
        );

        player.arm_transition_if_selected();

        let PlayerPreload::Ready { source, .. } = &player.preload else {
            panic!("secondary source should remain ready");
        };
        assert!(!source.decoder.is_worker());
        assert_eq!(decode_calls.load(Ordering::Acquire), 0);
        assert_eq!(player.transition.state(), TransitionState::Idle);
    }

    #[test]
    fn secondary_decoder_advances_independently() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        let decode_calls = Arc::new(AtomicUsize::new(0));
        set_playing_source(&mut player, track_uri(), scripted_loaded_track(0));
        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(
                next_track_uri(),
                0,
                Box::new(CountingPcmDecoder {
                    calls: decode_calls.clone(),
                    packets_remaining: Some(1),
                }),
            ),
        );

        assert!(
            player
                .start_secondary_decode()
                .expect("secondary worker should start")
        );
        let (_, packet) = wait_for_secondary_packet(&mut player);

        assert_eq!(
            packet.samples().expect("worker should produce PCM").len(),
            SECONDARY_PCM_CHUNK_FRAMES * NUM_CHANNELS as usize
        );
        assert!(decode_calls.load(Ordering::Acquire) >= 1);
        assert!(matches!(player.state, PlayerState::Playing { .. }));
        assert_eq!(player.sink_status, SinkStatus::Running);
    }

    #[test]
    fn blocked_secondary_decoder_does_not_block_current_audio_loop() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        let (command_tx, commands) = mpsc::unbounded_channel();
        player.commands = commands;
        let cancelled = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = std_mpsc::channel();
        set_playing_source(&mut player, track_uri(), scripted_loaded_track(0));
        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(
                next_track_uri(),
                0,
                Box::new(CancellationBlockedDecoder {
                    cancelled: cancelled.clone(),
                    entered: Some(entered_tx),
                }),
            ),
        );
        start_secondary_with_cancellation(&mut player, cancelled);
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("secondary decoder should enter its blocking read");

        let started = Instant::now();
        let mut context = Context::from_waker(std::task::Waker::noop());
        assert!(Pin::new(&mut player).poll(&mut context).is_pending());
        assert!(started.elapsed() < Duration::from_millis(250));
        assert!(matches!(player.state, PlayerState::EndOfTrack { .. }));

        let finished = secondary_worker_finished(&player);
        player.cancel_secondary_source("test cleanup");
        wait_until("blocked secondary worker did not exit", || {
            finished.load(Ordering::Acquire)
        });
        drop(command_tx);
    }

    #[test]
    fn bounded_secondary_queue_applies_backpressure() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        let (call_tx, call_rx) = std_mpsc::channel();
        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(
                next_track_uri(),
                0,
                Box::new(BackpressureDecoder {
                    calls: call_tx,
                    call: 0,
                }),
            ),
        );

        assert!(
            player
                .start_secondary_decode()
                .expect("secondary worker should start")
        );
        for expected_call in 1..=SECONDARY_PCM_CHANNEL_CAPACITY + 1 {
            assert_eq!(
                call_rx
                    .recv_timeout(Duration::from_secs(1))
                    .expect("decoder should fill queue and decode one blocked chunk"),
                expected_call
            );
        }
        assert!(call_rx.recv_timeout(Duration::from_millis(50)).is_err());

        let finished = secondary_worker_finished(&player);
        player.cancel_secondary_source("test cleanup");
        wait_until("backpressured secondary worker did not exit", || {
            finished.load(Ordering::Acquire)
        });
    }

    #[test]
    fn cancelling_active_secondary_resets_transition_and_stops_worker() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        let decode_calls = Arc::new(AtomicUsize::new(0));
        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(
                next_track_uri(),
                0,
                Box::new(CountingPcmDecoder {
                    calls: decode_calls.clone(),
                    packets_remaining: None,
                }),
            ),
        );
        arm_test_transition(&mut player);
        assert!(
            player
                .start_secondary_decode()
                .expect("secondary worker should start")
        );
        wait_until("secondary decoder did not advance", || {
            decode_calls.load(Ordering::Acquire) > 0
        });
        let finished = secondary_worker_finished(&player);

        player.cancel_secondary_source("test cancellation");

        assert!(matches!(player.preload, PlayerPreload::None));
        assert_eq!(player.transition.state(), TransitionState::Idle);
        wait_until("active secondary worker did not exit", || {
            finished.load(Ordering::Acquire)
        });
    }

    #[test]
    fn cancelling_network_blocked_secondary_is_nonblocking_and_reliable() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        let cancelled = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = std_mpsc::channel();
        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(
                next_track_uri(),
                0,
                Box::new(CancellationBlockedDecoder {
                    cancelled: cancelled.clone(),
                    entered: Some(entered_tx),
                }),
            ),
        );
        start_secondary_with_cancellation(&mut player, cancelled);
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("secondary decoder should enter its blocking read");
        let finished = secondary_worker_finished(&player);

        let started = Instant::now();
        player.cancel_secondary_source("network blocked test cancellation");

        assert!(started.elapsed() < Duration::from_millis(250));
        wait_until("network-blocked secondary worker did not exit", || {
            finished.load(Ordering::Acquire)
        });
    }

    #[test]
    fn secondary_eof_is_isolated_from_current_track() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        player.event_senders.push(event_tx);
        set_playing_source(&mut player, track_uri(), scripted_loaded_track(0));
        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(next_track_uri(), 0, Box::new(ScriptedDecoder)),
        );
        player
            .start_secondary_decode()
            .expect("secondary worker should start");
        let finished = secondary_worker_finished(&player);
        wait_until("secondary EOF was not produced", || {
            finished.load(Ordering::Acquire)
        });

        assert!(player.try_take_secondary_packet().is_none());
        assert!(matches!(player.state, PlayerState::Playing { .. }));
        assert!(matches!(player.preload, PlayerPreload::None));
        assert_eq!(player.sink_status, SinkStatus::Running);
        assert!(event_rx.try_recv().is_err());
    }

    #[test]
    fn secondary_transient_error_is_isolated_from_current_track() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        player.event_senders.push(event_tx);
        set_playing_source(&mut player, track_uri(), scripted_loaded_track(0));
        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(
                next_track_uri(),
                0,
                Box::new(ErrorDecoder {
                    error: Some(DecoderError::Io(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "scripted secondary timeout",
                    ))),
                }),
            ),
        );
        player
            .start_secondary_decode()
            .expect("secondary worker should start");
        let finished = secondary_worker_finished(&player);
        wait_until("secondary error was not produced", || {
            finished.load(Ordering::Acquire)
        });

        assert!(player.try_take_secondary_packet().is_none());
        assert!(matches!(player.state, PlayerState::Playing { .. }));
        assert!(matches!(player.preload, PlayerPreload::None));
        assert_eq!(player.sink_status, SinkStatus::Running);
        assert!(matches!(
            event_rx.try_recv(),
            Ok(PlayerEvent::LoadFailed {
                ref track_id,
                play_request_id: 7,
                error: PlayerLoadErrorKind::TransientNetwork,
                is_preload: true,
            }) if track_id == &next_track_uri()
        ));
        assert!(event_rx.try_recv().is_err());
    }

    #[test]
    fn secondary_permanent_error_uses_preload_unavailable_semantics() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        player.event_senders.push(event_tx);
        set_playing_source(&mut player, track_uri(), scripted_loaded_track(0));
        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(
                next_track_uri(),
                0,
                Box::new(ErrorDecoder {
                    error: Some(DecoderError::SymphoniaDecoder(
                        "scripted corrupt secondary".into(),
                    )),
                }),
            ),
        );
        player
            .start_secondary_decode()
            .expect("secondary worker should start");
        let finished = secondary_worker_finished(&player);
        wait_until("secondary permanent error was not produced", || {
            finished.load(Ordering::Acquire)
        });

        assert!(player.try_take_secondary_packet().is_none());
        assert!(matches!(player.state, PlayerState::Playing { .. }));
        assert_eq!(player.sink_status, SinkStatus::Running);
        assert!(matches!(
            event_rx.try_recv(),
            Ok(PlayerEvent::Unavailable {
                ref track_id,
                play_request_id: 7,
            }) if track_id == &next_track_uri()
        ));
        assert!(event_rx.try_recv().is_err());
    }

    #[test]
    fn stale_secondary_generation_cannot_deliver_pcm() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        let decode_calls = Arc::new(AtomicUsize::new(0));
        set_playing_source(&mut player, track_uri(), scripted_loaded_track(0));
        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(
                next_track_uri(),
                0,
                Box::new(CountingPcmDecoder {
                    calls: decode_calls.clone(),
                    packets_remaining: Some(1),
                }),
            ),
        );
        player
            .start_secondary_decode()
            .expect("secondary worker should start");
        wait_until("secondary decoder did not advance", || {
            decode_calls.load(Ordering::Acquire) > 0
        });

        player.next_secondary_generation();

        assert!(player.try_take_secondary_packet().is_none());
        assert!(matches!(player.preload, PlayerPreload::None));
        assert!(matches!(player.state, PlayerState::Playing { .. }));
    }

    #[test]
    fn worker_backed_secondary_promotes_without_reload_or_buffer_loss() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        let decode_calls = Arc::new(AtomicUsize::new(0));
        let next_track_id = next_track_uri();
        set_playing_source(&mut player, track_uri(), scripted_loaded_track(0));
        set_ready_secondary(
            &mut player,
            next_track_id.clone(),
            scripted_source(
                next_track_id.clone(),
                0,
                Box::new(CountingPcmDecoder {
                    calls: decode_calls.clone(),
                    packets_remaining: Some(2),
                }),
            ),
        );
        player
            .start_secondary_decode()
            .expect("secondary worker should start");
        wait_until("secondary decoder did not predecode", || {
            decode_calls.load(Ordering::Acquire) >= 2
        });

        player
            .handle_command_load(next_track_id.clone(), None, true, 0)
            .expect("worker-backed secondary should promote");

        let PlayerState::Playing {
            track_id, source, ..
        } = &mut player.state
        else {
            panic!("promoted secondary should become current");
        };
        assert_eq!(track_id, &next_track_id);
        assert!(source.decoder.is_worker());
        let packet = source
            .decoder
            .next_packet()
            .expect("worker-backed current decode should succeed")
            .expect("predecoded PCM should remain queued");
        assert_eq!(
            packet
                .1
                .samples()
                .expect("promoted worker should preserve PCM")
                .len(),
            SECONDARY_PCM_CHUNK_FRAMES * NUM_CHANNELS as usize
        );
        assert!(matches!(player.preload, PlayerPreload::None));
    }

    #[test]
    fn passthrough_never_starts_secondary_pcm_decode() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        player.config.passthrough = true;
        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(next_track_uri(), 0, Box::new(PanicDecoder)),
        );

        assert!(
            !player
                .start_secondary_decode()
                .expect("passthrough skips PCM")
        );
        let PlayerPreload::Ready { source, .. } = &player.preload else {
            panic!("passthrough secondary should remain ready");
        };
        assert!(!source.decoder.is_worker());
    }

    #[test]
    fn seek_load_and_stop_cancel_secondary_and_reset_transition() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        let current_track_id = track_uri();
        set_playing_source(
            &mut player,
            current_track_id.clone(),
            scripted_loaded_track(0),
        );

        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(next_track_uri(), 0, Box::new(PanicDecoder)),
        );
        arm_test_transition(&mut player);
        player
            .handle_command_seek(1_000)
            .expect("current source should seek");
        assert!(matches!(player.preload, PlayerPreload::None));
        assert_eq!(player.transition.state(), TransitionState::Idle);

        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(next_track_uri(), 0, Box::new(PanicDecoder)),
        );
        arm_test_transition(&mut player);
        player
            .handle_command_load(current_track_id, None, true, 1_000)
            .expect("same-source load should be accepted");
        assert!(matches!(player.preload, PlayerPreload::None));
        assert_eq!(player.transition.state(), TransitionState::Idle);

        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(next_track_uri(), 0, Box::new(PanicDecoder)),
        );
        arm_test_transition(&mut player);
        player.handle_player_stop();
        assert!(matches!(player.preload, PlayerPreload::None));
        assert_eq!(player.transition.state(), TransitionState::Idle);
        assert!(matches!(player.state, PlayerState::Stopped));
    }

    #[test]
    fn single_source_player_path_still_writes_identical_pcm() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        let packets = Arc::new(Mutex::new(Vec::new()));
        player.sink = Box::new(RecordingSink {
            packets: packets.clone(),
        });
        let samples = vec![0.25, -0.5, 0.75, -1.0];

        player.handle_packet(
            Some((
                AudioPacketPosition {
                    position_ms: 0,
                    skipped: false,
                },
                AudioPacket::Samples(samples.clone()),
            )),
            1.0,
        );

        assert_eq!(
            packets
                .lock()
                .expect("recorded packets mutex should not be poisoned")
                .as_slice(),
            [samples]
        );
        assert_eq!(player.transition.state(), TransitionState::Idle);
    }

    #[test]
    fn permanent_track_load_failure_emits_unavailable() {
        assert!(matches!(
            load_error_event(track_uri(), 7, PlayerLoadErrorKind::PermanentTrack, false),
            PlayerEvent::Unavailable {
                play_request_id: 7,
                ..
            }
        ));
    }

    #[test]
    fn transient_preload_failure_emits_load_failed() {
        assert!(matches!(
            load_error_event(track_uri(), 7, PlayerLoadErrorKind::TransientService, true),
            PlayerEvent::LoadFailed {
                play_request_id: 7,
                error: PlayerLoadErrorKind::TransientService,
                is_preload: true,
                ..
            }
        ));
    }

    #[test]
    fn transient_recovery_targets_same_uri_position_and_request() {
        let recovery = recovery();

        assert_eq!(recovery.track_id, track_uri());
        assert_eq!(recovery.position_ms, 42_123);
        assert_eq!(recovery.play_request_id, 7);
        assert!(recovery.start_playback);
    }

    #[test]
    fn scripted_initial_failure_retries_same_track_and_resumes_once() {
        let expected_uri = track_uri();
        let (attempts, resumes) = run_scripted_recovery(
            recovery(),
            &[
                ScriptedLoadOutcome::TransientFailure,
                ScriptedLoadOutcome::TransientFailure,
                ScriptedLoadOutcome::Success,
                ScriptedLoadOutcome::Success,
            ],
        );

        assert_eq!(attempts.len(), 3);
        assert!(attempts.iter().all(|(uri, position_ms, play)| {
            uri == &expected_uri && *position_ms == 42_123 && *play
        }));
        assert_eq!(resumes, 1);
    }

    #[test]
    fn play_pause_intent_is_preserved_and_updateable_during_recovery() {
        let mut recovery = recovery();
        recovery.set_play_intent(false);
        assert!(!recovery.start_playback);

        recovery.set_play_intent(true);
        assert!(recovery.start_playback);
        assert_eq!(recovery.position_ms, 42_123);
    }

    #[test]
    fn seek_invalidates_stale_recovery_and_preserves_same_track() {
        let mut recovery = recovery();
        recovery.restart(12, 88_000, true);

        assert!(!recovery.is_current(11));
        assert!(recovery.is_current(12));
        assert_eq!(recovery.track_id, track_uri());
        assert_eq!(recovery.position_ms, 88_000);
        assert!(matches!(recovery.phase, RecoveryPhase::WaitingForSession));
    }

    #[test]
    fn newer_load_next_or_stop_generation_makes_retry_stale() {
        let recovery = recovery();

        for newer_generation in [12, 13, 14] {
            assert!(!recovery.is_current(newer_generation));
        }
    }

    #[test]
    fn dropping_stale_loader_requests_cancellation() {
        let (_sender, receiver) = oneshot::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let loader = CancellableTrackLoader {
            receiver,
            cancelled: cancelled.clone(),
            active_controller: Arc::new(Mutex::new(None)),
            terminated: false,
        };

        drop(loader);

        assert!(cancelled.load(Ordering::Acquire));
    }

    #[test]
    fn recovery_sink_stop_and_restart_are_each_latched_once() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts.clone(), stops.clone());

        player.ensure_sink_stopped(true);
        player.ensure_sink_stopped(true);
        assert_eq!(stops.load(Ordering::Acquire), 1);

        player.ensure_sink_running();
        player.ensure_sink_running();
        assert_eq!(starts.load(Ordering::Acquire), 1);
    }

    #[test]
    fn track_retry_backoff_is_bounded_and_jittered() {
        let first = PlayerInternal::recovery_delay(11, 1);
        let later = PlayerInternal::recovery_delay(11, 8);

        assert!(first < later);
        assert!(later <= MAX_TRACK_RECOVERY_DELAY);
    }

    #[tokio::test(start_paused = true)]
    async fn fast_budget_exhaustion_enters_slow_latched_recovery() {
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_session(
            Session::new(SessionConfig::default(), None),
            starts.clone(),
            stops.clone(),
        );
        player.sink_status = SinkStatus::TemporarilyClosed;
        player.network_health = RecoveryHealth::Recovering;
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        player.event_senders.push(event_tx);

        let mut recovery = recovery();
        recovery.attempt = MAX_TRACK_RECOVERY_ATTEMPTS - 1;
        recovery.phase = RecoveryPhase::Loading(Box::pin(futures_util::future::ready(Err(
            PlayerLoadError::message(
                PlayerLoadErrorKind::TransientNetwork,
                "scripted fast-budget failure",
            ),
        ))));
        player.recovery_generation = recovery.generation;
        player.recovery = Some(recovery);

        assert!(poll_recovery_once(&mut player));

        let recovery = player.recovery.as_ref().expect("recovery remains latched");
        assert_eq!(recovery.mode, RecoveryMode::Slow);
        assert_eq!(recovery.track_id, track_uri());
        assert_eq!(recovery.position_ms, 42_123);
        assert!(matches!(recovery.phase, RecoveryPhase::Waiting(_)));
        assert_eq!(player.network_health, RecoveryHealth::Recovering);
        assert_eq!(starts.load(Ordering::Acquire), 0);
        assert_eq!(stops.load(Ordering::Acquire), 0);
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn slow_recovery_keeps_retrying_and_resumes_same_track_once() {
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_session(
            Session::new(SessionConfig::default(), None),
            starts.clone(),
            stops.clone(),
        );
        player.sink_status = SinkStatus::TemporarilyClosed;
        player.network_health = RecoveryHealth::Recovering;
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        player.event_senders.push(event_tx);
        player
            .recovery_load_script
            .push_back(Err(PlayerLoadError::message(
                PlayerLoadErrorKind::TransientNetwork,
                "scripted slow failure",
            )));
        player
            .recovery_load_script
            .push_back(Ok(scripted_loaded_track(42_123)));

        let mut recovery = recovery();
        recovery.attempt = MAX_TRACK_RECOVERY_ATTEMPTS;
        recovery.mode = RecoveryMode::Slow;
        let first_delay =
            PlayerInternal::slow_recovery_delay(recovery.generation, recovery.attempt + 1);
        recovery.phase = RecoveryPhase::Waiting(Box::pin(tokio::time::sleep(first_delay)));
        player.recovery_generation = recovery.generation;
        player.recovery = Some(recovery);

        tokio::time::advance(first_delay).await;
        assert!(poll_recovery_once(&mut player));
        assert!(poll_recovery_once(&mut player));

        let recovery = player
            .recovery
            .as_ref()
            .expect("slow recovery remains latched");
        assert_eq!(recovery.mode, RecoveryMode::Slow);
        assert_eq!(recovery.track_id, track_uri());
        assert_eq!(recovery.position_ms, 42_123);
        assert!(recovery.start_playback);
        assert_eq!(starts.load(Ordering::Acquire), 0);
        assert_eq!(stops.load(Ordering::Acquire), 0);
        assert!(event_rx.try_recv().is_err());

        let second_delay =
            PlayerInternal::slow_recovery_delay(recovery.generation, recovery.attempt + 1);
        tokio::time::advance(second_delay).await;
        assert!(poll_recovery_once(&mut player));
        assert!(poll_recovery_once(&mut player));

        assert!(player.recovery.is_none());
        assert_eq!(player.network_health, RecoveryHealth::Healthy);
        assert!(matches!(player.state, PlayerState::Playing { .. }));
        assert_eq!(starts.load(Ordering::Acquire), 1);
        assert_eq!(stops.load(Ordering::Acquire), 0);

        let mut terminal_events = 0;
        while let Ok(event) = event_rx.try_recv() {
            if matches!(
                event,
                PlayerEvent::EndOfTrack { .. }
                    | PlayerEvent::Unavailable { .. }
                    | PlayerEvent::LoadFailed { .. }
            ) {
                terminal_events += 1;
            }
        }
        assert_eq!(terminal_events, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn newer_load_command_cancels_slow_recovery_generation() {
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_session(
            Session::new(SessionConfig::default(), None),
            starts,
            stops,
        );
        let mut recovery = recovery();
        recovery.mode = RecoveryMode::Slow;
        recovery.attempt = MAX_TRACK_RECOVERY_ATTEMPTS;
        recovery.phase =
            RecoveryPhase::Waiting(Box::pin(tokio::time::sleep(MAX_SLOW_RECOVERY_DELAY)));
        player.recovery_generation = recovery.generation;
        player.recovery = Some(recovery);
        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(next_track_uri(), 0, Box::new(PanicDecoder)),
        );
        arm_test_transition(&mut player);

        let newer_track = SpotifyUri::from_uri("spotify:album:0sNOF9WDwhWunNAHPD3Baj")
            .expect("test URI should be valid");
        player
            .handle_command(PlayerCommand::Load {
                track_id: newer_track.clone(),
                play: true,
                position_ms: 0,
            })
            .expect("new Load should be accepted");

        assert!(player.recovery.is_none());
        assert!(player.recovery_generation > 11);
        assert!(matches!(player.preload, PlayerPreload::None));
        assert_eq!(player.transition.state(), TransitionState::Idle);
        assert!(matches!(
            player.state,
            PlayerState::Loading { ref track_id, .. } if track_id == &newer_track
        ));
        player.state = PlayerState::Stopped;
    }

    #[test]
    fn session_invalid_recovery_waits_instead_of_retrying_media() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        set_ready_secondary(
            &mut player,
            next_track_uri(),
            scripted_source(next_track_uri(), 0, Box::new(PanicDecoder)),
        );
        arm_test_transition(&mut player);

        player.begin_recovery(RecoveryRequest {
            track_id: track_uri(),
            play_request_id: 7,
            position_ms: 42_123,
            start_playback: true,
            kind: PlayerLoadErrorKind::SessionInvalid,
            failed_attempts: 0,
            buffer_starved: true,
        });

        let recovery = player.recovery.as_ref().expect("recovery remains latched");
        assert_eq!(recovery.attempt, 0);
        assert_eq!(recovery.track_id, track_uri());
        assert_eq!(recovery.position_ms, 42_123);
        assert!(recovery.start_playback);
        assert!(matches!(recovery.phase, RecoveryPhase::WaitingForSession));
        assert!(matches!(player.preload, PlayerPreload::None));
        assert_eq!(player.transition.state(), TransitionState::Idle);
    }

    #[test]
    fn replacement_session_restarts_latched_same_track_recovery() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops);
        player.recovery_generation = 11;
        player.recovery = Some(recovery());
        player.session.shutdown();
        let replacement = session(&runtime);

        let _guard = runtime.enter();
        player.handle_set_session(replacement);

        let recovery = player.recovery.as_ref().expect("recovery remains latched");
        assert_eq!(recovery.track_id, track_uri());
        assert_eq!(recovery.position_ms, 42_123);
        assert!(recovery.start_playback);
        assert!(recovery.generation > 11);
        assert!(matches!(recovery.phase, RecoveryPhase::Waiting(_)));
    }

    #[test]
    fn stop_cancels_latched_recovery_and_stops_sink_once() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let starts = Arc::new(AtomicUsize::new(0));
        let stops = Arc::new(AtomicUsize::new(0));
        let mut player = player_internal_with_sink(&runtime, starts, stops.clone());
        player.recovery_generation = 11;
        player.recovery = Some(recovery());

        player.handle_player_stop();
        player.handle_player_stop();

        assert!(player.recovery.is_none());
        assert!(matches!(player.state, PlayerState::Stopped));
        assert_eq!(stops.load(Ordering::Acquire), 1);
        assert!(player.recovery_generation > 11);
    }

    #[test]
    fn mid_track_timeout_is_recovery_not_end_of_track() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let session = session(&runtime);
        let error = PlayerLoadError::from_decoder_error(
            &session,
            DecoderError::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "scripted starvation",
            )),
        );
        let event = load_error_event(track_uri(), 7, error.kind, false);

        assert_eq!(error.kind, PlayerLoadErrorKind::TransientNetwork);
        assert!(matches!(event, PlayerEvent::LoadFailed { .. }));
        assert!(!matches!(event, PlayerEvent::EndOfTrack { .. }));
    }

    #[test]
    fn natural_eof_still_produces_end_of_track() {
        assert!(matches!(
            natural_end_of_track_event(track_uri(), 7),
            PlayerEvent::EndOfTrack {
                play_request_id: 7,
                ..
            }
        ));
    }

    #[test]
    fn corrupt_decoder_media_is_permanent_only_with_positive_decoder_evidence() {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        let session = session(&runtime);
        let error = PlayerLoadError::from_decoder_error(
            &session,
            DecoderError::SymphoniaDecoder("scripted corrupt container".into()),
        );

        assert_eq!(error.kind, PlayerLoadErrorKind::PermanentTrack);
        assert!(matches!(
            load_error_event(track_uri(), 7, error.kind, false),
            PlayerEvent::Unavailable { .. }
        ));
    }
}
