// SPDX-License-Identifier: MPL-2.0
/// Dart bindings to the project_alpha Rust core (ADR-0001).
///
/// Call `await RustLib.init()` once before any other function, for example at
/// the top of `main()`. The native library `libpa_frb` is built from
/// `core/crates/pa-frb` by the package's Native Assets hook and bundled with
/// the app automatically.
library;

export 'src/rust/api/anime_db.dart';
export 'src/rust/api/app_update.dart';
export 'src/rust/api/audiobooks.dart';
export 'src/rust/api/availability.dart';
export 'src/rust/api/cloud.dart';
export 'src/rust/api/content_kind.dart';
export 'src/rust/api/details.dart';
export 'src/rust/api/diagnostics.dart';
export 'src/rust/api/discovery.dart';
export 'src/rust/api/dlna.dart';
export 'src/rust/api/episodes.dart';
// coreEvents() and the other long streams wrap the raw calls (BUG-007,
// BUG-008).
export 'src/core_events.dart';
export 'src/rust/api/events.dart'
    hide coreEventsListen, coreStreamClose, coreStreamNextId;
export 'src/rust/api/files.dart';
export 'src/rust/api/folder_rules.dart';
export 'src/rust/api/images.dart';
export 'src/rust/api/jellyfin.dart';
export 'src/rust/api/knowledge.dart';
export 'src/rust/api/lan.dart';
export 'src/rust/api/library.dart';
export 'src/rust/api/library_labels.dart';
export 'src/rust/api/library_lists.dart';
export 'src/rust/api/library_query.dart';
export 'src/rust/api/lyrics.dart';
export 'src/rust/api/music.dart';
export 'src/rust/api/offline.dart';
export 'src/rust/api/personal_media.dart';
export 'src/rust/api/plex.dart';
export 'src/rust/api/profiles.dart';
export 'src/rust/api/recognition.dart';
export 'src/rust/api/remote_config.dart';
export 'src/rust/api/review.dart';
export 'src/rust/api/scrobble.dart';
export 'src/rust/api/secrets.dart';
export 'src/rust/api/skip.dart';
export 'src/rust/api/sources.dart';
export 'src/rust/api/subsonic.dart';
export 'src/rust/api/subtitle_search.dart';
export 'src/rust/api/subtitles.dart';
export 'src/rust/api/sync.dart';
export 'src/rust/api/trailers.dart';
export 'src/rust/api/trakt.dart';
export 'src/rust/api/trickplay.dart';
export 'src/rust/api/twitch.dart';
export 'src/rust/api/twitch_archive.dart';
export 'src/rust/api/twitch_chat.dart';
export 'src/rust/api/twitch_chat_replay.dart';
export 'src/rust/api/twitch_live.dart';
export 'src/rust/api/twitch_play.dart';
export 'src/rust/api/version.dart';
export 'src/rust/api/watch_marks.dart';
export 'src/rust/frb_generated.dart' show RustLib;
