use std::{
    io, mem,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    },
    thread::{self, JoinHandle},
};

use librespot_audio::StreamLoaderController;

use crate::{
    NUM_CHANNELS, SAMPLE_RATE, SpeedAutomation,
    decoder::{AudioDecoder, AudioPacket, AudioPacketPosition, DecoderError, DecoderResult},
    time_stretch::PitchPreservingTimeStretch,
};

/// Fixed-size chunks make the channel's PCM memory bound independent of decoder packet sizes.
/// 1024 frames is the small playback quantum already used by real-time backends in this crate.
pub(crate) const SECONDARY_PCM_CHUNK_FRAMES: usize = 1024;
/// Eight chunks hold 8192 frames: about 186 ms, or 128 KiB of stereo `f64` PCM at 44.1 kHz.
pub(crate) const SECONDARY_PCM_CHANNEL_CAPACITY: usize = 8;

const SECONDARY_PCM_CHUNK_SAMPLES: usize = SECONDARY_PCM_CHUNK_FRAMES * NUM_CHANNELS as usize;
pub(crate) const SECONDARY_PCM_ASSEMBLER_SAMPLES: usize = SECONDARY_PCM_CHUNK_SAMPLES * 2;

pub(crate) type Decoder = Box<dyn AudioDecoder + Send>;

/// Decoder execution owned by a `PlaybackSource`.
///
/// The worker variant has exclusive ownership of the decoder while the source remains movable
/// between preload and current roles.
pub(crate) enum SourceDecoder {
    Direct(Decoder),
    Worker(SecondaryDecodeWorker),
    Invalid,
}

impl SourceDecoder {
    pub(crate) fn direct(decoder: Decoder) -> Self {
        Self::Direct(decoder)
    }

    pub(crate) fn seek(&mut self, position_ms: u32) -> Result<u32, DecoderError> {
        match self {
            Self::Direct(decoder) => decoder.seek(position_ms),
            Self::Worker(_) => Err(DecoderError::Io(io::Error::new(
                io::ErrorKind::Unsupported,
                "seeking a worker-backed decoder is not implemented",
            ))),
            Self::Invalid => Err(Self::invalid_state_error()),
        }
    }

    pub(crate) fn next_packet(
        &mut self,
    ) -> DecoderResult<Option<(AudioPacketPosition, AudioPacket)>> {
        match self {
            Self::Direct(decoder) => decoder.next_packet(),
            Self::Worker(worker) => worker.recv_packet(),
            Self::Invalid => Err(Self::invalid_state_error()),
        }
    }

    pub(crate) fn start_secondary(
        &mut self,
        stream_loader_controller: StreamLoaderController,
        generation: u64,
        track_label: String,
        speed_automation: Option<SpeedAutomation>,
        source_position_ms: u32,
    ) -> Result<bool, DecoderError> {
        match self {
            Self::Worker(_) => return Ok(false),
            Self::Invalid => return Err(Self::invalid_state_error()),
            Self::Direct(_) => {}
        }

        let Self::Direct(decoder) = mem::replace(self, Self::Invalid) else {
            unreachable!("direct decoder changed while starting secondary worker");
        };
        *self = Self::Worker(SecondaryDecodeWorker::spawn(
            decoder,
            stream_loader_controller,
            generation,
            track_label,
            speed_automation,
            source_position_ms,
        )?);
        Ok(true)
    }

