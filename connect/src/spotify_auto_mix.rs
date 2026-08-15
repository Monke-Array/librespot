//! Pure Spotify Auto Mix overlap-geometry generation.
//!
//! This module intentionally contains no metadata retrieval, playback,
//! or rendering behavior.

use std::cmp::Ordering;

use sha1::{Digest, Sha1};

/// A raw beat supplied by Spotify's beat-analysis extension.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoBeat {
    /// Beat position in seconds, represented as the source protobuf `float`.
    pub time_seconds: f32,
    /// Beat ordinal/value supplied by the analysis, with `1` denoting a downbeat.
    pub value: i32,
    /// Confidence that this point is a beat.
    pub beat_confidence: f32,
    /// Confidence that this beat is a downbeat.
    pub downbeat_confidence: f32,
}

/// A normalized downbeat consumed by overlap-window generation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoDownbeat {
    /// Index of the source beat in the raw beat vector.
    pub raw_beat_index: usize,
    /// Rounded integral millisecond position used in emitted overlap geometry.
    pub position_ms: i64,
    /// Captured diagnostic position in seconds. Geometry arithmetic is derived
    /// independently from `position_ms` to preserve native `f32` ordering.
    pub position_seconds: f32,
    /// Native-style `float` product of beat and downbeat confidence.
    pub confidence: f32,
}

/// Per-track inputs needed by the overlap-geometry stage.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoTrackGeometryInput {
    /// Exact track duration in seconds used for transition-length constraints.
    pub duration_seconds: f32,
    /// Raw beat-analysis points in source order.
    pub beats: Vec<AutoBeat>,
}

/// Per-pair inputs needed by the overlap-geometry stage.
#[derive(Clone, Copy, Debug)]
pub struct AutoPairGeometryInput<'a> {
    /// Outgoing track geometry.
    pub track_a: &'a AutoTrackGeometryInput,
    /// Incoming track geometry.
    pub track_b: &'a AutoTrackGeometryInput,
    /// Item playback speed associated with the outgoing row.
    pub item_speed_a: f32,
    /// Item playback speed associated with the incoming row.
    pub item_speed_b: f32,
}

/// Recovered configuration for Spotify's overlap-geometry stage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoGeometryConfig {
    /// Whether the recovered alternate 2/4 downbeat-spacing rules are enabled.
    pub allow_2_4_time_signature: bool,
    /// Maximum accepted number of bars in one overlap.
    pub maximum_bars: usize,
    /// Maximum accepted overlap duration in seconds.
    pub maximum_duration_seconds: f32,
    /// Maximum accepted overlap duration as a fraction of either track.
    pub maximum_track_fraction: f32,
    /// Points at or below this confidence are rejected.
    pub downbeat_confidence_threshold: f32,
    /// Adjacent-interval population variance at or above this value is rejected.
    pub beat_interval_variance_threshold: f32,
    /// Maximum absolute difference between the effective duration ratio and one.
    pub maximum_speed_difference: f32,
}

impl Default for AutoGeometryConfig {
    fn default() -> Self {
        Self {
            allow_2_4_time_signature: true,
            maximum_bars: 16,
            maximum_duration_seconds: 35.0,
            maximum_track_fraction: 0.20,
            downbeat_confidence_threshold: 0.40,
            beat_interval_variance_threshold: 0.0005,
            maximum_speed_difference: 0.10,
        }
    }
}

/// A geometrically valid window within one track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoWindow {
    /// Index of the first normalized downbeat.
    pub start_index: usize,
    /// Index one `duration_bars` interval after `start_index`.
    pub end_index: usize,
    /// First downbeat position in milliseconds.
    pub start_ms: i64,
    /// Last downbeat position in milliseconds.
    pub end_ms: i64,
    /// Native-style `float` difference between the endpoint positions.
    pub duration_seconds: f32,
    /// Integral endpoint difference used in emitted overlap geometry.
    pub duration_ms: i64,
    /// Number of downbeat intervals in the window.
    pub duration_bars: usize,
    /// Lowest confidence among the inclusive endpoint range.
    pub minimum_confidence: f32,
    /// Population variance of adjacent downbeat intervals.
    pub interval_variance: f32,
}

/// One raw, geometrically eligible outgoing/incoming overlap candidate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeneratedOverlapCandidate {
    /// Outgoing normalized-downbeat start index.
    pub start_a_index: usize,
    /// Incoming normalized-downbeat start index.
    pub start_b_index: usize,
    /// Outgoing start position in milliseconds.
    pub start_a_ms: i64,
    /// Incoming start position in milliseconds.
    pub start_b_ms: i64,
    /// Outgoing endpoint position in milliseconds.
    pub end_a_ms: i64,
    /// Incoming endpoint position in milliseconds.
    pub end_b_ms: i64,
    /// Outgoing endpoint duration in milliseconds.
    pub duration_ms: i64,
    /// Incoming endpoint duration in milliseconds.
    pub duration_b_ms: i64,
    /// Number of bars represented by both windows.
    pub duration_bars: usize,
    /// Spotify currently emits one for the outgoing source.
    pub speed_a: f32,
    /// Unadjusted incoming/outgoing native window-duration ratio.
    pub speed_b: f32,
    /// Ratio used for item-speed eligibility.
    pub effective_window_ratio: f32,
    /// Outgoing window interval variance retained for validation and later scoring.
    pub interval_variance_a: f32,
    /// Incoming window interval variance retained for validation and later scoring.
    pub interval_variance_b: f32,
    /// Minimum outgoing confidence retained for validation and later scoring.
    pub minimum_confidence_a: f32,
    /// Minimum incoming confidence retained for validation and later scoring.
    pub minimum_confidence_b: f32,
}

/// A fade-in or fade-out cuepoint used by native scoring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoCuepoint {
    /// Cuepoint position in milliseconds.
    pub position_ms: i64,
    /// Cuepoint tempo in beats per minute.
    pub tempo_bpm: f32,
}

