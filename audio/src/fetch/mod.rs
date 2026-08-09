mod receive;

use std::{
    cmp::min,
    fs,
    io::{self, Read, Seek, SeekFrom},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    sync::{Condvar, Mutex},
    time::Duration,
};

use futures_util::{StreamExt, TryFutureExt, future::IntoStream};
use hyper::{Response, StatusCode, body::Incoming, header::CONTENT_RANGE};
use hyper_util::client::legacy::ResponseFuture;

use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::sync::{Semaphore, mpsc, oneshot};

use librespot_core::{
    Error, FileId, Session,
    cdn_url::CdnUrl,
    error::ErrorKind,
    http_client::{HttpClient, HttpClientError},
};

use self::receive::audio_file_fetch;

use crate::range_set::{Range, RangeSet};

pub type AudioFileResult = Result<(), librespot_core::Error>;
const MAX_INITIAL_RETRY_AFTER: Duration = Duration::from_secs(60);

/// Classification retained for streaming failures which cross the synchronous `Read` boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioFileErrorKind {
    TransientNetwork,
    TransientService,
    SessionInvalid,
    Cancelled,
    PermanentMedia,
}

/// A cloneable, structured range failure suitable for embedding in `std::io::Error`.
#[derive(Clone, Debug)]
pub struct AudioFileFailure {
    pub kind: AudioFileErrorKind,
    pub range: Range,
    source: Arc<Error>,
}

impl AudioFileFailure {
    pub(crate) fn new(kind: AudioFileErrorKind, range: Range, source: Error) -> Self {
        Self {
            kind,
            range,
            source: Arc::new(source),
        }
    }

    pub(crate) fn classify(session: &Session, range: Range, source: Error) -> Self {
        let kind = if session.is_invalid() {
            AudioFileErrorKind::SessionInvalid
        } else if let Some(HttpClientError::StatusCode(status)) =
            source.error.downcast_ref::<HttpClientError>()
        {
            classify_http_status(status.as_u16())
        } else if source.kind == ErrorKind::Unauthenticated {
            AudioFileErrorKind::SessionInvalid
        } else {
            match source.kind {
                ErrorKind::Cancelled => AudioFileErrorKind::Cancelled,
                ErrorKind::DeadlineExceeded
                | ErrorKind::Aborted
                | ErrorKind::DataLoss
                | ErrorKind::Unavailable => AudioFileErrorKind::TransientNetwork,
                ErrorKind::ResourceExhausted => AudioFileErrorKind::TransientService,
                ErrorKind::OutOfRange => AudioFileErrorKind::TransientService,
                _ => AudioFileErrorKind::TransientService,
            }
        };

        Self::new(kind, range, source)
    }

    fn io_kind(&self) -> io::ErrorKind {
        match self.kind {
            AudioFileErrorKind::TransientNetwork => io::ErrorKind::TimedOut,
            AudioFileErrorKind::TransientService => io::ErrorKind::WouldBlock,
            AudioFileErrorKind::SessionInvalid => io::ErrorKind::NotConnected,
            AudioFileErrorKind::Cancelled => io::ErrorKind::Interrupted,
            AudioFileErrorKind::PermanentMedia => io::ErrorKind::InvalidData,
        }
    }

    fn into_core_error(self) -> Error {
        match self.kind {
            AudioFileErrorKind::TransientNetwork => Error::unavailable(self),
            AudioFileErrorKind::TransientService => Error::resource_exhausted(self),
            AudioFileErrorKind::SessionInvalid => Error::unauthenticated(self),
            AudioFileErrorKind::Cancelled => Error::cancelled(self),
            AudioFileErrorKind::PermanentMedia => Error::data_loss(self),
        }
    }
}

fn classify_http_status(status: u16) -> AudioFileErrorKind {
    let _ = status;
    // A status from the media CDN can describe an expired signed URL or edge failure. It is not
    // positive evidence that the AP session or media item is invalid; range retry will refresh the
    // resolved URL candidates for authentication and expiry statuses.
    AudioFileErrorKind::TransientService
}

impl std::fmt::Display for AudioFileFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} while fetching range {}: {}",
            self.kind, self.range, self.source
        )
    }
}

