//! Pure Spotify Auto Mix overlap-geometry generation.
//!
//! This module intentionally contains no metadata retrieval, playback,
//! scoring, ranking, preset selection, or rendering behavior.

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

const BAR_ITERATION_ORDER: [usize; 5] = [32, 16, 8, 4, 2];
const ALTERNATE_TIME_MINIMUM_BPM: f32 = 70.0;
const ALTERNATE_TIME_MAXIMUM_BPM: f32 = 180.0;

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
}
