use std::{collections::HashMap, future::Future};

use librespot_core::Session;
use librespot_protocol::{
    extended_metadata::{
        BatchedEntityRequest, BatchedExtensionResponse, EntityRequest, ExtensionQuery,
    },
    extension_kind::ExtensionKind,
    transition_data::TransitionData,
};
use protobuf::{EnumOrUnknown, Message};

use crate::spotify_mix::{SpotifyTransitionError, SpotifyTransitionRecipe};

pub(crate) const TRANSITION_URI_ATTRIBUTE: &str = "automix.transition_uri";

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TransitionHydrationKey {
    pub playlist_uri: String,
    pub row_uid: String,
    pub transition_uri: String,
    pub outgoing_uri: String,
    pub incoming_uri: String,
}

impl TransitionHydrationKey {
    pub(crate) fn matches_pair(
        &self,
        playlist_uri: &str,
        row_uid: &str,
        transition_uri: &str,
        outgoing_uri: &str,
        incoming_uri: &str,
    ) -> bool {
        self.playlist_uri == playlist_uri
            && self.row_uid.eq_ignore_ascii_case(row_uid)
            && self.transition_uri == transition_uri
            && self.outgoing_uri == outgoing_uri
            && self.incoming_uri == incoming_uri
    }
}

#[derive(Clone, Debug)]
pub(crate) struct HydratedTransition {
    pub recipe: SpotifyTransitionRecipe,
}

#[derive(Clone, Debug)]
enum CacheEntry {
    Pending(u64),
    Ready(HydratedTransition),
    Unavailable,
}

#[derive(Clone, Debug)]
pub(crate) enum CacheLookup {
    Start(u64),
    Pending,
    Ready(HydratedTransition),
    Unavailable,
}

#[derive(Default)]
pub(crate) struct TransitionHydrationCache {
    entries: HashMap<TransitionHydrationKey, CacheEntry>,
    next_request_id: u64,
}

impl TransitionHydrationCache {
    pub(crate) fn lookup_or_begin(&mut self, key: TransitionHydrationKey) -> CacheLookup {
        match self.entries.get(&key) {
            Some(CacheEntry::Pending(_)) => CacheLookup::Pending,
            Some(CacheEntry::Ready(transition)) => CacheLookup::Ready(transition.clone()),
            Some(CacheEntry::Unavailable) => CacheLookup::Unavailable,
            None => {
                let request_id = self.next_request_id;
                self.next_request_id = self.next_request_id.wrapping_add(1);
                self.entries.insert(key, CacheEntry::Pending(request_id));
                CacheLookup::Start(request_id)
            }
        }
    }

    pub(crate) fn complete(
        &mut self,
        key: TransitionHydrationKey,
        request_id: u64,
        result: Result<HydratedTransition, String>,
    ) -> bool {
        if !matches!(self.entries.get(&key), Some(CacheEntry::Pending(id)) if *id == request_id) {
            return false;
        }
        self.entries.insert(
            key,
            match result {
                Ok(transition) => CacheEntry::Ready(transition),
                Err(_) => CacheEntry::Unavailable,
            },
        );
        true
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}

#[derive(Debug)]
pub(crate) struct HydrationResult {
    pub key: TransitionHydrationKey,
    pub request_id: u64,
    pub result: Result<HydratedTransition, String>,
}

#[derive(Clone)]
pub(crate) struct TransitionDataClient {
    session: Session,
}

impl TransitionDataClient {
    pub(crate) fn new(session: Session) -> Self {
        Self { session }
    }