    #[cfg(test)]
    pub(crate) fn start_secondary_with_cancellation(
        &mut self,
        stream_loader_controller: StreamLoaderController,
        generation: u64,
        track_label: String,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(), DecoderError> {
        let Self::Direct(decoder) = mem::replace(self, Self::Invalid) else {
            return Err(Self::invalid_state_error());
        };
        *self = Self::Worker(SecondaryDecodeWorker::spawn_with_cancellation(
            decoder,
            stream_loader_controller,
            generation,
            track_label,
            cancelled,
            None,
            0,
        )?);
        Ok(())
    }

    pub(crate) fn try_read_secondary(&mut self, samples: usize) -> SecondaryRead {
        match self {
            Self::Worker(worker) => worker.try_read(samples),
            Self::Direct(_) | Self::Invalid => SecondaryRead::Pending,
        }
    }

    pub(crate) fn secondary_pcm_readiness(&mut self, samples: usize) -> SecondaryPcmReadiness {
        if samples > SECONDARY_PCM_ASSEMBLER_SAMPLES {
            return SecondaryPcmReadiness::Pending;
        }

        match self {
            Self::Worker(worker) => worker.has_pcm(samples),
            Self::Direct(_) => SecondaryPcmReadiness::Pending,
            Self::Invalid => SecondaryPcmReadiness::Unavailable,
        }
    }

    pub(crate) fn is_worker(&self) -> bool {
        matches!(self, Self::Worker(_))
    }

    #[cfg(test)]
    pub(crate) fn direct_decoder_address(&self) -> Option<*const ()> {
        match self {
            Self::Direct(decoder) => Some((&**decoder) as *const dyn AudioDecoder as *const ()),
            Self::Worker(_) | Self::Invalid => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn worker_finished(&self) -> Option<Arc<AtomicBool>> {
        match self {
            Self::Worker(worker) => Some(worker.finished.clone()),
            Self::Direct(_) | Self::Invalid => None,
        }
    }

    fn invalid_state_error() -> DecoderError {
        DecoderError::Io(io::Error::other("source decoder is in an invalid state"))
    }
}

pub(crate) struct SecondaryDecodeMessage {
    pub generation: u64,
    pub event: SecondaryDecodeEvent,
}

pub(crate) enum SecondaryDecodeEvent {
    Packet(AudioPacketPosition, AudioPacket),
    Eof,
    Failed(DecoderError),
}

pub(crate) struct SecondaryPcmBlock {
    pub generation: u64,
    pub position: AudioPacketPosition,
    pub packet: AudioPacket,
    pub source_end_position_ms: Option<u32>,
}

pub(crate) enum SecondaryRead {
    Pcm(SecondaryPcmBlock),
    Pending,
    Eof {
        generation: u64,
    },
    Failed {
        generation: u64,
        error: DecoderError,
    },
    Disconnected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SecondaryPcmReadiness {
    Pending,
    Ready,
    Unavailable,
}

pub(crate) struct SecondaryDecodeWorker {
    receiver: Option<Receiver<SecondaryDecodeMessage>>,
    cancelled: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    stream_loader_controller: StreamLoaderController,
    thread: Option<JoinHandle<()>>,
    track_label: String,
    pcm_buffer: Vec<f64>,
    pcm_origin: Option<AudioPacketPosition>,
    pcm_consumed_frames: u64,
    pcm_generation: Option<u64>,
    pending_terminal: Option<SecondaryDecodeMessage>,
    time_stretch: Option<PitchPreservingTimeStretch>,
    time_stretch_flushed: bool,
}

impl SecondaryDecodeWorker {
    fn spawn(
        decoder: Decoder,
        stream_loader_controller: StreamLoaderController,
        generation: u64,
        track_label: String,
        speed_automation: Option<SpeedAutomation>,
        source_position_ms: u32,
    ) -> Result<Self, DecoderError> {
        let cancelled = Arc::new(AtomicBool::new(false));
        Self::spawn_with_cancellation(
            decoder,
            stream_loader_controller,
            generation,
            track_label,
            cancelled,
            speed_automation,
            source_position_ms,
        )
    }

    fn spawn_with_cancellation(
        decoder: Decoder,
        stream_loader_controller: StreamLoaderController,
        generation: u64,
        track_label: String,
        cancelled: Arc<AtomicBool>,
        speed_automation: Option<SpeedAutomation>,
        source_position_ms: u32,
    ) -> Result<Self, DecoderError> {
        let (sender, receiver) = sync_channel(SECONDARY_PCM_CHANNEL_CAPACITY);
        let finished = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let worker_finished = finished.clone();
        let worker_track_label = track_label.clone();

        let thread = thread::Builder::new()
            .name("librespot-secondary-decode".into())
            .spawn(move || {
                run_decode_worker(
                    decoder,
                    sender,
                    worker_cancelled,
                    worker_finished,
                    generation,
                    &worker_track_label,
                );
            })
            .map_err(DecoderError::Io)?;

        let time_stretch = speed_automation.and_then(|automation| {
            PitchPreservingTimeStretch::new(
                automation,
                std::time::Duration::from_millis(u64::from(source_position_ms)),
            )
        });
        if time_stretch.is_some() {
            debug!("[transition] incoming speed automation active track=<{track_label}>");
        }

        Ok(Self {
            receiver: Some(receiver),
            cancelled,
            finished,
            stream_loader_controller,
            thread: Some(thread),
            track_label,
            pcm_buffer: Vec::with_capacity(SECONDARY_PCM_ASSEMBLER_SAMPLES),
            pcm_origin: None,
            pcm_consumed_frames: 0,
            pcm_generation: None,
            pending_terminal: None,
            time_stretch,
            time_stretch_flushed: false,
        })
    }

    fn try_read(&mut self, samples: usize) -> SecondaryRead {
        Self::assert_read_request(samples);

        if self.time_stretch.is_some() {
            return self.try_read_stretched(samples);
        }

        while self.pcm_buffer.len() < samples && self.pending_terminal.is_none() {
            let message = match self
                .receiver
                .as_ref()
                .ok_or(TryRecvError::Disconnected)
                .and_then(Receiver::try_recv)
            {
                Ok(message) => message,
                Err(TryRecvError::Empty) => return SecondaryRead::Pending,
                Err(TryRecvError::Disconnected) => return SecondaryRead::Disconnected,
            };

            match message.event {
                SecondaryDecodeEvent::Packet(position, AudioPacket::Samples(packet)) => {
                    self.push_pcm(message.generation, position, packet);
                }
                SecondaryDecodeEvent::Packet(_, AudioPacket::Raw(_)) => {
                    self.pending_terminal = Some(SecondaryDecodeMessage {
                        generation: message.generation,
                        event: SecondaryDecodeEvent::Failed(DecoderError::PassthroughDecoder(
                            "secondary PCM assembler received encoded audio".into(),
                        )),
                    });
                }
                event @ (SecondaryDecodeEvent::Eof | SecondaryDecodeEvent::Failed(_)) => {
                    self.pending_terminal = Some(SecondaryDecodeMessage {
                        generation: message.generation,
                        event,
                    });
                }
            }
        }

        if self.pcm_buffer.len() >= samples {
            return SecondaryRead::Pcm(self.take_pcm(samples));
        }

        let message = self
            .pending_terminal
            .take()
            .expect("incomplete PCM without a terminal event must have returned pending");
        match message.event {
            SecondaryDecodeEvent::Eof => SecondaryRead::Eof {
                generation: message.generation,
            },
            SecondaryDecodeEvent::Failed(error) => SecondaryRead::Failed {
                generation: message.generation,
                error,
            },
            SecondaryDecodeEvent::Packet(_, _) => {
                unreachable!("only terminal events are stored")
            }
        }
    }

    fn has_pcm(&mut self, samples: usize) -> SecondaryPcmReadiness {
        Self::assert_readiness_request(samples);

        if self.time_stretch.is_some() {
            return self.has_stretched_pcm(samples);
        }

        while self.pcm_buffer.len() < samples && self.pending_terminal.is_none() {
            let message = match self
                .receiver
                .as_ref()
                .ok_or(TryRecvError::Disconnected)
                .and_then(Receiver::try_recv)
            {
                Ok(message) => message,
                Err(TryRecvError::Empty) => return SecondaryPcmReadiness::Pending,
                Err(TryRecvError::Disconnected) => return SecondaryPcmReadiness::Unavailable,
            };

            match message.event {
                SecondaryDecodeEvent::Packet(position, AudioPacket::Samples(packet)) => {
                    self.push_pcm(message.generation, position, packet);
                }
                SecondaryDecodeEvent::Packet(_, AudioPacket::Raw(_)) => {
                    self.pending_terminal = Some(SecondaryDecodeMessage {
                        generation: message.generation,
                        event: SecondaryDecodeEvent::Failed(DecoderError::PassthroughDecoder(
                            "secondary PCM assembler received encoded audio".into(),
                        )),
                    });
                }
                event @ (SecondaryDecodeEvent::Eof | SecondaryDecodeEvent::Failed(_)) => {
                    self.pending_terminal = Some(SecondaryDecodeMessage {
                        generation: message.generation,
                        event,
                    });
                }
            }
        }

        if self.pcm_buffer.len() >= samples {
            SecondaryPcmReadiness::Ready
        } else {
            SecondaryPcmReadiness::Unavailable
        }
    }

    fn assert_read_request(samples: usize) {
        Self::assert_sample_alignment(samples);
        assert!(
            samples <= SECONDARY_PCM_CHUNK_SAMPLES,
            "secondary PCM read exceeds the bounded assembler quantum"
        );
    }

    fn assert_readiness_request(samples: usize) {
        Self::assert_sample_alignment(samples);
        assert!(
            samples <= SECONDARY_PCM_ASSEMBLER_SAMPLES,
            "secondary PCM readiness exceeds the bounded assembler capacity"
        );
    }

    fn assert_sample_alignment(samples: usize) {
        assert!(samples > 0, "secondary PCM read must request samples");
        assert_eq!(
            samples % NUM_CHANNELS as usize,
            0,
            "secondary PCM read must be frame-aligned"
        );
    }

    fn take_pcm(&mut self, samples: usize) -> SecondaryPcmBlock {
        let channels = NUM_CHANNELS as usize;
        let frames = samples / channels;
        let origin = self
            .pcm_origin
            .as_ref()
            .expect("buffered PCM must retain its origin");
        let position_offset_ms = self.pcm_consumed_frames * 1000 / u64::from(SAMPLE_RATE);
        let position = AudioPacketPosition {
            position_ms: origin.position_ms.saturating_add(position_offset_ms as u32),
            skipped: origin.skipped && self.pcm_consumed_frames == 0,
        };
        self.pcm_consumed_frames += frames as u64;
        let packet = AudioPacket::Samples(self.pcm_buffer.drain(..samples).collect());
        let generation = self
            .pcm_generation
            .expect("buffered PCM must retain its generation");

        SecondaryPcmBlock {
            generation,
            position,
            packet,
            source_end_position_ms: None,
        }
    }

    fn try_read_stretched(&mut self, samples: usize) -> SecondaryRead {
        loop {
            if let Some(block) = self.take_stretched_if_available(samples) {
                return SecondaryRead::Pcm(block);
            }
            if self.pending_terminal.is_some() {
                self.flush_time_stretch();
                if let Some(block) = self.take_stretched_if_available(samples) {
                    return SecondaryRead::Pcm(block);
                }
                return self.take_pending_terminal();
            }

            let message = match self
                .receiver
                .as_ref()
                .ok_or(TryRecvError::Disconnected)
                .and_then(Receiver::try_recv)
            {
                Ok(message) => message,
                Err(TryRecvError::Empty) => return SecondaryRead::Pending,
                Err(TryRecvError::Disconnected) => return SecondaryRead::Disconnected,
            };
            self.push_stretched_message(message);
        }
    }

    fn has_stretched_pcm(&mut self, samples: usize) -> SecondaryPcmReadiness {
        loop {
            {
                let stretch = self
                    .time_stretch
                    .as_mut()
                    .expect("stretched readiness requires a processor");
                stretch.fill_output(samples);
                if stretch.available_samples() >= samples && self.pcm_generation.is_some() {
                    return SecondaryPcmReadiness::Ready;
                }
            }

            if self.pending_terminal.is_some() {
                self.flush_time_stretch();
                let stretch = self
                    .time_stretch
                    .as_mut()
                    .expect("stretched readiness requires a processor");
                stretch.fill_output(samples);
                return if stretch.available_samples() >= samples && self.pcm_generation.is_some() {
                    SecondaryPcmReadiness::Ready
                } else {
                    SecondaryPcmReadiness::Unavailable
                };
            }

            let message = match self
                .receiver
                .as_ref()
                .ok_or(TryRecvError::Disconnected)
                .and_then(Receiver::try_recv)
            {
                Ok(message) => message,
                Err(TryRecvError::Empty) => return SecondaryPcmReadiness::Pending,
                Err(TryRecvError::Disconnected) => return SecondaryPcmReadiness::Unavailable,
            };
            self.push_stretched_message(message);
        }
    }

    fn take_stretched_if_available(&mut self, samples: usize) -> Option<SecondaryPcmBlock> {
        let stretch = self.time_stretch.as_mut()?;
        stretch.fill_output(samples);
        if stretch.available_samples() < samples {
            return None;
        }

        let position_ms = duration_ms_u32(stretch.source_position());
        let packet = AudioPacket::Samples(stretch.take(samples));
        let source_end_position_ms = Some(duration_ms_u32(stretch.source_position()));
        let generation = self.pcm_generation?;
        let skipped = self
            .pcm_origin
            .as_ref()
            .is_some_and(|origin| origin.skipped && self.pcm_consumed_frames == 0);
        self.pcm_consumed_frames += (samples / NUM_CHANNELS as usize) as u64;
        Some(SecondaryPcmBlock {
            generation,
            position: AudioPacketPosition {
                position_ms,
                skipped,
            },
            packet,
            source_end_position_ms,
        })
    }

    fn push_stretched_message(&mut self, message: SecondaryDecodeMessage) {
        match message.event {
            SecondaryDecodeEvent::Packet(position, AudioPacket::Samples(samples)) => {
                if self.pcm_origin.is_none() {
                    self.pcm_origin = Some(position);
                    self.pcm_generation = Some(message.generation);
                    self.pcm_consumed_frames = 0;
                }
                if self.pcm_generation == Some(message.generation) {
                    self.time_stretch
                        .as_mut()
                        .expect("stretched message requires a processor")
                        .push(&samples);
                }
            }
            SecondaryDecodeEvent::Packet(_, AudioPacket::Raw(_)) => {
                self.pending_terminal = Some(SecondaryDecodeMessage {
                    generation: message.generation,
                    event: SecondaryDecodeEvent::Failed(DecoderError::PassthroughDecoder(
                        "time stretching received an encoded packet".into(),
                    )),
                });
            }
            event @ (SecondaryDecodeEvent::Eof | SecondaryDecodeEvent::Failed(_)) => {
                self.pending_terminal = Some(SecondaryDecodeMessage {
                    generation: message.generation,
                    event,
                });
            }
        }
    }

    fn flush_time_stretch(&mut self) {
        if !self.time_stretch_flushed {
            self.time_stretch
                .as_mut()
                .expect("flush requires a time stretcher")
                .flush();
            self.time_stretch_flushed = true;
        }
    }

    fn take_pending_terminal(&mut self) -> SecondaryRead {
        let message = self
            .pending_terminal
            .take()
            .expect("terminal read requires a pending event");
        match message.event {
            SecondaryDecodeEvent::Eof => SecondaryRead::Eof {
                generation: message.generation,
            },
            SecondaryDecodeEvent::Failed(error) => SecondaryRead::Failed {
                generation: message.generation,
                error,
            },
            SecondaryDecodeEvent::Packet(_, _) => unreachable!("only terminal events are stored"),
        }
    }

    fn recv_packet(&mut self) -> DecoderResult<Option<(AudioPacketPosition, AudioPacket)>> {
        if self.time_stretch.is_some() {
            return self.recv_stretched_packet();
        }
        if !self.pcm_buffer.is_empty() {
            let block = self.take_pcm(self.pcm_buffer.len());
            return Ok(Some((block.position, block.packet)));
        }

        if let Some(message) = self.pending_terminal.take() {
            return Self::message_into_packet(message);
        }

        let message = self
            .receiver
            .as_ref()
            .ok_or_else(|| {
                DecoderError::Io(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "decoder worker cancelled",
                ))
            })?
            .recv()
            .map_err(|error| DecoderError::Io(io::Error::new(io::ErrorKind::BrokenPipe, error)))?;

        match message.event {
            SecondaryDecodeEvent::Packet(position, AudioPacket::Samples(packet)) => {
                self.push_pcm(message.generation, position, packet);
                let block = self.take_pcm(self.pcm_buffer.len());
                Ok(Some((block.position, block.packet)))
            }
            SecondaryDecodeEvent::Packet(position, packet @ AudioPacket::Raw(_)) => {
                Ok(Some((position, packet)))
            }
            SecondaryDecodeEvent::Eof => Ok(None),
            SecondaryDecodeEvent::Failed(error) => Err(error),
        }
    }

    fn recv_stretched_packet(
        &mut self,
    ) -> DecoderResult<Option<(AudioPacketPosition, AudioPacket)>> {
        loop {
            if let Some(block) = self.take_stretched_if_available(SECONDARY_PCM_CHUNK_SAMPLES) {
                return Ok(Some((block.position, block.packet)));
            }
            if self.pending_terminal.is_some() {
                self.flush_time_stretch();
                let available = self
                    .time_stretch
                    .as_ref()
                    .expect("stretched receive requires a processor")
                    .available_samples();
                if available > 0 {
                    let channels = NUM_CHANNELS as usize;
                    let samples = available.min(SECONDARY_PCM_CHUNK_SAMPLES) / channels * channels;
                    if let Some(block) = self.take_stretched_if_available(samples) {
                        return Ok(Some((block.position, block.packet)));
                    }
                }
                let message = self
                    .pending_terminal
                    .take()
                    .expect("terminal event remains pending after stretch flush");
                return Self::message_into_packet(message);
            }

            let message = self
                .receiver
                .as_ref()
                .ok_or_else(|| {
                    DecoderError::Io(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "decoder worker cancelled",
                    ))
                })?
                .recv()
                .map_err(|error| {
                    DecoderError::Io(io::Error::new(io::ErrorKind::BrokenPipe, error))
                })?;
            self.push_stretched_message(message);
        }
    }

    fn push_pcm(&mut self, generation: u64, position: AudioPacketPosition, mut packet: Vec<f64>) {
        assert!(packet.len() <= SECONDARY_PCM_CHUNK_SAMPLES);
        assert!(self.pcm_buffer.len() + packet.len() <= SECONDARY_PCM_ASSEMBLER_SAMPLES);
        if self.pcm_origin.is_none() {
            self.pcm_origin = Some(position);
            self.pcm_consumed_frames = 0;
            self.pcm_generation = Some(generation);
        } else {
            debug_assert_eq!(self.pcm_generation, Some(generation));
        }
        self.pcm_buffer.append(&mut packet);
    }

    fn message_into_packet(
        message: SecondaryDecodeMessage,
    ) -> DecoderResult<Option<(AudioPacketPosition, AudioPacket)>> {
        match message.event {
            SecondaryDecodeEvent::Packet(position, packet) => Ok(Some((position, packet))),
            SecondaryDecodeEvent::Eof => Ok(None),
            SecondaryDecodeEvent::Failed(error) => Err(error),
        }
    }

    fn cancel(&mut self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) && !self.finished.load(Ordering::Acquire) {
            debug!("Secondary decode cancelled for <{}>", self.track_label);
        }

        // Closing the compressed stream wakes AudioFileStreaming reads waiting on its condition
        // variable. Dropping the receiver separately wakes a worker blocked by PCM backpressure.
        self.stream_loader_controller.close();
        self.receiver.take();

        // Never join on the sink/playback thread. The two wakeups above make the worker converge
        // promptly; dropping JoinHandle only detaches it while it exits.
        self.thread.take();
    }
}

fn duration_ms_u32(duration: std::time::Duration) -> u32 {
    u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)
}

