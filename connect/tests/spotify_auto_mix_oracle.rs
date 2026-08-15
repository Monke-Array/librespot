use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use librespot_connect::spotify_auto_mix::{
    AutoBeat, AutoCamelotKey, AutoCuepoint, AutoCuepoints, AutoGeometryConfig,
    AutoPairGeometryInput, AutoPairScoringInput, AutoTrackGeometryInput, AutoTrackScoringInput,
    AutoTransitionOverlap, AutoVocalActivity, GeneratedOverlapCandidate, enumerate_windows,
    generate_overlap_candidates, generate_ranked_transitions, normalize_downbeats, rank_presets,
    score_candidate,
};
use serde_json::Value;

const SPEED_TOLERANCE: f64 = 7.629_394_531_25e-6;
const MAX_OBSERVED_SCORE_DIFFERENCE: f64 = 7.152_557_373_046_875e-7;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct GeometryKey {
    start_a_ms: i64,
    start_b_ms: i64,
    duration_bars: usize,
    duration_ms: i64,
}

impl From<&GeneratedOverlapCandidate> for GeometryKey {
    fn from(candidate: &GeneratedOverlapCandidate) -> Self {
        Self {
            start_a_ms: candidate.start_a_ms,
            start_b_ms: candidate.start_b_ms,
            duration_bars: candidate.duration_bars,
            duration_ms: candidate.duration_ms,
        }
    }
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tools/spotify-automix-oracle")
}

fn read_json(path: impl AsRef<Path>) -> Value {
    let path = path.as_ref();
    serde_json::from_str(
        &fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("failed to read fixture {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("failed to parse fixture {}: {error}", path.display()))
}

fn required<'a>(value: &'a Value, key: &str) -> &'a Value {
    value
        .get(key)
        .unwrap_or_else(|| panic!("fixture field {key:?} is missing from {value}"))
}

fn number(value: &Value) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("expected fixture number, got {value}"))
}

fn integer(value: &Value) -> i64 {
    value
        .as_i64()
        .unwrap_or_else(|| panic!("expected fixture integer, got {value}"))
}

fn string(value: &Value) -> &str {
    value
        .as_str()
        .unwrap_or_else(|| panic!("expected fixture string, got {value}"))
}

fn load_track(uri: &str) -> (Value, AutoTrackGeometryInput) {
    let id = uri
        .rsplit(':')
        .next()
        .unwrap_or_else(|| panic!("invalid fixture track URI {uri}"));
    let fixture = read_json(fixture_root().join("tracks").join(format!("{id}.json")));
    let beats = required(required(required(&fixture, "decoded"), "beats"), "beats")
        .as_array()
        .expect("decoded beats must be an array")
        .iter()
        .map(|beat| AutoBeat {
            time_seconds: number(required(beat, "timeSeconds")) as f32,
            value: integer(required(beat, "value")) as i32,
            beat_confidence: number(required(beat, "beatConfidence")) as f32,
            downbeat_confidence: number(required(beat, "downbeatConfidence")) as f32,
        })
        .collect();
    let input = AutoTrackGeometryInput {
        duration_seconds: number(required(required(&fixture, "duration"), "seconds")) as f32,
        beats,
    };
    (fixture, input)
}

fn load_manifest_tracks() -> (Value, HashMap<String, (Value, AutoTrackGeometryInput)>) {
    let manifest = read_json(fixture_root().join("manifest-2026-08-14.json"));
    let mut tracks = HashMap::new();
    for track in required(&manifest, "tracks")
        .as_array()
        .expect("manifest tracks must be an array")
    {
        let uri = string(required(track, "canonicalTrackUri"));
        tracks.insert(uri.to_owned(), load_track(uri));
    }
    (manifest, tracks)
}

fn optional_cuepoint(value: &Value) -> Option<AutoCuepoint> {
    (!value.is_null()).then(|| AutoCuepoint {
        position_ms: integer(required(value, "positionMs")),
        tempo_bpm: number(required(value, "tempoBpm")) as f32,
    })
}

fn cuepoint_list(value: &Value) -> Vec<AutoCuepoint> {
    value
        .as_array()
        .expect("cuepoint candidates must be an array")
        .iter()
        .map(|cuepoint| AutoCuepoint {
            position_ms: integer(required(cuepoint, "positionMs")),
            tempo_bpm: number(required(cuepoint, "tempoBpm")) as f32,
        })
        .collect()
}

