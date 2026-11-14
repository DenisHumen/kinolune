// SPDX-License-Identifier: MPL-2.0
//! VOD chapters from GQL `moments` (06 §2.3, source 2), full builds only.
//! The client-id and the persisted-query hash come from the remote config.

use serde::Deserialize;
use serde_json::json;

use super::{
    CHAPTERS_OPERATION, Chapter, ChapterGame, ChapterSource, ChaptersError, GqlChapterSettings,
    vod_duration_ms,
};
use crate::model::Video;
use crate::transport::{HttpRequest, Method, Transport};

/// Default GQL endpoint.
pub(super) const GQL_URL: &str = "https://gql.twitch.tv/gql";

pub(super) fn request_body(settings: &GqlChapterSettings, video_id: &str) -> serde_json::Value {
    json!({
        "operationName": CHAPTERS_OPERATION,
        "variables": {
            "includePrivate": false,
            "videoID": video_id,
        },
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": settings.moments_hash,
            },
        },
    })
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
#[serde(rename_all = "camelCase")]
struct GqlVideo {
    #[serde(default)]
    moments: Option<Moments>,
    /// Present in some versions of the query: the VOD's category.
    #[serde(default)]
    game: Option<GqlGame>,
    #[serde(default)]
    length_seconds: Option<u64>,
}

#[derive(Deserialize)]
struct Moments {
    #[serde(default)]
    edges: Vec<Edge>,
}

#[derive(Deserialize)]
struct Edge {
    node: Moment,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Moment {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    position_milliseconds: u64,
    #[serde(default)]
    duration_milliseconds: u64,
    #[serde(default)]
    description: String,
    #[serde(default)]
    details: Option<Details>,
}

#[derive(Deserialize)]
struct Details {
    #[serde(default)]
    game: Option<GqlGame>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlGame {
    #[serde(default)]
    id: String,
    #[serde(default)]
    display_name: String,
    #[serde(default, rename = "boxArtURL")]
    box_art_url: Option<String>,
}

impl GqlGame {
    fn into_chapter_game(self, fallback_name: &str) -> Option<ChapterGame> {
        let name = if self.display_name.is_empty() {
            fallback_name
        } else {
            &self.display_name
        };
        let mut game = ChapterGame::new(&self.id, name)?;
        game.box_art_url = self.box_art_url.filter(|u| !u.is_empty());
        Some(game)
    }
}

/// Sends the request and maps the answer.
pub(super) async fn fetch_chapters(
    transport: &dyn Transport,
    url: &str,
    settings: &GqlChapterSettings,
    video: &Video,
) -> Result<Vec<Chapter>, ChaptersError> {
    let body = serde_json::to_vec(&request_body(settings, &video.id))
        .map_err(|e| ChaptersError::Decode(e.to_string()))?;
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
        .map_err(|e| ChaptersError::Transport(e.0))?;
    parse_response(response.status, &response.body, vod_duration_ms(video))
}

/// Maps a GQL answer to chapters of a VOD `duration_ms` long (0 = take the
/// length from the answer). `GAME_CHANGE` moments become chapters; a VOD
/// without moments but with a category gets one chapter (like yt-dlp); an
/// unknown VOD gets none.
pub(super) fn parse_response(
    status: u16,
    body: &[u8],
    duration_ms: u64,
) -> Result<Vec<Chapter>, ChaptersError> {
    let parsed: Response = serde_json::from_slice(body).map_err(|e| {
        if (200..300).contains(&status) {
            ChaptersError::Decode(e.to_string())
        } else {
            ChaptersError::Gql {
                status,
                message: String::from_utf8_lossy(&body[..body.len().min(200)]).into_owned(),
            }
        }
    })?;
    if let Some(error) = parsed.errors.first() {
        let lower = error.message.to_ascii_lowercase();
        return Err(if lower.contains("persistedquerynotfound") {
            ChaptersError::GqlHashInvalid
        } else if lower.contains("integrity") {
            ChaptersError::IntegrityRequired
        } else {
            ChaptersError::Gql {
                status,
                message: error.message.clone(),
            }
        });
    }
    if !(200..300).contains(&status) {
        return Err(ChaptersError::Gql {
            status,
            message: parsed.message.unwrap_or_default(),
        });
    }
    let Some(video) = parsed.data.and_then(|d| d.video) else {
        return Ok(Vec::new());
    };
    let duration_ms = if duration_ms > 0 {
        duration_ms
    } else {
        video.length_seconds.unwrap_or(0).saturating_mul(1000)
    };
    let mut chapters: Vec<Chapter> = video
        .moments
        .map(|m| m.edges)
        .unwrap_or_default()
        .into_iter()
        .map(|e| e.node)
        .filter(|m| m.kind == "GAME_CHANGE")
        .filter_map(|m| {
            let game = m.details?.game?.into_chapter_game(&m.description)?;
            let start = m.position_milliseconds;
            let end = (m.duration_milliseconds > 0)
                .then(|| start.saturating_add(m.duration_milliseconds));
            Some(Chapter {
                game,
                title: None,
                start_ms: start,
                end_ms: end,
                source: ChapterSource::GqlMoments,
            })
        })
        .collect();
    chapters.sort_by_key(|c| c.start_ms);
    // An open-ended moment runs to the next one or to the VOD end.
    let starts: Vec<u64> = chapters.iter().skip(1).map(|c| c.start_ms).collect();
    for (i, chapter) in chapters.iter_mut().enumerate() {
        let next = starts.get(i).copied();
        if chapter.end_ms.is_none() {
            chapter.end_ms = next.or((duration_ms > 0).then_some(duration_ms));
        }
        if duration_ms > 0 {
            chapter.end_ms = chapter.end_ms.map(|e| e.min(duration_ms));
        }
    }
    chapters.retain(|c| c.end_ms.is_none_or(|e| e > c.start_ms));
    if chapters.is_empty()
        && let Some(game) = video.game.and_then(|g| g.into_chapter_game(""))
    {
        chapters.push(Chapter {
            game,
            title: None,
            start_ms: 0,
            end_ms: (duration_ms > 0).then_some(duration_ms),
            source: ChapterSource::GqlMoments,
        });
    }
    Ok(chapters)
}