/// Ordered cuepoint inputs for one track.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutoCuepoints {
    /// Native best fade-in cuepoint, if supplied.
    pub best_fade_in: Option<AutoCuepoint>,
    /// Native best fade-out cuepoint, if supplied.
    pub best_fade_out: Option<AutoCuepoint>,
    /// Fade-in candidates in stored order.
    pub fade_in_candidates: Vec<AutoCuepoint>,
    /// Fade-out candidates in stored order.
    pub fade_out_candidates: Vec<AutoCuepoint>,
}

/// Uniformly sampled vocal-activity probabilities for one track.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutoVocalActivity {
    /// Source audio sample rate used by the activity timeline.
    pub source_sample_rate_hz: u32,
    /// Smoothing window recorded with the fixture data.
    pub smoothing_window_size: u32,
    /// Source-sample position of the first stored probability.
    pub first_window_sample_start: i64,
    /// Source samples between adjacent stored probabilities.
    pub samples_between_windows: u32,
    /// Stored integer probability values.
    pub probabilities: Vec<u8>,
}

/// Parsed Camelot key used by native compatibility scoring.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AutoCamelotKey {
    /// Camelot number in the inclusive range 1 through 12.
    pub number: u8,
    /// Camelot mode letter, `A` or `B`.
    pub letter: char,
}

impl AutoCamelotKey {
    /// Parses a native Camelot value such as `9A` or `12B`.
    pub fn parse(value: &str) -> Option<Self> {
        let letter = value.chars().last()?;
        let number = value.strip_suffix(letter)?;
        let number = number.parse().ok()?;
        (matches!(number, 1..=12) && matches!(letter, 'A' | 'B')).then_some(Self { number, letter })
    }
}

/// Complete per-track input for the pure scoring and ranking stages.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoTrackScoringInput {
    /// Geometry-stage input recovered in milestone 1.
    pub geometry: AutoTrackGeometryInput,
    /// Millisecond-precision context duration consumed by base scoring.
    pub scoring_duration_seconds: f32,
    /// Playable URI used by deterministic preset selection.
    pub playable_uri: String,
    /// Cuepoint metadata used by overlap and fallback scoring.
    pub cuepoints: AutoCuepoints,
    /// Vocal-activity timeline.
    pub vocal_activity: AutoVocalActivity,
    /// Parsed Camelot key, with missing or invalid metadata represented by `None`.
    pub camelot_key: Option<AutoCamelotKey>,
    /// Playable-track tempo used to reject cuepoints from incompatible audio.
    pub bpm: f32,
    /// Descriptor-type-1 concept URIs.
    pub genre_concept_uris: Vec<String>,
    /// Whether the metadata service considers the track mixable.
    pub mixable: bool,
    /// Genre-based beatmatchability supplied by the metadata service.
    pub genre_based_beatmatchability: f32,
}

/// Per-pair input for the pure scoring and ranking pipeline.
#[derive(Clone, Copy, Debug)]
pub struct AutoPairScoringInput<'a> {
    /// Outgoing track.
    pub track_a: &'a AutoTrackScoringInput,
    /// Incoming track.
    pub track_b: &'a AutoTrackScoringInput,
    /// Outgoing playlist item speed.
    pub item_speed_a: f32,
    /// Incoming playlist item speed.
    pub item_speed_b: f32,
}

/// Native score vector retained for Pareto ranking.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoScoreComponents {
    /// Track-position score and rejection component.
    pub base_score: f32,
    /// Fade cuepoint alignment score.
    pub cuepoint_score: f32,
    /// Vocal-collision score.
    pub vocal_score: f32,
    /// Camelot compatibility score.
    pub key_score: f32,
    /// Genre-duration preference score.
    pub genre_score: f32,
}

/// One scored beatmatched candidate before conversion to the result shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoScoredCandidate {
    /// Geometry produced by milestone 1.
    pub geometry: GeneratedOverlapCandidate,
    /// Full component vector used for dominance.
    pub components: AutoScoreComponents,
    /// Native weighted scalar score.
    pub computed_score: f32,
    /// Nondominated layer assigned after per-bar pruning.
    pub pareto_layer: usize,
}

/// Flat overlap shape returned by the pure Auto producer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoTransitionOverlap {
    pub start_a_ms: i64,
    pub start_b_ms: i64,
    pub duration_ms: i64,
    pub duration_bars: usize,
    pub speed_a: f32,
    pub speed_b: f32,
    pub is_beatmatched: bool,
}

/// One deterministic preset recommendation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoRankedPreset {
    pub preset_id: u8,
    pub computed_score: f32,
}

/// One final ranked transition from the pure pipeline.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoRankedTransition {
    pub overlap: AutoTransitionOverlap,
    pub computed_score: f32,
    pub components: Option<AutoScoreComponents>,
    pub pareto_layer: Option<usize>,
    pub ranked_presets: Vec<AutoRankedPreset>,
}

/// Stage counts and final results for one pair.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoPipelineResult {
    pub raw_candidate_count: usize,
    pub base_valid_candidate_count: usize,
    pub per_bar_retained_count: usize,
    pub ranked_transitions: Vec<AutoRankedTransition>,
}

const BAR_ITERATION_ORDER: [usize; 5] = [32, 16, 8, 4, 2];
const ALTERNATE_TIME_MINIMUM_BPM: f32 = 70.0;
const ALTERNATE_TIME_MAXIMUM_BPM: f32 = 180.0;
const BASE_SCORE_REJECTION_THRESHOLD: f32 = 0.40;
const VOCAL_BIN_COUNT: usize = 32;
const VOCAL_COLLISION_THRESHOLD: f32 = 0.20;
const VOCAL_PROBABILITY_SCALE: f32 = 1.0 / 255.0;
const SHORT_TRANSITION_CONCEPTS: [&str; 7] = [
    "spotify:concept:24pGOSaKeoU6bobuwqnMbJ",
    "spotify:concept:3LiuRrANWrpeGfMuENUG04",
    "spotify:concept:08Q9hhtRafUxsXWyH2Bo9V",
    "spotify:concept:6HmfjrBSjLZsidG2E8Iggk",
    "spotify:concept:3hvUlYaIyCcKNGFiVnqZaQ",
    "spotify:concept:0lLE9kiNdPuaXWpH2jhWUj",
    "spotify:concept:4UjCilZFeXnwJOPLmjiE42",
];
const LONG_TRANSITION_CONCEPTS: [&str; 4] = [
    "spotify:concept:4NyuO3zCN13qIP8k4vNS0D",
    "spotify:concept:6Vwq57ZEeDu8TNVzd5b9rC",
    "spotify:concept:1J9pUDx6minjAWzoY3j9jv",
    "spotify:concept:0d5BcmOXdnNbXcgd7KiZrl",
];
const SHORT_PRESET_POOL: [u8; 3] = [1, 10, 19];
const LONG_PRESET_POOL: [u8; 11] = [1, 2, 3, 4, 5, 17, 18, 8, 9, 10, 19];