fn scoring_input(fixture: &Value, geometry: AutoTrackGeometryInput) -> AutoTrackScoringInput {
    let decoded = required(fixture, "decoded");
    let cuepoints = required(decoded, "cuepoints");
    let vocal = required(decoded, "vocalActivity");
    let audio_attributes = required(decoded, "audioAttributesV2");
    let camelot = audio_attributes
        .get("key")
        .and_then(|key| key.get("camelotKey"))
        .and_then(|camelot| camelot.get("value"))
        .and_then(Value::as_str)
        .and_then(AutoCamelotKey::parse);
    let genre_concept_uris = required(decoded, "trackDescriptors")
        .as_array()
        .expect("track descriptors must be an array")
        .iter()
        .filter(|descriptor| {
            required(descriptor, "types")
                .as_array()
                .expect("descriptor types must be an array")
                .iter()
                .any(|kind| integer(kind) == 1)
        })
        .map(|descriptor| string(required(descriptor, "conceptUri")).to_owned())
        .collect();
    let mixability = required(decoded, "mixability");

    AutoTrackScoringInput {
        scoring_duration_seconds: required(fixture, "duration")
            .get("scoringSeconds")
            .map_or_else(
                || number(required(required(fixture, "duration"), "seconds")) as f32,
                |duration| number(duration) as f32,
            ),
        geometry,
        playable_uri: string(required(required(fixture, "identity"), "playableTrackUri"))
            .to_owned(),
        cuepoints: AutoCuepoints {
            best_fade_in: optional_cuepoint(required(cuepoints, "bestFadeIn")),
            best_fade_out: optional_cuepoint(required(cuepoints, "bestFadeOut")),
            fade_in_candidates: cuepoint_list(required(cuepoints, "fadeInCandidates")),
            fade_out_candidates: cuepoint_list(required(cuepoints, "fadeOutCandidates")),
        },
        vocal_activity: AutoVocalActivity {
            source_sample_rate_hz: integer(required(vocal, "sourceSampleRateHz")) as u32,
            smoothing_window_size: integer(required(vocal, "smoothingWindowSize")) as u32,
            first_window_sample_start: integer(required(vocal, "firstWindowSampleStart")),
            samples_between_windows: integer(required(vocal, "samplesBetweenWindows")) as u32,
            probabilities: required(vocal, "probabilities")
                .as_array()
                .expect("vocal probabilities must be an array")
                .iter()
                .map(|probability| integer(probability) as u8)
                .collect(),
        },
        camelot_key: camelot,
        bpm: number(required(audio_attributes, "bpm")) as f32,
        genre_concept_uris,
        mixable: required(mixability, "mixable").as_bool().unwrap(),
        genre_based_beatmatchability: number(required(mixability, "genreBasedBeatmatchability"))
            as f32,
    }
}

fn load_scoring_tracks(manifest: &Value) -> HashMap<String, AutoTrackScoringInput> {
    required(manifest, "tracks")
        .as_array()
        .expect("manifest tracks must be an array")
        .iter()
        .map(|track| {
            let uri = string(required(track, "canonicalTrackUri"));
            let (fixture, geometry) = load_track(uri);
            (uri.to_owned(), scoring_input(&fixture, geometry))
        })
        .collect()
}

fn expected_geometry(overlap: &Value) -> GeometryKey {
    GeometryKey {
        start_a_ms: integer(required(overlap, "startAMs")),
        start_b_ms: integer(required(overlap, "startBMs")),
        duration_bars: integer(required(overlap, "durationBars")) as usize,
        duration_ms: integer(required(overlap, "durationMs")),
    }
}

fn candidate_map(
    candidates: &[GeneratedOverlapCandidate],
) -> HashMap<GeometryKey, &GeneratedOverlapCandidate> {
    candidates
        .iter()
        .map(|candidate| (GeometryKey::from(candidate), candidate))
        .collect()
}

fn closest_candidates(
    expected: GeometryKey,
    candidates: &[GeneratedOverlapCandidate],
) -> Vec<GeometryKey> {
    let mut closest: Vec<_> = candidates
        .iter()
        .filter(|candidate| candidate.duration_bars == expected.duration_bars)
        .map(GeometryKey::from)
        .collect();
    closest.sort_by_key(|candidate| {
        candidate.start_a_ms.abs_diff(expected.start_a_ms)
            + candidate.start_b_ms.abs_diff(expected.start_b_ms)
            + candidate.duration_ms.abs_diff(expected.duration_ms)
    });
    closest.truncate(5);
    closest
}

