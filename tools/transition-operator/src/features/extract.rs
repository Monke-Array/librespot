use crate::dsp::{
    BiquadDf2t, PcmBuffer, measure_sample_peak, measure_true_peak_bs1770_4x, rbj_coefficients,
};
use crate::error::{Error, Result};
use crate::features::{
    AnalysisIdentity, CueFeature, CueKind, FeatureSnapshotBodyV2, FeatureSnapshotV2, FeatureWindow,
    PairFeatures, RhythmFeatures, SourceFeatures, WindowKind, finalize_feature_snapshot,
};
use crate::geometry::{DurationMode, GeometryProposal, GeometryProposalCore};
use crate::model::FilterKind;
use crate::scalar::div_round_nearest_away;
use sha2::{Digest, Sha256};

const SAMPLE_RATE: i64 = 44_100;
const ENVELOPE_HOP: usize = 441;
const MAX_DRY_FRAMES: i64 = 705_600;
const MAX_EFFECT_FRAMES: i64 = 264_600;
const FALLBACK_FRAMES: i64 = 220_500;
const ALGORITHM_DESCRIPTION: &str = concat!(
    "transition-local-features/2.0.0;pcm=s16le-stereo-44100;",
    "rhythm=10ms-stereo-rms-positive-db-flux-autocorrelation-40-240bpm-with-slow-octave-fold-and-p90-prominence-confidence-floor500k-fullat250k;",
    "meter=4-phase-onset-contrast-confidence-floor500k-fullat200k-no-fallback;cue=nearest-bounds-safe-detected-downbeat;",
    "window=16s-pre-cue;bands=rbj-q707-lp250-hp250-lp4000-hp4000;",
    "bass=rbj-lp180;stability=one-second-band-total-variation;",
    "energy=400ms-stereo-rms-db-mean-and-population-sd;",
    "transient=10ms-positive-db-flux-clamped-at12db-and-cue-relative-temporal-iou-minus2.5s-to-minus0.5s;",
    "peak=bs1770-4x-v1;vocal=absent"
);

#[derive(Clone, Debug)]
pub struct CanonicalSource {
    pcm: PcmBuffer,
}

impl CanonicalSource {
    pub fn from_s16le_stereo_44100(bytes: &[u8]) -> Result<Self> {
        Ok(Self {
            pcm: PcmBuffer::from_s16le_stereo_44100(bytes)?,
        })
    }