fn rounded_position_ms(time_seconds: f32) -> i64 {
    (f64::from(time_seconds) * 1000.0).round() as i64
}

fn fixture_position_seconds(position_ms: i64) -> f32 {
    (position_ms as f64 * 0.001_f64) as f32
}

fn native_position_seconds(position_ms: i64) -> f32 {
    position_ms as f32 * 0.001_f32
}

fn alternate_spacing_has_valid_bpm(
    beats: &[AutoBeat],
    current_index: usize,
    later_index: usize,
) -> bool {
    let current_ms = rounded_position_ms(beats[current_index].time_seconds);
    let later_ms = rounded_position_ms(beats[later_index].time_seconds);
    let delta_ms = later_ms - current_ms;
    if delta_ms <= 0 {
        return false;
    }

    let implied_bpm = 240_000.0_f32 / delta_ms as f32;
    (ALTERNATE_TIME_MINIMUM_BPM..=ALTERNATE_TIME_MAXIMUM_BPM).contains(&implied_bpm)
}

/// Converts raw beat-analysis points into the native normalized-downbeat vector.
pub fn normalize_downbeats(
    beats: &[AutoBeat],
    allow_2_4_time_signature: bool,
) -> Vec<AutoDownbeat> {
    let downbeat_indexes: Vec<_> = beats
        .iter()
        .enumerate()
        .filter_map(|(index, beat)| (beat.value == 1).then_some(index))
        .collect();

    let mut normalized = Vec::with_capacity(downbeat_indexes.len());
    for (ordinal, &current_index) in downbeat_indexes.iter().enumerate() {
        let next_index = downbeat_indexes.get(ordinal + 1).copied();
        let second_index = downbeat_indexes.get(ordinal + 2).copied();

        let normally_spaced = next_index.is_some_and(|next| next - current_index == 4);
        let terminal = next_index.is_none();
        let alternate_second = allow_2_4_time_signature
            && second_index.is_some_and(|second| {
                second - current_index == 4
                    && alternate_spacing_has_valid_bpm(beats, current_index, second)
            });
        let alternate_next = allow_2_4_time_signature
            && next_index.is_some_and(|next| {
                next - current_index == 2
                    && alternate_spacing_has_valid_bpm(beats, current_index, next)
            });

        if !(normally_spaced || alternate_second || alternate_next || terminal) {
            continue;
        }

        let beat = beats[current_index];
        let position_ms = rounded_position_ms(beat.time_seconds);
        normalized.push(AutoDownbeat {
            raw_beat_index: current_index,
            position_ms,
            // The durable capture fixture serialized this helper value via
            // JavaScript's double arithmetic. Native window calculations
            // convert the integral milliseconds directly to `float`; see
            // `native_position_seconds` below.
            position_seconds: fixture_position_seconds(position_ms),
            confidence: beat.beat_confidence * beat.downbeat_confidence,
        });
    }

    normalized
}

fn window_statistics(points: &[AutoDownbeat]) -> (f32, f32) {
    let interval_count = points.len() - 1;
    let mut sum = 0.0_f32;
    let mut squared_sum = 0.0_f32;
    for pair in points.windows(2) {
        let interval = native_position_seconds(pair[1].position_ms)
            - native_position_seconds(pair[0].position_ms);
        sum += interval;
        squared_sum += interval * interval;
    }

    let count = interval_count as f32;
    let mean = sum / count;
    let variance = squared_sum / count - mean * mean;
    let minimum_confidence = points
        .iter()
        .map(|point| point.confidence)
        .fold(f32::INFINITY, f32::min);
    (minimum_confidence, variance)
}

/// Enumerates every valid single-track window in native bar iteration order.
pub fn enumerate_windows(
    track: &AutoTrackGeometryInput,
    downbeats: &[AutoDownbeat],
    config: AutoGeometryConfig,
) -> Vec<AutoWindow> {
    let mut windows = Vec::new();

    for duration_bars in BAR_ITERATION_ORDER {
        if duration_bars > config.maximum_bars {
            continue;
        }

        for start_index in 0..downbeats.len() {
            let Some(end_index) = start_index.checked_add(duration_bars) else {
                continue;
            };
            // Corrected segment bounds require a real inclusive endpoint.
            if end_index >= downbeats.len() {
                break;
            }

            let points = &downbeats[start_index..=end_index];
            let (minimum_confidence, interval_variance) = window_statistics(points);
            if minimum_confidence <= config.downbeat_confidence_threshold
                || interval_variance >= config.beat_interval_variance_threshold
            {
                continue;
            }

            let start = downbeats[start_index];
            let end = downbeats[end_index];
            let duration_seconds = native_position_seconds(end.position_ms)
                - native_position_seconds(start.position_ms);
            if duration_seconds >= track.duration_seconds
                || duration_seconds >= config.maximum_duration_seconds
                || duration_seconds >= track.duration_seconds * config.maximum_track_fraction
            {
                continue;
            }

            windows.push(AutoWindow {
                start_index,
                end_index,
                start_ms: start.position_ms,
                end_ms: end.position_ms,
                duration_seconds,
                duration_ms: end.position_ms - start.position_ms,
                duration_bars,
                minimum_confidence,
                interval_variance,
            });
        }
    }

    windows
}

