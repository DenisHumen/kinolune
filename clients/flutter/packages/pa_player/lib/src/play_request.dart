// SPDX-License-Identifier: MPL-2.0

import 'subtitles/subtitle_search.dart';
import 'tracks/track_policy.dart';
import 'trickplay.dart';
import 'video/osd_model.dart';

/// What to play. The first four fields are the release 0.1 contract
/// (docs/dev/release-0.1-contract.md); the rest are optional extras used by
/// the music UI.
class PlayRequest {
  const PlayRequest({
    required this.uri,
    required this.title,
    this.startPositionMs = 0,
    this.subtitleUri,
    this.artist,
    this.album,
    this.albumId,
    this.artworkUri,
    this.durationMs,
    this.subtitles,
    this.seriesKey,
    this.subtitleMemory,
    this.originalLanguage,
    this.trickplay,
    this.chapters,
    this.skipSegments = const [],
    this.itemId,
    this.subtitleSearch,
  });

  /// File path, `file://` URI or any URI libmpv understands.
  final String uri;
  final String title;

  /// Resume position. `0` starts from the beginning.
  final int startPositionMs;

  /// External subtitle (SRT/ASS/VTT) to load on open.
  final String? subtitleUri;

  /// Music: track artist (shown under the title).
  final String? artist;

  /// Music: album name.
  final String? album;

  /// Music: library album id (MUS-001 `music_albums.id`). Groups tracks for
  /// album ReplayGain ([ReplayGainMode.auto]); falls back to [album].
  final String? albumId;

  /// Music: cover art (file path or http(s) URL).
  final String? artworkUri;

  /// Known duration (e.g. from the library) for display before the file opens.
  final int? durationMs;

  /// Sidecar subtitles from the library (PLAY-007), loaded on open. Null:
  /// unknown, the engine looks for subtitle files next to a local file by
  /// itself; empty: the library knows there are none.
  final List<ExternalSubtitle>? subtitles;

  /// Series the item belongs to: the subtitle choice and delay are kept
  /// per series ([PlayerScreen.onSubtitleMemory]). Null: not remembered.
  final String? seriesKey;

  /// What was remembered for [seriesKey]: subtitle and audio choices and
  /// delays (PLAY-007, PLAY-004).
  final TrackMemory? subtitleMemory;

  /// Original language of the title (TMDB `original_language`, ISO 639-1
  /// or 639-2), for the "Original" audio and subtitle languages
  /// (PLAY-004). Null: unknown.
  final String? originalLanguage;

  /// Seek previews of the file (PLAY-009), shown while scrubbing. The
  /// player owns it: [PlayerScreen] disposes it when the file stops.
  final TrickplaySource? trickplay;

  /// Chapters known to the app (Twitch VOD chapters, TWI-008; probe data,
  /// LIB-006). Null: the OSD shows the chapters the engine reads from the
  /// file ([EngineState.chapters]).
  final List<PlaybackChapter>? chapters;

  /// Parts the user may skip (intro, recap, credits; PLAY-011 finds
  /// them). The OSD shows "Skip Intro" while the playhead is in one.
  final List<SkipSegment> skipSegments;

  /// Library item id of the file (music: the track id, MUS-003). Lets the
  /// app find the track again after a restart, when [uri] may have
  /// changed (a network stream gets a new loopback address).
  final String? itemId;

  /// Subtitles from the internet for this file (PLAY-008): «Найти
  /// субтитры» in the subtitle menu. Null: no such row (no library file,
  /// a stream).
  final SubtitleSearch? subtitleSearch;

  @override
  String toString() => 'PlayRequest($title, $uri)';
}

/// A subtitle file for the current video (a sidecar from the library).
class ExternalSubtitle {
  const ExternalSubtitle({
    required this.uri,
    required this.title,
    this.language,
    this.forced = false,
    this.hearingImpaired = false,
    this.isDefault = false,
    this.parts,
  });

  /// Local path or URL (loopback stream proxy for network sources). For a
  /// subtitle of several [parts]: the first file it has.
  final String uri;

  /// A movie of several parts played as one timeline (LIB-020, LIB-021):
  /// the subtitle file of each part in play order (`Movie.cd1.srt`,
  /// `Movie.cd2.srt`), null where a part has none. The engine joins them
  /// into one track on the common timeline (`partSubtitlesUri`); an engine
  /// that cannot loads [uri] if it belongs to part 1, else nothing. Null
  /// for a subtitle of the whole file or timeline.
  final List<String?>? parts;

  /// Track title; also identifies the loaded track, so it should be unique
  /// among the subtitles of a file (the file name is).
  final String title;

  /// ISO 639 code, if known.
  final String? language;
  final bool forced;
  final bool hearingImpaired;
  final bool isDefault;

  @override
  String toString() => 'ExternalSubtitle($title)';
}