fn assert_candidate_invariants(
    pair_label: &str,
    track_a: &AutoTrackGeometryInput,
    track_b: &AutoTrackGeometryInput,
    item_speed_a: f32,
    item_speed_b: f32,
    candidates: &[GeneratedOverlapCandidate],
    config: AutoGeometryConfig,
) {
    let downbeats_a = normalize_downbeats(&track_a.beats, config.allow_2_4_time_signature);
    let downbeats_b = normalize_downbeats(&track_b.beats, config.allow_2_4_time_signature);
    let windows_a = enumerate_windows(track_a, &downbeats_a, config);
    let windows_b = enumerate_windows(track_b, &downbeats_b, config);
    let windows_a: HashMap<_, _> = windows_a
        .iter()
        .map(|window| ((window.start_ms, window.duration_bars), window))
        .collect();
    let windows_b: HashMap<_, _> = windows_b
        .iter()
        .map(|window| ((window.start_ms, window.duration_bars), window))
        .collect();

    for candidate in candidates {
        let window_a = windows_a
            .get(&(candidate.start_a_ms, candidate.duration_bars))
            .unwrap_or_else(|| {
                panic!("{pair_label}: candidate has no valid A window: {candidate:?}")
            });
        let window_b = windows_b
            .get(&(candidate.start_b_ms, candidate.duration_bars))
            .unwrap_or_else(|| {
                panic!("{pair_label}: candidate has no valid B window: {candidate:?}")
            });

        assert_eq!(
            candidate.start_a_index, window_a.start_index,
            "{pair_label}"
        );
        assert_eq!(
            candidate.start_b_index, window_b.start_index,
            "{pair_label}"
        );
        assert_eq!(candidate.end_a_ms, window_a.end_ms, "{pair_label}");
        assert_eq!(candidate.end_b_ms, window_b.end_ms, "{pair_label}");
        assert_eq!(candidate.duration_ms, window_a.duration_ms, "{pair_label}");
        assert_eq!(
            candidate.duration_b_ms, window_b.duration_ms,
            "{pair_label}"
        );
        assert!(
            candidate.duration_bars <= config.maximum_bars,
            "{pair_label}"
        );
        assert!(candidate.minimum_confidence_a > config.downbeat_confidence_threshold);
        assert!(candidate.minimum_confidence_b > config.downbeat_confidence_threshold);
        assert!(candidate.interval_variance_a < config.beat_interval_variance_threshold);
        assert!(candidate.interval_variance_b < config.beat_interval_variance_threshold);
        assert!(window_a.duration_seconds < config.maximum_duration_seconds);
        assert!(window_b.duration_seconds < config.maximum_duration_seconds);
        assert!(
            window_a.duration_seconds < track_a.duration_seconds * config.maximum_track_fraction
        );
        assert!(
            window_b.duration_seconds < track_b.duration_seconds * config.maximum_track_fraction
        );
        assert_eq!(candidate.speed_a, 1.0);
        assert_eq!(
            candidate.speed_b.to_bits(),
            (window_b.duration_seconds / window_a.duration_seconds).to_bits()
        );
        let effective = candidate.speed_b * item_speed_a / item_speed_b;
        assert_eq!(
            candidate.effective_window_ratio.to_bits(),
            effective.to_bits()
        );
        assert!((effective - 1.0).abs() <= config.maximum_speed_difference);
    }
}

#[test]
fn all_normalized_downbeat_fixtures_match() {
    let (manifest, tracks) = load_manifest_tracks();
    let mut checked = 0;

    for track_manifest in required(&manifest, "tracks").as_array().unwrap() {
        let uri = string(required(track_manifest, "canonicalTrackUri"));
        let (fixture, track) = tracks.get(uri).unwrap();
        let generated = normalize_downbeats(&track.beats, true);
        let expected = required(required(fixture, "decoded"), "normalizedDownbeats")
            .as_array()
            .unwrap();
        assert_eq!(generated.len(), expected.len(), "{uri}");
        for (index, (generated, expected)) in generated.iter().zip(expected).enumerate() {
            assert_eq!(
                generated.raw_beat_index,
                integer(required(expected, "rawBeatIndex")) as usize,
                "{uri} point {index}"
            );
            assert_eq!(
                generated.position_ms,
                integer(required(expected, "positionMs")),
                "{uri} point {index}"
            );
            let expected_seconds = number(required(expected, "positionSeconds")) as f32;
            assert_eq!(
                generated.position_seconds.to_bits(),
                expected_seconds.to_bits(),
                "{uri} point {index}"
            );
            let expected_confidence = number(required(expected, "confidence")) as f32;
            assert_eq!(
                generated.confidence.to_bits(),
                expected_confidence.to_bits(),
                "{uri} point {index}"
            );
        }
        checked += 1;
    }

    assert_eq!(checked, 9);
}

