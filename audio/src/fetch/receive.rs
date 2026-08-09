use std::{
    cmp::{max, min},
    io::{Seek, SeekFrom, Write},
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use futures_util::{Future, StreamExt, stream::FuturesUnordered};
use http_body_util::BodyExt;
use hyper::StatusCode;
use tempfile::NamedTempFile;
use tokio::sync::{mpsc, oneshot};

use librespot_core::{
    Error,
    cdn_url::CdnUrl,
    http_client::{HttpClient, HttpClientError},
    session::Session,
};

use crate::range_set::{Range, RangeSet};

use super::{
    AudioFetchParams, AudioFileError, AudioFileErrorKind, AudioFileFailure, AudioFileResult,
    AudioFileShared, StreamLoaderCommand, StreamingRequest,
};

struct PartialFileData {
    offset: usize,
    data: Bytes,
}

enum ReceivedData {
    Throughput(usize),
    ResponseTime(Duration),
    Data(PartialFileData),
}

struct RequestFailure {
    source: Error,
    retry_after: Option<Duration>,
    refresh_urls: bool,
}

enum RangeRequestOutcome {
    Complete,
    Failed {
        range: Range,
        attempt: usize,
        failure: RequestFailure,
    },
    Retry {
        range: Range,
        attempt: usize,
        refresh_urls: bool,
    },
}

type RangeRequestFuture = Pin<Box<dyn Future<Output = RangeRequestOutcome> + Send>>;

const ONE_SECOND: Duration = Duration::from_secs(1);
const DOWNLOAD_STATUS_POISON_MSG: &str = "audio download status mutex should not be poisoned";

async fn receive_data(
    shared: Arc<AudioFileShared>,
    file_data_tx: mpsc::Sender<ReceivedData>,
    mut request: StreamingRequest,
) -> RangeRequestOutcome {
    let mut offset = request.offset;
    let mut actual_length = 0;

    let permit = match shared.download_slots.acquire().await {
        Ok(permit) => permit,
        Err(error) => {
            return RangeRequestOutcome::Failed {
                range: Range::new(request.offset, request.length),
                attempt: request.attempt,
                failure: RequestFailure {
                    source: error.into(),
                    retry_after: None,
                    refresh_urls: false,
                },
            };
        }
    };

    let request_time = Instant::now();
    let initial_response = request.initial_response.take();
    let measure_network = initial_response.is_none();
    let response_result = match initial_response {
        Some(response) => Ok(response),
        None => match tokio::time::timeout(
            AudioFetchParams::get().download_timeout,
            request.streamer.next(),
        )
        .await
        {
            Ok(Some(Ok(response))) => Ok(response),
            Ok(Some(Err(error))) => Err(error.into()),
            Ok(None) => Err(AudioFileError::NoData.into()),
            Err(_) => Err(AudioFileError::WaitTimeout.into()),
        },
    };

    let mut response = match response_result {
        Ok(response) => response,
        Err(source) => {
            return RangeRequestOutcome::Failed {
                range: Range::new(request.offset, request.length),
                attempt: request.attempt,
                failure: RequestFailure {
                    source,
                    retry_after: None,
                    refresh_urls: false,
                },
            };
        }
    };

    if measure_network {
        let duration = Instant::now().duration_since(request_time);
        if duration.as_millis() > 0 {
            let _ = file_data_tx
                .send(ReceivedData::ResponseTime(duration))
                .await;
        }
    }

    let code = response.status();
    if code != StatusCode::PARTIAL_CONTENT {
        let retry_after = (code == StatusCode::TOO_MANY_REQUESTS)
            .then(|| HttpClient::get_retry_after(response.headers()))
            .flatten();
        return RangeRequestOutcome::Failed {
            range: Range::new(request.offset, request.length),
            attempt: request.attempt,
            failure: RequestFailure {
                source: HttpClientError::StatusCode(code).into(),
                retry_after,
                refresh_urls: matches!(
                    code,
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::GONE
                ),
            },
        };
    }

    while actual_length < request.length {
        if shared.closed.load(std::sync::atomic::Ordering::Acquire) {
            return RangeRequestOutcome::Complete;
        }

        let frame = match tokio::time::timeout(
            AudioFetchParams::get().download_timeout,
            response.body_mut().frame(),
        )
        .await
        {
            Ok(Some(Ok(frame))) => frame,
            Ok(Some(Err(error))) => {
                return RangeRequestOutcome::Failed {
                    range: Range::new(offset, request.length - actual_length),
                    attempt: request.attempt,
                    failure: RequestFailure {
                        source: error.into(),
                        retry_after: None,
                        refresh_urls: false,
                    },
                };
            }
            Ok(None) => break,
            Err(_) => {
                return RangeRequestOutcome::Failed {
                    range: Range::new(offset, request.length - actual_length),
                    attempt: request.attempt,
                    failure: RequestFailure {
                        source: AudioFileError::WaitTimeout.into(),
                        retry_after: None,
                        refresh_urls: false,
                    },
                };
            }
        };

        let Ok(mut data) = frame.into_data() else {
            continue;
        };
        let remaining = request.length - actual_length;
        if data.len() > remaining {
            data.truncate(remaining);
        }
        let data_size = data.len();
        if data_size == 0 {
            continue;
        }
        if file_data_tx
            .send(ReceivedData::Data(PartialFileData { offset, data }))
            .await
            .is_err()
        {
            return RangeRequestOutcome::Complete;
        }
        actual_length += data_size;
        offset += data_size;
    }

    if measure_network {
        let duration = Instant::now().duration_since(request_time).as_millis();
        if actual_length > 0 && duration > 0 {
            let throughput = ONE_SECOND.as_millis() as usize * actual_length / duration as usize;
            let _ = file_data_tx
                .send(ReceivedData::Throughput(throughput))
                .await;
        }
    }

    drop(permit);

    if actual_length != request.length {
        let missing = Range::new(offset, request.length - actual_length);
        return RangeRequestOutcome::Failed {
            range: missing,
            attempt: request.attempt,
            failure: RequestFailure {
                source: Error::data_loss(format!(
                    "incomplete CDN body: received {actual_length} of {} bytes",
                    request.length
                )),
                retry_after: None,
                refresh_urls: false,
            },
        };
    }

    RangeRequestOutcome::Complete
}

struct AudioFileFetch {
    session: Session,
    cdn_url: CdnUrl,
    shared: Arc<AudioFileShared>,
    output: Option<NamedTempFile>,

    file_data_tx: mpsc::Sender<ReceivedData>,
    complete_tx: Option<oneshot::Sender<NamedTempFile>>,
    network_response_times: Vec<Duration>,
    requests: FuturesUnordered<RangeRequestFuture>,
    next_cdn_candidate: usize,

    params: AudioFetchParams,
}

const MAX_RANGE_RETRIES: usize = 5;
const MAX_RANGE_RETRY_DELAY: Duration = Duration::from_secs(8);
const MAX_RANGE_RETRY_AFTER: Duration = Duration::from_secs(60);

fn range_retry_delay(range: Range, next_attempt: usize, retry_after: Option<Duration>) -> Duration {
    let exponent = next_attempt.saturating_sub(1).min(5) as u32;
    let base_ms = 250_u64.saturating_mul(1_u64 << exponent);
    let jitter_ms =
        ((range.start as u64).wrapping_mul(31) ^ (next_attempt as u64).wrapping_mul(97)) % 200;
    let backoff = Duration::from_millis(base_ms + jitter_ms).min(MAX_RANGE_RETRY_DELAY);
    retry_after.map_or(backoff, |retry_after| {
        retry_after.min(MAX_RANGE_RETRY_AFTER).max(backoff)
    })
}

fn cdn_candidate_index(next_candidate: usize, candidate_count: usize) -> usize {
    next_candidate % candidate_count.max(1)
}

fn range_retry_allowed(attempt: usize, kind: AudioFileErrorKind, closed: bool) -> bool {
    attempt < MAX_RANGE_RETRIES && kind != AudioFileErrorKind::SessionInvalid && !closed
}

// Might be replaced by enum from std once stable
#[derive(PartialEq, Eq)]
enum ControlFlow {
    Break,
    Continue,
}

impl AudioFileFetch {
    fn has_download_slots_available(&self) -> bool {
        self.shared.download_slots.available_permits() > 0
    }

    async fn download_range(
        &mut self,
        offset: usize,
        mut length: usize,
        attempt: usize,
        refresh_urls: bool,
    ) -> AudioFileResult {
        if offset >= self.shared.file_size
            || self
                .shared
                .closed
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Ok(());
        }

        if refresh_urls {
            debug!("Refreshing CDN candidates before retrying range at offset {offset}");
            self.cdn_url = CdnUrl::new(self.cdn_url.file_id)
                .resolve_audio(&self.session)
                .await?;
            self.next_cdn_candidate = 0;
        }

        if length < self.params.minimum_download_size {
            length = self.params.minimum_download_size;
        }

        // If we are in streaming mode (so not seeking) then start downloading as large
        // of chunks as possible for better throughput and improved CPU usage, while
        // still being reasonably responsive (~1 second) in case we want to seek.
        if self.shared.is_download_streaming() {
            let throughput = self.shared.throughput();
            length = max(length, throughput);
        }

        if offset + length > self.shared.file_size {
            length = self.shared.file_size - offset;
        }
        let mut ranges_to_request = RangeSet::new();
        ranges_to_request.add_range(&Range::new(offset, length));

        // The iteration that follows spawns streamers fast, without awaiting them,
        // so holding the lock for the entire scope of this function should be faster
        // then locking and unlocking multiple times.
        let mut download_status = self
            .shared
            .download_status
            .lock()
            .expect(DOWNLOAD_STATUS_POISON_MSG);

        ranges_to_request.subtract_range_set(&download_status.downloaded);
        ranges_to_request.subtract_range_set(&download_status.requested);

        for range in ranges_to_request.iter() {
            let candidates = self.cdn_url.try_get_urls()?;
            let candidate_count = candidates.len();
            let candidate_index = cdn_candidate_index(self.next_cdn_candidate, candidate_count);
            self.next_cdn_candidate = self.next_cdn_candidate.wrapping_add(1);
            let streamer = self.session.spclient().stream_from_cdn(
                candidates[candidate_index],
                range.start,
                range.length,
            );

            download_status.requested.add_range(range);

            debug!(
                "Fetching compressed range {} using CDN candidate {}/{} (attempt {})",
                range,
                candidate_index + 1,
                candidate_count,
                attempt + 1
            );

            match streamer {
                Ok(streamer) => {
                    let streaming_request = StreamingRequest {
                        streamer,
                        initial_response: None,
                        offset: range.start,
                        length: range.length,
                        attempt,
                    };

                    self.requests.push(Box::pin(receive_data(
                        self.shared.clone(),
                        self.file_data_tx.clone(),
                        streaming_request,
                    )));
                }
                Err(source) => {
                    let range = *range;
                    self.requests.push(Box::pin(async move {
                        RangeRequestOutcome::Failed {
                            range,
                            attempt,
                            failure: RequestFailure {
                                source,
                                retry_after: None,
                                refresh_urls: false,
                            },
                        }
                    }));
                }
            }
        }

        Ok(())
    }

    async fn pre_fetch_more_data(&mut self, bytes: usize) -> AudioFileResult {
        // determine what is still missing
        let mut missing_data = RangeSet::new();
        missing_data.add_range(&Range::new(0, self.shared.file_size));
        {
            let download_status = self
                .shared
                .download_status
                .lock()
                .expect(DOWNLOAD_STATUS_POISON_MSG);
            missing_data.subtract_range_set(&download_status.downloaded);
            missing_data.subtract_range_set(&download_status.requested);
        }

        // download data from after the current read position first
        let mut tail_end = RangeSet::new();
        let read_position = self.shared.read_position();
        tail_end.add_range(&Range::new(
            read_position,
            self.shared.file_size - read_position,
        ));
        let tail_end = tail_end.intersection(&missing_data);

        if !tail_end.is_empty() {
            let range = tail_end.get_range(0);
            let offset = range.start;
            let length = min(range.length, bytes);
            self.download_range(offset, length, 0, false).await?;
        } else if !missing_data.is_empty() {
            // ok, the tail is downloaded, download something fom the beginning.
            let range = missing_data.get_range(0);
            let offset = range.start;
            let length = min(range.length, bytes);
            self.download_range(offset, length, 0, false).await?;
        }

        Ok(())
    }

    fn handle_file_data(&mut self, data: ReceivedData) -> Result<ControlFlow, Error> {
        match data {
            ReceivedData::Throughput(mut throughput) => {
                if throughput < self.params.minimum_throughput {
                    warn!(
                        "Throughput {} kbps lower than minimum {}, setting to minimum",
                        throughput / 1000,
                        self.params.minimum_throughput / 1000,
                    );
                    throughput = self.params.minimum_throughput;
                }

                let old_throughput = self.shared.throughput();
                let avg_throughput = if old_throughput > 0 {
                    (old_throughput + throughput) / 2
                } else {
                    throughput
                };

                // print when the new estimate deviates by more than 10% from the last
                if f32::abs((avg_throughput as f32 - old_throughput as f32) / old_throughput as f32)
                    > 0.1
                {
                    trace!(
                        "Throughput now estimated as: {} kbps",
                        avg_throughput / 1000
                    );
                }

                self.shared.set_throughput(avg_throughput);
            }
            ReceivedData::ResponseTime(mut response_time) => {
                if response_time > self.params.maximum_assumed_ping_time {
                    warn!(
                        "Time to first byte {} ms exceeds maximum {}, setting to maximum",
                        response_time.as_millis(),
                        self.params.maximum_assumed_ping_time.as_millis()
                    );
                    response_time = self.params.maximum_assumed_ping_time;
                }

                let old_ping_time_ms = self.shared.ping_time().as_millis();

                // prune old response times. Keep at most two so we can push a third.
                while self.network_response_times.len() >= 3 {
                    self.network_response_times.remove(0);
                }

                // record the response time
                self.network_response_times.push(response_time);

                // stats::median is experimental. So we calculate the median of up to three ourselves.
                let ping_time = {
                    match self.network_response_times.len() {
                        1 => self.network_response_times[0],
                        2 => (self.network_response_times[0] + self.network_response_times[1]) / 2,
                        3 => {
                            let mut times = self.network_response_times.clone();
                            times.sort_unstable();
                            times[1]
                        }
                        _ => unreachable!(),
                    }
                };

                // print when the new estimate deviates by more than 10% from the last
                if f32::abs(
                    (ping_time.as_millis() as f32 - old_ping_time_ms as f32)
                        / old_ping_time_ms as f32,
                ) > 0.1
                {
                    trace!(
                        "Time to first byte now estimated as: {} ms",
                        ping_time.as_millis()
                    );
                }

                // store our new estimate for everyone to see
                self.shared.set_ping_time(ping_time);
            }
            ReceivedData::Data(data) => {
                match self.output.as_mut() {
                    Some(output) => {
                        output.seek(SeekFrom::Start(data.offset as u64))?;
                        output.write_all(data.data.as_ref())?;
                    }
                    None => return Err(AudioFileError::Output.into()),
                }

                let received_range = Range::new(data.offset, data.data.len());

                self.shared
                    .degraded
                    .store(false, std::sync::atomic::Ordering::Release);
                let full = {
                    let mut download_status = self
                        .shared
                        .download_status
                        .lock()
                        .expect(DOWNLOAD_STATUS_POISON_MSG);
                    download_status.downloaded.add_range(&received_range);
                    self.shared.cond.notify_all();

                    download_status.downloaded.contained_length_from_value(0)
                        >= self.shared.file_size
                };

                for failure in [&self.shared.latest_failure, &self.shared.last_failure] {
                    let mut failure = failure.lock().expect(DOWNLOAD_STATUS_POISON_MSG);
                    if failure.as_ref().is_some_and(|failure| {
                        failure.range.start < received_range.end()
                            && received_range.start < failure.range.end()
                    }) {
                        *failure = None;
                    }
                }

                if full {
                    self.finish()?;
                    return Ok(ControlFlow::Break);
                }
            }
        }

        Ok(ControlFlow::Continue)
    }

    async fn handle_stream_loader_command(
        &mut self,
        cmd: StreamLoaderCommand,
    ) -> Result<ControlFlow, Error> {
        match cmd {
            StreamLoaderCommand::Fetch(request) => {
                self.download_range(request.start, request.length, 0, false)
                    .await?
            }
            StreamLoaderCommand::Close => {
                self.shared
                    .closed
                    .store(true, std::sync::atomic::Ordering::Release);
                self.shared.cond.notify_all();
                return Ok(ControlFlow::Break);
            }
        }

        Ok(ControlFlow::Continue)
    }

    async fn handle_request_outcome(&mut self, outcome: RangeRequestOutcome) {
        match outcome {
            RangeRequestOutcome::Complete => {}
            RangeRequestOutcome::Retry {
                range,
                attempt,
                refresh_urls,
            } => {
                {
                    let mut status = self
                        .shared
                        .download_status
                        .lock()
                        .expect(DOWNLOAD_STATUS_POISON_MSG);
                    status.requested.subtract_range(&range);
                }
                if let Err(source) = self
                    .download_range(range.start, range.length, attempt, refresh_urls)
                    .await
                {
                    self.shared
                        .download_status
                        .lock()
                        .expect(DOWNLOAD_STATUS_POISON_MSG)
                        .requested
                        .add_range(&range);
                    self.requests.push(Box::pin(async move {
                        RangeRequestOutcome::Failed {
                            range,
                            attempt,
                            failure: RequestFailure {
                                source,
                                retry_after: None,
                                refresh_urls: true,
                            },
                        }
                    }));
                }
            }
            RangeRequestOutcome::Failed {
                range,
                attempt,
                failure,
            } => {
                let classified = AudioFileFailure::classify(&self.session, range, failure.source);
                *self
                    .shared
                    .latest_failure
                    .lock()
                    .expect(DOWNLOAD_STATUS_POISON_MSG) = Some(classified.clone());
                self.shared
                    .degraded
                    .store(true, std::sync::atomic::Ordering::Release);

                if range_retry_allowed(
                    attempt,
                    classified.kind,
                    self.shared
                        .closed
                        .load(std::sync::atomic::Ordering::Acquire),
                ) {
                    let next_attempt = attempt + 1;
                    let delay = range_retry_delay(range, next_attempt, failure.retry_after);
                    let candidate_count = self
                        .cdn_url
                        .try_get_urls()
                        .map_or(1, |candidates| candidates.len());
                    let refresh_urls = failure.refresh_urls || next_attempt % candidate_count == 0;
                    debug!(
                        "Compressed range {} failed as {:?}; retry {} in {:?}{}",
                        range,
                        classified.kind,
                        next_attempt,
                        delay,
                        if refresh_urls {
                            " after refreshing CDN candidates"
                        } else {
                            " with CDN failover"
                        }
                    );
                    self.shared.cond.notify_all();
                    self.requests.push(Box::pin(async move {
                        tokio::time::sleep(delay).await;
                        RangeRequestOutcome::Retry {
                            range,
                            attempt: next_attempt,
                            refresh_urls,
                        }
                    }));
                } else {
                    debug!(
                        "Compressed range {} exhausted retries with classification {:?}",
                        range, classified.kind
                    );
                    let mut status = self
                        .shared
                        .download_status
                        .lock()
                        .expect(DOWNLOAD_STATUS_POISON_MSG);
                    status.requested.subtract_range(&range);
                    *self
                        .shared
                        .last_failure
                        .lock()
                        .expect(DOWNLOAD_STATUS_POISON_MSG) = Some(classified);
                    self.shared.cond.notify_all();
                }
            }
        }
    }

    fn finish(&mut self) -> AudioFileResult {
        let output = self.output.take();

        let complete_tx = self.complete_tx.take();

        if let Some(mut output) = output {
            output.rewind()?;
            if let Some(complete_tx) = complete_tx {
                complete_tx
                    .send(output)
                    .map_err(|_| AudioFileError::Channel)?;
            }
        }

        Ok(())
    }
}