/// Generates the complete raw set of geometrically eligible A/B overlaps.
pub fn generate_overlap_candidates(
    pair: AutoPairGeometryInput<'_>,
    config: AutoGeometryConfig,
) -> Vec<GeneratedOverlapCandidate> {
    if !pair.item_speed_a.is_finite()
        || !pair.item_speed_b.is_finite()
        || pair.item_speed_a <= 0.0
        || pair.item_speed_b <= 0.0
    {
        return Vec::new();
    }

    let downbeats_a = normalize_downbeats(&pair.track_a.beats, config.allow_2_4_time_signature);
    let downbeats_b = normalize_downbeats(&pair.track_b.beats, config.allow_2_4_time_signature);
    let windows_a = enumerate_windows(pair.track_a, &downbeats_a, config);
    let windows_b = enumerate_windows(pair.track_b, &downbeats_b, config);
    let mut candidates = Vec::new();

    for window_a in windows_a {
        for window_b in windows_b
            .iter()
            .filter(|window| window.duration_bars == window_a.duration_bars)
        {
            let native_window_ratio = window_b.duration_seconds / window_a.duration_seconds;
            let effective_window_ratio =
                native_window_ratio * pair.item_speed_a / pair.item_speed_b;
            if (effective_window_ratio - 1.0).abs() > config.maximum_speed_difference {
                continue;
            }

            candidates.push(GeneratedOverlapCandidate {
                start_a_index: window_a.start_index,
                start_b_index: window_b.start_index,
                start_a_ms: window_a.start_ms,
                start_b_ms: window_b.start_ms,
                end_a_ms: window_a.end_ms,
                end_b_ms: window_b.end_ms,
                duration_ms: window_a.duration_ms,
                duration_b_ms: window_b.duration_ms,
                duration_bars: window_a.duration_bars,
                speed_a: 1.0,
                speed_b: native_window_ratio,
                effective_window_ratio,
                interval_variance_a: window_a.interval_variance,
                interval_variance_b: window_b.interval_variance,
                minimum_confidence_a: window_a.minimum_confidence,
                minimum_confidence_b: window_b.minimum_confidence,
            });
        }
    }

    candidates
}

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

/// Calculates the recovered native track-position score.
pub fn base_score(
    candidate: &GeneratedOverlapCandidate,
    duration_a_seconds: f32,
    duration_b_seconds: f32,
) -> f32 {
    let duration_a_seconds = (duration_a_seconds * 1000.0_f32).trunc() * 0.001_f32;
    let duration_b_seconds = (duration_b_seconds * 1000.0_f32).trunc() * 0.001_f32;
    let start_a = native_position_seconds(candidate.start_a_ms);
    let end_b = native_position_seconds(candidate.end_b_ms);
    let diminishing_a = (90.0_f32 / duration_a_seconds).min(0.15);
    let diminishing_b = (90.0_f32 / duration_b_seconds).min(0.15);
    let knee_a = 0.5_f32 + 15.0_f32 / duration_a_seconds;
    let position_b = (0.5_f32 - 15.0_f32 / duration_b_seconds).max(diminishing_b + 0.01_f32);
    let late_a =
        clamp01(((start_a / duration_a_seconds) - knee_a) / ((1.0_f32 - diminishing_a) - knee_a));
    let early_b =
        clamp01(((end_b / duration_b_seconds) - position_b) / (diminishing_b - position_b));
    late_a * early_b
}

fn cuepoint_shape(cuepoint: AutoCuepoint, start_ms: i64, end_ms: i64) -> Option<f32> {
    let cuepoint = native_position_seconds(cuepoint.position_ms);
    let start = native_position_seconds(start_ms);
    let end = native_position_seconds(end_ms);
    let middle = start + (end - start) / 2.0_f32;

    if (cuepoint - middle).abs() < 0.5_f32 {
        Some(1.0)
    } else if (cuepoint - end).abs() < 0.5_f32 {
        Some(0.8)
    } else if (cuepoint - start).abs() < 0.5_f32 {
        Some(0.8)
    } else {
        None
    }
}

fn cuepoint_side_score(
    best: Option<AutoCuepoint>,
    candidates: &[AutoCuepoint],
    track_bpm: f32,
    start_ms: i64,
    end_ms: i64,
) -> f32 {
    let tempo_matches = |cuepoint: AutoCuepoint| {
        track_bpm.is_finite()
            && track_bpm > 0.0
            && (cuepoint.tempo_bpm / track_bpm - 1.0).abs() <= 0.10
    };
    if best.is_some_and(|cuepoint| !tempo_matches(cuepoint)) {
        return 0.0;
    }
    if let Some(shape) = best
        .filter(|&cuepoint| tempo_matches(cuepoint))
        .and_then(|cuepoint| cuepoint_shape(cuepoint, start_ms, end_ms))
    {
        return shape;
    }

    candidates
        .iter()
        .filter(|&&cuepoint| tempo_matches(cuepoint))
        .find_map(|&cuepoint| cuepoint_shape(cuepoint, start_ms, end_ms))
        .map_or(0.0, |shape| shape * 0.95_f32)
}

/// Calculates native fade-out/fade-in cuepoint alignment for one overlap.
pub fn cuepoint_score(
    candidate: &GeneratedOverlapCandidate,
    track_a: &AutoTrackScoringInput,
    track_b: &AutoTrackScoringInput,
) -> f32 {
    let outgoing = cuepoint_side_score(
        track_a.cuepoints.best_fade_out,
        &track_a.cuepoints.fade_out_candidates,
        track_a.bpm,
        candidate.start_a_ms,
        candidate.end_a_ms,
    );
    let incoming = cuepoint_side_score(
        track_b.cuepoints.best_fade_in,
        &track_b.cuepoints.fade_in_candidates,
        track_b.bpm,
        candidate.start_b_ms,
        candidate.end_b_ms,
    );
    (outgoing + incoming) / 2.0_f32
}