    pub fn from_pcm(pcm: PcmBuffer) -> Result<Self> {
        let mut bytes = Vec::with_capacity(pcm.frame_count().saturating_mul(4));
        for frame in pcm.frames() {
            for sample in frame {
                if !sample.is_finite() || !(-1.0..1.0).contains(sample) {
                    return Err(Error::new(
                        "NON_CANONICAL_PCM",
                        "test/source PCM must fit canonical signed 16-bit range",
                    ));
                }
                let quantized = (sample * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16;
                bytes.extend_from_slice(&quantized.to_le_bytes());
            }
        }
        Self::from_s16le_stereo_44100(&bytes)
    }

    pub fn pcm(&self) -> &PcmBuffer {
        &self.pcm
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowMeasurements {
    pub transient_activity_ppm: i64,
    pub transient_density_ppm: i64,
    pub bass_occupancy_ppm: i64,
    pub spectral_stability_ppm: i64,
    pub low_occupancy_ppm: i64,
    pub mid_occupancy_ppm: i64,
    pub high_occupancy_ppm: i64,
    pub short_term_loudness_mlu: i64,
    pub energy_variability_mdb: i64,
}

#[derive(Clone, Debug)]
pub struct SignalFeatures {
    pub analysis: AnalysisIdentity,
    pub source_pcm_sha256: String,
    pub source_frames: i64,
    pub sample_peak_mdbfs: i64,
    pub true_peak_mdbtp: i64,
    pub rhythm: Option<RhythmFeatures>,
    pcm: PcmBuffer,
    onset_envelope: Vec<f64>,
}

impl SignalFeatures {
    pub fn measure_window(&self, start_frame: i64, end_frame: i64) -> Result<WindowMeasurements> {
        if start_frame < 0 || start_frame >= end_frame || end_frame > self.source_frames {
            return Err(Error::new(
                "INVALID_FEATURE_WINDOW",
                "measurement window is outside canonical PCM",
            ));
        }
        measure_window_impl(&self.pcm, &self.onset_envelope, start_frame, end_frame)
    }

    pub fn pcm(&self) -> &PcmBuffer {
        &self.pcm
    }
}

#[derive(Clone, Debug)]
pub struct GeometrySet {
    pub fallback_geometry: GeometryProposal,
    pub geometries: Vec<GeometryProposal>,
}

pub fn feature_algorithm_sha256() -> String {
    hex(&Sha256::digest(ALGORITHM_DESCRIPTION.as_bytes()))
}

pub fn extract_signal_features(source: &CanonicalSource) -> Result<SignalFeatures> {
    if source.pcm.frame_count() == 0 {
        return Err(Error::new("EMPTY_PCM", "feature extraction requires PCM"));
    }
    let source_pcm_sha256 = source
        .pcm
        .source_pcm_sha256()
        .ok_or_else(|| Error::new("NON_CANONICAL_PCM", "source PCM identity is missing"))?
        .to_owned();
    let algorithm_sha256 = feature_algorithm_sha256();
    let analysis = AnalysisIdentity::finalize(
        "transition-local-features".to_owned(),
        "2.0.0".to_owned(),
        algorithm_sha256,
    )?;
    let sample_peak = measure_sample_peak(&source.pcm)?;
    let true_peak = measure_true_peak_bs1770_4x(&source.pcm)?.max(sample_peak);
    let onset_envelope = onset_envelope(&source.pcm);
    let source_frames = i64::try_from(source.pcm.frame_count())
        .map_err(|_| Error::new("INTEGER_OVERFLOW", "source frame count exceeds i64"))?;
    let rhythm = detect_rhythm(&onset_envelope, source_frames);
    Ok(SignalFeatures {
        analysis,
        source_pcm_sha256,
        source_frames,
        sample_peak_mdbfs: amplitude_mdb(sample_peak),
        true_peak_mdbtp: amplitude_mdb(true_peak),
        rhythm,
        pcm: source.pcm.clone(),
        onset_envelope,
    })
}

pub fn build_feature_snapshot(
    outgoing: &SignalFeatures,
    incoming: &SignalFeatures,
) -> Result<FeatureSnapshotV2> {
    if outgoing.analysis != incoming.analysis {
        return Err(Error::new(
            "ANALYSIS_IDENTITY_MISMATCH",
            "pair sources must use the same feature algorithm",
        ));
    }
    let incoming_rate = rhythm_rate_ppm(outgoing.rhythm.as_ref(), incoming.rhythm.as_ref());
    let incoming_pre_frames = incoming_rate
        .and_then(|rate| div_round_nearest_away(MAX_DRY_FRAMES.checked_mul(rate)?, 1_000_000).ok())
        .unwrap_or(MAX_DRY_FRAMES)
        .max(MAX_DRY_FRAMES);
    let outgoing_cue = choose_cue(outgoing, CueKind::Outro, MAX_DRY_FRAMES)?;
    let incoming_cue = choose_cue(incoming, CueKind::Intro, incoming_pre_frames)?;
    let outgoing_start = outgoing_cue.source_frame - MAX_DRY_FRAMES;
    let incoming_start = incoming_cue.source_frame - incoming_pre_frames;
    let outgoing_measurements =
        outgoing.measure_window(outgoing_start, outgoing_cue.source_frame)?;
    let incoming_measurements =
        incoming.measure_window(incoming_start, incoming_cue.source_frame)?;
    let outgoing_window = feature_window(
        WindowKind::OutgoingTransition,
        outgoing_start,
        outgoing_cue.source_frame + 1,
        &outgoing_measurements,
        hard_cut_safe(&outgoing.pcm, outgoing_cue.source_frame)?,
    );
    let incoming_end = (incoming_cue.source_frame + MAX_EFFECT_FRAMES).min(incoming.source_frames);
    if incoming_end < incoming_cue.source_frame + MAX_EFFECT_FRAMES {
        return Err(Error::new(
            "PAIR_INELIGIBLE",
            "incoming cue cannot provide the maximum effect window",
        ));
    }
    let incoming_window = feature_window(
        WindowKind::IncomingTransition,
        incoming_start,
        incoming_end,
        &incoming_measurements,
        hard_cut_safe(&incoming.pcm, incoming_cue.source_frame)?,
    );
    let outgoing_source = SourceFeatures::finalize(
        outgoing.source_pcm_sha256.clone(),
        outgoing.source_frames,
        &outgoing.analysis,
        vec![outgoing_cue],
        outgoing.rhythm.clone(),
        vec![outgoing_window],
        outgoing.sample_peak_mdbfs,
        outgoing.true_peak_mdbtp,
    )?;
    let incoming_source = SourceFeatures::finalize(
        incoming.source_pcm_sha256.clone(),
        incoming.source_frames,
        &incoming.analysis,
        vec![incoming_cue],
        incoming.rhythm.clone(),
        vec![incoming_window],
        incoming.sample_peak_mdbfs,
        incoming.true_peak_mdbtp,
    )?;
    let spectral_overlap_ppm =
        histogram_intersection(&outgoing_measurements, &incoming_measurements);
    let bass_collision_ppm = geometric_mean_ppm(
        outgoing_measurements.bass_occupancy_ppm,
        incoming_measurements.bass_occupancy_ppm,
    );
    let transient_collision = aligned_transient_collision(
        outgoing,
        incoming,
        outgoing_source.cues[0].source_frame,
        incoming_source.cues[0].source_frame,
        incoming_rate.unwrap_or(1_000_000),
    )?;
    let alignment_error_ppm_of_beat = incoming_rate.map(|_| 0);
    let body = FeatureSnapshotBodyV2 {
        schema_version: "transition-feature-snapshot/2".to_owned(),
        analysis: outgoing.analysis.clone(),
        pair: PairFeatures {
            outgoing_cue_id: outgoing_source.cues[0].cue_id.clone(),
            incoming_cue_id: incoming_source.cues[0].cue_id.clone(),
            outgoing_window_id: outgoing_source.windows[0].window_id.clone(),
            incoming_window_id: incoming_source.windows[0].window_id.clone(),
            vocal_collision_ppm: None,
            vocal_collision_span_frames: None,
            transient_collision_ppm: Some(transient_collision.value_ppm),
            transient_collision_span_frames: transient_collision.span_frames,
            bass_collision_ppm: Some(bass_collision_ppm),
            spectral_overlap_ppm: Some(spectral_overlap_ppm),
            energy_delta_mdb: Some(
                incoming_measurements.short_term_loudness_mlu
                    - outgoing_measurements.short_term_loudness_mlu,
            ),
            alignment_error_ppm_of_beat,
            collision_start_frame: transient_collision.start_frame,
            collision_end_frame: transient_collision.end_frame,
        },
        outgoing: outgoing_source,
        incoming: incoming_source,
    };
    finalize_feature_snapshot(body)
}

pub fn propose_geometries(snapshot: &FeatureSnapshotV2) -> Result<GeometrySet> {
    crate::features::validate_feature_snapshot(snapshot)?;
    let outgoing_cue = snapshot
        .outgoing
        .cues
        .iter()
        .find(|cue| cue.cue_id == snapshot.pair.outgoing_cue_id)
        .ok_or_else(|| Error::new("INVALID_PAIR_FEATURES", "outgoing cue is missing"))?;
    let incoming_cue = snapshot
        .incoming
        .cues
        .iter()
        .find(|cue| cue.cue_id == snapshot.pair.incoming_cue_id)
        .ok_or_else(|| Error::new("INVALID_PAIR_FEATURES", "incoming cue is missing"))?;
    let cue_confidence = outgoing_cue.confidence_ppm.min(incoming_cue.confidence_ppm);
    let windows = vec![
        snapshot.pair.outgoing_window_id.clone(),
        snapshot.pair.incoming_window_id.clone(),
    ];
    let fallback_geometry = geometry(
        snapshot,
        outgoing_cue,
        incoming_cue,
        &windows,
        DurationMode::Seconds,
        FALLBACK_FRAMES,
        0,
        1_000_000,
        cue_confidence,
        0,
        0,
        1_000_000,
        500_000,
    )?;
    let mut geometries = Vec::new();
    if cue_confidence >= 700_000 {
        let (beat_confidence, downbeat_confidence) = rhythm_confidences(snapshot);
        let rate = rhythm_rate_ppm(
            snapshot.outgoing.rhythm.as_ref(),
            snapshot.incoming.rhythm.as_ref(),
        )
        .unwrap_or(1_000_000);
        for frames in [132_300, 220_500] {
            geometries.push(geometry(
                snapshot,
                outgoing_cue,
                incoming_cue,
                &windows,
                DurationMode::Seconds,
                frames,
                0,
                rate,
                cue_confidence,
                beat_confidence,
                downbeat_confidence,
                snapshot
                    .pair
                    .alignment_error_ppm_of_beat
                    .unwrap_or(1_000_000),
                cue_confidence,
            )?);
        }
        if beat_confidence >= 750_000
            && downbeat_confidence >= 700_000
            && snapshot
                .pair
                .alignment_error_ppm_of_beat
                .unwrap_or(1_000_000)
                <= 31_250
        {
            geometries.push(geometry(
                snapshot,
                outgoing_cue,
                incoming_cue,
                &windows,
                DurationMode::Cut,
                1_323,
                0,
                rate,
                cue_confidence,
                beat_confidence,
                downbeat_confidence,
                0,
                cue_confidence.min(beat_confidence).min(downbeat_confidence),
            )?);
            let beat_interval = median_beat_interval(snapshot.outgoing.rhythm.as_ref().unwrap())?;
            let meter = snapshot
                .outgoing
                .rhythm
                .as_ref()
                .and_then(|rhythm| rhythm.meter_beats)
                .unwrap_or(4);
            for bars in [1, 2, 4] {
                let frames = beat_interval
                    .checked_mul(meter)
                    .and_then(|value| value.checked_mul(bars))
                    .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "bar geometry overflow"))?;
                if frames <= MAX_DRY_FRAMES {
                    geometries.push(geometry(
                        snapshot,
                        outgoing_cue,
                        incoming_cue,
                        &windows,
                        DurationMode::Bars,
                        frames,
                        bars,
                        rate,
                        cue_confidence,
                        beat_confidence,
                        downbeat_confidence,
                        0,
                        cue_confidence.min(beat_confidence).min(downbeat_confidence),
                    )?);
                }
            }
        }
    }
    Ok(GeometrySet {
        fallback_geometry,
        geometries,
    })
}

fn choose_cue(
    features: &SignalFeatures,
    kind: CueKind,
    required_pre_frames: i64,
) -> Result<CueFeature> {
    let minimum = required_pre_frames;
    let maximum = features.source_frames - MAX_EFFECT_FRAMES - 1;
    if minimum >= maximum {
        return Err(Error::new(
            "PAIR_INELIGIBLE",
            "source cannot provide the certified transition window",
        ));
    }
    if let Some(rhythm) = &features.rhythm {
        let target = match kind {
            CueKind::Intro => MAX_DRY_FRAMES,
            CueKind::Outro => features.source_frames - MAX_EFFECT_FRAMES - 1,
        };
        if let Some(frame) = rhythm
            .downbeat_frames
            .iter()
            .copied()
            .filter(|frame| (minimum..=maximum).contains(frame))
            .min_by_key(|frame| (frame - target).abs())
        {
            return Ok(CueFeature::draft(
                frame,
                kind,
                rhythm
                    .beat_confidence_ppm
                    .min(rhythm.downbeat_confidence_ppm),
            ));
        }
    }
    let frame = match kind {
        CueKind::Intro => minimum,
        CueKind::Outro => maximum,
    };
    Ok(CueFeature::draft(frame, kind, 0))
}

fn feature_window(
    kind: WindowKind,
    start_frame: i64,
    end_frame: i64,
    measured: &WindowMeasurements,
    hard_cut_safe: bool,
) -> FeatureWindow {
    FeatureWindow::draft(
        kind,
        start_frame,
        end_frame,
        None,
        Some(measured.transient_activity_ppm),
        Some(measured.transient_density_ppm),
        Some(hard_cut_safe),
        Some(measured.bass_occupancy_ppm),
        Some(measured.spectral_stability_ppm),
        Some(measured.low_occupancy_ppm),
        Some(measured.mid_occupancy_ppm),
        Some(measured.high_occupancy_ppm),
        Some(measured.short_term_loudness_mlu),
        Some(measured.energy_variability_mdb),
    )
}

fn onset_envelope(pcm: &PcmBuffer) -> Vec<f64> {
    let levels: Vec<_> = pcm
        .frames()
        .chunks(ENVELOPE_HOP)
        .map(|frames| {
            let mean_square = frames
                .iter()
                .map(|frame| (frame[0] * frame[0] + frame[1] * frame[1]) * 0.5)
                .sum::<f64>()
                / frames.len() as f64;
            10.0 * mean_square.max(1e-12).log10()
        })
        .collect();
    let mut onset = vec![0.0; levels.len()];
    for index in 1..levels.len() {
        onset[index] = (levels[index] - levels[index - 1]).clamp(0.0, 24.0);
    }
    onset
}

fn detect_rhythm(onset: &[f64], source_frames: i64) -> Option<RhythmFeatures> {
    if onset.len() < 400 || onset.iter().map(|value| value * value).sum::<f64>() < 1.0 {
        return None;
    }
    let mut correlations = vec![0.0; 151];
    let mut best: Option<(usize, f64)> = None;
    for lag in 25..=150usize {
        if lag >= onset.len() / 2 {
            continue;
        }
        let mut dot = 0.0;
        let mut left = 0.0;
        let mut right = 0.0;
        for index in lag..onset.len() {
            dot += onset[index] * onset[index - lag];
            left += onset[index] * onset[index];
            right += onset[index - lag] * onset[index - lag];
        }
        let correlation = dot / (left * right).sqrt().max(f64::MIN_POSITIVE);
        correlations[lag] = correlation;
        if best.is_none_or(|(old_lag, old)| {
            correlation > old + 1e-12 || ((correlation - old).abs() <= 1e-12 && lag < old_lag)
        }) {
            best = Some((lag, correlation));
        }
    }
    let (mut lag, correlation) = best?;
    if correlation < 0.20 {
        return None;
    }
    // A strongly accented four-beat pulse train often has its largest
    // autocorrelation at half tempo. Fold a sub-70 BPM maximum to the supported
    // double-time interpretation only when the half-lag itself retains at least
    // half of the measured periodic evidence. No grid is invented when that
    // harmonic evidence is absent.
    if lag > 85 {
        let half_candidates = [lag / 2, lag.div_ceil(2)];
        if let Some(candidate) = half_candidates
            .into_iter()
            .filter(|candidate| *candidate >= 25)
            .max_by(|left, right| correlations[*left].total_cmp(&correlations[*right]))
        {
            if correlations[candidate] >= correlation * 0.5 {
                lag = candidate;
            }
        }
    }
    let phase = (0..lag).max_by(|left, right| {
        phase_score(onset, *left, lag)
            .total_cmp(&phase_score(onset, *right, lag))
            .then_with(|| right.cmp(left))
    })?;
    let beats: Vec<i64> = (phase..onset.len())
        .step_by(lag)
        .map(|bin| (bin * ENVELOPE_HOP + ENVELOPE_HOP / 2) as i64)
        .filter(|frame| *frame < source_frames)
        .collect();
    if beats.len() < 8 {
        return None;
    }
    let mut phase_scores: Vec<_> = (0..4)
        .map(|bar_phase| {
            let score = (bar_phase..beats.len())
                .step_by(4)
                .enumerate()
                .map(|(index, _)| {
                    onset
                        .get(phase + (bar_phase + index * 4) * lag)
                        .copied()
                        .unwrap_or(0.0)
                })
                .sum::<f64>();
            (bar_phase, score)
        })
        .collect();
    phase_scores.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    let best_phase = phase_scores[0].0;
    let best_score = phase_scores[0].1;
    let second_score = phase_scores[1].1;
    let contrast =
        ((best_score - second_score) / best_score.max(f64::MIN_POSITIVE)).clamp(0.0, 1.0);
    let mut distribution = correlations[25..].to_vec();
    distribution.sort_by(f64::total_cmp);
    let p90 = distribution[distribution.len() * 9 / 10];
    let correlation_ppm = (correlation.clamp(0.0, 1.0) * 1_000_000.0).round() as i64;
    let p90_ppm = (p90.clamp(0.0, 1.0) * 1_000_000.0).round() as i64;
    // Confidence is a closed calibration of peak prominence above the lag-search
    // background, not the raw positive-envelope correlation (whose nonzero
    // baseline varies with onset density). Accepted but unprominent estimates
    // remain below the M2 reliability gate.
    let beat_confidence_ppm = periodicity_confidence_ppm(correlation_ppm, p90_ppm).ok()?;
    // A four-beat phase is independently reliable only when its accented-onset
    // sum separates from the runner-up. Zero contrast cannot inherit beat-grid
    // confidence and masquerade as a downbeat observation.
    let phase_contrast_ppm = (contrast * 1_000_000.0).round() as i64;
    let downbeat_confidence_ppm =
        phase_confidence_ppm(beat_confidence_ppm, phase_contrast_ppm).ok()?;
    let downbeats = beats
        .iter()
        .enumerate()
        .filter_map(|(index, frame)| ((index + 4 - best_phase) % 4 == 0).then_some(*frame))
        .collect();
    Some(RhythmFeatures {
        beat_frames: beats,
        downbeat_frames: downbeats,
        meter_beats: Some(4),
        tempo_millibpm: div_round_nearest_away(60_000 * 1000, lag as i64 * 10).ok()?,
        beat_confidence_ppm,
        downbeat_confidence_ppm,
    })
}

fn phase_score(onset: &[f64], phase: usize, lag: usize) -> f64 {
    (phase..onset.len())
        .step_by(lag)
        .map(|index| onset[index])
        .sum()
}

fn measure_window_impl(
    pcm: &PcmBuffer,
    onset: &[f64],
    start_frame: i64,
    end_frame: i64,
) -> Result<WindowMeasurements> {
    let start = usize::try_from(start_frame)
        .map_err(|_| Error::new("INTEGER_OVERFLOW", "window start exceeds usize"))?;
    let end = usize::try_from(end_frame)
        .map_err(|_| Error::new("INTEGER_OVERFLOW", "window end exceeds usize"))?;
    let frames = pcm
        .frames()
        .get(start..end)
        .ok_or_else(|| Error::new("INVALID_FEATURE_WINDOW", "window PCM is missing"))?;
    let (band_totals, block_bands) = band_energy(frames)?;
    let band_sum = band_totals.iter().sum::<f64>();
    if band_sum <= f64::MIN_POSITIVE {
        return Err(Error::new(
            "UNMEASURABLE_FEATURE_WINDOW",
            "feature window contains no measurable energy",
        ));
    }
    let low = (band_totals[0] / band_sum * 1_000_000.0).round() as i64;
    let mid = (band_totals[1] / band_sum * 1_000_000.0).round() as i64;
    let high = 1_000_000 - low - mid;
    let stability = spectral_stability(&block_bands);
    let energy_blocks = energy_blocks(frames);
    let mean = energy_blocks.iter().sum::<f64>() / energy_blocks.len() as f64;
    let variance = energy_blocks
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / energy_blocks.len() as f64;
    let onset_start = start / ENVELOPE_HOP;
    let onset_end = end.div_ceil(ENVELOPE_HOP).min(onset.len());
    let window_onset = &onset[onset_start.min(onset_end)..onset_end];
    let active = window_onset.iter().filter(|value| **value >= 3.0).count();
    let density_hz = active as f64 / (frames.len() as f64 / SAMPLE_RATE as f64);
    let transient_density_ppm = (density_hz / 8.0 * 1_000_000.0)
        .round()
        .clamp(0.0, 1_000_000.0) as i64;
    let top_mean = if window_onset.is_empty() {
        0.0
    } else {
        let mut sorted = window_onset.to_vec();
        sorted.sort_by(|left, right| right.total_cmp(left));
        let count = sorted.len().div_ceil(10).max(1);
        sorted[..count].iter().sum::<f64>() / count as f64
    };
    let transient_activity_ppm = (top_mean / 12.0 * 1_000_000.0)
        .round()
        .clamp(0.0, 1_000_000.0) as i64;
    let bass_occupancy_ppm = (band_totals[3] / band_sum * 1_000_000.0)
        .round()
        .clamp(0.0, 1_000_000.0) as i64;
    Ok(WindowMeasurements {
        transient_activity_ppm,
        transient_density_ppm,
        bass_occupancy_ppm,
        spectral_stability_ppm: stability,
        low_occupancy_ppm: low,
        mid_occupancy_ppm: mid,
        high_occupancy_ppm: high,
        short_term_loudness_mlu: mean.round().clamp(-120_000.0, 24_000.0) as i64,
        energy_variability_mdb: variance.sqrt().round().clamp(0.0, 120_000.0) as i64,
    })
}

fn band_energy(frames: &[[f64; 2]]) -> Result<([f64; 4], Vec<[f64; 3]>)> {
    let mut low = BiquadDf2t::new(rbj_coefficients(FilterKind::LowpassBiquadV1, 250_000, 707)?);
    let mut mid_high = BiquadDf2t::new(rbj_coefficients(
        FilterKind::HighpassBiquadV1,
        250_000,
        707,
    )?);
    let mut mid_low = BiquadDf2t::new(rbj_coefficients(
        FilterKind::LowpassBiquadV1,
        4_000_000,
        707,
    )?);
    let mut high = BiquadDf2t::new(rbj_coefficients(
        FilterKind::HighpassBiquadV1,
        4_000_000,
        707,
    )?);
    let mut bass = BiquadDf2t::new(rbj_coefficients(FilterKind::LowpassBiquadV1, 180_000, 707)?);
    let mut totals = [0.0; 4];
    let mut blocks = vec![[0.0; 3]; frames.len().div_ceil(SAMPLE_RATE as usize)];
    for (index, frame) in frames.iter().enumerate() {
        let mono = (frame[0] + frame[1]) * 0.5;
        let bands = [
            low.process(mono),
            mid_low.process(mid_high.process(mono)),
            high.process(mono),
        ];
        for band in 0..3 {
            let energy = bands[band] * bands[band];
            totals[band] += energy;
            blocks[index / SAMPLE_RATE as usize][band] += energy;
        }
        let bass_sample = bass.process(mono);
        totals[3] += bass_sample * bass_sample;
    }
    Ok((totals, blocks))
}

fn spectral_stability(blocks: &[[f64; 3]]) -> i64 {
    let distributions: Vec<[f64; 3]> = blocks
        .iter()
        .filter_map(|block| {
            let sum = block.iter().sum::<f64>();
            (sum > f64::MIN_POSITIVE).then(|| [block[0] / sum, block[1] / sum, block[2] / sum])
        })
        .collect();
    if distributions.len() < 2 {
        return 0;
    }
    let variation = distributions
        .windows(2)
        .map(|pair| {
            (0..3)
                .map(|band| (pair[1][band] - pair[0][band]).abs())
                .sum::<f64>()
                * 0.5
        })
        .sum::<f64>()
        / (distributions.len() - 1) as f64;
    ((1.0 - variation).clamp(0.0, 1.0) * 1_000_000.0).round() as i64
}

fn energy_blocks(frames: &[[f64; 2]]) -> Vec<f64> {
    let block = (SAMPLE_RATE as usize * 2) / 5;
    frames
        .chunks(block)
        .map(|values| {
            let mean_square = values
                .iter()
                .map(|frame| (frame[0] * frame[0] + frame[1] * frame[1]) * 0.5)
                .sum::<f64>()
                / values.len() as f64;
            10_000.0 * mean_square.max(1e-12).log10()
        })
        .collect()
}

fn hard_cut_safe(pcm: &PcmBuffer, frame: i64) -> Result<bool> {
    let index = usize::try_from(frame)
        .map_err(|_| Error::new("INTEGER_OVERFLOW", "cue frame exceeds usize"))?;
    let value = pcm
        .frames()
        .get(index)
        .ok_or_else(|| Error::new("INVALID_CUE", "cue sample is outside source"))?;
    Ok(value.iter().all(|sample| sample.abs() <= 1.0 / 32_768.0))
}

fn histogram_intersection(left: &WindowMeasurements, right: &WindowMeasurements) -> i64 {
    left.low_occupancy_ppm.min(right.low_occupancy_ppm)
        + left.mid_occupancy_ppm.min(right.mid_occupancy_ppm)
        + left.high_occupancy_ppm.min(right.high_occupancy_ppm)
}

fn geometric_mean_ppm(left: i64, right: i64) -> i64 {
    ((left as f64 * right as f64).sqrt().round() as i64).clamp(0, 1_000_000)
}

/// Calibrates periodicity from the selected autocorrelation peak's prominence
/// above the lag-search background. A non-prominent peak receives the neutral
/// detector floor (500k); prominence of 250k or more is fully confident.
pub fn periodicity_confidence_ppm(peak_ppm: i64, background_p90_ppm: i64) -> Result<i64> {
    if !(0..=1_000_000).contains(&peak_ppm)
        || !(0..=1_000_000).contains(&background_p90_ppm)
        || peak_ppm < background_p90_ppm
    {
        return Err(Error::new(
            "INVALID_RHYTHM_CONFIDENCE_INPUT",
            "autocorrelation peak and background must be ordered ppm values",
        ));
    }
    if background_p90_ppm == 1_000_000 {
        return Ok(500_000);
    }
    let prominence_ppm = div_round_nearest_away(
        (peak_ppm - background_p90_ppm)
            .checked_mul(1_000_000)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "prominence overflow"))?,
        1_000_000 - background_p90_ppm,
    )?;
    Ok(500_000 + prominence_ppm.saturating_mul(2).min(500_000))
}

