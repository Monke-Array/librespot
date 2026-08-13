use std::collections::HashMap;

use data_encoding::HEXLOWER;
use librespot_core::{Error, Session, SpotifyUri, esperanto};
use librespot_protocol::{
    playlist_episode_decoration_policy::PlaylistEpisodeDecorationPolicy,
    playlist_get_request::{PlaylistGetRequest, PlaylistGetResponse},
    playlist_query::PlaylistQuery,
    playlist_query::playlist_query::BoolPredicate,
    playlist_request,
    playlist_request_decoration_policy::{
        PlaylistItemDecorationPolicy, PlaylistRequestDecorationPolicy,
    },
    playlist_track_decoration_policy::PlaylistTrackDecorationPolicy,
    playlist4_external,
    response_status::ResponseStatus,
    supported_link_types_in_playlists::LinkType,
};
use protobuf::{Message, MessageField};

pub(crate) const PLAYLIST_DATA_SERVICE: &str =
    "spotify.playlist_esperanto.proto.PlaylistDataService";
pub(crate) const GET_METHOD: &str = "Get";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PlaylistItem {
    pub row_uid: String,
    pub uri: String,
    pub format_list_attributes: HashMap<String, String>,
}

#[derive(Clone)]
pub(crate) struct PlaylistDataServiceClient {
    session: Session,
}

impl PlaylistDataServiceClient {
    pub(crate) fn new(session: Session) -> Self {
        Self { session }
    }

    pub(crate) async fn get_item(
        &self,
        playlist_uri: &str,
        row_uid: &str,
    ) -> Result<Option<PlaylistItem>, Error> {
        let request = get_item_request(playlist_uri, row_uid);
        let session = self.session.clone();
        let response: PlaylistGetResponse = esperanto::call_unary(
            PLAYLIST_DATA_SERVICE,
            GET_METHOD,
            &request,
            move |frame| async move { execute_playlist_data_get(&session, frame).await },
        )
        .await?;

        let status = response.status.as_ref().map(|status| status.status_code);
        if !matches!(status, Some(200..=299)) {
            let reason = response
                .status
                .as_ref()
                .map(|status| status.reason.as_str())
                .filter(|reason| !reason.is_empty())
                .unwrap_or("PlaylistDataService/Get failed");
            return Err(Error::unavailable(reason.to_owned()));
        }

        let Some(item) = response.data.as_ref().and_then(|data| data.item.first()) else {
            return Ok(None);
        };
        Ok(Some(project_item(item)))
    }
}