#[test]
fn official_beatmatched_geometry_is_contained_in_raw_candidates() {
    let (manifest, tracks) = load_manifest_tracks();
    let oracle = read_json(fixture_root().join("get_computed_transitions_2026-08-14.json"));
    let run = &required(&oracle, "runs").as_array().unwrap()[0];
    let official_entries = required(required(run, "response"), "computedTransitions")
        .as_array()
        .unwrap();

    let pairs = required(&manifest, "pairs").as_array().unwrap();
    assert_eq!(pairs.len(), official_entries.len());
    let config = AutoGeometryConfig::default();
    let mut total_candidates = 0;
    let mut expected_count = 0;
    let mut found_count = 0;
    let mut bit_exact_speed_count = 0;
    let mut maximum_speed_difference = 0.0_f64;
    let mut missing = Vec::new();

    for (pair_index, (pair, official_entry)) in pairs.iter().zip(official_entries).enumerate() {
        let label = string(required(pair, "label"));
        let track_a_uri = string(required(pair, "trackAUri"));
        let track_b_uri = string(required(pair, "trackBUri"));
        let item_speed_a = number(required(pair, "itemSpeedA")) as f32;
        let item_speed_b = number(required(pair, "itemSpeedB")) as f32;
        let track_a = &tracks.get(track_a_uri).unwrap().1;
        let track_b = &tracks.get(track_b_uri).unwrap().1;
        let candidates = generate_overlap_candidates(
            AutoPairGeometryInput {
                track_a,
                track_b,
                item_speed_a,
                item_speed_b,
            },
            config,
        );
        assert_candidate_invariants(
            label,
            track_a,
            track_b,
            item_speed_a,
            item_speed_b,
            &candidates,
            config,
        );
        total_candidates += candidates.len();
        eprintln!(
            "pair={pair_index} label={label} raw_candidates={}",
            candidates.len()
        );
        let by_geometry = candidate_map(&candidates);

        for (rank, ranked) in required(official_entry, "rankedTransitions")
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let overlap = required(ranked, "overlap");
            if !required(overlap, "isBeatmatched").as_bool().unwrap() {
                continue;
            }
            expected_count += 1;
            let expected = expected_geometry(overlap);
            let Some(candidate) = by_geometry.get(&expected) else {
                missing.push(format!(
                    "pair={label} rank={rank} expected={expected:?} closest={:?}",
                    closest_candidates(expected, &candidates)
                ));
                continue;
            };
            found_count += 1;
            assert_eq!(
                candidate.speed_a,
                number(required(overlap, "speedA")) as f32
            );
            let expected_speed_b = number(required(overlap, "speedB"));
            let difference = (candidate.speed_b as f64 - expected_speed_b).abs();
            maximum_speed_difference = maximum_speed_difference.max(difference);
            assert!(
                difference <= SPEED_TOLERANCE,
                "pair={label} rank={rank} expected speedB={expected_speed_b} generated={} difference={difference}",
                candidate.speed_b
            );
            if candidate.speed_b.to_bits() == (expected_speed_b as f32).to_bits() {
                bit_exact_speed_count += 1;
            }
        }
    }

    assert!(
        missing.is_empty(),
        "missing official geometry:\n{}",
        missing.join("\n")
    );
    assert_eq!(expected_count, 105);
    assert_eq!(found_count, expected_count);
    eprintln!(
        "raw_total={total_candidates} official={expected_count} found={found_count} speed_bit_exact={bit_exact_speed_count} max_speed_difference={maximum_speed_difference:.15}"
    );
}

