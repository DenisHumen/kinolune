// SPDX-License-Identifier: MPL-2.0

/// Kinolune media library for release 0.1: sources, scanner, filename
/// parser v0, TMDB metadata, SQLite storage and playback progress.
/// See docs/dev/release-0.1-contract.md.
library;

export 'src/anime_db.dart';
export 'src/audiobooks.dart';
export 'src/content_kinds.dart';
export 'src/auto_recognition.dart';
export 'src/artwork_cache.dart'
    show
        ArtworkCache,
        ArtworkCacheUsage,
        RustArtworkCache,
        defaultArtworkCacheBytes;
export 'src/library.dart'
    show Library, continueWatchingMin, continueWatchingMax, showIdFor;
export 'src/episode_order.dart';
export 'src/item_details.dart';
export 'src/library_labels.dart';
export 'src/library_lists.dart';
export 'src/library_profiles.dart';
export 'src/library_query.dart';
export 'src/models.dart';
export 'src/playback_session.dart' show resumePositionMs, nextEpisodeIn;
export 'src/open_library.dart'
    show
        OpenedLibrary,
        openAppLibrary,
        coreDbFileName,
        legacyV0DbFileName,
        freeLegacyV0BackupPath,
        newestLegacyV0Backup;
export 'src/rust_library.dart' show RustLibrary;
export 'src/personal_media.dart';
export 'src/source_files.dart';
export 'src/review_inbox.dart';
export 'src/music.dart';
export 'src/music_index.dart';
export 'src/online_subtitles.dart';
export 'src/music_tags.dart' show TrackInfo, trackInfoFromPath, readTags;
export 'src/play_parts.dart';
export 'src/parser/filename_parser.dart'
    show ParsedName, parseName, parsePath, normalizeTitle, seasonFromFolder;
export 'src/search_groups.dart';
export 'src/skip_segments.dart';
export 'src/subtitles.dart';
export 'src/trailers.dart';
export 'src/untitled_episode.dart';
export 'src/trickplay.dart';
export 'src/watch_marks.dart';
export 'src/scanner.dart'
    show MediaType, ScannedFile, mediaTypeOf, isExtraFile, scanFolder;
export 'src/smb.dart'
    show SmbUrl, SmbResolver, SmbUnsupportedException, SmbMountException;
export 'src/tmdb.dart'
    show
        tmdbAttribution,
        tmdbImageUrl,
        TmdbClient,
        TmdbException,
        TmdbSearchResult;