impl std::error::Error for AudioFileFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

const DOWNLOAD_STATUS_POISON_MSG: &str = "audio download status mutex should not be poisoned";

#[derive(Error, Debug)]
pub enum AudioFileError {
    #[error("other end of channel disconnected")]
    Channel,
    #[error("required header not found")]
    Header,
    #[error("streamer received no data")]
    NoData,
    #[error("no output available")]
    Output,
    #[error("invalid status code {0}")]
    StatusCode(StatusCode),
    #[error("wait timeout exceeded")]
    WaitTimeout,
}

impl From<AudioFileError> for Error {
    fn from(err: AudioFileError) -> Self {
        match err {
            AudioFileError::Channel => Error::aborted(err),
            AudioFileError::Header => Error::unavailable(err),
            AudioFileError::NoData => Error::unavailable(err),
            AudioFileError::Output => Error::aborted(err),
            AudioFileError::StatusCode(_) => Error::failed_precondition(err),
            AudioFileError::WaitTimeout => Error::deadline_exceeded(err),
        }
    }
}

#[derive(Clone)]
pub struct AudioFetchParams {
    /// The minimum size of a block that is requested from the Spotify servers in one request.
    /// This is the block size that is typically requested while doing a `seek()` on a file.
    /// The Symphonia decoder requires this to be a power of 2 and > 32 kB.
    /// Note: smaller requests can happen if part of the block is downloaded already.
    pub minimum_download_size: usize,

    /// The minimum network throughput that we expect. Together with the minimum download size,
    /// this will determine the time we will wait for a response.
    pub minimum_throughput: usize,

    /// The ping time that is used for calculations before a ping time was actually measured.
    pub initial_ping_time_estimate: Duration,

    /// If the measured ping time to the Spotify server is larger than this value, it is capped
    /// to avoid run-away block sizes and pre-fetching.
    pub maximum_assumed_ping_time: Duration,

    /// Before playback starts, this many seconds of data must be present.
    /// Note: the calculations are done using the nominal bitrate of the file. The actual amount
    /// of audio data may be larger or smaller.
    pub read_ahead_before_playback: Duration,

    /// While playing back, this many seconds of data ahead of the current read position are
    /// requested.
    /// Note: the calculations are done using the nominal bitrate of the file. The actual amount
    /// of audio data may be larger or smaller.
    pub read_ahead_during_playback: Duration,

    /// Minimum compressed bytes requested ahead of the current read position. This complements
    /// the time targets for low-bitrate media without allocating an in-memory audio buffer.
    pub minimum_read_ahead_bytes: usize,

    /// If the amount of data that is pending (requested but not received) is less than a certain amount,
    /// data is pre-fetched in addition to the read ahead settings above. The threshold for requesting more
    /// data is calculated as `<pending bytes> < PREFETCH_THRESHOLD_FACTOR * <ping time> * <nominal data rate>`
    pub prefetch_threshold_factor: f32,

    /// The time we will wait to obtain status updates on downloading.
    pub download_timeout: Duration,
}

impl Default for AudioFetchParams {
    fn default() -> Self {
        let minimum_download_size = 64 * 1024;
        let minimum_throughput = 8 * 1024;
        Self {
            minimum_download_size,
            minimum_throughput,
            initial_ping_time_estimate: Duration::from_millis(500),
            maximum_assumed_ping_time: Duration::from_millis(1500),
            read_ahead_before_playback: Duration::from_secs(5),
            read_ahead_during_playback: Duration::from_secs(5),
            minimum_read_ahead_bytes: 256 * 1024,
            prefetch_threshold_factor: 4.0,
            download_timeout: Duration::from_secs(
                (minimum_download_size / minimum_throughput) as u64,
            ),
        }
    }
}

static AUDIO_FETCH_PARAMS: OnceLock<AudioFetchParams> = OnceLock::new();

impl AudioFetchParams {
    pub fn set(params: AudioFetchParams) -> Result<(), AudioFetchParams> {
        AUDIO_FETCH_PARAMS.set(params)
    }

    pub fn get() -> &'static AudioFetchParams {
        AUDIO_FETCH_PARAMS.get_or_init(AudioFetchParams::default)
    }
}