#[test]
fn item_speed_fixture_changes_membership_without_changing_shared_geometry() {
    let fixture = read_json(fixture_root().join("item-speed-experiment-2026-08-14.json"));
    let run = required(&fixture, "run");
    let pairs = required(run, "pairs").as_array().unwrap();
    let official = required(required(run, "response"), "computedTransitions")
        .as_array()
        .unwrap();
    let (_, track_a) = load_track(string(required(&pairs[0], "trackAUri")));
    let (_, track_b) = load_track(string(required(&pairs[0], "trackBUri")));
    let config = AutoGeometryConfig::default();
    let mut generated = Vec::new();

    for (pair, official_entry) in pairs.iter().zip(official) {
        let label = string(required(pair, "label"));
        let item_speed_a = number(required(pair, "itemSpeedA")) as f32;
        let item_speed_b = number(required(pair, "itemSpeedB")) as f32;
        let candidates = generate_overlap_candidates(
            AutoPairGeometryInput {
                track_a: &track_a,
                track_b: &track_b,
                item_speed_a,
                item_speed_b,
            },
            config,
        );
        let by_geometry = candidate_map(&candidates);
        for ranked in required(official_entry, "rankedTransitions")
            .as_array()
            .unwrap()
        {
            let overlap = required(ranked, "overlap");
            let geometry = expected_geometry(overlap);
            assert!(
                by_geometry.contains_key(&geometry),
                "item-speed case {label} is missing official geometry {geometry:?}"
            );
        }
        eprintln!(
            "item_speed_case={label} raw_candidates={}",
            candidates.len()
        );
        generated.push((label.to_owned(), candidates));
    }

    let baseline: HashMap<_, _> = generated[0]
        .1
        .iter()
        .map(|candidate| (GeometryKey::from(candidate), candidate.speed_b.to_bits()))
        .collect();
    for (label, candidates) in generated.iter().skip(1) {
        let adjusted: HashMap<_, _> = candidates
            .iter()
            .map(|candidate| (GeometryKey::from(candidate), candidate.speed_b.to_bits()))
            .collect();
        let shared: HashSet<_> = baseline
            .keys()
            .filter(|key| adjusted.contains_key(key))
            .collect();
        let shared_count = shared.len();
        let baseline_only = baseline.len() - shared_count;
        let adjusted_only = adjusted.len() - shared_count;
        assert!(!shared.is_empty(), "{label} has no shared raw candidates");
        assert!(
            baseline.keys().any(|key| !adjusted.contains_key(key))
                || adjusted.keys().any(|key| !baseline.contains_key(key)),
            "{label} did not alter raw candidate membership"
        );
        for key in shared {
            assert_eq!(
                baseline[key], adjusted[key],
                "{label} changed shared speedB for {key:?}"
            );
        }
        eprintln!(
            "item_speed_delta={label} shared={} baseline_only={baseline_only} adjusted_only={adjusted_only}",
            shared_count
        );
    }
}

#[test]
fn pure_module_contains_no_live_integration() {
    let source = include_str!("../src/spotify_auto_mix.rs");
    for forbidden in [
        "Session",
        "SpClient",
        "TransitionEngine",
        "tokio::",
        "async fn",
    ] {
        assert!(
            !source.contains(forbidden),
            "unexpected live integration dependency: {forbidden}"
        );
    }
}

fn overlap_key(overlap: AutoTransitionOverlap) -> (bool, i64, i64, usize, i64) {
    (
        overlap.is_beatmatched,
        overlap.start_a_ms,
        overlap.start_b_ms,
        overlap.duration_bars,
        overlap.duration_ms,
    )
}

fn oracle_overlap_key(overlap: &Value) -> (bool, i64, i64, usize, i64) {
    (
        required(overlap, "isBeatmatched").as_bool().unwrap(),
        integer(required(overlap, "startAMs")),
        integer(required(overlap, "startBMs")),
        integer(required(overlap, "durationBars")) as usize,
        integer(required(overlap, "durationMs")),
    )
}