fn vocal_bin_average(activity: &AutoVocalActivity, start_ms: i64, end_ms: i64, bin: usize) -> f32 {
    if activity.source_sample_rate_hz == 0
        || activity.samples_between_windows == 0
        || activity.probabilities.is_empty()
        || end_ms <= start_ms
    {
        return 0.0;
    }

    let start_sample = start_ms as f64 * f64::from(activity.source_sample_rate_hz) / 1000.0;
    let end_sample = end_ms as f64 * f64::from(activity.source_sample_rate_hz) / 1000.0;
    let span = end_sample - start_sample;
    let bin_start = start_sample + span * bin as f64 / VOCAL_BIN_COUNT as f64;
    let bin_end = start_sample + span * (bin + 1) as f64 / VOCAL_BIN_COUNT as f64;
    let first = activity.first_window_sample_start as f64;
    let step = f64::from(activity.samples_between_windows);
    let first_index = ((bin_start - first) / step).ceil().max(0.0) as usize;
    let end_index = ((bin_end - first) / step).ceil().max(0.0) as usize;
    let end_index = end_index.min(activity.probabilities.len());
    if first_index >= end_index {
        return 0.0;
    }

    let mut sum = 0.0_f32;
    for probability in &activity.probabilities[first_index..end_index] {
        sum += f32::from(*probability) * VOCAL_PROBABILITY_SCALE;
    }
    sum / (end_index - first_index) as f32
}

/// Calculates the recovered 32-bin vocal-collision score.
pub fn vocal_score(
    candidate: &GeneratedOverlapCandidate,
    vocal_a: &AutoVocalActivity,
    vocal_b: &AutoVocalActivity,
) -> f32 {
    let mut collisions = 0_u32;
    for bin in 0..VOCAL_BIN_COUNT {
        let outgoing = vocal_bin_average(vocal_a, candidate.start_a_ms, candidate.end_a_ms, bin);
        let incoming = vocal_bin_average(vocal_b, candidate.start_b_ms, candidate.end_b_ms, bin);
        if outgoing >= VOCAL_COLLISION_THRESHOLD && incoming >= VOCAL_COLLISION_THRESHOLD {
            collisions += 1;
        }
    }

    (1.0_f32 - collisions as f32 / VOCAL_BIN_COUNT as f32).powf(1.0)
}

fn camelot_distance(a: u8, b: u8) -> u8 {
    let direct = a.abs_diff(b);
    direct.min(12 - direct)
}

/// Calculates the recovered Camelot compatibility score.
pub fn key_score(
    key_a: Option<AutoCamelotKey>,
    key_b: Option<AutoCamelotKey>,
    duration_bars: usize,
) -> f32 {
    let compatible = match (key_a, key_b) {
        (Some(a), Some(b)) => {
            let distance = camelot_distance(a.number, b.number);
            let same_letter = a.letter == b.letter;
            (same_letter && distance <= 1)
                || (!same_letter && distance == 0)
                || (!same_letter && distance == 1)
        }
        _ => true,
    };
    if compatible {
        1.0
    } else if duration_bars <= 4 {
        0.9
    } else if duration_bars <= 8 {
        0.8
    } else {
        0.6
    }
}

fn has_concept(track: &AutoTrackScoringInput, concepts: &[&str]) -> bool {
    track
        .genre_concept_uris
        .iter()
        .any(|uri| concepts.contains(&uri.as_str()))
}

/// Calculates the recovered genre-duration preference score.
pub fn genre_score(
    track_a: &AutoTrackScoringInput,
    track_b: &AutoTrackScoringInput,
    duration_bars: usize,
) -> f32 {
    if has_concept(track_a, &SHORT_TRANSITION_CONCEPTS)
        || has_concept(track_b, &SHORT_TRANSITION_CONCEPTS)
    {
        if duration_bars <= 4 {
            1.0
        } else if duration_bars <= 8 {
            0.8
        } else {
            0.6
        }
    } else if has_concept(track_a, &LONG_TRANSITION_CONCEPTS)
        && has_concept(track_b, &LONG_TRANSITION_CONCEPTS)
    {
        if duration_bars < 8 {
            0.6
        } else if duration_bars < 16 {
            0.8
        } else {
            1.0
        }
    } else {
        1.0
    }
}

/// Scores one geometrically eligible candidate, rejecting low base scores.
pub fn score_candidate(
    candidate: GeneratedOverlapCandidate,
    pair: AutoPairScoringInput<'_>,
) -> Option<AutoScoredCandidate> {
    let base_score = base_score(
        &candidate,
        pair.track_a.scoring_duration_seconds,
        pair.track_b.scoring_duration_seconds,
    );
    if base_score < BASE_SCORE_REJECTION_THRESHOLD {
        return None;
    }
    let components = AutoScoreComponents {
        base_score,
        cuepoint_score: cuepoint_score(&candidate, pair.track_a, pair.track_b),
        vocal_score: vocal_score(
            &candidate,
            &pair.track_a.vocal_activity,
            &pair.track_b.vocal_activity,
        ),
        key_score: key_score(
            pair.track_a.camelot_key,
            pair.track_b.camelot_key,
            candidate.duration_bars,
        ),
        genre_score: genre_score(pair.track_a, pair.track_b, candidate.duration_bars),
    };
    let component_sum = components.cuepoint_score
        + (components.vocal_score + (components.key_score + components.genre_score));
    Some(AutoScoredCandidate {
        geometry: candidate,
        components,
        computed_score: components.base_score * component_sum,
        pareto_layer: usize::MAX,
    })
}

fn initial_candidate_order(a: &AutoScoredCandidate, b: &AutoScoredCandidate) -> Ordering {
    b.geometry
        .duration_bars
        .cmp(&a.geometry.duration_bars)
        .then_with(|| b.computed_score.total_cmp(&a.computed_score))
}

fn dominates(a: AutoScoreComponents, b: AutoScoreComponents) -> bool {
    let a = [
        a.base_score,
        a.cuepoint_score,
        a.vocal_score,
        a.key_score,
        a.genre_score,
    ];
    let b = [
        b.base_score,
        b.cuepoint_score,
        b.vocal_score,
        b.key_score,
        b.genre_score,
    ];
    a.iter().zip(b).all(|(a, b)| *a >= b) && a.iter().zip(b).any(|(a, b)| *a > b)
}

fn assign_pareto_layers(candidates: &mut [AutoScoredCandidate]) {
    let mut remaining: Vec<_> = (0..candidates.len()).collect();
    let mut layer = 0;
    while !remaining.is_empty() {
        let nondominated: Vec<_> = remaining
            .iter()
            .copied()
            .filter(|&candidate| {
                !remaining.iter().copied().any(|other| {
                    other != candidate
                        && dominates(
                            candidates[other].components,
                            candidates[candidate].components,
                        )
                })
            })
            .collect();
        for &candidate in &nondominated {
            candidates[candidate].pareto_layer = layer;
        }
        remaining.retain(|candidate| !nondominated.contains(candidate));
        layer += 1;
    }
}

