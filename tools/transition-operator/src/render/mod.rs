mod artifact;
mod environment;
mod ffmpeg;
mod qc;
mod request;
mod source;

pub use artifact::*;
pub use environment::*;
pub use ffmpeg::*;
pub use qc::*;
pub use request::*;
pub use source::*;

use crate::canonical::canonical_json;
use crate::dsp::{
    LookaheadPeakLimiter, PcmBuffer, SafetyMeasurements, TimeStretchBackend, TimeStretchCue,
    apply_crossover_band_gain, apply_duck, apply_filter_envelope, apply_gain, apply_gate,
    apply_pair_gain, measure_loudness, measure_sample_peak, measure_true_peak_bs1770_4x,
    quantize_pcm24, render_feedforward_tail, validate_safety_measurements,
};
use crate::error::{Error, Result};
use crate::identity::ValidatedPlan;
use crate::model::{Operation, Target};
use crate::scalar::div_round_nearest_away;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug)]
pub struct ReferenceRenderer<B, T> {
    backend: B,
    time_stretch: T,
    environment: RendererEnvironment,
}

impl<B, T> ReferenceRenderer<B, T>
where
    B: ArtifactBackend,
    T: TimeStretchBackend,
{
    pub fn new(backend: B, time_stretch: T, environment: RendererEnvironment) -> Result<Self> {
        environment.sha256()?;
        Ok(Self {
            backend,
            time_stretch,
            environment,
        })
    }

    pub fn render(
        &self,
        plan: &ValidatedPlan,
        request: &ValidatedRenderRequest,
        locator: &impl SourceLocator,
    ) -> Result<RenderRecord> {
        if request.request().source_locator_manifest_sha256 != locator.manifest_sha256() {
            return Err(Error::new(
                "SOURCE_MANIFEST_MISMATCH",
                "request and source locator manifest identities differ",
            ));
        }
        let render_id = render_identity(plan, request, &self.environment)?;
        let sources = resolve_validated_sources(plan, locator, &self.backend)?;
        let mut backend_programs = sources.private_backend_programs.clone();
        let request_body = request.request();
        let processing_start = request_body
            .output_start_frame
            .checked_sub(request_body.processing_warm_up_frames)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "render warm-up start overflow"))?;
        let processing_frames = frame_count(processing_start, request_body.output_end_frame)?;
        let warm_up = usize::try_from(request_body.processing_warm_up_frames)
            .map_err(|_| Error::new("INTEGER_OVERFLOW", "render warm-up conversion overflow"))?;
        let output_frames = frame_count(
            request_body.output_start_frame,
            request_body.output_end_frame,
        )?;

        let mut outgoing = extract_normal_window(
            &sources.outgoing,
            plan.plan().sources.outgoing.cue_source_frame,
            processing_start,
            processing_frames,
            request_body.output_start_frame,
            true,
        )?;
        let time_map = plan
            .plan()
            .operations
            .iter()
            .find_map(|operation| match operation {
                Operation::TimeMap(value) => Some(value),
                _ => None,
            });
        let mut stage_trace = Vec::new();
        let outgoing_input = peak_measurements(&outgoing)?;
        let (mut incoming, incoming_input) = if let Some(value) = time_map {
            let output_cue =
                usize::try_from(processing_start.checked_neg().ok_or_else(|| {
                    Error::new("INTEGER_OVERFLOW", "time-map output cue overflow")
                })?)
                .map_err(|_| {
                    Error::new(
                        "INTEGER_OVERFLOW",
                        "time-map output cue conversion overflow",
                    )
                })?;
            let input_cue_numerator = i64::try_from(output_cue)
                .ok()
                .and_then(|cue| cue.checked_mul(value.source_rate_ppm))
                .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "time-map input cue overflow"))?;
            let input_cue =
                usize::try_from(div_round_nearest_away(input_cue_numerator, 1_000_000)?).map_err(
                    |_| Error::new("INTEGER_OVERFLOW", "time-map input cue conversion overflow"),
                )?;
            let input_frames = processing_frames
                .checked_mul(usize::try_from(value.source_rate_ppm).map_err(|_| {
                    Error::new("INTEGER_OVERFLOW", "time-map rate conversion overflow")
                })?)
                .and_then(|product| product.checked_add(999_999))
                .map(|product| product / 1_000_000)
                .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "time-map input length overflow"))?;
            let source_start = plan
                .plan()
                .sources
                .incoming
                .cue_source_frame
                .checked_sub(i64::try_from(input_cue).map_err(|_| {
                    Error::new(
                        "INTEGER_OVERFLOW",
                        "time-map source start conversion overflow",
                    )
                })?)
                .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "time-map source start overflow"))?;
            let input = extract_contiguous(&sources.incoming, source_start, input_frames)?;
            let input_measurements = peak_measurements(&input)?;
            if let Some(program) = self
                .time_stretch
                .private_program_text(value.source_rate_ppm, processing_frames)?
            {
                backend_programs.push(program);
            }
            stage_trace.push("time_map:incoming".to_owned());
            (
                self.time_stretch.process(
                    &input,
                    value.source_rate_ppm,
                    processing_frames,
                    TimeStretchCue {
                        input_frame: input_cue,
                        output_frame: output_cue,
                    },
                )?,
                input_measurements,
            )
        } else {
            let input = extract_normal_window(
                &sources.incoming,
                plan.plan().sources.incoming.cue_source_frame,
                processing_start,
                processing_frames,
                request_body.output_start_frame,
                false,
            )?;
            let input_measurements = peak_measurements(&input)?;
            (input, input_measurements)
        };

        for operation in &plan.plan().operations {
            match operation {
                Operation::FilterEnvelope(value) => {
                    apply_filter_envelope(
                        target_mut(&mut outgoing, &mut incoming, value.target),
                        processing_start,
                        value,
                    )?;
                    stage_trace.push(format!("spectral:{}", target_name(value.target)));
                }
                Operation::CrossoverBandGain(value) => {
                    apply_crossover_band_gain(
                        target_mut(&mut outgoing, &mut incoming, value.target),
                        processing_start,
                        value,
                    )?;
                    stage_trace.push(format!("spectral:{}", target_name(value.target)));
                }
                _ => {}
            }
        }
        for operation in &plan.plan().operations {
            match operation {
                Operation::DuckEnvelope(value) => {
                    apply_duck(
                        target_mut(&mut outgoing, &mut incoming, value.target),
                        processing_start,
                        value,
                    )?;
                    stage_trace.push(format!("dynamics:{}", target_name(value.target)));
                }
                Operation::RhythmicGate(value) => {
                    apply_gate(
                        target_mut(&mut outgoing, &mut incoming, value.target),
                        processing_start,
                        value,
                    )?;
                    stage_trace.push(format!("dynamics:{}", target_name(value.target)));
                }
                _ => {}
            }
        }

        let mut outgoing_tail = zero_buffer(processing_frames)?;
        let mut incoming_tail = zero_buffer(processing_frames)?;
        for operation in &plan.plan().operations {
            if let Operation::FeedforwardDelayTail(value) = operation {
                let rendered = render_feedforward_tail(
                    match value.target {
                        Target::Outgoing => &outgoing,
                        Target::Incoming => &incoming,
                    },
                    processing_start,
                    processing_start,
                    request_body.output_end_frame,
                    value,
                )?;
                match value.target {
                    Target::Outgoing => outgoing_tail = rendered,
                    Target::Incoming => incoming_tail = rendered,
                }
                stage_trace.push(format!("tail_capture:{}", target_name(value.target)));
            }
        }
        for operation in &plan.plan().operations {
            if let Operation::GainEnvelope(value) = operation {
                apply_gain(
                    target_mut(&mut outgoing, &mut incoming, value.target),
                    processing_start,
                    value,
                )?;
                stage_trace.push(format!("primary_gain:{}", target_name(value.target)));
            }
        }

        let mut bus_frames = Vec::with_capacity(processing_frames);
        for index in 0..processing_frames {
            let outgoing_dry = outgoing.frame(index);
            let incoming_dry = incoming.frame(index);
            let outgoing_wet = outgoing_tail.frame(index);
            let incoming_wet = incoming_tail.frame(index);
            bus_frames.push([
                outgoing_dry[0] + outgoing_wet[0] + incoming_dry[0] + incoming_wet[0],
                outgoing_dry[1] + outgoing_wet[1] + incoming_dry[1] + incoming_wet[1],
            ]);
        }
        stage_trace.push("sum".to_owned());
        let mut output = PcmBuffer::from_frames(bus_frames)?.slice(warm_up, processing_frames)?;
        if output.frame_count() != output_frames {
            return Err(Error::new(
                "OUTPUT_LENGTH_MISMATCH",
                "rendered PCM does not match the requested output length",
            ));
        }
        let summed_bus_sample_peak = measure_sample_peak(&output)?;
        apply_pair_gain(&mut output, plan.plan().output_safety.pair_output_gain_mdb)?;
        stage_trace.push("pair_gain".to_owned());
        let post_pair_gain_sample_peak = measure_sample_peak(&output)?;
        let predicted_limiter_demand_mdb = limiter_demand_mdb(
            post_pair_gain_sample_peak,
            plan.plan().output_safety.sample_peak_ceiling_mdbfs,
        );
        let limiter = LookaheadPeakLimiter::new(
            plan.plan().output_safety.sample_peak_ceiling_mdbfs,
            plan.plan().output_safety.lookahead_frames,
            plan.plan().output_safety.release_frames,
            plan.plan().output_safety.activity_threshold_mdb,
        )?;
        let limiter_measurements = limiter.process(
            &mut output,
            request_body.output_start_frame,
            plan.plan().timeline.dry_start_frame,
            plan.plan().timeline.effect_end_frame,
        )?;
        stage_trace.push("limiter".to_owned());
        let safety_measurements = SafetyMeasurements {
            sample_peak: measure_sample_peak(&output)?,
            true_peak: measure_true_peak_bs1770_4x(&output)?,
            loudness_mdb: measure_loudness(&output)?,
            maximum_gain_reduction_mdb: limiter_measurements.maximum_gain_reduction_mdb,
            limiter_active_fraction_ppm: limiter_measurements.active_fraction_ppm,
        };
        validate_safety_measurements(&safety_measurements, &plan.plan().output_safety)?;
        stage_trace.push("measurement".to_owned());
        let pcm24 = quantize_pcm24(&output)?;
        stage_trace.push("quantization".to_owned());
        let pre_encode_pcm_sha256 = hex(&Sha256::digest(&pcm24));
        if let Some(program) = self.backend.private_encode_program_text()? {
            backend_programs.push(program);
        }
        let artifact_bytes = self.backend.encode_pcm24_flac_bytes(&pcm24)?;
        stage_trace.push("encode".to_owned());
        if let Some(program) = self.backend.private_artifact_decode_program_text()? {
            backend_programs.push(program);
        }
        let decoded_pcm = self.backend.decode_flac_pcm24_bytes(&artifact_bytes)?;
        let decoded_pcm_sha256 = hex(&Sha256::digest(&decoded_pcm));
        if decoded_pcm_sha256 != pre_encode_pcm_sha256 {
            return Err(Error::new(
                "ENCODE_DECODE_PCM_MISMATCH",
                "decoded artifact PCM differs from pre-encode canonical PCM",
            ));
        }
        let decoded = PcmBuffer::from_s24le_stereo_44100(&decoded_pcm)?;
        if decoded.frame_count() != output_frames {
            return Err(Error::new(
                "ENCODED_OUTPUT_LENGTH_MISMATCH",
                "decoded artifact length differs from the render request",
            ));
        }
        stage_trace.push("decode_verify".to_owned());
        let render_program_sha256 = hex(&Sha256::digest(canonical_json(&(
            &stage_trace,
            &backend_programs,
        ))?));
        Ok(RenderRecord {
            schema_version: "transition-render-record/1".to_owned(),
            render_id: render_id.as_str().to_owned(),
            candidate_id: request_body.candidate_id.clone(),
            plan_sha256: plan.plan_hash().hex(),
            render_request_sha256: request.request_hash_hex(),
            renderer_environment_sha256: self.environment.sha256()?.hex(),
            render_program_sha256,
            qc_status: RenderQcStatus::Accepted,
            measurements: RenderMeasurements {
                outgoing_input,
                incoming_input,
                summed_bus_sample_peak,
                post_pair_gain_sample_peak,
                predicted_limiter_demand_mdb,
                post_limiter: safety_measurements,
                decoded_artifact: PeakMeasurements {
                    sample_peak: measure_sample_peak(&decoded)?,
                    true_peak: measure_true_peak_bs1770_4x(&decoded)?,
                },
            },
            artifact: ArtifactRecord {
                format: ArtifactFormat::FlacPcm24Stereo44100V1,
                frame_count: i64::try_from(output_frames).map_err(|_| {
                    Error::new(
                        "INTEGER_OVERFLOW",
                        "artifact frame count conversion overflow",
                    )
                })?,
                pre_encode_pcm_sha256,
                decoded_pcm_sha256,
                container_sha256: hex(&Sha256::digest(&artifact_bytes)),
                bytes: artifact_bytes,
            },
            private_provenance: PrivateRenderProvenance {
                stage_trace,
                backend_programs,
            },
        })
    }
}