#[test]
fn complete_scoring_ranking_and_preset_pipeline_reports_strict_oracle_parity() {
    let manifest = read_json(fixture_root().join("manifest-2026-08-14.json"));
    let tracks = load_scoring_tracks(&manifest);
    let oracle = read_json(fixture_root().join("get_computed_transitions_2026-08-14.json"));
    let run = &required(&oracle, "runs").as_array().unwrap()[0];
    let official_entries = required(required(run, "response"), "computedTransitions")
        .as_array()
        .unwrap();
    let pairs = required(&manifest, "pairs").as_array().unwrap();

    let mut raw_total = 0;
    let mut base_valid_total = 0;
    let mut retained_total = 0;
    let mut expected_rank = 0;
    let mut exact_rank = 0;
    let mut score_vector_tie_rank = 0;
    let mut exact_scores = 0;
    let mut compared_scores = 0;
    let mut maximum_score_difference = 0.0_f64;
    let mut exact_presets = 0;
    let mut exact_positional_presets = 0;
    let mut expected_presets = 0;
    let mut exact_fallbacks = 0;
    let mut non_tie_rank_discrepancies = Vec::new();
    let mut score_discrepancies = Vec::new();

    for (pair, official_entry) in pairs.iter().zip(official_entries) {
        let label = string(required(pair, "label"));
        let track_a = &tracks[string(required(pair, "trackAUri"))];
        let track_b = &tracks[string(required(pair, "trackBUri"))];
        let pair_input = AutoPairScoringInput {
            track_a,
            track_b,
            item_speed_a: number(required(pair, "itemSpeedA")) as f32,
            item_speed_b: number(required(pair, "itemSpeedB")) as f32,
        };
        let result = generate_ranked_transitions(pair_input, AutoGeometryConfig::default());
        let official = required(official_entry, "rankedTransitions")
            .as_array()
            .unwrap();

        raw_total += result.raw_candidate_count;
        base_valid_total += result.base_valid_candidate_count;
        retained_total += result.per_bar_retained_count;
        eprintln!(
            "score_pair={label} raw={} base_valid={} retained={} final={} expected={}",
            result.raw_candidate_count,
            result.base_valid_candidate_count,
            result.per_bar_retained_count,
            result.ranked_transitions.len(),
            official.len()
        );
        assert_eq!(
            result.ranked_transitions.len(),
            official.len(),
            "{label}: returned transition count"
        );

        let generated_by_key: HashMap<_, _> = result
            .ranked_transitions
            .iter()
            .map(|ranked| (overlap_key(ranked.overlap), ranked))
            .collect();

        for (rank, expected) in official.iter().enumerate() {
            expected_rank += 1;
            let expected_overlap = required(expected, "overlap");
            let key = oracle_overlap_key(expected_overlap);
            let expected_is_beatmatched = required(expected_overlap, "isBeatmatched")
                .as_bool()
                .unwrap();

            let mut recovered_raw = None;
            let identity_candidate = if let Some(generated) = generated_by_key.get(&key) {
                Some((
                    generated.computed_score,
                    generated.components,
                    generated.overlap.speed_a,
                    generated.overlap.speed_b,
                ))
            } else if expected_is_beatmatched {
                let raw = generate_overlap_candidates(
                    AutoPairGeometryInput {
                        track_a: &track_a.geometry,
                        track_b: &track_b.geometry,
                        item_speed_a: pair_input.item_speed_a,
                        item_speed_b: pair_input.item_speed_b,
                    },
                    AutoGeometryConfig::default(),
                )
                .into_iter()
                .find(|candidate| {
                    (
                        true,
                        candidate.start_a_ms,
                        candidate.start_b_ms,
                        candidate.duration_bars,
                        candidate.duration_ms,
                    ) == key
                })
                .unwrap_or_else(|| panic!("{label} rank={rank}: oracle geometry is not raw-valid"));
                let scored = score_candidate(raw, pair_input).unwrap_or_else(|| {
                    panic!("{label} rank={rank}: oracle geometry failed scoring")
                });
                recovered_raw = Some(scored);
                Some((
                    scored.computed_score,
                    Some(scored.components),
                    scored.geometry.speed_a,
                    scored.geometry.speed_b,
                ))
            } else {
                None
            };
            let identity_candidate = identity_candidate
                .unwrap_or_else(|| panic!("{label} rank={rank}: expected candidate missing"));

            let generated_at_rank = &result.ranked_transitions[rank];
            if overlap_key(generated_at_rank.overlap) == key {
                exact_rank += 1;
            } else {
                let same_score_vector = generated_at_rank.components == identity_candidate.1
                    && generated_at_rank.computed_score.to_bits() == identity_candidate.0.to_bits()
                    && generated_at_rank.overlap.duration_bars
                        == integer(required(expected_overlap, "durationBars")) as usize;
                if same_score_vector {
                    score_vector_tie_rank += 1;
                } else {
                    non_tie_rank_discrepancies.push(format!(
                        "{label} rank={rank}: generated={:?} expected={key:?} generated_score={} expected_identity_score={}",
                        overlap_key(generated_at_rank.overlap),
                        generated_at_rank.computed_score,
                        identity_candidate.0
                    ));
                }
            }

            assert_eq!(
                identity_candidate.2.to_bits(),
                (number(required(expected_overlap, "speedA")) as f32).to_bits(),
                "{label} rank={rank} speedA"
            );
            assert_eq!(
                identity_candidate.3.to_bits(),
                (number(required(expected_overlap, "speedB")) as f32).to_bits(),
                "{label} rank={rank} speedB"
            );

            let expected_score = number(required(expected, "computedScore"));
            let difference = (f64::from(identity_candidate.0) - expected_score).abs();
            maximum_score_difference = maximum_score_difference.max(difference);
            compared_scores += 1;
            if identity_candidate.0.to_bits() == (expected_score as f32).to_bits() {
                exact_scores += 1;
            } else {
                score_discrepancies.push(format!(
                    "{label} rank={rank}: generated={} expected={expected_score} components={:?}",
                    identity_candidate.0, identity_candidate.1
                ));
            }

            let expected_shape = AutoTransitionOverlap {
                start_a_ms: integer(required(expected_overlap, "startAMs")),
                start_b_ms: integer(required(expected_overlap, "startBMs")),
                duration_ms: integer(required(expected_overlap, "durationMs")),
                duration_bars: integer(required(expected_overlap, "durationBars")) as usize,
                speed_a: number(required(expected_overlap, "speedA")) as f32,
                speed_b: number(required(expected_overlap, "speedB")) as f32,
                is_beatmatched: expected_is_beatmatched,
            };
            let direct_presets =
                rank_presets(&track_a.playable_uri, &track_b.playable_uri, expected_shape);
            let expected_preset_values = required(expected, "rankedPresets").as_array().unwrap();
            expected_presets += 1;
            let presets_match = direct_presets.len() == expected_preset_values.len()
                && direct_presets.iter().zip(expected_preset_values).all(
                    |(generated, expected)| {
                        generated.preset_id
                            == integer(required(required(expected, "preset"), "id")) as u8
                            && generated.computed_score.to_bits()
                                == (number(required(expected, "computedScore")) as f32).to_bits()
                    },
                );
            if presets_match {
                exact_presets += 1;
            }
            if generated_at_rank.ranked_presets == direct_presets {
                exact_positional_presets += 1;
            }

            if !expected_is_beatmatched {
                assert!(recovered_raw.is_none());
                let generated = generated_by_key
                    .get(&key)
                    .unwrap_or_else(|| panic!("{label}: fallback identity mismatch"));
                assert_eq!(generated.computed_score.to_bits(), 0.0_f32.to_bits());
                assert_eq!(generated.ranked_presets, direct_presets);
                exact_fallbacks += 1;
            }
        }
    }

    eprintln!(
        "score_summary raw={raw_total} base_valid={base_valid_total} retained={retained_total} final={expected_rank} exact_rank={exact_rank} score_vector_ties={score_vector_tie_rank} scores={exact_scores}/{compared_scores} max_score_difference={maximum_score_difference:.15} presets={exact_presets}/{expected_presets} positional_presets={exact_positional_presets}/{expected_presets} fallbacks={exact_fallbacks}/2"
    );
    if !score_discrepancies.is_empty() {
        eprintln!(
            "non_bit_exact_scores ({}):\n{}",
            score_discrepancies.len(),
            score_discrepancies
                .iter()
                .take(40)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    assert_eq!(raw_total, 224_036);
    assert_eq!(base_valid_total, 13_601);
    assert_eq!(retained_total, 105);
    assert_eq!(expected_rank, 107);
    assert_eq!(exact_rank, 98);
    assert_eq!(score_vector_tie_rank, 9);
    assert!(
        non_tie_rank_discrepancies.is_empty(),
        "non-tie rank discrepancies:\n{}",
        non_tie_rank_discrepancies.join("\n")
    );
    assert_eq!(compared_scores, 107);
    assert!(
        exact_scores >= 75,
        "computed-score bit parity regressed to {exact_scores}/107"
    );
    assert!(
        maximum_score_difference <= MAX_OBSERVED_SCORE_DIFFERENCE,
        "computed-score difference regressed to {maximum_score_difference}"
    );
    assert_eq!(exact_presets, expected_presets);
    assert_eq!(exact_presets, 107);
    assert_eq!(exact_fallbacks, 2);
}