/// Combines beat-grid confidence with independent four-beat phase contrast.
/// An unseparated phase contributes 500k; contrast of 200k or more contributes
/// full phase confidence. The beat confidence remains an upper bound.
pub fn phase_confidence_ppm(beat_confidence_ppm: i64, phase_contrast_ppm: i64) -> Result<i64> {
    if !(0..=1_000_000).contains(&beat_confidence_ppm)
        || !(0..=1_000_000).contains(&phase_contrast_ppm)
    {
        return Err(Error::new(
            "INVALID_RHYTHM_CONFIDENCE_INPUT",
            "beat confidence and phase contrast must be ppm values",
        ));
    }
    let scaled_contrast = div_round_nearest_away(
        phase_contrast_ppm
            .checked_mul(5)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "phase contrast overflow"))?,
        2,
    )?;
    let phase_confidence = 500_000 + scaled_contrast.min(500_000);
    div_round_nearest_away(
        beat_confidence_ppm
            .checked_mul(phase_confidence)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "phase confidence overflow"))?,
        1_000_000,
    )
}

/// Temporal intersection-over-union for aligned onset-strength sequences.
/// This intentionally distinguishes simultaneous transients from two windows
/// that merely contain the same aggregate amount of transient activity.
pub fn temporal_onset_iou_ppm(left: &[i64], right: &[i64]) -> Result<i64> {
    if left.len() != right.len() || left.is_empty() {
        return Err(Error::new(
            "INVALID_TRANSIENT_SEQUENCE",
            "onset sequences must have the same nonzero length",
        ));
    }
    let mut intersection = 0_i64;
    let mut union = 0_i64;
    for (&left, &right) in left.iter().zip(right) {
        if !(0..=1_000_000).contains(&left) || !(0..=1_000_000).contains(&right) {
            return Err(Error::new(
                "INVALID_TRANSIENT_SEQUENCE",
                "onset strengths must be ppm values",
            ));
        }
        intersection = intersection
            .checked_add(left.min(right))
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "onset intersection overflow"))?;
        union = union
            .checked_add(left.max(right))
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "onset union overflow"))?;
    }
    if union == 0 {
        Ok(0)
    } else {
        div_round_nearest_away(
            intersection
                .checked_mul(1_000_000)
                .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "onset ratio overflow"))?,
            union,
        )
    }
}

