use std::{collections::HashMap, future::Future};

use crate::{
    playlist_data::{PlaylistDataServiceClient, PlaylistItem},
    spotify_mix::{
        BACKEND_RECIPE_ATTRIBUTE, RECIPE_ATTRIBUTE, SpotifyTransitionError, SpotifyTransitionRecipe,
    },
};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TransitionRowKey {
    pub playlist_uri: String,
    pub row_uid: String,
}

#[derive(Clone, Debug)]
pub(crate) struct HydratedTransitionRow {
    pub recipe: Option<SpotifyTransitionRecipe>,
    #[allow(dead_code)]
    pub backend_auto_transition: Option<String>,
}

#[derive(Clone, Debug)]
enum CacheEntry {
    Pending,
    Ready(HydratedTransitionRow),
    Unavailable,
}

#[derive(Clone, Debug)]
pub(crate) enum CacheLookup {
    Start,
    Pending,
    Ready(HydratedTransitionRow),
    Unavailable,
}

#[derive(Default)]
pub(crate) struct TransitionHydrationCache {
    entries: HashMap<TransitionRowKey, CacheEntry>,
}

impl TransitionHydrationCache {
    pub(crate) fn lookup_or_begin(&mut self, key: TransitionRowKey) -> CacheLookup {
        match self.entries.get(&key) {
            Some(CacheEntry::Pending) => CacheLookup::Pending,
            Some(CacheEntry::Ready(row)) => CacheLookup::Ready(row.clone()),
            Some(CacheEntry::Unavailable) => CacheLookup::Unavailable,
            None => {
                self.entries.insert(key, CacheEntry::Pending);
                CacheLookup::Start
            }
        }
    }

