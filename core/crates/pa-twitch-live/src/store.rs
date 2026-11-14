// SPDX-License-Identifier: MPL-2.0
//! Where the monitor keeps live timelines between processes.
//!
//! [`TimelineStore`] is the seam; [`LibraryTimelineStore`] (feature
//! `library`) writes them into the TWI-007 model through
//! `pa_twitch::chapters::store` — the same rows the chapter resolver of
//! TWI-008 reads — and resumes open broadcasts after a restart, so a
//! broadcast is announced once even when an Android background check runs
//! in a fresh process every time.

use pa_twitch::Video;
use pa_twitch::chapters::{LiveTimeline, TimelineChange};

/// Storage of live timelines. Errors are logged by the implementation and
/// never stop the monitor.
pub trait TimelineStore: Send {
    /// Broadcasts that were live when the last process stopped.
    fn open_timelines(&mut self, now_ms: i64) -> Vec<LiveTimeline>;

    /// Stores a change of a timeline.
    fn save(&mut self, change: &TimelineChange, now_ms: i64);

    /// Stores the archive VOD of an ended broadcast with the chapters of
    /// our timeline.
    fn save_vod(&mut self, video: &Video, timeline: &LiveTimeline, now_ms: i64) {
        let _ = (video, timeline, now_ms);
    }
}

/// Keeps nothing (tests, the demo).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoStore;

impl TimelineStore for NoStore {
    fn open_timelines(&mut self, _: i64) -> Vec<LiveTimeline> {
        Vec::new()
    }

    fn save(&mut self, _: &TimelineChange, _: i64) {}
}

#[cfg(feature = "library")]
pub use library::{LibraryTimelineStore, RESUME_MAX_AGE_MS};

#[cfg(feature = "library")]
mod library {
    use pa_library::rusqlite::{Connection, params};
    use pa_twitch::Video;
    use pa_twitch::chapters::store as chapters_store;
    use pa_twitch::chapters::{LiveTimeline, ResolvedChapters, TimelineChange, vod_duration_ms};

    use super::TimelineStore;

    /// Open broadcasts older than this are not resumed (the process was
    /// gone for days; the broadcast is long over).
    pub const RESUME_MAX_AGE_MS: i64 = 48 * 3600 * 1000;

    /// Live timelines in the library database.
    pub struct LibraryTimelineStore {
        conn: Connection,
    }

    impl std::fmt::Debug for LibraryTimelineStore {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("LibraryTimelineStore")
                .finish_non_exhaustive()
        }
    }

    impl LibraryTimelineStore {
        /// Store over an open library connection (migrated).
        pub fn new(conn: Connection) -> Self {
            Self { conn }
        }

        /// Opens the library database at `path` (a second connection next
        /// to the app's).
        pub fn open(path: &str) -> Result<Self, pa_library::Error> {
            Ok(Self::new(pa_library::open(path)?))
        }

        /// The connection (tests).
        pub fn connection(&self) -> &Connection {
            &self.conn
        }

        fn open_stream_ids(&self, now_ms: i64) -> Result<Vec<String>, pa_library::Error> {
            let mut stmt = self.conn.prepare(
                "SELECT stream_id FROM twitch_live_sessions
                 WHERE ended_at_ms IS NULL AND started_at_ms >= ?1
                 ORDER BY started_at_ms",
            )?;
            let rows = stmt.query_map(params![now_ms.saturating_sub(RESUME_MAX_AGE_MS)], |r| {
                r.get::<_, String>(0)
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        }
    }

    impl TimelineStore for LibraryTimelineStore {
        fn open_timelines(&mut self, now_ms: i64) -> Vec<LiveTimeline> {
            let ids = self.open_stream_ids(now_ms).unwrap_or_default();
            ids.iter()
                .filter_map(|id| {
                    chapters_store::load_live_timeline(&self.conn, id)
                        .ok()
                        .flatten()
                })
                .collect()
        }

        fn save(&mut self, change: &TimelineChange, now_ms: i64) {
            // A failed write costs chapter precision, not the notification.
            let _ = chapters_store::store_live_timeline(&mut self.conn, change.timeline(), now_ms);
        }

        fn save_vod(&mut self, video: &Video, timeline: &LiveTimeline, now_ms: i64) {
            let chapters = timeline.vod_chapters(vod_duration_ms(video));
            let resolved = ResolvedChapters {
                chapters: (!chapters.is_empty()).then_some(chapters),
                gql_failure: None,
            };
            let _ = chapters_store::store_vod_chapters(&mut self.conn, video, &resolved, now_ms);
        }
    }
}