struct TransientCollision {
    value_ppm: i64,
    span_frames: Option<i64>,
    start_frame: Option<i64>,
    end_frame: Option<i64>,
}

fn aligned_transient_collision(
    outgoing: &SignalFeatures,
    incoming: &SignalFeatures,
    outgoing_cue_frame: i64,
    incoming_cue_frame: i64,
    incoming_rate_ppm: i64,
) -> Result<TransientCollision> {
    const START: i64 = -110_250;
    const END: i64 = -22_050;
    let mut outgoing_strengths = Vec::new();
    let mut incoming_strengths = Vec::new();
    let mut current_start = None;
    let mut best: Option<(i64, i64)> = None;
    let mut timeline = START;
    while timeline < END {
        let outgoing_frame = outgoing_cue_frame
            .checked_add(timeline)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "collision frame overflow"))?;
        let incoming_offset = div_round_nearest_away(
            timeline
                .checked_mul(incoming_rate_ppm)
                .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "collision mapping overflow"))?,
            1_000_000,
        )?;
        let incoming_frame = incoming_cue_frame
            .checked_add(incoming_offset)
            .ok_or_else(|| Error::new("INTEGER_OVERFLOW", "collision frame overflow"))?;
        let outgoing_strength = onset_strength_at(outgoing, outgoing_frame);
        let incoming_strength = onset_strength_at(incoming, incoming_frame);
        outgoing_strengths.push(outgoing_strength);
        incoming_strengths.push(incoming_strength);
        if outgoing_strength >= 250_000 && incoming_strength >= 250_000 {
            current_start.get_or_insert(timeline);
        } else if let Some(start) = current_start.take() {
            let candidate = (start, timeline);
            if best.is_none_or(|old| candidate.1 - candidate.0 > old.1 - old.0) {
                best = Some(candidate);
            }
        }
        timeline += ENVELOPE_HOP as i64;
    }
    if let Some(start) = current_start {
        let candidate = (start, END);
        if best.is_none_or(|old| candidate.1 - candidate.0 > old.1 - old.0) {
            best = Some(candidate);
        }
    }
    let value_ppm = temporal_onset_iou_ppm(&outgoing_strengths, &incoming_strengths)?;
    let best = if (300_000..=700_000).contains(&value_ppm) {
        best
    } else {
        None
    };
    Ok(TransientCollision {
        value_ppm,
        span_frames: best.map(|(start, end)| end - start),
        start_frame: best.map(|value| value.0),
        end_frame: best.map(|value| value.1),
    })
}