pub enum AudioFile {
    Cached(fs::File),
    Streaming(AudioFileStreaming),
}

#[derive(Debug)]
pub struct StreamingRequest {
    streamer: IntoStream<ResponseFuture>,
    initial_response: Option<Response<Incoming>>,
    offset: usize,
    length: usize,
    attempt: usize,
}

#[derive(Debug)]
pub enum StreamLoaderCommand {
    Fetch(Range), // signal the stream loader to fetch a range of the file
    Close,        // terminate and don't load any more data
}

#[derive(Clone)]
pub struct StreamLoaderController {
    channel_tx: Option<mpsc::UnboundedSender<StreamLoaderCommand>>,
    stream_shared: Option<Arc<AudioFileShared>>,
    file_size: usize,
}

impl StreamLoaderController {
    pub fn len(&self) -> usize {
        self.file_size
    }

    pub fn is_empty(&self) -> bool {
        self.file_size == 0
    }

    pub fn range_available(&self, range: Range) -> bool {
        if let Some(ref shared) = self.stream_shared {
            let download_status = shared
                .download_status
                .lock()
                .expect(DOWNLOAD_STATUS_POISON_MSG);

            range.length
                <= download_status
                    .downloaded
                    .contained_length_from_value(range.start)
        } else {
            range.length <= self.len() - range.start
        }
    }

    pub fn range_to_end_available(&self) -> bool {
        match self.stream_shared {
            Some(ref shared) => {
                let read_position = shared.read_position();
                self.range_available(Range::new(read_position, self.len() - read_position))
            }
            None => true,
        }
    }

    pub fn ping_time(&self) -> Option<Duration> {
        self.stream_shared.as_ref().map(|shared| shared.ping_time())
    }

    pub fn is_degraded(&self) -> bool {
        self.stream_shared
            .as_ref()
            .is_some_and(|shared| shared.degraded.load(Ordering::Acquire))
    }

    fn send_stream_loader_command(&self, command: StreamLoaderCommand) {
        if let Some(ref channel) = self.channel_tx {
            // Ignore the error in case the channel has been closed already.
            // This means that the file was completely downloaded.
            let _ = channel.send(command);
        }
    }

    pub fn fetch(&self, range: Range) {
        // signal the stream loader to fetch a range of the file
        self.send_stream_loader_command(StreamLoaderCommand::Fetch(range));
    }

    pub fn fetch_blocking(&self, mut range: Range) -> AudioFileResult {
        // signal the stream loader to tech a range of the file and block until it is loaded.

        // ensure the range is within the file's bounds.
        if range.start >= self.len() {
            range.length = 0;
        } else if range.end() > self.len() {
            range.length = self.len() - range.start;
        }

        self.fetch(range);

        if let Some(ref shared) = self.stream_shared {
            let mut download_status = shared
                .download_status
                .lock()
                .expect(DOWNLOAD_STATUS_POISON_MSG);
            let download_timeout = AudioFetchParams::get().download_timeout;

            while range.length
                > download_status
                    .downloaded
                    .contained_length_from_value(range.start)
            {
                if shared.closed.load(Ordering::Acquire) {
                    return Err(Error::cancelled("audio range request was closed"));
                }
                if let Some(failure) = shared.failure_at(range.start) {
                    return Err(failure.into_core_error());
                }

                let (new_download_status, wait_result) = shared
                    .cond
                    .wait_timeout(download_status, download_timeout)
                    .expect(DOWNLOAD_STATUS_POISON_MSG);

                download_status = new_download_status;
                if wait_result.timed_out() {
                    if let Some(failure) = shared
                        .failure_at(range.start)
                        .or_else(|| shared.latest_failure_at(range.start))
                    {
                        return Err(failure.into_core_error());
                    }
                    return Err(AudioFileError::WaitTimeout.into());
                }

                if range.length
                    > (download_status
                        .downloaded
                        .union(&download_status.requested)
                        .contained_length_from_value(range.start))
                {
                    // For some reason, the requested range is neither downloaded nor requested.
                    // This could be due to a network error. Request it again.
                    self.fetch(range);
                }
            }
        }

        Ok(())
    }

