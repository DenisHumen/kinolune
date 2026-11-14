// SPDX-License-Identifier: MPL-2.0
//! Chapters in the library (feature `library`): the TWI-007 model of
//! `pa-library` (`twitch_live_sessions`, `twitch_videos`,
//! `twitch_category_segments`, `twitch_categories`).
//!
//! The 24-hour cache cap of `pa_library::twitch::purge_expired` applies:
//! chapters go with their VOD or live session unless the user saved it.
//! While they are stored they are not fetched again
//! ([`stored_source`] → [`super::VodChaptersRequest::stored`]).

use pa_library::Error;
use pa_library::rusqlite::Connection;
use pa_library::twitch::{self as lib, CategorySegment, LiveSession, SegmentOwner};

use super::{
    Chapter, ChapterGame, ChapterSource, LiveTimeline, ResolvedChapters, TimelineSegment,
    vod_duration_ms,
};
use crate::model::{Video, VideoType};
use crate::util::parse_rfc3339_ms;

fn is_twitch_id(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('0') && s.bytes().all(|b| b.is_ascii_digit())
}

/// A Helix VOD as the library models it; `None` for kinds the library does
/// not know or without a parsable time.
pub fn model_video(video: &Video) -> Option<pa_model::Video> {
    let video_type = match video.kind {
        VideoType::Archive => pa_model::VideoType::Archive,
        VideoType::Highlight => pa_model::VideoType::Highlight,
        VideoType::Upload => pa_model::VideoType::Upload,
        VideoType::Other => return None,
    };
    let published_at_ms =
        parse_rfc3339_ms(&video.published_at).or_else(|| parse_rfc3339_ms(&video.created_at))?;
    Some(pa_model::Video {
        video_id: video.id.clone(),
        user_id: video.user_id.clone(),
        stream_id: video.stream_id.clone().filter(|s| !s.is_empty()),
        video_type,
        title: video.title.clone(),
        published_at_ms,
        duration_ms: vod_duration_ms(video),
        view_count: video.view_count,
        thumbnail_url: (!video.thumbnail_url.is_empty()).then(|| video.thumbnail_url.clone()),
        is_sub_only: None,
        availability: pa_model::VideoAvailability::Online,
    })
}

/// Stores a VOD fetched from Helix at `now_ms`. Returns false if the
/// library cannot model it ([`model_video`]).
pub fn store_video(conn: &Connection, video: &Video, now_ms: i64) -> Result<bool, Error> {
    let Some(model) = model_video(video) else {
        return Ok(false);
    };
    lib::upsert_video(conn, &model, now_ms)?;
    Ok(true)
}

/// Stores the categories of `chapters` (keeping known box art) and replaces
/// the chapters of `owner`, which must be stored. Chapters whose category
/// id is not a Twitch id are skipped.
pub fn store_chapters(
    conn: &mut Connection,
    owner: &SegmentOwner,
    chapters: &[Chapter],
    now_ms: i64,
) -> Result<(), Error> {
    let chapters: Vec<&Chapter> = chapters
        .iter()
        .filter(|c| is_twitch_id(&c.game.id))
        .collect();
    let tx = conn.transaction()?;
    for chapter in &chapters {
        let known = lib::category(&tx, &chapter.game.id)?;
        let name = if chapter.game.name.is_empty() {
            match &known {
                Some(k) => k.name.clone(),
                // A category is never stored without a name.
                None => chapter.game.id.clone(),
            }
        } else {
            chapter.game.name.clone()
        };
        let (box_art_url, igdb_id) = match known {
            Some(k) => (
                chapter.game.box_art_url.clone().or(k.box_art_url),
                k.igdb_id,
            ),
            None => (chapter.game.box_art_url.clone(), None),
        };
        lib::upsert_category(
            &tx,
            &lib::Category {
                game_id: chapter.game.id.clone(),
                name,
                box_art_url,
                igdb_id,
            },
            now_ms,
        )?;
    }
    tx.commit()?;
    let segments: Vec<CategorySegment> = chapters
        .iter()
        .map(|c| CategorySegment {
            game_id: c.game.id.clone(),
            title_at_time: c.title.clone(),
            start_ms: c.start_ms,
            end_ms: c.end_ms,
            source: c.source.as_str().to_owned(),
        })
        .collect();
    lib::replace_segments(conn, owner, &segments)
}