    pub(crate) fn complete(
        &mut self,
        key: TransitionRowKey,
        result: Result<HydratedTransitionRow, String>,
    ) {
        self.entries.insert(
            key,
            match result {
                Ok(row) => CacheEntry::Ready(row),
                Err(_) => CacheEntry::Unavailable,
            },
        );
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}

#[derive(Debug)]
pub(crate) struct HydrationResult {
    pub key: TransitionRowKey,
    pub outgoing_uri: String,
    pub result: Result<HydratedTransitionRow, String>,
}

pub(crate) async fn hydrate_transition_row(
    client: PlaylistDataServiceClient,
    key: TransitionRowKey,
    outgoing_uri: String,
) -> HydrationResult {
    let result = hydrate(&client, &key, &outgoing_uri).await;
    HydrationResult {
        key,
        outgoing_uri,
        result,
    }
}

async fn hydrate(
    client: &PlaylistDataServiceClient,
    key: &TransitionRowKey,
    outgoing_uri: &str,
) -> Result<HydratedTransitionRow, String> {
    hydrate_with(key, outgoing_uri, || async {
        client
            .get_item(&key.playlist_uri, &key.row_uid)
            .await
            .map_err(|error| format!("PlaylistDataService/Get failed: {}", error.kind))
    })
    .await
}

async fn hydrate_with<Fetch, FetchFuture>(
    key: &TransitionRowKey,
    outgoing_uri: &str,
    fetch: Fetch,
) -> Result<HydratedTransitionRow, String>
where
    Fetch: FnOnce() -> FetchFuture,
    FetchFuture: Future<Output = Result<Option<PlaylistItem>, String>>,
{
    let item = fetch()
        .await?
        .ok_or_else(|| "requested playlist row was not returned".to_owned())?;

    validate_hydrated_item(key, outgoing_uri, item)
}

fn validate_hydrated_item(
    key: &TransitionRowKey,
    outgoing_uri: &str,
    item: PlaylistItem,
) -> Result<HydratedTransitionRow, String> {
    if !item.row_uid.eq_ignore_ascii_case(&key.row_uid) {
        return Err("returned playlist row UID did not match the request".to_owned());
    }
    if item.uri != outgoing_uri {
        return Err("returned playlist row URI did not match the outgoing track".to_owned());
    }

    let recipe = item
        .format_list_attributes
        .get(RECIPE_ATTRIBUTE)
        .map(|encoded| SpotifyTransitionRecipe::from_base64(encoded))
        .transpose()
        .map_err(|error: SpotifyTransitionError| error.to_string())?;

    Ok(HydratedTransitionRow {
        recipe,
        backend_auto_transition: item
            .format_list_attributes
            .get(BACKEND_RECIPE_ATTRIBUTE)
            .cloned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use data_encoding::BASE64;
    use librespot_protocol::automix_transition::{Overlap, Transition};
    use protobuf::{Message, MessageField};

    const TRACK_A: &str = "spotify:track:2TpxZ7JUBn3uw46aR7qd6V";

    fn key(context: &str) -> TransitionRowKey {
        TransitionRowKey {
            playlist_uri: context.to_owned(),
            row_uid: "00112233445566778899aabbccddeeff".to_owned(),
        }
    }

    #[test]
    fn pending_and_ready_entries_prevent_duplicate_requests() {
        let mut cache = TransitionHydrationCache::default();
        let key = key("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M");
        assert!(matches!(
            cache.lookup_or_begin(key.clone()),
            CacheLookup::Start
        ));
        assert!(matches!(
            cache.lookup_or_begin(key.clone()),
            CacheLookup::Pending
        ));

        cache.complete(
            key.clone(),
            Ok(HydratedTransitionRow {
                recipe: None,
                backend_auto_transition: None,
            }),
        );
        assert!(matches!(
            cache.lookup_or_begin(key),
            CacheLookup::Ready(HydratedTransitionRow { recipe: None, .. })
        ));
    }

    #[test]
    fn playlist_uri_is_part_of_the_cache_key() {
        let mut cache = TransitionHydrationCache::default();
        assert!(matches!(
            cache.lookup_or_begin(key("spotify:playlist:one")),
            CacheLookup::Start
        ));
        assert!(matches!(
            cache.lookup_or_begin(key("spotify:playlist:two")),
            CacheLookup::Start
        ));
    }

    #[test]
    fn failures_are_cached_as_unavailable() {
        let mut cache = TransitionHydrationCache::default();
        let key = key("spotify:playlist:one");
        assert!(matches!(
            cache.lookup_or_begin(key.clone()),
            CacheLookup::Start
        ));
        cache.complete(key.clone(), Err("service failed".into()));
        assert!(matches!(
            cache.lookup_or_begin(key),
            CacheLookup::Unavailable
        ));
    }

    #[test]
    fn clearing_cache_cancels_context_ownership() {
        let mut cache = TransitionHydrationCache::default();
        let key = key("spotify:playlist:one");
        assert!(matches!(
            cache.lookup_or_begin(key.clone()),
            CacheLookup::Start
        ));
        cache.clear();
        assert!(matches!(cache.lookup_or_begin(key), CacheLookup::Start));
    }

    fn item(row_uid: &str, uri: &str, recipe: Option<String>) -> PlaylistItem {
        let mut format_list_attributes = HashMap::new();
        if let Some(recipe) = recipe {
            format_list_attributes.insert(RECIPE_ATTRIBUTE.to_owned(), recipe);
        }
        PlaylistItem {
            row_uid: row_uid.to_owned(),
            uri: uri.to_owned(),
            format_list_attributes,
        }
    }

    fn valid_recipe() -> String {
        let transition = Transition {
            overlap: MessageField::some(Overlap {
                start_a_ms: Some(120_000),
                start_b_ms: Some(0),
                duration_ms: Some(5_000),
                track_a_uri: Some(TRACK_A.to_owned()),
                track_b_uri: Some("spotify:track:4uLU6hMCjMI75M1A2tKUQC".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        };
        BASE64.encode(&transition.write_to_bytes().unwrap())
    }

    #[test]
    fn valid_row_hydrates_a_decoded_recipe() {
        let key = key("spotify:playlist:one");
        let hydrated = validate_hydrated_item(
            &key,
            TRACK_A,
            item(&key.row_uid, TRACK_A, Some(valid_recipe())),
        )
        .unwrap();
        assert!(hydrated.recipe.is_some());
    }

    #[tokio::test]
    async fn asynchronous_hydration_succeeds() {
        let key = key("spotify:playlist:one");
        let expected = item(&key.row_uid, TRACK_A, Some(valid_recipe()));
        let hydrated = hydrate_with(&key, TRACK_A, || async { Ok(Some(expected)) })
            .await
            .unwrap();
        assert!(hydrated.recipe.is_some());
    }

    #[tokio::test]
    async fn service_failure_is_safe() {
        let key = key("spotify:playlist:one");
        let result = hydrate_with(&key, TRACK_A, || async {
            Err("service unavailable".to_owned())
        })
        .await;
        assert_eq!(result.unwrap_err(), "service unavailable");
    }

    #[test]
    fn recipe_absence_is_a_successful_cached_result() {
        let key = key("spotify:playlist:one");
        let hydrated =
            validate_hydrated_item(&key, TRACK_A, item(&key.row_uid, TRACK_A, None)).unwrap();
        assert!(hydrated.recipe.is_none());
    }

    #[test]
    fn mismatching_uid_is_rejected() {
        let key = key("spotify:playlist:one");
        let error = validate_hydrated_item(
            &key,
            TRACK_A,
            item("ffeeddccbbaa99887766554433221100", TRACK_A, None),
        )
        .unwrap_err();
        assert!(error.contains("UID"));
    }

    #[test]
    fn mismatching_uri_is_rejected() {
        let key = key("spotify:playlist:one");
        let error = validate_hydrated_item(
            &key,
            TRACK_A,
            item(&key.row_uid, "spotify:track:wrong", None),
        )
        .unwrap_err();
        assert!(error.contains("URI"));
    }
}
