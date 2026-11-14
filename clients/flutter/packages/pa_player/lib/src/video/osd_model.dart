// SPDX-License-Identifier: MPL-2.0

// Data the player OSD (UI-013) shows on top of the engine state: chapters,
// skip segments (Skip Intro), zoom modes, and the small pure helpers the
// OSD and its tests share.

import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/painting.dart';

/// A chapter of the timeline: a container chapter of the file (mpv
/// `chapter-list`, the same data the LIB-006 probe reads), a part of a
/// split movie (LIB-020 EDL), or a Twitch VOD chapter (TWI-008) passed by
/// the app in [PlayRequest.chapters].
@immutable
class PlaybackChapter {
  const PlaybackChapter({required this.start, this.end, this.title});

  final Duration start;

  /// Null: up to the next chapter or the end of the media.
  final Duration? end;

  /// Null or empty: shown as "Chapter N".
  final String? title;

  @override
  bool operator ==(Object other) =>
      other is PlaybackChapter &&
      other.start == start &&
      other.end == end &&
      other.title == title;

  @override
  int get hashCode => Object.hash(start, end, title);

  @override
  String toString() => 'PlaybackChapter($start, $title)';
}

/// Index of the chapter playing at [position] in [chapters] (sorted by
/// start), or -1 before the first one.
int chapterIndexAt(List<PlaybackChapter> chapters, Duration position) {
  var found = -1;
  for (var i = 0; i < chapters.length; i++) {
    if (chapters[i].start <= position) {
      found = i;
    } else {
      break;
    }
  }
  return found;
}

/// Where "previous chapter" goes from [position]: the start of the current
/// chapter, or of the one before it when less than [grace] has played
/// (like a CD player's "back" button).
Duration? previousChapterStart(
  List<PlaybackChapter> chapters,
  Duration position, {
  Duration grace = const Duration(seconds: 3),
}) {
  final i = chapterIndexAt(chapters, position);
  if (i < 0) return chapters.isEmpty ? null : Duration.zero;
  if (position - chapters[i].start > grace || i == 0) return chapters[i].start;
  return chapters[i - 1].start;
}

/// Start of the chapter after the one playing at [position]; null at the
/// last one.
Duration? nextChapterStart(List<PlaybackChapter> chapters, Duration position) {
  final i = chapterIndexAt(chapters, position);
  return i + 1 < chapters.length ? chapters[i + 1].start : null;
}

/// What a skippable segment is (PLAY-011 detects them: IntroDB, chapter
/// names, server data).
enum SkipKind { intro, recap, credits }

/// A part of the media the user may skip: the OSD shows "Skip Intro" /
/// "Skip Credits" while the playhead is inside it and seeks to [end].
@immutable
class SkipSegment {
  const SkipSegment({
    required this.kind,
    required this.start,
    required this.end,
  });

  final SkipKind kind;
  final Duration start;
  final Duration end;

  bool contains(Duration position) => position >= start && position < end;

  @override
  bool operator ==(Object other) =>
      other is SkipSegment &&
      other.kind == kind &&
      other.start == start &&
      other.end == end;

  @override
  int get hashCode => Object.hash(kind, start, end);

  @override
  String toString() => 'SkipSegment(${kind.name}, $start–$end)';
}

/// The segment playing at [position], if any. The last second of a
/// segment offers nothing: skipping it would be pointless.
SkipSegment? skipSegmentAt(List<SkipSegment> segments, Duration position) {
  for (final s in segments) {
    if (s.contains(position) &&
        s.end - position > const Duration(seconds: 1)) {
      return s;
    }
  }
  return null;
}

/// How the picture fills the screen.
enum VideoZoom {
  /// Whole picture, black bars where the aspect differs (default).
  fit,

  /// Fills the screen, cropping the overflow.
  fill,

  /// Fills the screen, distorting the picture.
  stretch,

  /// The picture forced to 16:9 (anamorphic files with a wrong flag).
  ratio16x9,

  /// The picture forced to 4:3.
  ratio4x3,

  /// The picture forced to 2.35:1 (scope).
  ratio235;

  /// The forced display aspect, or null to keep the file's.
  double? get aspect => switch (this) {
    ratio16x9 => 16 / 9,
    ratio4x3 => 4 / 3,
    ratio235 => 2.35,
    _ => null,
  };

  /// [BoxFit] of the video surface in this mode.
  BoxFit get fit => switch (this) {
    fill => BoxFit.cover,
    stretch => BoxFit.fill,
    ratio16x9 || ratio4x3 || ratio235 => BoxFit.fill,
    fit => BoxFit.contain,
  };

  /// The next mode (the zoom key cycles through them).
  VideoZoom get next => values[(index + 1) % values.length];
}

/// Pinch zoom range of the picture.
const double kMinPinchScale = 1.0;
const double kMaxPinchScale = 4.0;

/// [scale] limited to the pinch range.
double clampPinchScale(double scale) =>
    math.min(kMaxPinchScale, math.max(kMinPinchScale, scale));

/// Seek steps offered in the settings (seconds).
const List<int> kSkipStepsSeconds = [5, 10, 15, 30, 60];
