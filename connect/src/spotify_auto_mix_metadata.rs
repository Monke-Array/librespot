//! Live metadata acquisition and adaptation for Spotify Auto Mix.
//!
//! This module deliberately stops at constructing the pure scoring input. It does not choose or
//! apply transitions and it keeps canonical context identity separate from playable identity.

use std::collections::HashMap;

use librespot_core::{Session, SpotifyUri};
use librespot_protocol::{
    entity_extension_data::EntityExtensionData,
    extended_metadata::{
        BatchedEntityRequest, BatchedExtensionResponse, EntityRequest, ExtensionQuery,
    },
    extension_kind::ExtensionKind,
    spotify_auto_mix_metadata::{
        AudioAttributesV2, Beat, Beats, Cuepoint, Cuepoints, ExtensionDescriptorData, Mixability,
        VocalActivity,
    },
};
use protobuf::{Enum, EnumOrUnknown, Message};
use serde_json::Value;
use thiserror::Error;

use crate::spotify_auto_mix::{
    AutoBeat, AutoCamelotKey, AutoCuepoint, AutoCuepoints, AutoTrackGeometryInput,
    AutoTrackScoringInput, AutoVocalActivity,
};

const CANONICAL_KINDS: [ExtensionKind; 3] = [
    ExtensionKind::TRACK_DESCRIPTOR,
    ExtensionKind::CUEPOINTS,
    ExtensionKind::MIXABILITY,
];
const PLAYABLE_KINDS: [ExtensionKind; 3] = [
    ExtensionKind::BEATS,
    ExtensionKind::VOCAL_ACTIVITY,
    ExtensionKind::AUDIO_ATTRIBUTES_V2,
];

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AutoTrackIdentity {
    pub canonical_uri: String,
    pub playable_uri: String,
    pub canonical_duration_ms: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AutoAnalysisBar {
    pub start_seconds: f64,
    pub duration_seconds: f64,
    pub confidence: f64,
    pub peak_linear_loudness: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AutoAudioAnalysis {
    pub duration_seconds: f64,
    pub end_of_fade_in_seconds: f64,
    pub start_of_fade_out_seconds: f64,
    pub bars: Vec<AutoAnalysisBar>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LoadedAutoTrackMetadata {
    pub scoring_input: AutoTrackScoringInput,
    pub audio_analysis: AutoAudioAnalysis,
}

#[derive(Debug, Error, PartialEq)]
pub(crate) enum AutoMetadataError {
    #[error("invalid Auto track identity: {0}")]
    InvalidIdentity(String),
    #[error("Auto metadata request failed: {0}")]
    Request(String),
    #[error("Auto metadata response is missing {0}")]
    Missing(&'static str),
    #[error("invalid Auto metadata envelope: {0}")]
    InvalidEnvelope(String),
    #[error("invalid Auto metadata payload: {0}")]
    InvalidPayload(String),
    #[error("invalid Auto audio analysis: {0}")]
    InvalidAudioAnalysis(String),
}

#[derive(Clone)]
pub(crate) struct AutoMixMetadataClient {
    session: Session,
}

impl AutoMixMetadataClient {
    pub(crate) fn new(session: Session) -> Self {
        Self { session }
    }

    pub(crate) async fn load_track(
        &self,
        identity: &AutoTrackIdentity,
    ) -> Result<LoadedAutoTrackMetadata, AutoMetadataError> {
        validate_identity(identity)?;
        let (extension_result, analysis_result) = tokio::join!(
            self.fetch_extensions(identity),
            self.fetch_audio_analysis(&identity.playable_uri)
        );
        let extensions = extension_result?;
        let analysis = analysis_result?;
        adapt_metadata(identity, decode_extensions(identity, extensions)?, analysis)
    }

    pub(crate) async fn load_pair(
        &self,
        track_a: &AutoTrackIdentity,
        track_b: &AutoTrackIdentity,
    ) -> Result<(LoadedAutoTrackMetadata, LoadedAutoTrackMetadata), AutoMetadataError> {
        let (track_a, track_b) = tokio::join!(self.load_track(track_a), self.load_track(track_b));
        Ok((track_a?, track_b?))
    }

    async fn fetch_extensions(
        &self,
        identity: &AutoTrackIdentity,
    ) -> Result<BatchedExtensionResponse, AutoMetadataError> {
        self.session
            .spclient()
            .get_extended_metadata(extension_request(identity))
            .await
            .map_err(|error| AutoMetadataError::Request(error.to_string()))
    }

    async fn fetch_audio_analysis(
        &self,
        playable_uri: &str,
    ) -> Result<AutoAudioAnalysis, AutoMetadataError> {
        let track_id = track_id(playable_uri)?;
        let uri = format!("hm://audio-attributes/v1/audio-analysis/{track_id}");
        let request = self
            .session
            .mercury()
            .get(uri)
            .map_err(|error| AutoMetadataError::Request(error.to_string()))?;
        let response = request
            .await
            .map_err(|error| AutoMetadataError::Request(error.to_string()))?;
        let payload = response
            .payload
            .first()
            .ok_or(AutoMetadataError::Missing("audio-analysis response body"))?;
        parse_audio_analysis(payload)
    }
}

fn validate_identity(identity: &AutoTrackIdentity) -> Result<(), AutoMetadataError> {
    track_id(&identity.canonical_uri)?;
    track_id(&identity.playable_uri)?;
    if identity.canonical_duration_ms == 0 {
        return Err(AutoMetadataError::InvalidIdentity(
            "canonical duration must be greater than zero".to_owned(),
        ));
    }
    Ok(())
}

fn track_id(uri: &str) -> Result<String, AutoMetadataError> {
    match SpotifyUri::from_uri(uri) {
        Ok(track @ SpotifyUri::Track { .. }) => track
            .to_id()
            .map_err(|error| AutoMetadataError::InvalidIdentity(error.to_string())),
        _ => Err(AutoMetadataError::InvalidIdentity(format!(
            "{uri:?} is not a track URI"
        ))),
    }
}

fn extension_request(identity: &AutoTrackIdentity) -> BatchedEntityRequest {
    BatchedEntityRequest {
        entity_request: vec![
            entity_request(&identity.canonical_uri, &CANONICAL_KINDS),
            entity_request(&identity.playable_uri, &PLAYABLE_KINDS),
        ],
        ..Default::default()
    }
}

fn entity_request(uri: &str, kinds: &[ExtensionKind]) -> EntityRequest {
    EntityRequest {
        entity_uri: uri.to_owned(),
        query: kinds
            .iter()
            .copied()
            .map(|extension_kind| ExtensionQuery {
                extension_kind: EnumOrUnknown::new(extension_kind),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

#[derive(Debug)]
struct DecodedExtensions {
    descriptors: ExtensionDescriptorData,
    cuepoints: Cuepoints,
    beats: Beats,
    vocal_activity: VocalActivity,
    mixability: Mixability,
    audio_attributes: AudioAttributesV2,
}

fn decode_extensions(
    identity: &AutoTrackIdentity,
    response: BatchedExtensionResponse,
) -> Result<DecodedExtensions, AutoMetadataError> {
    let mut payloads = HashMap::new();
    for array in response.extended_metadata {
        let kind = array.extension_kind.value();
        if !CANONICAL_KINDS
            .iter()
            .chain(&PLAYABLE_KINDS)
            .any(|expected| expected.value() == kind)
        {
            continue;
        }
        if payloads.contains_key(&kind) {
            return Err(AutoMetadataError::InvalidEnvelope(format!(
                "extension kind {kind} was returned more than once"
            )));
        }
        let entity = exactly_one(array.extension_data, kind)?;
        let expected_uri = if CANONICAL_KINDS
            .iter()
            .any(|expected| expected.value() == kind)
        {
            &identity.canonical_uri
        } else {
            &identity.playable_uri
        };
        if entity.entity_uri != *expected_uri {
            return Err(AutoMetadataError::InvalidEnvelope(format!(
                "extension kind {kind} returned entity {:?}, expected {expected_uri:?}",
                entity.entity_uri
            )));
        }
        let payload = entity.extension_data.into_option().ok_or_else(|| {
            AutoMetadataError::InvalidEnvelope(format!("extension kind {kind} had no payload"))
        })?;
        payloads.insert(kind, (entity.entity_uri, payload.value));
    }

    Ok(DecodedExtensions {
        descriptors: decode_payload(
            &mut payloads,
            ExtensionKind::TRACK_DESCRIPTOR,
            "track descriptor",
        )?,
        cuepoints: decode_payload(&mut payloads, ExtensionKind::CUEPOINTS, "cuepoints")?,
        beats: decode_payload(&mut payloads, ExtensionKind::BEATS, "beats")?,
        vocal_activity: decode_payload(
            &mut payloads,
            ExtensionKind::VOCAL_ACTIVITY,
            "vocal activity",
        )?,
        mixability: decode_payload(&mut payloads, ExtensionKind::MIXABILITY, "mixability")?,
        audio_attributes: decode_payload(
            &mut payloads,
            ExtensionKind::AUDIO_ATTRIBUTES_V2,
            "audio attributes v2",
        )?,
    })
}

fn exactly_one(
    mut entities: Vec<EntityExtensionData>,
    kind: i32,
) -> Result<EntityExtensionData, AutoMetadataError> {
    if entities.len() != 1 {
        return Err(AutoMetadataError::InvalidEnvelope(format!(
            "extension kind {kind} returned {} entities",
            entities.len()
        )));
    }
    Ok(entities.remove(0))
}

fn decode_payload<M: Message>(
    payloads: &mut HashMap<i32, (String, Vec<u8>)>,
    kind: ExtensionKind,
    name: &'static str,
) -> Result<M, AutoMetadataError> {
    let (_, bytes) = payloads
        .remove(&kind.value())
        .ok_or(AutoMetadataError::Missing(name))?;
    M::parse_from_bytes(&bytes)
        .map_err(|_| AutoMetadataError::InvalidPayload(format!("{name} was not valid protobuf")))
}

fn adapt_metadata(
    identity: &AutoTrackIdentity,
    extensions: DecodedExtensions,
    audio_analysis: AutoAudioAnalysis,
) -> Result<LoadedAutoTrackMetadata, AutoMetadataError> {
    let DecodedExtensions {
        descriptors,
        cuepoints,
        beats,
        vocal_activity,
        mixability,
        audio_attributes,
    } = extensions;

    if beats.beats_per_bar == 0 || beats.beats_hash.is_empty() {
        return Err(AutoMetadataError::InvalidPayload(
            "beats was missing its bar count or hash".to_owned(),
        ));
    }
    let beat_inputs = beats
        .beats
        .iter()
        .map(adapt_beat)
        .collect::<Result<Vec<_>, _>>()?;
    if beat_inputs.is_empty() {
        return Err(AutoMetadataError::InvalidPayload(
            "beats contained no beat points".to_owned(),
        ));
    }

    let bpm = finite_f32(audio_attributes.bpm, "audio attributes bpm")?;
    if bpm <= 0.0 {
        return Err(AutoMetadataError::InvalidPayload(
            "audio attributes bpm must be greater than zero".to_owned(),
        ));
    }
    let camelot_key = audio_attributes
        .key
        .as_ref()
        .and_then(|key| key.camelot_key.as_ref())
        .and_then(|camelot| AutoCamelotKey::parse(&camelot.value));

    let source_sample_rate_hz = integral_u32(
        f64::from(vocal_activity.source_sample_rate_hz),
        "vocal source sample rate",
        false,
    )?;
    let smoothing_window_size = nonnegative_u32(
        vocal_activity.smoothing_window_size,
        "vocal smoothing window size",
    )?;
    let samples_between_windows = nonnegative_u32(
        vocal_activity.samples_between_windows,
        "vocal samples between windows",
    )?;
    if samples_between_windows == 0 {
        return Err(AutoMetadataError::InvalidPayload(
            "vocal samples between windows must be greater than zero".to_owned(),
        ));
    }
    let probabilities = vocal_activity
        .vocal_activity_probabilities
        .into_iter()
        .map(|probability| {
            u8::try_from(probability).map_err(|_| {
                AutoMetadataError::InvalidPayload(
                    "vocal activity probability was outside 0..=255".to_owned(),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if probabilities.is_empty() {
        return Err(AutoMetadataError::InvalidPayload(
            "vocal activity contained no probabilities".to_owned(),
        ));
    }

    let genre_based_beatmatchability = finite_f32(
        mixability.genre_based_beatmatchability,
        "genre beatmatchability",
    )?;
    let scoring_duration_seconds = identity.canonical_duration_ms as f32 / 1000.0;

    Ok(LoadedAutoTrackMetadata {
        scoring_input: AutoTrackScoringInput {
            geometry: AutoTrackGeometryInput {
                duration_seconds: audio_analysis.duration_seconds as f32,
                beats: beat_inputs,
            },
            scoring_duration_seconds,
            playable_uri: identity.playable_uri.clone(),
            cuepoints: AutoCuepoints {
                best_fade_in: cuepoints
                    .best_fade_in_cuepoint
                    .as_ref()
                    .map(adapt_cuepoint)
                    .transpose()?,
                best_fade_out: cuepoints
                    .best_fade_out_cuepoint
                    .as_ref()
                    .map(adapt_cuepoint)
                    .transpose()?,
                fade_in_candidates: cuepoints
                    .fade_in_cuepoints
                    .iter()
                    .map(adapt_cuepoint)
                    .collect::<Result<_, _>>()?,
                fade_out_candidates: cuepoints
                    .fade_out_cuepoints
                    .iter()
                    .map(adapt_cuepoint)
                    .collect::<Result<_, _>>()?,
            },
            vocal_activity: AutoVocalActivity {
                source_sample_rate_hz,
                smoothing_window_size,
                first_window_sample_start: i64::from(vocal_activity.first_window_sample_start),
                samples_between_windows,
                probabilities,
            },
            camelot_key,
            bpm,
            genre_concept_uris: descriptors
                .descriptor
                .into_iter()
                .filter(|descriptor| descriptor.types.contains(&1))
                .map(|descriptor| descriptor.concept_uri)
                .filter(|uri| !uri.is_empty())
                .collect(),
            mixable: mixability.mixable,
            genre_based_beatmatchability,
        },
        audio_analysis,
    })
}

fn adapt_beat(beat: &Beat) -> Result<AutoBeat, AutoMetadataError> {
    let values = [
        (beat.time, "beat time"),
        (beat.duration, "beat duration"),
        (beat.beat_confidence, "beat confidence"),
        (beat.downbeat_confidence, "downbeat confidence"),
    ];
    for (value, name) in values {
        if !value.is_finite() {
            return Err(AutoMetadataError::InvalidPayload(format!(
                "{name} was not finite"
            )));
        }
    }
    Ok(AutoBeat {
        time_seconds: beat.time,
        value: i32::try_from(beat.value)
            .map_err(|_| AutoMetadataError::InvalidPayload("beat value exceeded i32".to_owned()))?,
        beat_confidence: beat.beat_confidence,
        downbeat_confidence: beat.downbeat_confidence,
    })
}

fn adapt_cuepoint(cuepoint: &Cuepoint) -> Result<AutoCuepoint, AutoMetadataError> {
    if !cuepoint.tempo_bpm.is_finite() {
        return Err(AutoMetadataError::InvalidPayload(
            "cuepoint tempo was not finite".to_owned(),
        ));
    }
    Ok(AutoCuepoint {
        position_ms: cuepoint.position_ms,
        tempo_bpm: cuepoint.tempo_bpm,
    })
}

fn finite_f32(value: f64, name: &str) -> Result<f32, AutoMetadataError> {
    let value = value as f32;
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| AutoMetadataError::InvalidPayload(format!("{name} was not a finite f32")))
}

fn integral_u32(value: f64, name: &str, allow_zero: bool) -> Result<u32, AutoMetadataError> {
    if !value.is_finite()
        || value.fract() != 0.0
        || value < f64::from(!allow_zero as u8)
        || value > f64::from(u32::MAX)
    {
        return Err(AutoMetadataError::InvalidPayload(format!(
            "{name} was not a valid u32"
        )));
    }
    Ok(value as u32)
}

fn nonnegative_u32(value: i32, name: &str) -> Result<u32, AutoMetadataError> {
    u32::try_from(value)
        .map_err(|_| AutoMetadataError::InvalidPayload(format!("{name} must not be negative")))
}

#[derive(Clone, Copy)]
struct AnalysisSegment {
    start: f64,
    duration: f64,
    loudness_max_db: f64,
}

fn parse_audio_analysis(payload: &[u8]) -> Result<AutoAudioAnalysis, AutoMetadataError> {
    let root: Value = serde_json::from_slice(payload).map_err(|error| {
        AutoMetadataError::InvalidAudioAnalysis(format!("response was not JSON: {error}"))
    })?;
    let track = object(&root, "track")?;
    let duration_seconds = number(track, "duration")?;
    let end_of_fade_in_seconds = number(track, "end_of_fade_in")?;
    let start_of_fade_out_seconds = number(track, "start_of_fade_out")?;
    if duration_seconds <= 0.0 || end_of_fade_in_seconds < 0.0 || start_of_fade_out_seconds < 0.0 {
        return Err(AutoMetadataError::InvalidAudioAnalysis(
            "track timing values were outside their valid ranges".to_owned(),
        ));
    }

    let segments = array(&root, "segments")?
        .iter()
        .map(|value| {
            let start = number(value, "start")?;
            let duration = number(value, "duration")?;
            let _confidence = number(value, "confidence")?;
            let loudness_max_db = number(value, "loudness_max")?;
            if start < 0.0 || duration <= 0.0 {
                return Err(AutoMetadataError::InvalidAudioAnalysis(
                    "segment timing values were outside their valid ranges".to_owned(),
                ));
            }
            Ok(AnalysisSegment {
                start,
                duration,
                loudness_max_db,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if segments.is_empty() {
        return Err(AutoMetadataError::Missing("audio-analysis segments"));
    }

    let bars = array(&root, "bars")?
        .iter()
        .map(|value| {
            let start_seconds = number(value, "start")?;
            let duration_seconds = number(value, "duration")?;
            let confidence = number(value, "confidence")?;
            if start_seconds < 0.0 || duration_seconds <= 0.0 {
                return Err(AutoMetadataError::InvalidAudioAnalysis(
                    "bar timing values were outside their valid ranges".to_owned(),
                ));
            }
            let bar_end = start_seconds + duration_seconds;
            let peak_linear_loudness = segments
                .iter()
                .filter(|segment| {
                    segment.start < bar_end && segment.start + segment.duration > start_seconds
                })
                .map(|segment| db_to_linear(segment.loudness_max_db))
                .reduce(f64::max)
                .ok_or(AutoMetadataError::InvalidAudioAnalysis(
                    "bar did not overlap any segment".to_owned(),
                ))?;
            if !peak_linear_loudness.is_finite() {
                return Err(AutoMetadataError::InvalidAudioAnalysis(
                    "bar peak linear loudness was not finite".to_owned(),
                ));
            }
            Ok(AutoAnalysisBar {
                start_seconds,
                duration_seconds,
                confidence,
                peak_linear_loudness,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if bars.is_empty() {
        return Err(AutoMetadataError::Missing("audio-analysis bars"));
    }

    Ok(AutoAudioAnalysis {
        duration_seconds,
        end_of_fade_in_seconds,
        start_of_fade_out_seconds,
        bars,
    })
}

fn object<'a>(value: &'a Value, key: &str) -> Result<&'a Value, AutoMetadataError> {
    value.get(key).ok_or_else(|| {
        AutoMetadataError::InvalidAudioAnalysis(format!("missing object field {key:?}"))
    })
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], AutoMetadataError> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| {
            AutoMetadataError::InvalidAudioAnalysis(format!("missing array field {key:?}"))
        })
}

fn number(value: &Value, key: &str) -> Result<f64, AutoMetadataError> {
    value
        .get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| {
            AutoMetadataError::InvalidAudioAnalysis(format!(
                "missing or non-finite number field {key:?}"
            ))
        })
}

fn db_to_linear(loudness_db: f64) -> f64 {
    10.0_f64.powf(loudness_db / 20.0)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use data_encoding::BASE64;
    use librespot_protocol::{
        entity_extension_data::EntityExtensionData,
        extended_metadata::EntityExtensionDataArray,
        spotify_auto_mix_metadata::{AudioKey, CamelotKey, ExtensionDescriptor},
    };
    use protobuf::{MessageField, well_known_types::any::Any};

    use super::*;

    const CANONICAL_URI: &str = "spotify:track:7tFiyTwD0nx5a1eklYtX2J";
    const PLAYABLE_URI: &str = "spotify:track:2JiDi0qAXsPwhPqA2qaKGt";

    fn identity() -> AutoTrackIdentity {
        AutoTrackIdentity {
            canonical_uri: CANONICAL_URI.to_owned(),
            playable_uri: PLAYABLE_URI.to_owned(),
            canonical_duration_ms: 354_320,
        }
    }

    fn audio_analysis() -> AutoAudioAnalysis {
        AutoAudioAnalysis {
            duration_seconds: 355.154_66,
            end_of_fade_in_seconds: 0.0,
            start_of_fade_out_seconds: 333.937_77,
            bars: vec![AutoAnalysisBar {
                start_seconds: 0.0,
                duration_seconds: 4.0,
                confidence: 0.9,
                peak_linear_loudness: 0.5,
            }],
        }
    }

    fn decoded_extensions() -> DecodedExtensions {
        DecodedExtensions {
            descriptors: ExtensionDescriptorData {
                descriptor: vec![ExtensionDescriptor {
                    types: vec![1],
                    concept_uri: "spotify:concept:rock".to_owned(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            cuepoints: Cuepoints {
                best_fade_in_cuepoint: MessageField::some(Cuepoint {
                    position_ms: 1_070,
                    tempo_bpm: 143.88,
                    ..Default::default()
                }),
                ..Default::default()
            },
            beats: Beats {
                beats_per_bar: 4,
                beats: vec![Beat {
                    time: 0.07,
                    value: 1,
                    beat_confidence: 1.0,
                    downbeat_confidence: 1.0,
                    ..Default::default()
                }],
                beats_hash: "hash".to_owned(),
                ..Default::default()
            },
            vocal_activity: VocalActivity {
                source_sample_rate_hz: 22_050.0,
                smoothing_window_size: 1,
                first_window_sample_start: 0,
                samples_between_windows: 315,
                vocal_activity_probabilities: vec![0, 128, 255],
                ..Default::default()
            },
            mixability: Mixability {
                mixable: true,
                genre_based_beatmatchability: 1.0,
                ..Default::default()
            },
            audio_attributes: AudioAttributesV2 {
                bpm: 89.0,
                key: MessageField::some(AudioKey {
                    camelot_key: MessageField::some(CamelotKey {
                        value: "6B".to_owned(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
        }
    }

    fn captured_fixture() -> Value {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../tools/spotify-automix-oracle/tracks/7tFiyTwD0nx5a1eklYtX2J.json");
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    fn captured_extension<M: Message>(fixture: &Value, name: &str) -> M {
        let encoded = fixture["rawExtensions"][name]["protobufBase64"]
            .as_str()
            .unwrap();
        M::parse_from_bytes(&BASE64.decode(encoded.as_bytes()).unwrap()).unwrap()
    }

    #[test]
    fn request_routes_canonical_and_playable_metadata_separately() {
        let request = extension_request(&identity());
        assert_eq!(request.entity_request.len(), 2);
        assert_eq!(request.entity_request[0].entity_uri, CANONICAL_URI);
        assert_eq!(
            request.entity_request[0]
                .query
                .iter()
                .map(|query| query.extension_kind.value())
                .collect::<Vec<_>>(),
            vec![6, 28, 219]
        );
        assert_eq!(request.entity_request[1].entity_uri, PLAYABLE_URI);
        assert_eq!(
            request.entity_request[1]
                .query
                .iter()
                .map(|query| query.extension_kind.value())
                .collect::<Vec<_>>(),
            vec![217, 218, 222]
        );
    }

    #[test]
    fn adapter_preserves_canonical_scoring_duration() {
        let loaded = adapt_metadata(&identity(), decoded_extensions(), audio_analysis()).unwrap();
        assert_eq!(loaded.scoring_input.scoring_duration_seconds, 354.32);
        assert_eq!(loaded.scoring_input.geometry.duration_seconds, 355.154_66);
        assert_eq!(loaded.scoring_input.playable_uri, PLAYABLE_URI);
    }

    #[test]
    fn captured_substitution_fixture_decodes_and_adapts() {
        let fixture = captured_fixture();
        let fixture_identity = &fixture["identity"];
        let identity = AutoTrackIdentity {
            canonical_uri: fixture_identity["canonicalTrackUri"]
                .as_str()
                .unwrap()
                .to_owned(),
            playable_uri: fixture_identity["playableTrackUri"]
                .as_str()
                .unwrap()
                .to_owned(),
            canonical_duration_ms: 354_320,
        };
        let extensions = DecodedExtensions {
            descriptors: captured_extension(&fixture, "TRACK_DESCRIPTOR"),
            cuepoints: captured_extension(&fixture, "CUEPOINTS"),
            beats: captured_extension(&fixture, "BEATS"),
            vocal_activity: captured_extension(&fixture, "VOCAL_ACTIVITY"),
            mixability: captured_extension(&fixture, "MIXABILITY"),
            audio_attributes: captured_extension(&fixture, "AUDIO_ATTRIBUTES_V2"),
        };
        let analysis_payload = serde_json::to_vec(&fixture["decoded"]["audioAnalysis"]).unwrap();
        let analysis = parse_audio_analysis(&analysis_payload).unwrap();
        let loaded = adapt_metadata(&identity, extensions, analysis).unwrap();

        assert_eq!(loaded.scoring_input.scoring_duration_seconds, 354.32);
        assert_eq!(loaded.scoring_input.geometry.duration_seconds, 355.154_66);
        assert_eq!(loaded.scoring_input.playable_uri, PLAYABLE_URI);
        assert_eq!(
            loaded.scoring_input.camelot_key,
            AutoCamelotKey::parse("6B")
        );
        assert!(!loaded.scoring_input.genre_concept_uris.is_empty());
        assert!(!loaded.audio_analysis.bars.is_empty());
    }

    #[test]
    fn audio_analysis_converts_peak_db_to_linear_per_bar() {
        let parsed = parse_audio_analysis(
            br#"{
                "track":{"duration":12.0,"end_of_fade_in":0.2,"start_of_fade_out":11.0},
                "bars":[{"start":0.0,"duration":4.0,"confidence":0.8}],
                "segments":[
                    {"start":0.0,"duration":2.0,"confidence":0.5,"loudness_max":-20.0},
                    {"start":2.0,"duration":2.0,"confidence":0.7,"loudness_max":-6.020599913279624}
                ]
            }"#,
        )
        .unwrap();
        assert!((parsed.bars[0].peak_linear_loudness - 0.5).abs() < 1.0e-12);
    }

    fn extension<M: Message>(
        kind: ExtensionKind,
        uri: &str,
        message: &M,
    ) -> EntityExtensionDataArray {
        EntityExtensionDataArray {
            extension_kind: EnumOrUnknown::new(kind),
            extension_data: vec![EntityExtensionData {
                entity_uri: uri.to_owned(),
                extension_data: MessageField::some(Any {
                    value: message.write_to_bytes().unwrap(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn missing_extension_is_unavailable_without_panicking() {
        let decoded = decoded_extensions();
        let response = BatchedExtensionResponse {
            extended_metadata: vec![
                extension(
                    ExtensionKind::TRACK_DESCRIPTOR,
                    CANONICAL_URI,
                    &decoded.descriptors,
                ),
                extension(ExtensionKind::CUEPOINTS, CANONICAL_URI, &decoded.cuepoints),
            ],
            ..Default::default()
        };
        assert_eq!(
            decode_extensions(&identity(), response).unwrap_err(),
            AutoMetadataError::Missing("beats")
        );
    }

    #[test]
    fn missing_audio_analysis_field_is_an_error() {
        assert!(matches!(
            parse_audio_analysis(br#"{"track":{},"bars":[],"segments":[]}"#),
            Err(AutoMetadataError::InvalidAudioAnalysis(_))
        ));
    }
}