    pub fn fetch_next_and_wait(
        &self,
        request_length: usize,
        wait_length: usize,
    ) -> AudioFileResult {
        match self.stream_shared {
            Some(ref shared) => {
                let start = shared.read_position();

                let request_range = Range {
                    start,
                    length: request_length,
                };
                self.fetch(request_range);

                let wait_range = Range {
                    start,
                    length: wait_length,
                };
                self.fetch_blocking(wait_range)
            }
            None => Ok(()),
        }
    }

    pub fn set_random_access_mode(&self) {
        // optimise download strategy for random access
        if let Some(ref shared) = self.stream_shared {
            shared.set_download_streaming(false)
        }
    }

    pub fn set_stream_mode(&self) {
        // optimise download strategy for streaming
        if let Some(ref shared) = self.stream_shared {
            shared.set_download_streaming(true)
        }
    }

    pub fn close(&self) {
        // terminate stream loading and don't load any more data for this file.
        self.send_stream_loader_command(StreamLoaderCommand::Close);
    }

    pub fn from_local_file(file_size: u64) -> Self {
        Self {
            channel_tx: None,
            stream_shared: None,
            file_size: file_size as usize,
        }
    }
}

pub struct AudioFileStreaming {
    read_file: fs::File,
    position: u64,
    stream_loader_command_tx: mpsc::UnboundedSender<StreamLoaderCommand>,
    shared: Arc<AudioFileShared>,
}

struct AudioFileDownloadStatus {
    requested: RangeSet,
    downloaded: RangeSet,
}

struct AudioFileShared {
    file_size: usize,
    bytes_per_second: usize,
    cond: Condvar,
    download_status: Mutex<AudioFileDownloadStatus>,
    download_streaming: AtomicBool,
    download_slots: Semaphore,
    ping_time_ms: AtomicUsize,
    read_position: AtomicUsize,
    throughput: AtomicUsize,
    degraded: AtomicBool,
    closed: AtomicBool,
    latest_failure: Mutex<Option<AudioFileFailure>>,
    last_failure: Mutex<Option<AudioFileFailure>>,
}

impl AudioFileShared {
    fn is_download_streaming(&self) -> bool {
        self.download_streaming.load(Ordering::Acquire)
    }

    fn set_download_streaming(&self, streaming: bool) {
        self.download_streaming.store(streaming, Ordering::Release)
    }

    fn ping_time(&self) -> Duration {
        let ping_time_ms = self.ping_time_ms.load(Ordering::Acquire);
        if ping_time_ms > 0 {
            Duration::from_millis(ping_time_ms as u64)
        } else {
            AudioFetchParams::get().initial_ping_time_estimate
        }
    }

    fn set_ping_time(&self, duration: Duration) {
        self.ping_time_ms
            .store(duration.as_millis() as usize, Ordering::Release)
    }

    fn throughput(&self) -> usize {
        self.throughput.load(Ordering::Acquire)
    }

    fn set_throughput(&self, throughput: usize) {
        self.throughput.store(throughput, Ordering::Release)
    }

    fn read_position(&self) -> usize {
        self.read_position.load(Ordering::Acquire)
    }

    fn set_read_position(&self, position: u64) {
        self.read_position
            .store(position as usize, Ordering::Release)
    }

    fn failure_at(&self, position: usize) -> Option<AudioFileFailure> {
        self.last_failure
            .lock()
            .expect(DOWNLOAD_STATUS_POISON_MSG)
            .as_ref()
            .filter(|failure| range_contains_position(failure.range, position))
            .cloned()
    }

    fn latest_failure_at(&self, position: usize) -> Option<AudioFileFailure> {
        self.latest_failure
            .lock()
            .expect(DOWNLOAD_STATUS_POISON_MSG)
            .as_ref()
            .filter(|failure| range_contains_position(failure.range, position))
            .cloned()
    }
}

fn range_contains_position(range: Range, position: usize) -> bool {
    range.start <= position && position < range.end()
}