impl Drop for SecondaryDecodeWorker {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn run_decode_worker(
    mut decoder: Decoder,
    sender: SyncSender<SecondaryDecodeMessage>,
    cancelled: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    generation: u64,
    track_label: &str,
) {
    debug!("Secondary decode started for <{track_label}>");
    let mut reported_ready = false;

    while !cancelled.load(Ordering::Acquire) {
        match decoder.next_packet() {
            Ok(Some((position, AudioPacket::Samples(samples)))) => {
                for (chunk_index, chunk) in samples.chunks(SECONDARY_PCM_CHUNK_SAMPLES).enumerate()
                {
                    if cancelled.load(Ordering::Acquire) {
                        finished.store(true, Ordering::Release);
                        return;
                    }

                    let frame_offset = chunk_index * SECONDARY_PCM_CHUNK_FRAMES;
                    let position_offset_ms =
                        (frame_offset as u64 * 1000 / u64::from(SAMPLE_RATE)) as u32;
                    let chunk_position = AudioPacketPosition {
                        position_ms: position.position_ms.saturating_add(position_offset_ms),
                        skipped: position.skipped && chunk_index == 0,
                    };
                    let message = SecondaryDecodeMessage {
                        generation,
                        event: SecondaryDecodeEvent::Packet(
                            chunk_position,
                            AudioPacket::Samples(chunk.to_vec()),
                        ),
                    };

                    if sender.send(message).is_err() {
                        finished.store(true, Ordering::Release);
                        return;
                    }
                    if !reported_ready {
                        debug!("Secondary PCM ready for transition for <{track_label}>");
                        reported_ready = true;
                    }
                }
            }
            Ok(Some((_, AudioPacket::Raw(_)))) => {
                let error = DecoderError::PassthroughDecoder(
                    "secondary PCM decode received an encoded packet".into(),
                );
                debug!("Secondary decode failed for <{track_label}>: {error}");
                let _ = sender.send(SecondaryDecodeMessage {
                    generation,
                    event: SecondaryDecodeEvent::Failed(error),
                });
                break;
            }
            Ok(None) => {
                debug!("Secondary EOF for <{track_label}>");
                let _ = sender.send(SecondaryDecodeMessage {
                    generation,
                    event: SecondaryDecodeEvent::Eof,
                });
                break;
            }
            Err(error) => {
                debug!("Secondary decode failed for <{track_label}>: {error}");
                let _ = sender.send(SecondaryDecodeMessage {
                    generation,
                    event: SecondaryDecodeEvent::Failed(error),
                });
                break;
            }
        }
    }

    finished.store(true, Ordering::Release);
}
