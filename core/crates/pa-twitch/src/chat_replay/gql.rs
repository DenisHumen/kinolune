// SPDX-License-Identifier: MPL-2.0
//! GQL `VideoCommentsByOffsetOrCursor` (06 §2.5): one page of a VOD's chat
//! by offset (whole seconds) or by the cursor of the last comment. The
//! client-id and the persisted-query hash come from the remote config.

use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::td::{Comment, Commenter, Emoticon, Fragment, Message, UserBadge};
use super::{COMMENTS_OPERATION, ChatReplayError, ChatReplaySettings};
use crate::transport::{HttpRequest, Method, Transport};

/// Default GQL endpoint.
pub(super) const GQL_URL: &str = "https://gql.twitch.tv/gql";

/// Where a page starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PageStart {
    /// Comments from this second on.
    Offset(u64),
    /// Comments after the one with this cursor.
    Cursor(String),
}

/// One answer.
#[derive(Debug, Clone, Default)]
pub(super) struct Page {
    /// Comments in the answer's order, deleted ones left out.
    pub comments: Vec<Comment>,
    /// Twitch has more comments after this page.
    pub has_next: bool,
    /// Cursor of the last comment, for [`PageStart::Cursor`].
    pub last_cursor: Option<String>,
    /// Comments of the page Twitch marked deleted (left out).
    pub deleted: usize,
}

pub(super) fn request_body(
    settings: &ChatReplaySettings,
    video_id: &str,
    start: &PageStart,
) -> Value {
    let variables = match start {
        PageStart::Offset(secs) => json!({ "videoID": video_id, "contentOffsetSeconds": secs }),
        PageStart::Cursor(cursor) => json!({ "videoID": video_id, "cursor": cursor }),
    };
    json!({
        "operationName": COMMENTS_OPERATION,
        "variables": variables,
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": settings.comments_hash,
            },
        },
    })
}