impl AudioFile {
    pub async fn open(
        session: &Session,
        file_id: FileId,
        bytes_per_second: usize,
    ) -> Result<AudioFile, Error> {
        if let Some(file) = session.cache().and_then(|cache| cache.file(file_id)) {
            debug!("File {file_id} already in cache");
            return Ok(AudioFile::Cached(file));
        }

        debug!("Downloading file {file_id}");

        let (complete_tx, complete_rx) = oneshot::channel();

        let streaming =
            AudioFileStreaming::open(session.clone(), file_id, complete_tx, bytes_per_second);

        let session_ = session.clone();
        session.spawn(complete_rx.map_ok(move |mut file| {
            debug!("Downloading file {file_id} complete");

            if let Some(cache) = session_.cache() {
                if let Some(cache_id) = cache.file_path(file_id) {
                    if let Err(e) = cache.save_file(file_id, &mut file) {
                        error!("Error caching file {file_id} to {cache_id:?}: {e}");
                    } else {
                        debug!("File {file_id} cached to {cache_id:?}");
                    }
                }
            }
        }));

        Ok(AudioFile::Streaming(streaming.await?))
    }

    pub fn get_stream_loader_controller(&self) -> Result<StreamLoaderController, Error> {
        let controller = match self {
            AudioFile::Streaming(stream) => StreamLoaderController {
                channel_tx: Some(stream.stream_loader_command_tx.clone()),
                stream_shared: Some(stream.shared.clone()),
                file_size: stream.shared.file_size,
            },
            AudioFile::Cached(file) => StreamLoaderController {
                channel_tx: None,
                stream_shared: None,
                file_size: file.metadata()?.len() as usize,
            },
        };

        Ok(controller)
    }

    pub fn is_cached(&self) -> bool {
        matches!(self, AudioFile::Cached { .. })
    }
}