    async fn fetch(&self, transition_uri: &str) -> Result<TransitionData, String> {
        debug!("[spotify-mix] fetching TRANSITION_DATA kind=244");
        let response = self
            .session
            .spclient()
            .get_extended_metadata(transition_data_request(transition_uri))
            .await
            .map_err(|error| format!("TRANSITION_DATA fetch failed: {}", error.kind))?;
        decode_transition_data_response(transition_uri, response)
    }
}

pub(crate) fn transition_uri(track: &librespot_protocol::player::ProvidedTrack) -> Option<&str> {
    track
        .metadata
        .get(TRANSITION_URI_ATTRIBUTE)
        .map(String::as_str)
        .filter(|uri| {
            let mut parts = uri.split(':');
            matches!(parts.next(), Some("spotify"))
                && matches!(parts.next(), Some("transition"))
                && parts.next().is_some_and(|id| {
                    !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
                })
                && parts.next().is_some_and(|revision| {
                    !revision.is_empty() && revision.bytes().all(|byte| byte.is_ascii_digit())
                })
                && parts.next().is_none()
        })
}

fn transition_data_request(transition_uri: &str) -> BatchedEntityRequest {
    BatchedEntityRequest {
        entity_request: vec![EntityRequest {
            entity_uri: transition_uri.to_owned(),
            query: vec![ExtensionQuery {
                extension_kind: EnumOrUnknown::new(ExtensionKind::TRANSITION_DATA),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn decode_transition_data_response(
    requested_uri: &str,
    response: BatchedExtensionResponse,
) -> Result<TransitionData, String> {
    let data_array = response
        .extended_metadata
        .into_iter()
        .next()
        .ok_or_else(|| "TRANSITION_DATA response contained no extension data".to_owned())?;
    if data_array.extension_kind.enum_value().ok() != Some(ExtensionKind::TRANSITION_DATA) {
        return Err("extended-metadata response kind was not TRANSITION_DATA".to_owned());
    }
    let mut entity = data_array
        .extension_data
        .into_iter()
        .next()
        .ok_or_else(|| "TRANSITION_DATA response contained no entity".to_owned())?;
    if entity.entity_uri != requested_uri {
        return Err("extended-metadata entity URI did not match the request".to_owned());
    }
    let payload = entity
        .extension_data
        .take()
        .ok_or_else(|| "TRANSITION_DATA response contained no payload".to_owned())?;
    TransitionData::parse_from_bytes(&payload.value)
        .map_err(|_| "TRANSITION_DATA payload was not a valid protobuf".to_owned())
}

pub(crate) async fn hydrate_transition_data(
    client: TransitionDataClient,
    key: TransitionHydrationKey,
    request_id: u64,
) -> HydrationResult {
    let result = hydrate_with(&key, || client.fetch(&key.transition_uri)).await;
    HydrationResult {
        key,
        request_id,
        result,
    }
}

async fn hydrate_with<Fetch, FetchFuture>(
    key: &TransitionHydrationKey,
    fetch: Fetch,
) -> Result<HydratedTransition, String>
where
    Fetch: FnOnce() -> FetchFuture,
    FetchFuture: Future<Output = Result<TransitionData, String>>,
{
    let data = fetch().await?;
    validate_transition_data(key, &data)?;
    debug!("[spotify-mix] transition data validated");
    if !data.latest_transition_uri.is_empty() && data.latest_transition_uri != data.transition_uri {
        debug!(
            "[spotify-mix] latest transition URI differs from requested URI latest_uri={}",
            data.latest_transition_uri
        );
    }
    if data.transition.is_empty() {
        return Err("TRANSITION_DATA did not contain a saved recipe".to_owned());
    }
    debug!(
        "[spotify-mix] saved recipe retrieved transition_uri={}",
        key.transition_uri
    );
    let recipe = SpotifyTransitionRecipe::from_base64(&data.transition)
        .map_err(|error: SpotifyTransitionError| error.to_string())?;
    Ok(HydratedTransition { recipe })
}

fn validate_transition_data(
    key: &TransitionHydrationKey,
    data: &TransitionData,
) -> Result<(), String> {
    if data.transition_uri != key.transition_uri {
        return Err("returned transition URI did not match the request".to_owned());
    }
    if data.playlist_uri != key.playlist_uri {
        return Err("returned playlist URI did not match the active context".to_owned());
    }
    if data.item_id != decode_row_uid(&key.row_uid)? {
        return Err("returned item ID did not match the outgoing row UID".to_owned());
    }
    if data.track_a_uri != key.outgoing_uri {
        return Err("returned track A URI did not match the outgoing track".to_owned());
    }
    if data.track_b_uri != key.incoming_uri {
        return Err("returned track B URI did not match the incoming track".to_owned());
    }
    Ok(())
}

fn decode_row_uid(uid: &str) -> Result<Vec<u8>, String> {
    if uid.is_empty() || uid.len() % 2 != 0 || !uid.is_ascii() {
        return Err("outgoing row UID was not valid hexadecimal".to_owned());
    }
    uid.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair)
                .map_err(|_| "outgoing row UID was not valid hexadecimal".to_owned())?;
            u8::from_str_radix(pair, 16)
                .map_err(|_| "outgoing row UID was not valid hexadecimal".to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use data_encoding::BASE64;
    use librespot_protocol::{
        automix_transition::{Curve, CurvePoint, CurveSet, Overlap, Preset, Transition},
        extended_metadata::{EntityExtensionDataArray, EntityRequest},
        player::ProvidedTrack,
    };
    use protobuf::{MessageField, well_known_types::any::Any};

    const PLAYLIST: &str = "spotify:playlist:6FLKuf4VbDj61m893Kd4hq";
    const ROW_UID: &str = "0fc58722cb9faa18";
    const TRANSITION_URI: &str = "spotify:transition:2vwr8lGr3wloJKe3bqUSRf:1786638792016";
    const TRACK_A: &str = "spotify:track:2BMRUAA1oTc7e9JPlr6xbZ";
    const TRACK_B: &str = "spotify:track:5g9lS8deSIxItFBmZRC4vN";

    fn key() -> TransitionHydrationKey {
        TransitionHydrationKey {
            playlist_uri: PLAYLIST.to_owned(),
            row_uid: ROW_UID.to_owned(),
            transition_uri: TRANSITION_URI.to_owned(),
            outgoing_uri: TRACK_A.to_owned(),
            incoming_uri: TRACK_B.to_owned(),
        }
    }

    fn curve(from: f64, to: f64) -> CurveSet {
        CurveSet {
            curves: vec![Curve {
                points: vec![
                    CurvePoint {
                        x: Some(0.0),
                        y: Some(from),
                        ..Default::default()
                    },
                    CurvePoint {
                        x: Some(1.0),
                        y: Some(to),
                        ..Default::default()
                    },
                ],
                start: Some(0.0),
                end: Some(1.0),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn valid_recipe() -> String {
        let transition = Transition {
            overlap: MessageField::some(Overlap {
                start_a_ms: Some(208_960),
                start_b_ms: Some(2_763),
                duration_ms: Some(6_090),
                speed_a: Some(1.0),
                speed_b: Some(1.0),
                track_a_uri: Some(TRACK_A.to_owned()),
                track_b_uri: Some(TRACK_B.to_owned()),
                item_speed_a: Some(1.0),
                item_speed_b: Some(1.0),
                ..Default::default()
            }),
            preset: MessageField::some(Preset {
                volume_out_curve_override: MessageField::some(curve(1.0, 0.0)),
                volume_in_curve_override: MessageField::some(curve(0.0, 1.0)),
                ..Default::default()
            }),
            ..Default::default()
        };
        BASE64.encode(&transition.write_to_bytes().unwrap())
    }

    fn data() -> TransitionData {
        TransitionData {
            transition_uri: TRANSITION_URI.to_owned(),
            latest_transition_uri: TRANSITION_URI.to_owned(),
            playlist_uri: PLAYLIST.to_owned(),
            item_id: vec![0x0f, 0xc5, 0x87, 0x22, 0xcb, 0x9f, 0xaa, 0x18],
            transition: valid_recipe(),
            track_a_uri: TRACK_A.to_owned(),
            track_b_uri: TRACK_B.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn transition_uri_is_discovered_from_outgoing_metadata() {
        let mut track = ProvidedTrack::new();
        track
            .metadata
            .insert(TRANSITION_URI_ATTRIBUTE.into(), TRANSITION_URI.into());
        assert_eq!(transition_uri(&track), Some(TRANSITION_URI));
        track
            .metadata
            .insert(TRANSITION_URI_ATTRIBUTE.into(), "https://invalid".into());
        assert_eq!(transition_uri(&track), None);
        track.metadata.insert(
            TRANSITION_URI_ATTRIBUTE.into(),
            "spotify:transition:id:not-a-revision".into(),
        );
        assert_eq!(transition_uri(&track), None);
    }

    #[test]
    fn request_uses_transition_entity_and_extension_kind_244() {
        let request = transition_data_request(TRANSITION_URI);
        let EntityRequest {
            entity_uri, query, ..
        } = &request.entity_request[0];
        assert_eq!(entity_uri, TRANSITION_URI);
        assert_eq!(query[0].extension_kind.value(), 244);
        assert_eq!(
            query[0].extension_kind.enum_value().unwrap(),
            ExtensionKind::TRANSITION_DATA
        );
    }

    #[test]
    fn transition_data_protobuf_decodes() {
        let bytes = data().write_to_bytes().unwrap();
        let decoded = TransitionData::parse_from_bytes(&bytes).unwrap();
        assert_eq!(decoded.item_id, decode_row_uid(ROW_UID).unwrap());
        assert_eq!(decoded.transition, valid_recipe());
    }

    #[test]
    fn extended_metadata_response_decodes_transition_data() {
        let response = BatchedExtensionResponse {
            extended_metadata: vec![EntityExtensionDataArray {
                extension_kind: EnumOrUnknown::new(ExtensionKind::TRANSITION_DATA),
                extension_data: vec![
                    librespot_protocol::entity_extension_data::EntityExtensionData {
                        entity_uri: TRANSITION_URI.to_owned(),
                        extension_data: MessageField::some(Any {
                            value: data().write_to_bytes().unwrap(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            decode_transition_data_response(TRANSITION_URI, response)
                .unwrap()
                .playlist_uri,
            PLAYLIST
        );
    }

    #[test]
    fn envelope_mismatches_are_rejected() {
        let cases: Vec<(&str, Box<dyn Fn(&mut TransitionData)>)> = vec![
            (
                "transition URI",
                Box::new(|data| data.transition_uri = "spotify:transition:other".into()),
            ),
            (
                "playlist URI",
                Box::new(|data| data.playlist_uri = "spotify:playlist:other".into()),
            ),
            ("item ID", Box::new(|data| data.item_id = vec![0; 8])),
            (
                "track A",
                Box::new(|data| data.track_a_uri = "spotify:track:other".into()),
            ),
            (
                "track B",
                Box::new(|data| data.track_b_uri = "spotify:track:other".into()),
            ),
        ];
        for (expected, mutate) in cases {
            let mut value = data();
            mutate(&mut value);
            assert!(
                validate_transition_data(&key(), &value)
                    .unwrap_err()
                    .contains(expected)
            );
        }
    }

    #[test]
    fn row_uid_hex_is_compared_to_raw_item_id_bytes() {
        assert_eq!(decode_row_uid(ROW_UID).unwrap(), data().item_id);
        assert!(decode_row_uid("not-hex").is_err());
    }

    #[tokio::test]
    async fn valid_recipe_is_decoded_during_hydration() {
        assert!(hydrate_with(&key(), || async { Ok(data()) }).await.is_ok());
    }

    #[tokio::test]
    async fn malformed_recipe_and_fetch_failure_are_safe() {
        let mut malformed = data();
        malformed.transition = "not base64".to_owned();
        assert!(
            hydrate_with(&key(), || async { Ok(malformed) })
                .await
                .is_err()
        );
        assert_eq!(
            hydrate_with(&key(), || async { Err("service unavailable".to_owned()) })
                .await
                .unwrap_err(),
            "service unavailable"
        );
    }

    #[test]
    fn cache_prevents_duplicate_fetches_and_separates_transition_uris() {
        let mut cache = TransitionHydrationCache::default();
        let key = key();
        let CacheLookup::Start(request_id) = cache.lookup_or_begin(key.clone()) else {
            panic!("first lookup should start a request");
        };
        assert!(matches!(
            cache.lookup_or_begin(key.clone()),
            CacheLookup::Pending
        ));
        assert!(cache.complete(key.clone(), request_id, Err("failed".into())));
        assert!(matches!(
            cache.lookup_or_begin(key.clone()),
            CacheLookup::Unavailable
        ));
        let mut newer = key;
        newer.transition_uri.push_str(":newer");
        assert!(matches!(
            cache.lookup_or_begin(newer),
            CacheLookup::Start(_)
        ));
    }

    #[test]
    fn stale_context_or_pair_does_not_match_hydration_key() {
        let key = key();
        assert!(key.matches_pair(PLAYLIST, ROW_UID, TRANSITION_URI, TRACK_A, TRACK_B));
        assert!(!key.matches_pair(
            "spotify:playlist:other",
            ROW_UID,
            TRANSITION_URI,
            TRACK_A,
            TRACK_B
        ));
        assert!(!key.matches_pair(
            PLAYLIST,
            ROW_UID,
            "spotify:transition:other",
            TRACK_A,
            TRACK_B
        ));
    }

    #[test]
    fn clearing_cache_releases_context_ownership() {
        let mut cache = TransitionHydrationCache::default();
        let key = key();
        let CacheLookup::Start(cancelled_request_id) = cache.lookup_or_begin(key.clone()) else {
            panic!("first lookup should start a request");
        };
        cache.clear();
        let CacheLookup::Start(replacement_request_id) = cache.lookup_or_begin(key.clone()) else {
            panic!("lookup after cancellation should start another request");
        };
        assert_ne!(cancelled_request_id, replacement_request_id);
        assert!(!cache.complete(
            key.clone(),
            cancelled_request_id,
            Err("stale result".into())
        ));
        assert!(matches!(cache.lookup_or_begin(key), CacheLookup::Pending));
    }
}