fn final_candidate_order(a: &AutoScoredCandidate, b: &AutoScoredCandidate) -> Ordering {
    a.pareto_layer
        .cmp(&b.pareto_layer)
        .then_with(|| b.computed_score.total_cmp(&a.computed_score))
        .then_with(|| b.geometry.duration_bars.cmp(&a.geometry.duration_bars))
}

fn preset_hash_milliseconds(position_ms: i64) -> i32 {
    (native_position_seconds(position_ms) * 1000.0_f32).trunc() as i32
}

/// Builds the complete deterministic native preset order.
pub fn rank_presets(
    track_a_playable_uri: &str,
    track_b_playable_uri: &str,
    overlap: AutoTransitionOverlap,
) -> Vec<AutoRankedPreset> {
    let selected = if overlap.is_beatmatched {
        let pool: &[u8] = if overlap.duration_bars < 4 {
            &SHORT_PRESET_POOL
        } else {
            &LONG_PRESET_POOL
        };
        let mut hasher = Sha1::new();
        hasher.update(track_a_playable_uri.as_bytes());
        hasher.update(track_b_playable_uri.as_bytes());
        hasher.update((overlap.duration_bars as i32).to_le_bytes());
        hasher.update(preset_hash_milliseconds(overlap.start_a_ms).to_le_bytes());
        hasher.update(preset_hash_milliseconds(overlap.start_b_ms).to_le_bytes());
        let digest = hasher.finalize();
        let selector = u64::from_le_bytes(digest[..8].try_into().expect("SHA-1 prefix length"));
        pool[selector as usize % pool.len()]
    } else {
        1
    };

    let mut ranked = Vec::with_capacity(23);
    ranked.push(AutoRankedPreset {
        preset_id: selected,
        computed_score: 1.0,
    });
    ranked.extend(
        (0_u8..=22)
            .filter(|&id| id != selected)
            .map(|preset_id| AutoRankedPreset {
                preset_id,
                computed_score: 0.5,
            }),
    );
    ranked
}

fn beatmatched_result(
    candidate: AutoScoredCandidate,
    pair: AutoPairScoringInput<'_>,
) -> AutoRankedTransition {
    let overlap = AutoTransitionOverlap {
        start_a_ms: candidate.geometry.start_a_ms,
        start_b_ms: candidate.geometry.start_b_ms,
        duration_ms: candidate.geometry.duration_ms,
        duration_bars: candidate.geometry.duration_bars,
        speed_a: candidate.geometry.speed_a,
        speed_b: candidate.geometry.speed_b,
        is_beatmatched: true,
    };
    AutoRankedTransition {
        overlap,
        computed_score: candidate.computed_score,
        components: Some(candidate.components),
        pareto_layer: Some(candidate.pareto_layer),
        ranked_presets: rank_presets(
            &pair.track_a.playable_uri,
            &pair.track_b.playable_uri,
            overlap,
        ),
    }
}

fn fallback_result(pair: AutoPairScoringInput<'_>) -> AutoRankedTransition {
    const DURATION_MS: i64 = 5_000;
    const HALF_DURATION_MS: i64 = DURATION_MS / 2;
    let outgoing_midpoint = pair
        .track_a
        .cuepoints
        .fade_out_candidates
        .iter()
        .map(|cuepoint| cuepoint.position_ms)
        .max()
        .or_else(|| {
            pair.track_a
                .cuepoints
                .best_fade_out
                .map(|cuepoint| cuepoint.position_ms)
        })
        .unwrap_or_else(|| (pair.track_a.geometry.duration_seconds * 1000.0_f32).trunc() as i64);
    let incoming_midpoint = pair
        .track_b
        .cuepoints
        .best_fade_in
        .map_or(HALF_DURATION_MS, |cuepoint| cuepoint.position_ms);
    let overlap = AutoTransitionOverlap {
        start_a_ms: (outgoing_midpoint - HALF_DURATION_MS).max(0),
        start_b_ms: (incoming_midpoint - HALF_DURATION_MS).max(0),
        duration_ms: DURATION_MS,
        duration_bars: 0,
        speed_a: 1.0,
        speed_b: 1.0,
        is_beatmatched: false,
    };
    AutoRankedTransition {
        overlap,
        computed_score: 0.0,
        components: None,
        pareto_layer: None,
        ranked_presets: rank_presets(
            &pair.track_a.playable_uri,
            &pair.track_b.playable_uri,
            overlap,
        ),
    }
}

