// SPDX-License-Identifier: MPL-2.0

import 'models.dart';

/// What a skippable segment is (PLAY-011).
enum MediaSkipKind { intro, recap, credits, preview }

/// A part of a file the player may skip: an intro, a recap, the credits
/// or a preview of the next episode.
class MediaSkipSegment {
  const MediaSkipSegment({
    required this.kind,
    required this.startMs,
    required this.endMs,
    this.origin = '',
    this.provider,
    this.providerFile,
  });

  final MediaSkipKind kind;
  final int startMs;
  final int endMs;

  /// Where it comes from: `user`, `import`, `chapter`, `audio`, `video`
  /// (best first; the library keeps the best one of each kind).
  final String origin;

  /// For `import` (PLAY-015): whose markers — `plex`, `jellyfin` or `edl`.
  final String? provider;

  /// For `import`: the sidecar file next to the video the markers were
  /// read from (its name), `null` when the app handed them over.
  final String? providerFile;

  @override
  bool operator ==(Object other) =>
      other is MediaSkipSegment &&
      other.kind == kind &&
      other.startMs == startMs &&
      other.endMs == endMs &&
      other.origin == origin &&
      other.provider == provider &&
      other.providerFile == providerFile;

  @override
  int get hashCode =>
      Object.hash(kind, startMs, endMs, origin, provider, providerFile);

  @override
  String toString() =>
      'MediaSkipSegment(${kind.name}, $startMs–$endMs, $origin'
      '${provider == null ? '' : ' $provider'})';
}

/// A library that knows intros and credits of its files (PLAY-011): chapter
/// names, an audio fingerprint shared by the episodes of a season, credits
/// in the picture, and markers of Plex, Jellyfin or EDL files next to the
/// videos (PLAY-015). The analysis runs in the background, a season at a
/// time.
abstract interface class LibrarySkipSegments {
  /// The segments of the file [item] plays, in time order; empty until its
  /// season was analysed (or if nothing was found).
  Future<List<MediaSkipSegment>> skipSegments(LibraryItem item);

  /// Queues the season (or folder) of [item]'s file for analysis in the
  /// background; returns at once. Files analysed before are not read again.
  Future<void> analyzeSkipSegments(LibraryItem item);
}