/// Stores a live timeline: the live session and its chapters.
pub fn store_live_timeline(
    conn: &mut Connection,
    timeline: &LiveTimeline,
    now_ms: i64,
) -> Result<(), Error> {
    lib::upsert_live_session(
        conn,
        &LiveSession {
            stream_id: timeline.stream_id.clone(),
            user_id: timeline.user_id.clone(),
            started_at_ms: timeline.started_at_ms,
            ended_at_ms: timeline.ended_at_ms,
            title_last: timeline.title_last.clone(),
            peak_viewers: None,
            video_id: None,
        },
        now_ms,
    )?;
    store_chapters(
        conn,
        &SegmentOwner::Stream(timeline.stream_id.clone()),
        &timeline.chapters(),
        now_ms,
    )
}

/// A stored live timeline (to resume a tracker after a restart), with
/// category names from the stored categories.
pub fn load_live_timeline(
    conn: &Connection,
    stream_id: &str,
) -> Result<Option<LiveTimeline>, Error> {
    let Some(session) = lib::live_session(conn, stream_id)? else {
        return Ok(None);
    };
    let base = session.started_at_ms;
    let at = |offset: u64| base.saturating_add(i64::try_from(offset).unwrap_or(i64::MAX));
    let mut segments = Vec::new();
    for s in lib::segments(conn, &SegmentOwner::Stream(stream_id.to_owned()))? {
        let known = lib::category(conn, &s.game_id)?;
        segments.push(TimelineSegment {
            game: Some(ChapterGame {
                id: s.game_id.clone(),
                name: known.as_ref().map(|k| k.name.clone()).unwrap_or_default(),
                box_art_url: known.and_then(|k| k.box_art_url),
            }),
            title: s.title_at_time,
            started_at_ms: at(s.start_ms),
            ended_at_ms: s.end_ms.map(at),
            source: ChapterSource::parse(&s.source).unwrap_or(ChapterSource::Polling),
        });
    }
    Ok(Some(LiveTimeline {
        stream_id: session.stream_id,
        user_id: session.user_id,
        started_at_ms: base,
        ended_at_ms: session.ended_at_ms,
        title_last: session.title_last,
        segments,
    }))
}

/// The best source of the chapters stored for a VOD: our timeline, then
/// GQL, then the fallback; `None` without chapters.
pub fn stored_source(conn: &Connection, video_id: &str) -> Result<Option<ChapterSource>, Error> {
    let rank = |s: ChapterSource| match s {
        ChapterSource::EventSub | ChapterSource::Polling => 3,
        ChapterSource::GqlMoments => 2,
        ChapterSource::Fallback => 1,
    };
    Ok(
        lib::segments(conn, &SegmentOwner::Video(video_id.to_owned()))?
            .iter()
            .filter_map(|s| ChapterSource::parse(&s.source))
            .max_by_key(|s| rank(*s)),
    )
}

/// Stores a VOD and, if the resolver found new ones, its chapters; links
/// the VOD to its stored live session. Returns whether chapters were
/// written.
pub fn store_vod_chapters(
    conn: &mut Connection,
    video: &Video,
    resolved: &ResolvedChapters,
    now_ms: i64,
) -> Result<bool, Error> {
    if !store_video(conn, video, now_ms)? {
        return Ok(false);
    }
    if let Some(stream_id) = video.stream_id.as_deref().filter(|s| !s.is_empty())
        && let Some(mut session) = lib::live_session(conn, stream_id)?
        && session.video_id.is_none()
    {
        session.video_id = Some(video.id.clone());
        lib::upsert_live_session(conn, &session, now_ms)?;
    }
    let Some(chapters) = &resolved.chapters else {
        return Ok(false);
    };
    store_chapters(
        conn,
        &SegmentOwner::Video(video.id.clone()),
        chapters,
        now_ms,
    )?;
    Ok(true)
}