fn onset_strength_at(features: &SignalFeatures, source_frame: i64) -> i64 {
    let value = usize::try_from(source_frame)
        .ok()
        .and_then(|frame| features.onset_envelope.get(frame / ENVELOPE_HOP))
        .copied()
        .unwrap_or(0.0);
    (value / 12.0 * 1_000_000.0).round().clamp(0.0, 1_000_000.0) as i64
}

fn rhythm_rate_ppm(
    outgoing: Option<&RhythmFeatures>,
    incoming: Option<&RhythmFeatures>,
) -> Option<i64> {
    let outgoing = outgoing?;
    let incoming = incoming?;
    let rate = div_round_nearest_away(
        outgoing.tempo_millibpm.checked_mul(1_000_000)?,
        incoming.tempo_millibpm,
    )
    .ok()?;
    (920_000..=1_080_000).contains(&rate).then_some(rate)
}

fn rhythm_confidences(snapshot: &FeatureSnapshotV2) -> (i64, i64) {
    match (&snapshot.outgoing.rhythm, &snapshot.incoming.rhythm) {
        (Some(outgoing), Some(incoming)) => (
            outgoing
                .beat_confidence_ppm
                .min(incoming.beat_confidence_ppm),
            outgoing
                .downbeat_confidence_ppm
                .min(incoming.downbeat_confidence_ppm),
        ),
        _ => (0, 0),
    }
}