pub(crate) fn get_item_request(playlist_uri: &str, row_uid: &str) -> PlaylistGetRequest {
    PlaylistGetRequest {
        uri: playlist_uri.to_owned(),
        query: MessageField::some(PlaylistQuery {
            bool_predicates: vec![protobuf::EnumOrUnknown::new(
                BoolPredicate::NOT_RECOMMENDATION,
            )],
            supported_placeholder_types: [
                LinkType::SHOW,
                LinkType::KALLAX,
                LinkType::PODCAST_CHAPTER,
            ]
            .into_iter()
            .map(protobuf::EnumOrUnknown::new)
            .collect(),
            show_unavailable: true,
            item_id_filter: row_uid.to_owned(),
            ..Default::default()
        }),
        policy: MessageField::some(PlaylistRequestDecorationPolicy {
            // `Item.uri` is the common decoration used for track-link verification.
            item: MessageField::some(PlaylistItemDecorationPolicy {
                uri: true,
                ..Default::default()
            }),
            track: MessageField::some(PlaylistTrackDecorationPolicy {
                row_id: true,
                format_list_attributes: true,
                ..Default::default()
            }),
            episode: MessageField::some(PlaylistEpisodeDecorationPolicy {
                row_id: true,
                format_list_attributes: true,
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Execute the desktop service contract against the same playlist-v2 Cosmos
/// source already used by librespot. Esperanto itself is an in-process
/// JS/native boundary, not a remotely addressable AP service.
async fn execute_playlist_data_get(session: &Session, frame: Vec<u8>) -> Result<Vec<u8>, Error> {
    let call = esperanto::UnaryCall::decode(&frame)?;
    if call.service != PLAYLIST_DATA_SERVICE || call.method != GET_METHOD {
        return Err(Error::unimplemented(format!(
            "unsupported Esperanto unary call {}/{}",
            call.service, call.method
        )));
    }

    let request = PlaylistGetRequest::parse_from_bytes(&call.payload)?;
    let SpotifyUri::Playlist { id, .. } = SpotifyUri::from_uri(&request.uri)? else {
        return Err(Error::invalid_argument("playlist_uri"));
    };
    let bytes = session.spclient().get_playlist(&id).await?;
    let playlist = playlist4_external::SelectedListContent::parse_from_bytes(&bytes)?;
    project_get_response(&request, &playlist)
        .write_to_bytes()
        .map_err(Into::into)
}

fn project_get_response(
    request: &PlaylistGetRequest,
    playlist: &playlist4_external::SelectedListContent,
) -> PlaylistGetResponse {
    let requested_uid = request
        .query
        .as_ref()
        .map(|query| query.item_id_filter.as_str());
    let item = playlist
        .contents
        .as_ref()
        .into_iter()
        .flat_map(|contents| &contents.items)
        .find(|item| {
            item.attributes.as_ref().is_some_and(|attributes| {
                requested_uid.is_some_and(|uid| {
                    HEXLOWER
                        .encode(attributes.item_id())
                        .eq_ignore_ascii_case(uid)
                })
            })
        })
        .map(|item| {
            let attributes = item.attributes.get_or_default();
            playlist_request::Item {
                row_id: Some(HEXLOWER.encode(attributes.item_id())),
                uri: Some(item.uri().to_owned()),
                format_list_attributes: attributes
                    .format_attributes
                    .iter()
                    .map(|attribute| {
                        librespot_protocol::playlist_playlist_state::FormatListAttribute {
                            key: Some(attribute.key().to_owned()),
                            value: Some(attribute.value().to_owned()),
                            ..Default::default()
                        }
                    })
                    .collect(),
                ..Default::default()
            }
        });

    PlaylistGetResponse {
        status: MessageField::some(ResponseStatus {
            status_code: 200,
            ..Default::default()
        }),
        data: MessageField::some(playlist_request::Response {
            item: item.into_iter().collect(),
            ..Default::default()
        }),
        query: request.query.clone(),
        ..Default::default()
    }
}

fn project_item(item: &playlist_request::Item) -> PlaylistItem {
    PlaylistItem {
        row_uid: item.row_id().to_owned(),
        uri: item.uri().to_owned(),
        format_list_attributes: item
            .format_list_attributes
            .iter()
            .map(|attribute| (attribute.key().to_owned(), attribute.value().to_owned()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYLIST_URI: &str = "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M";
    const ROW_UID: &str = "00112233445566778899aabbccddeeff";

    #[test]
    fn get_request_uses_row_filter_and_minimal_decorations() {
        let request = get_item_request(PLAYLIST_URI, ROW_UID);
        assert_eq!(request.uri, PLAYLIST_URI);
        assert_eq!(request.query.item_id_filter, ROW_UID);
        assert!(request.query.show_unavailable);
        assert_eq!(
            request.query.bool_predicates[0].enum_value().unwrap(),
            BoolPredicate::NOT_RECOMMENDATION
        );
        assert_eq!(
            request
                .query
                .supported_placeholder_types
                .iter()
                .map(|value| value.enum_value().unwrap())
                .collect::<Vec<_>>(),
            vec![LinkType::SHOW, LinkType::KALLAX, LinkType::PODCAST_CHAPTER]
        );

        let policy = request.policy.as_ref().unwrap();
        assert!(policy.item.uri);
        assert!(policy.track.row_id);
        assert!(policy.track.format_list_attributes);
        assert!(policy.episode.row_id);
        assert!(policy.episode.format_list_attributes);
    }

    #[test]
    fn service_projection_filters_by_real_row_id() {
        let request = get_item_request(PLAYLIST_URI, ROW_UID);
        let playlist = playlist4_external::SelectedListContent {
            contents: MessageField::some(playlist4_external::ListItems {
                items: vec![playlist4_external::Item {
                    uri: Some("spotify:track:2TpxZ7JUBn3uw46aR7qd6V".into()),
                    attributes: MessageField::some(playlist4_external::ItemAttributes {
                        item_id: Some(
                            data_encoding::HEXLOWER_PERMISSIVE
                                .decode(ROW_UID.as_bytes())
                                .unwrap(),
                        ),
                        format_attributes: vec![playlist4_external::FormatListAttribute {
                            key: Some("automix.auto_transition_recipe".into()),
                            value: Some("recipe".into()),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        let response = project_get_response(&request, &playlist);
        assert_eq!(response.status.status_code, 200);
        assert_eq!(response.data.item.len(), 1);
        let item = project_item(&response.data.item[0]);
        assert_eq!(item.row_uid, ROW_UID);
        assert_eq!(
            item.format_list_attributes
                .get("automix.auto_transition_recipe")
                .map(String::as_str),
            Some("recipe")
        );
    }

    #[test]
    fn service_projection_returns_no_item_for_a_different_row() {
        let request = get_item_request(PLAYLIST_URI, ROW_UID);
        let playlist = playlist4_external::SelectedListContent::default();
        let response = project_get_response(&request, &playlist);
        assert!(response.data.item.is_empty());
    }
}