pub(super) async fn fetch_page(
    transport: &dyn Transport,
    url: &str,
    settings: &ChatReplaySettings,
    video_id: &str,
    start: &PageStart,
) -> Result<Page, ChatReplayError> {
    let body = serde_json::to_vec(&request_body(settings, video_id, start))
        .map_err(|e| ChatReplayError::Decode(e.to_string()))?;
    let request = HttpRequest {
        method: Method::Post,
        url: url.to_owned(),
        headers: vec![
            ("Client-ID".to_owned(), settings.client_id.clone()),
            ("Content-Type".to_owned(), "application/json".to_owned()),
        ],
        body: Some(body),
    };
    let response = transport
        .send(request)
        .await
        .map_err(|e| ChatReplayError::Transport(e.0))?;
    parse_page(response.status, &response.body, video_id)
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    data: Option<Data>,
    #[serde(default)]
    errors: Vec<GqlError>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
struct GqlError {
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct Data {
    #[serde(default)]
    video: Option<GqlVideo>,
}

#[derive(Deserialize)]
struct GqlVideo {
    #[serde(default)]
    creator: Option<Creator>,
    #[serde(default)]
    comments: Option<Comments>,
}

#[derive(Deserialize)]
struct Creator {
    #[serde(default)]
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Comments {
    #[serde(default)]
    edges: Vec<Edge>,
    #[serde(default)]
    page_info: Option<PageInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    #[serde(default)]
    has_next_page: bool,
}

#[derive(Deserialize)]
struct Edge {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    node: Option<Node>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Node {
    #[serde(default)]
    id: String,
    #[serde(default)]
    commenter: Option<GqlUser>,
    #[serde(default)]
    content_offset_seconds: f64,
    #[serde(default)]
    created_at: Option<String>,
    /// `null` for a message removed by a moderator.
    #[serde(default)]
    message: Option<GqlMessage>,
    #[serde(default)]
    state: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlUser {
    #[serde(default)]
    id: String,
    #[serde(default)]
    login: String,
    #[serde(default)]
    display_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlMessage {
    #[serde(default)]
    fragments: Vec<GqlFragment>,
    #[serde(default)]
    user_badges: Vec<GqlBadge>,
    #[serde(default)]
    user_color: Option<String>,
    #[serde(default)]
    is_deleted: Option<bool>,
}

#[derive(Deserialize)]
struct GqlFragment {
    #[serde(default)]
    text: String,
    #[serde(default)]
    emote: Option<GqlEmote>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlEmote {
    #[serde(default, rename = "emoteID")]
    emote_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlBadge {
    #[serde(default, rename = "setID")]
    set_id: String,
    #[serde(default)]
    version: String,
}

/// Maps a GQL answer to a page of TwitchDownloader comments.
pub(super) fn parse_page(status: u16, body: &[u8], video_id: &str) -> Result<Page, ChatReplayError> {
    let parsed: Response = serde_json::from_slice(body).map_err(|e| {
        if (200..300).contains(&status) {
            ChatReplayError::Decode(e.to_string())
        } else {
            ChatReplayError::Gql {
                status,
                message: String::from_utf8_lossy(&body[..body.len().min(200)]).into_owned(),
            }
        }
    })?;
    if let Some(error) = parsed.errors.first() {
        let lower = error.message.to_ascii_lowercase();
        return Err(if lower.contains("persistedquerynotfound") {
            ChatReplayError::GqlHashInvalid
        } else if lower.contains("integrity") {
            ChatReplayError::IntegrityRequired
        } else {
            ChatReplayError::Gql {
                status,
                message: error.message.clone(),
            }
        });
    }
    if !(200..300).contains(&status) {
        return Err(ChatReplayError::Gql {
            status,
            message: parsed.message.unwrap_or_default(),
        });
    }
    let video = parsed
        .data
        .and_then(|d| d.video)
        .ok_or(ChatReplayError::NotFound)?;
    let channel_id = video.creator.map(|c| c.id).filter(|id| !id.is_empty());
    let Some(comments) = video.comments else {
        return Ok(Page::default());
    };
    let mut page = Page {
        has_next: comments.page_info.is_some_and(|p| p.has_next_page),
        ..Page::default()
    };
    for edge in comments.edges {
        if edge.cursor.is_some() {
            page.last_cursor = edge.cursor;
        }
        let Some(node) = edge.node else { continue };
        let deleted = node
            .state
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("deleted"));
        let Some(message) = node.message.filter(|m| m.is_deleted != Some(true)) else {
            page.deleted += 1;
            continue;
        };
        if deleted {
            page.deleted += 1;
            continue;
        }
        let user = node.commenter.unwrap_or(GqlUser {
            id: String::new(),
            login: String::new(),
            display_name: String::new(),
        });
        let fragments: Vec<Fragment> = message
            .fragments
            .into_iter()
            .map(|f| Fragment {
                text: f.text,
                emoticon: f
                    .emote
                    .filter(|e| !e.emote_id.is_empty())
                    .map(|e| Emoticon {
                        emoticon_id: e.emote_id,
                    }),
            })
            .collect();
        page.comments.push(Comment {
            id: node.id,
            created_at: node.created_at,
            channel_id: channel_id.clone(),
            content_type: Some("video".into()),
            content_id: Some(video_id.to_owned()),
            content_offset_seconds: node.content_offset_seconds,
            commenter: Commenter {
                display_name: user.display_name,
                id: user.id,
                name: user.login,
                extra: Map::new(),
            },
            message: Message {
                body: fragments.iter().map(|f| f.text.as_str()).collect(),
                fragments,
                user_badges: message
                    .user_badges
                    .into_iter()
                    .filter(|b| !b.set_id.is_empty())
                    .map(|b| UserBadge {
                        id: b.set_id,
                        version: b.version,
                    })
                    .collect(),
                user_color: message.user_color.filter(|c| !c.is_empty()),
                bits_spent: None,
                extra: Map::new(),
            },
            extra: Map::new(),
        });
    }
    Ok(page)
}