fn median_beat_interval(rhythm: &RhythmFeatures) -> Result<i64> {
    let mut intervals: Vec<_> = rhythm
        .beat_frames
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect();
    if intervals.is_empty() {
        return Err(Error::new(
            "INVALID_RHYTHM_GRID",
            "beat grid has no intervals",
        ));
    }
    intervals.sort_unstable();
    Ok(intervals[intervals.len() / 2])
}

#[allow(clippy::too_many_arguments)]
fn geometry(
    snapshot: &FeatureSnapshotV2,
    outgoing_cue: &CueFeature,
    incoming_cue: &CueFeature,
    windows: &[String],
    duration_mode: DurationMode,
    requested_dry_frames: i64,
    resolved_bar_count: i64,
    incoming_source_rate_ppm: i64,
    cue_confidence_ppm: i64,
    beat_confidence_ppm: i64,
    downbeat_confidence_ppm: i64,
    alignment_error_ppm_of_beat: i64,
    geometry_quality_ppm: i64,
) -> Result<GeometryProposal> {
    GeometryProposal::finalize(GeometryProposalCore {
        outgoing_pcm_sha256: snapshot.outgoing.source_pcm_sha256.clone(),
        incoming_pcm_sha256: snapshot.incoming.source_pcm_sha256.clone(),
        feature_snapshot_sha256: snapshot.snapshot_sha256.clone(),
        outgoing_pcm_frame_count: snapshot.outgoing.source_frames,
        incoming_pcm_frame_count: snapshot.incoming.source_frames,
        outgoing_cue_id: outgoing_cue.cue_id.clone(),
        outgoing_cue_source_frame: outgoing_cue.source_frame,
        incoming_cue_id: incoming_cue.cue_id.clone(),
        incoming_cue_source_frame: incoming_cue.source_frame,
        duration_mode,
        requested_dry_frames,
        resolved_bar_count,
        incoming_source_rate_ppm,
        cue_confidence_ppm,
        beat_confidence_ppm,
        downbeat_confidence_ppm,
        alignment_error_ppm_of_beat,
        geometry_quality_ppm,
        feature_window_ids: windows.to_vec(),
    })
}

fn amplitude_mdb(amplitude: f64) -> i64 {
    (20_000.0 * amplitude.max(1e-6).log10())
        .round()
        .clamp(-120_000.0, 12_000.0) as i64
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