pub(super) async fn audio_file_fetch(
    session: Session,
    cdn_url: CdnUrl,
    shared: Arc<AudioFileShared>,
    initial_request: StreamingRequest,
    output: NamedTempFile,
    mut stream_loader_command_rx: mpsc::UnboundedReceiver<StreamLoaderCommand>,
    complete_tx: oneshot::Sender<NamedTempFile>,
) -> AudioFileResult {
    // Keep body-to-disk handoff bounded. This makes each response future yield after a few frames,
    // so received compressed audio becomes readable instead of accumulating a full range in RAM.
    let (file_data_tx, mut file_data_rx) = mpsc::channel(8);

    {
        let requested_range = Range::new(initial_request.offset, initial_request.length);

        let mut download_status = shared
            .download_status
            .lock()
            .expect(DOWNLOAD_STATUS_POISON_MSG);
        download_status.requested.add_range(&requested_range);
    }

    let params = AudioFetchParams::get();

    let requests = FuturesUnordered::new();
    requests.push(Box::pin(receive_data(
        shared.clone(),
        file_data_tx.clone(),
        initial_request,
    )) as RangeRequestFuture);

    let mut fetch = AudioFileFetch {
        session: session.clone(),
        cdn_url,
        shared,
        output: Some(output),

        file_data_tx,
        complete_tx: Some(complete_tx),
        network_response_times: Vec::with_capacity(3),
        requests,
        next_cdn_candidate: 1,

        params: params.clone(),
    };

    loop {
        tokio::select! {
            cmd = stream_loader_command_rx.recv() => {
                match cmd {
                        Some(cmd) => {
                            if fetch.handle_stream_loader_command(cmd).await? == ControlFlow::Break {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            data = file_data_rx.recv() => {
                match data {
                    Some(data) => {
                        if fetch.handle_file_data(data)? == ControlFlow::Break {
                            break;
                        }
                    }
                    None => break,
                }
            },
            outcome = fetch.requests.next(), if !fetch.requests.is_empty() => {
                if let Some(outcome) = outcome {
                    fetch.handle_request_outcome(outcome).await;
                }
            },
            else => (),
        }

        if fetch.shared.is_download_streaming() && fetch.has_download_slots_available() {
            let bytes_pending: usize = {
                let download_status = fetch
                    .shared
                    .download_status
                    .lock()
                    .expect(DOWNLOAD_STATUS_POISON_MSG);

                download_status
                    .requested
                    .minus(&download_status.downloaded)
                    .len()
            };

            let ping_time_seconds = fetch.shared.ping_time().as_secs_f32();
            let throughput = fetch.shared.throughput();

            let desired_pending_bytes = max(
                (params.prefetch_threshold_factor
                    * ping_time_seconds
                    * fetch.shared.bytes_per_second as f32) as usize,
                (ping_time_seconds * throughput as f32) as usize,
            );

            if bytes_pending < desired_pending_bytes {
                fetch
                    .pre_fetch_more_data(desired_pending_bytes - bytes_pending)
                    .await?;
            }
        }
    }

    fetch
        .shared
        .closed
        .store(true, std::sync::atomic::Ordering::Release);
    fetch.shared.cond.notify_all();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_retry_backoff_is_bounded_and_increases() {
        let range = Range::new(64 * 1024, 64 * 1024);
        let first = range_retry_delay(range, 1, None);
        let later = range_retry_delay(range, 5, None);
        let bounded = range_retry_delay(range, 100, None);

        assert!(first < later);
        assert!(bounded <= MAX_RANGE_RETRY_DELAY);
    }

    #[test]
    fn retry_after_is_respected_by_range_retry() {
        let range = Range::new(0, 64 * 1024);
        let retry_after = Duration::from_secs(3);

        assert!(range_retry_delay(range, 1, Some(retry_after)) >= retry_after);
    }

    #[test]
    fn pathological_retry_after_is_bounded() {
        let range = Range::new(0, 64 * 1024);

        assert_eq!(
            range_retry_delay(range, 1, Some(Duration::from_secs(24 * 60 * 60))),
            MAX_RANGE_RETRY_AFTER
        );
    }

    #[test]
    fn retryable_host_failure_rotates_to_alternate_candidate() {
        let candidates = 3;
        let sequence: Vec<_> = (0..6)
            .map(|next| cdn_candidate_index(next, candidates))
            .collect();

        assert_eq!(sequence, vec![0, 1, 2, 0, 1, 2]);
    }

    #[test]
    fn retry_budget_is_finite() {
        assert!(range_retry_allowed(
            MAX_RANGE_RETRIES - 1,
            AudioFileErrorKind::TransientNetwork,
            false
        ));
        assert!(!range_retry_allowed(
            MAX_RANGE_RETRIES,
            AudioFileErrorKind::TransientNetwork,
            false
        ));
    }

    #[test]
    fn invalid_session_is_not_retried_by_range_layer() {
        assert!(!range_retry_allowed(
            0,
            AudioFileErrorKind::SessionInvalid,
            false
        ));
    }
}