fn extract_normal_window(
    source: &PcmBuffer,
    cue_source_frame: i64,
    timeline_start: i64,
    frames: usize,
    output_start: i64,
    allow_after_handoff_zero: bool,
) -> Result<PcmBuffer> {
    let mut output = Vec::with_capacity(frames);
    for offset in 0..frames {
        let timeline = timeline_start
            .checked_add(
                i64::try_from(offset)
                    .map_err(|_| Error::new("INTEGER_OVERFLOW", "source offset overflow"))?,
            )
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "source timeline overflow"))?;
        let source_frame = cue_source_frame
            .checked_add(timeline)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "source frame overflow"))?;
        if let Ok(index) = usize::try_from(source_frame) {
            if let Some(frame) = source.frames().get(index) {
                output.push(*frame);
                continue;
            }
        }
        if (source_frame < 0 && timeline < output_start)
            || (allow_after_handoff_zero && timeline >= 0)
        {
            output.push([0.0, 0.0]);
        } else {
            return Err(Error::new(
                "DSP_SOURCE_WINDOW_MISSING",
                "required audible source frame is unavailable",
            ));
        }
    }
    PcmBuffer::from_frames(output)
}

fn extract_contiguous(source: &PcmBuffer, start: i64, frames: usize) -> Result<PcmBuffer> {
    let start = usize::try_from(start).map_err(|_| {
        Error::new(
            "DSP_SOURCE_WINDOW_MISSING",
            "time-stretch source begins before canonical PCM",
        )
    })?;
    let end = start
        .checked_add(frames)
        .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "source window end overflow"))?;
    source.slice(start, end)
}