impl AudioFileStreaming {
    pub async fn open(
        session: Session,
        file_id: FileId,
        complete_tx: oneshot::Sender<NamedTempFile>,
        bytes_per_second: usize,
    ) -> Result<AudioFileStreaming, Error> {
        let cdn_url = CdnUrl::new(file_id).resolve_audio(&session).await?;

        let minimum_download_size = AudioFetchParams::get().minimum_download_size;

        let mut initial_request = None;
        let mut last_error = None;
        let urls = cdn_url.try_get_urls()?;
        for (candidate_index, url) in urls.iter().enumerate() {
            // When the audio file is really small, this `download_size` may turn out to be
            // larger than the audio file we're going to stream later on. This is OK; requesting
            // `Content-Range` > `Content-Length` will return the complete file with status code
            // 206 Partial Content.
            for status_attempt in 0..2 {
                let mut streamer =
                    match session
                        .spclient()
                        .stream_from_cdn(*url, 0, minimum_download_size)
                    {
                        Ok(streamer) => streamer,
                        Err(error) => {
                            last_error = Some(error);
                            break;
                        }
                    };

                // Get the headers to learn the file size. The body remains streaming and is
                // consumed by `audio_file_fetch` so startup bytes become usable immediately.
                let streamer_result =
                    tokio::time::timeout(Duration::from_secs(10), streamer.next())
                        .await
                        .map_err(|_| AudioFileError::WaitTimeout.into())
                        .and_then(|x| x.ok_or_else(|| AudioFileError::NoData.into()))
                        .and_then(|x| x.map_err(Error::from));

                match streamer_result {
                    Ok(response) if response.status() == StatusCode::PARTIAL_CONTENT => {
                        debug!(
                            "Opened audio stream using CDN candidate {}/{}",
                            candidate_index + 1,
                            urls.len()
                        );
                        initial_request = Some((response, streamer));
                        break;
                    }
                    Ok(response) => {
                        let status = response.status();
                        if status == StatusCode::TOO_MANY_REQUESTS && status_attempt == 0 {
                            if let Some(delay) = HttpClient::get_retry_after(response.headers()) {
                                let delay = delay.min(MAX_INITIAL_RETRY_AFTER);
                                debug!(
                                    "Initial CDN candidate was rate limited; retrying after {delay:?}"
                                );
                                tokio::time::sleep(delay).await;
                                continue;
                            }
                        }
                        debug!(
                            "CDN candidate {}/{} returned retryable status {status}; failing over",
                            candidate_index + 1,
                            urls.len()
                        );
                        last_error = Some(HttpClientError::StatusCode(status).into());
                        break;
                    }
                    Err(error) => {
                        debug!(
                            "CDN candidate {}/{} failed before streaming; failing over",
                            candidate_index + 1,
                            urls.len()
                        );
                        last_error = Some(error);
                        break;
                    }
                }
            }

            if initial_request.is_some() {
                break;
            }
        }

        let Some((response, streamer)) = initial_request else {
            return Err(last_error.unwrap_or_else(|| {
                Error::unavailable(format!("{} CDN candidates failed", urls.len()))
            }));
        };

        let header_value = response
            .headers()
            .get(CONTENT_RANGE)
            .ok_or(AudioFileError::Header)?;
        let str_value = header_value.to_str()?;
        let hyphen_index = str_value.find('-').unwrap_or_default();
        let slash_index = str_value.find('/').unwrap_or_default();
        let upper_bound: usize = str_value[hyphen_index + 1..slash_index].parse()?;
        let file_size = str_value[slash_index + 1..].parse()?;

        let initial_request = StreamingRequest {
            streamer,
            initial_response: Some(response),
            offset: 0,
            length: upper_bound + 1,
            attempt: 0,
        };

        let shared = Arc::new(AudioFileShared {
            file_size,
            bytes_per_second,
            cond: Condvar::new(),
            download_status: Mutex::new(AudioFileDownloadStatus {
                requested: RangeSet::new(),
                downloaded: RangeSet::new(),
            }),
            download_streaming: AtomicBool::new(false),
            download_slots: Semaphore::new(1),
            ping_time_ms: AtomicUsize::new(0),
            read_position: AtomicUsize::new(0),
            throughput: AtomicUsize::new(0),
            degraded: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            latest_failure: Mutex::new(None),
            last_failure: Mutex::new(None),
        });

        let write_file = NamedTempFile::new_in(session.config().tmp_dir.clone())?;
        write_file.as_file().set_len(file_size as u64)?;

        let read_file = write_file.reopen()?;

        let (stream_loader_command_tx, stream_loader_command_rx) =
            mpsc::unbounded_channel::<StreamLoaderCommand>();

        session.spawn(audio_file_fetch(
            session.clone(),
            cdn_url,
            shared.clone(),
            initial_request,
            write_file,
            stream_loader_command_rx,
            complete_tx,
        ));

        Ok(AudioFileStreaming {
            read_file,
            position: 0,
            stream_loader_command_tx,
            shared,
        })
    }
}

impl Read for AudioFileStreaming {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let offset = self.position as usize;

        if offset >= self.shared.file_size {
            return Ok(0);
        }

        let length = min(output.len(), self.shared.file_size - offset);
        if length == 0 {
            return Ok(0);
        }

        let params = AudioFetchParams::get();
        let read_ahead_during_playback = params.read_ahead_during_playback;
        let length_to_request = if self.shared.is_download_streaming() {
            let time_target = (read_ahead_during_playback.as_secs_f32()
                * self.shared.bytes_per_second as f32) as usize;
            let length_to_request = length + time_target.max(params.minimum_read_ahead_bytes);

            // Due to the read-ahead stuff, we potentially request more than the actual request demanded.
            min(length_to_request, self.shared.file_size - offset)
        } else {
            length
        };

        let mut ranges_to_request = RangeSet::new();
        ranges_to_request.add_range(&Range::new(offset, length_to_request));

        let mut download_status = self
            .shared
            .download_status
            .lock()
            .expect(DOWNLOAD_STATUS_POISON_MSG);

        ranges_to_request.subtract_range_set(&download_status.downloaded);
        ranges_to_request.subtract_range_set(&download_status.requested);

        for &range in ranges_to_request.iter() {
            self.stream_loader_command_tx
                .send(StreamLoaderCommand::Fetch(range))
                .map_err(|err| io::Error::new(io::ErrorKind::BrokenPipe, err))?;
        }

