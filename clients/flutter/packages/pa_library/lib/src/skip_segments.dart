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
  });

  final MediaSkipKind kind;
  final int startMs;
  final int endMs;

  /// Where it comes from: `chapter`, `audio`, `video`, `import`, `user`.
  final String origin;

  @override
  bool operator ==(Object other) =>
      other is MediaSkipSegment &&
      other.kind == kind &&
      other.startMs == startMs &&
      other.endMs == endMs &&
      other.origin == origin;

  @override
  int get hashCode => Object.hash(kind, startMs, endMs, origin);

  @override
  String toString() => 'MediaSkipSegment(${kind.name}, $startMs–$endMs, $origin)';
}

/// A library that knows intros and credits of its files (PLAY-011): chapter
/// names, an audio fingerprint shared by the episodes of a season, credits
/// in the picture. The analysis runs in the background, a season at a time.
abstract interface class LibrarySkipSegments {
  /// The segments of the file [item] plays, in time order; empty until its
  /// season was analysed (or if nothing was found).
  Future<List<MediaSkipSegment>> skipSegments(LibraryItem item);

  /// Queues the season (or folder) of [item]'s file for analysis in the
  /// background; returns at once. Files analysed before are not read again.
  Future<void> analyzeSkipSegments(LibraryItem item);
}