fn target_mut<'a>(
    outgoing: &'a mut PcmBuffer,
    incoming: &'a mut PcmBuffer,
    target: Target,
) -> &'a mut PcmBuffer {
    match target {
        Target::Outgoing => outgoing,
        Target::Incoming => incoming,
    }
}

fn target_name(target: Target) -> &'static str {
    match target {
        Target::Outgoing => "outgoing",
        Target::Incoming => "incoming",
    }
}

fn zero_buffer(frames: usize) -> Result<PcmBuffer> {
    PcmBuffer::from_frames(vec![[0.0, 0.0]; frames])
}

fn peak_measurements(buffer: &PcmBuffer) -> Result<PeakMeasurements> {
    Ok(PeakMeasurements {
        sample_peak: measure_sample_peak(buffer)?,
        true_peak: measure_true_peak_bs1770_4x(buffer)?,
    })
}

fn frame_count(start: i64, end: i64) -> Result<usize> {
    end.checked_sub(start)
        .and_then(|frames| usize::try_from(frames).ok())
        .filter(|frames| *frames > 0)
        .ok_or_else(|| Error::new("INVALID_RENDER_WINDOW", "render frame count is invalid"))
}

fn limiter_demand_mdb(peak: f64, ceiling_mdbfs: i64) -> i64 {
    if peak == 0.0 {
        return 0;
    }
    let ceiling = 10_f64.powf(ceiling_mdbfs as f64 / 20_000.0);
    (20_000.0 * (peak / ceiling).max(1.0).log10()).round() as i64
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