        let download_timeout = AudioFetchParams::get().download_timeout;
        while !download_status.downloaded.contains(offset) {
            if self.shared.closed.load(Ordering::Acquire) {
                let failure = AudioFileFailure::new(
                    AudioFileErrorKind::Cancelled,
                    Range::new(offset, length),
                    Error::cancelled("audio stream was closed"),
                );
                return Err(io::Error::new(failure.io_kind(), failure));
            }
            if let Some(failure) = self.shared.failure_at(offset) {
                return Err(io::Error::new(failure.io_kind(), failure));
            }

            let (new_download_status, wait_result) = self
                .shared
                .cond
                .wait_timeout(download_status, download_timeout)
                .expect(DOWNLOAD_STATUS_POISON_MSG);

            download_status = new_download_status;
            if wait_result.timed_out() {
                if let Some(failure) = self
                    .shared
                    .failure_at(offset)
                    .or_else(|| self.shared.latest_failure_at(offset))
                {
                    return Err(io::Error::new(failure.io_kind(), failure));
                }
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    Error::deadline_exceeded(AudioFileError::WaitTimeout),
                ));
            }
        }
        let available_length = download_status
            .downloaded
            .contained_length_from_value(offset);

        drop(download_status);

        self.position = self.read_file.seek(SeekFrom::Start(offset as u64))?;
        let read_len = min(length, available_length);
        let read_len = self.read_file.read(&mut output[..read_len])?;

        self.position += read_len as u64;
        self.shared.set_read_position(self.position);

        Ok(read_len)
    }
}

impl Drop for AudioFileStreaming {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
        let _ = self
            .stream_loader_command_tx
            .send(StreamLoaderCommand::Close);
        self.shared.cond.notify_all();
    }
}

impl Seek for AudioFileStreaming {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        // If we are already at this position, we don't need to switch download mode.
        // These checks and locks are less expensive than interrupting streaming.
        let current_position = self.position as i64;
        let requested_pos = match pos {
            SeekFrom::Start(pos) => pos as i64,
            SeekFrom::End(pos) => self.shared.file_size as i64 - pos - 1,
            SeekFrom::Current(pos) => current_position + pos,
        };
        if requested_pos == current_position {
            return Ok(current_position as u64);
        }

        // Again if we have already downloaded this part.
        let available = self
            .shared
            .download_status
            .lock()
            .expect(DOWNLOAD_STATUS_POISON_MSG)
            .downloaded
            .contains(requested_pos as usize);

        let mut was_streaming = false;
        if !available {
            // Ensure random access mode if we need to download this part.
            // Checking whether we are streaming now is a micro-optimization
            // to save an atomic load.
            was_streaming = self.shared.is_download_streaming();
            if was_streaming {
                self.shared.set_download_streaming(false);
            }
        }

        self.position = self.read_file.seek(pos)?;
        self.shared.set_read_position(self.position);

        if !available && was_streaming {
            self.shared.set_download_streaming(true);
        }

        Ok(self.position)
    }
}

impl Read for AudioFile {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        match *self {
            AudioFile::Cached(ref mut file) => file.read(output),
            AudioFile::Streaming(ref mut file) => file.read(output),
        }
    }
}

impl Seek for AudioFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match *self {
            AudioFile::Cached(ref mut file) => file.seek(pos),
            AudioFile::Streaming(ref mut file) => file.seek(pos),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_ahead_range_does_not_apply_to_buffered_position() {
        let failed_ahead = Range::new(512 * 1024, 64 * 1024);

        assert!(!range_contains_position(failed_ahead, 128 * 1024));
        assert!(range_contains_position(failed_ahead, 512 * 1024));
    }

    #[test]
    fn default_buffer_policy_has_time_and_byte_floors() {
        let params = AudioFetchParams::default();

        assert!(params.read_ahead_before_playback >= Duration::from_secs(5));
        assert!(params.read_ahead_during_playback >= Duration::from_secs(5));
        assert!(params.minimum_read_ahead_bytes >= 256 * 1024);
    }

    #[test]
    fn cdn_status_alone_never_permanently_poisons_media() {
        for status in [401, 403, 408, 416, 429, 500, 503] {
            assert_eq!(
                classify_http_status(status),
                AudioFileErrorKind::TransientService
            );
        }
    }
}