/// Runs pure geometry, scoring, pruning, Pareto ranking, and preset selection.
pub fn generate_ranked_transitions(
    pair: AutoPairScoringInput<'_>,
    geometry_config: AutoGeometryConfig,
) -> AutoPipelineResult {
    let low_beatmatchability = pair.track_a.genre_based_beatmatchability < 0.5_f32
        && pair.track_b.genre_based_beatmatchability < 0.5_f32;
    let raw = if low_beatmatchability {
        Vec::new()
    } else {
        generate_overlap_candidates(
            AutoPairGeometryInput {
                track_a: &pair.track_a.geometry,
                track_b: &pair.track_b.geometry,
                item_speed_a: pair.item_speed_a,
                item_speed_b: pair.item_speed_b,
            },
            geometry_config,
        )
    };
    let raw_candidate_count = raw.len();
    let mut scored: Vec<_> = raw
        .into_iter()
        .filter_map(|candidate| score_candidate(candidate, pair))
        .collect();
    let base_valid_candidate_count = scored.len();
    scored.sort_by(initial_candidate_order);

    let mut retained = Vec::with_capacity(20);
    let mut current_bars = None;
    let mut retained_for_bars = 0;
    for candidate in scored {
        if current_bars != Some(candidate.geometry.duration_bars) {
            current_bars = Some(candidate.geometry.duration_bars);
            retained_for_bars = 0;
        }
        if retained_for_bars < 5 {
            retained.push(candidate);
            retained_for_bars += 1;
        }
    }
    let per_bar_retained_count = retained.len();
    assign_pareto_layers(&mut retained);
    retained.sort_by(final_candidate_order);
    let ranked_transitions = if retained.is_empty() {
        vec![fallback_result(pair)]
    } else {
        retained
            .into_iter()
            .map(|candidate| beatmatched_result(candidate, pair))
            .collect()
    };

    AutoPipelineResult {
        raw_candidate_count,
        base_valid_candidate_count,
        per_bar_retained_count,
        ranked_transitions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beat(time_seconds: f32, value: i32) -> AutoBeat {
        AutoBeat {
            time_seconds,
            value,
            beat_confidence: 1.0,
            downbeat_confidence: (value == 1) as u8 as f32,
        }
    }

    fn points(times: &[f32]) -> Vec<AutoDownbeat> {
        times
            .iter()
            .enumerate()
            .map(|(index, &time)| AutoDownbeat {
                raw_beat_index: index * 4,
                position_ms: (time * 1000.0).round() as i64,
                position_seconds: time,
                confidence: 1.0,
            })
            .collect()
    }

    fn track(duration_seconds: f32, times: &[f32]) -> AutoTrackGeometryInput {
        let mut beats = Vec::new();
        for &time in times {
            beats.extend([
                beat(time, 1),
                beat(time + 0.25, 2),
                beat(time + 0.50, 3),
                beat(time + 0.75, 4),
            ]);
        }
        AutoTrackGeometryInput {
            duration_seconds,
            beats,
        }
    }

    fn scoring_track() -> AutoTrackScoringInput {
        AutoTrackScoringInput {
            geometry: AutoTrackGeometryInput {
                duration_seconds: 300.0,
                beats: Vec::new(),
            },
            scoring_duration_seconds: 300.0,
            playable_uri: "spotify:track:test".to_owned(),
            cuepoints: AutoCuepoints::default(),
            vocal_activity: AutoVocalActivity::default(),
            camelot_key: None,
            bpm: 120.0,
            genre_concept_uris: Vec::new(),
            mixable: true,
            genre_based_beatmatchability: 1.0,
        }
    }

    fn candidate(start_a_ms: i64, start_b_ms: i64, duration_ms: i64) -> GeneratedOverlapCandidate {
        GeneratedOverlapCandidate {
            start_a_index: 0,
            start_b_index: 0,
            start_a_ms,
            start_b_ms,
            end_a_ms: start_a_ms + duration_ms,
            end_b_ms: start_b_ms + duration_ms,
            duration_ms,
            duration_b_ms: duration_ms,
            duration_bars: 4,
            speed_a: 1.0,
            speed_b: 1.0,
            effective_window_ratio: 1.0,
            interval_variance_a: 0.0,
            interval_variance_b: 0.0,
            minimum_confidence_a: 1.0,
            minimum_confidence_b: 1.0,
        }
    }

    #[test]
    fn normal_four_beat_spacing_and_terminal_are_retained() {
        let raw = track(20.0, &[0.0, 2.0, 4.0]).beats;
        let normalized = normalize_downbeats(&raw, true);
        assert_eq!(
            normalized
                .iter()
                .map(|point| point.position_ms)
                .collect::<Vec<_>>(),
            [0, 2000, 4000]
        );
    }

    #[test]
    fn alternate_two_four_spacing_is_supported() {
        let raw = vec![beat(0.0, 1), beat(1.0, 2), beat(2.0, 1)];
        assert_eq!(normalize_downbeats(&raw, false).len(), 1);
        assert_eq!(normalize_downbeats(&raw, true).len(), 2);
    }

    #[test]
    fn confidence_uses_float_product() {
        let raw = [AutoBeat {
            time_seconds: 1.0,
            value: 1,
            beat_confidence: 0.7,
            downbeat_confidence: 0.6,
        }];
        let normalized = normalize_downbeats(&raw, true);
        assert_eq!(
            normalized[0].confidence.to_bits(),
            (0.7_f32 * 0.6).to_bits()
        );
    }

    #[test]
    fn confidence_threshold_is_strictly_greater() {
        let track = AutoTrackGeometryInput {
            duration_seconds: 100.0,
            beats: Vec::new(),
        };
        let mut downbeats = points(&[0.0, 1.0, 2.0]);
        downbeats[1].confidence = 0.40;
        assert!(enumerate_windows(&track, &downbeats, AutoGeometryConfig::default()).is_empty());
    }

    #[test]
    fn interval_variance_threshold_is_enforced() {
        let track = AutoTrackGeometryInput {
            duration_seconds: 100.0,
            beats: Vec::new(),
        };
        let downbeats = points(&[0.0, 1.0, 3.0]);
        assert!(enumerate_windows(&track, &downbeats, AutoGeometryConfig::default()).is_empty());
    }

    #[test]
    fn maximum_bars_is_enforced() {
        let track = AutoTrackGeometryInput {
            duration_seconds: 1000.0,
            beats: Vec::new(),
        };
        let times: Vec<_> = (0..=32).map(|value| value as f32).collect();
        let windows = enumerate_windows(&track, &points(&times), AutoGeometryConfig::default());
        assert!(windows.iter().all(|window| window.duration_bars <= 16));
        assert!(windows.iter().any(|window| window.duration_bars == 16));
        assert!(!windows.iter().any(|window| window.duration_bars == 32));
    }

    #[test]
    fn maximum_seconds_is_enforced() {
        let track = AutoTrackGeometryInput {
            duration_seconds: 1000.0,
            beats: Vec::new(),
        };
        assert!(
            enumerate_windows(
                &track,
                &points(&[0.0, 20.0, 40.0]),
                AutoGeometryConfig::default(),
            )
            .is_empty()
        );
    }

    #[test]
    fn maximum_track_fraction_is_enforced() {
        let track = AutoTrackGeometryInput {
            duration_seconds: 100.0,
            beats: Vec::new(),
        };
        assert!(
            enumerate_windows(
                &track,
                &points(&[0.0, 10.0, 20.0]),
                AutoGeometryConfig::default(),
            )
            .is_empty()
        );
    }

    #[test]
    fn item_speed_changes_eligibility_not_emitted_speed() {
        let track_a = track(100.0, &[0.0, 1.0, 2.0]);
        let track_b = track(100.0, &[0.0, 1.05, 2.10]);
        let config = AutoGeometryConfig::default();
        let baseline = generate_overlap_candidates(
            AutoPairGeometryInput {
                track_a: &track_a,
                track_b: &track_b,
                item_speed_a: 1.0,
                item_speed_b: 1.0,
            },
            config,
        );
        let adjusted = generate_overlap_candidates(
            AutoPairGeometryInput {
                track_a: &track_a,
                track_b: &track_b,
                item_speed_a: 1.0,
                item_speed_b: 1.2,
            },
            config,
        );

        assert_eq!(baseline.len(), 1);
        assert!(adjusted.is_empty());
        assert_eq!(baseline[0].speed_a, 1.0);
        let expected_speed_b = (2_100_f32 * 0.001_f32) / (2_000_f32 * 0.001_f32);
        assert_eq!(baseline[0].speed_b.to_bits(), expected_speed_b.to_bits());
    }

    #[test]
    fn camelot_parsing_and_compatibility_tiers_match_native_rules() {
        let key_12a = AutoCamelotKey::parse("12A").unwrap();
        let key_1a = AutoCamelotKey::parse("1A").unwrap();
        let key_1b = AutoCamelotKey::parse("1B").unwrap();
        let key_3a = AutoCamelotKey::parse("3A").unwrap();
        assert!(AutoCamelotKey::parse("13A").is_none());
        assert!(AutoCamelotKey::parse("é").is_none());
        assert_eq!(key_score(Some(key_12a), Some(key_1a), 16), 1.0);
        assert_eq!(key_score(Some(key_1a), Some(key_1b), 16), 1.0);
        assert_eq!(key_score(Some(key_12a), Some(key_1b), 16), 1.0);
        assert_eq!(key_score(Some(key_1a), Some(key_3a), 4), 0.9);
        assert_eq!(key_score(Some(key_1a), Some(key_3a), 8), 0.8);
        assert_eq!(key_score(Some(key_1a), Some(key_3a), 16), 0.6);
        assert_eq!(key_score(None, Some(key_3a), 16), 1.0);
    }

    #[test]
    fn cuepoints_use_best_then_first_compatible_candidate() {
        let best = AutoCuepoint {
            position_ms: 5_000,
            tempo_bpm: 120.0,
        };
        let candidates = [
            AutoCuepoint {
                position_ms: 10_000,
                tempo_bpm: 120.0,
            },
            AutoCuepoint {
                position_ms: 5_000,
                tempo_bpm: 120.0,
            },
        ];
        assert_eq!(
            cuepoint_side_score(Some(best), &candidates, 120.0, 0, 10_000),
            1.0
        );
        assert_eq!(
            cuepoint_side_score(None, &candidates, 120.0, 0, 10_000),
            0.76
        );

        let incompatible_best = AutoCuepoint {
            tempo_bpm: 90.0,
            ..best
        };
        assert_eq!(
            cuepoint_side_score(Some(incompatible_best), &candidates, 120.0, 0, 10_000),
            0.0
        );
    }

    #[test]
    fn vocal_score_uses_thirty_two_collision_bins() {
        let activity = AutoVocalActivity {
            source_sample_rate_hz: 1_000,
            smoothing_window_size: 1_000,
            first_window_sample_start: 0,
            samples_between_windows: 1_000,
            probabilities: vec![255; 32],
        };
        let silent = AutoVocalActivity {
            probabilities: vec![0; 32],
            ..activity.clone()
        };
        let overlap = candidate(0, 0, 32_000);
        assert_eq!(vocal_score(&overlap, &activity, &activity), 0.0);
        assert_eq!(vocal_score(&overlap, &activity, &silent), 1.0);
    }

    #[test]
    fn genre_groups_select_the_recovered_duration_shape() {
        let mut short = scoring_track();
        short.genre_concept_uris = vec![SHORT_TRANSITION_CONCEPTS[0].to_owned()];
        let mut long_a = scoring_track();
        let mut long_b = scoring_track();
        long_a.genre_concept_uris = vec![LONG_TRANSITION_CONCEPTS[0].to_owned()];
        long_b.genre_concept_uris = vec![LONG_TRANSITION_CONCEPTS[1].to_owned()];
        let neutral = scoring_track();

        assert_eq!(genre_score(&short, &neutral, 4), 1.0);
        assert_eq!(genre_score(&short, &neutral, 8), 0.8);
        assert_eq!(genre_score(&short, &neutral, 16), 0.6);
        assert_eq!(genre_score(&long_a, &long_b, 4), 0.6);
        assert_eq!(genre_score(&long_a, &long_b, 8), 0.8);
        assert_eq!(genre_score(&long_a, &long_b, 16), 1.0);
        assert_eq!(genre_score(&long_a, &neutral, 16), 1.0);
    }

    #[test]
    fn pareto_layers_peel_complete_score_vectors() {
        let geometry = candidate(0, 0, 10_000);
        let mut candidates = [
            AutoScoredCandidate {
                geometry,
                components: AutoScoreComponents {
                    base_score: 1.0,
                    cuepoint_score: 1.0,
                    vocal_score: 1.0,
                    key_score: 1.0,
                    genre_score: 1.0,
                },
                computed_score: 4.0,
                pareto_layer: usize::MAX,
            },
            AutoScoredCandidate {
                geometry,
                components: AutoScoreComponents {
                    base_score: 0.9,
                    cuepoint_score: 1.0,
                    vocal_score: 1.0,
                    key_score: 1.0,
                    genre_score: 1.0,
                },
                computed_score: 3.9,
                pareto_layer: usize::MAX,
            },
            AutoScoredCandidate {
                geometry,
                components: AutoScoreComponents {
                    base_score: 1.0,
                    cuepoint_score: 0.9,
                    vocal_score: 1.0,
                    key_score: 1.0,
                    genre_score: 1.0,
                },
                computed_score: 3.9,
                pareto_layer: usize::MAX,
            },
        ];
        assign_pareto_layers(&mut candidates);
        assert_eq!(candidates[0].pareto_layer, 0);
        assert_eq!(candidates[1].pareto_layer, 1);
        assert_eq!(candidates[2].pareto_layer, 1);
    }
}
