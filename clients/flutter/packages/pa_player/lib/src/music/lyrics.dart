// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/foundation.dart';

import '../play_request.dart';

/// Where lyrics came from (MUS-006), best first within each kind.
enum LyricsSource {
  /// ID3v2 `SYLT` in the file.
  embeddedSynced,

  /// The `.lrc` file next to the track.
  lrcFile,

  /// The plain lyrics tag of the file (`USLT`, `LYRICS`, `©lyr`); it may
  /// hold LRC text.
  embedded,

  /// lrclib.net, looked up from the device.
  lrclib,
}

/// One line of synchronised lyrics.
@immutable
class LyricLine {
  const LyricLine(this.time, this.text);

  /// When the line starts, from the start of the track.
  final Duration time;

  /// The words; empty for an instrumental gap.
  final String text;

  @override
  bool operator ==(Object other) =>
      other is LyricLine && other.time == time && other.text == text;

  @override
  int get hashCode => Object.hash(time, text);

  @override
  String toString() => 'LyricLine(${time.inMilliseconds} ms, $text)';
}

/// The lyrics of a track: synchronised [lines], or [plainText].
@immutable
class Lyrics {
  /// Synchronised lyrics; [lines] must be sorted by time.
  const Lyrics.synced(this.lines, {required this.source}) : plainText = null;

  /// Lyrics without timing.
  const Lyrics.plain(String text, {required this.source})
    : plainText = text,
      lines = const [];

  final List<LyricLine> lines;
  final String? plainText;
  final LyricsSource source;

  bool get synced => lines.isNotEmpty;

  @override
  String toString() => synced
      ? 'Lyrics.synced(${lines.length} lines, $source)'
      : 'Lyrics.plain($source)';
}

/// Looks up the lyrics of a track; null when there are none. The app gives
/// one to [NowPlayingScreen.lyrics] (tags, `.lrc`, LRCLIB).
typedef LyricsLoader = Future<Lyrics?> Function(PlayRequest track);

/// `[mm:ss]`, `[mm:ss.x]`, `[mm:ss.xx]`, `[mm:ss.xxx]`, also with `:` before
/// the fraction (some editors) and hours of minutes (`[120:00.00]`).
final _timeTag = RegExp(r'^\[(\d{1,4}):(\d{1,2})(?:[.:](\d{1,3}))?\]');

/// `[offset:+250]`, `[ar:Artist]` and the other ID tags.
final _idTag = RegExp(r'^\[([A-Za-z#]+):([^\]]*)\]\s*$');

/// Word timing of enhanced LRC: `<00:12.34>`.
final _wordTag = RegExp(r'<\d{1,4}:\d{1,2}(?:[.:]\d{1,3})?>');

Duration _tagTime(RegExpMatch m) {
  final min = int.parse(m[1]!);
  final sec = int.parse(m[2]!);
  final frac = m[3];
  var ms = 0;
  if (frac != null) {
    // .5 → 500 ms, .05 → 50 ms, .005 → 5 ms.
    ms = int.parse(frac.padRight(3, '0'));
  }
  return Duration(minutes: min, seconds: sec, milliseconds: ms);
}

/// Parses LRC text into lines sorted by time.
///
/// - A line may carry several time tags (`[00:12.00][01:30.00]Chorus`):
///   one [LyricLine] for each.
/// - `[offset:+/-ms]` shifts every line: a positive offset shows the lines
///   earlier (the LRC convention). Times stay at 0 or later.
/// - ID tags (`[ar:]`, `[ti:]`, `[al:]`, `[by:]`, `[length:]`…) and lines
///   without a time tag are skipped; word timing (`<00:12.34>`) is removed.
/// - Lines with the same time keep their order in the file.
///
/// Empty when the text has no time tags: it is plain lyrics.
List<LyricLine> parseLrc(String text) {
  var offset = Duration.zero;
  final lines = <(Duration, int, String)>[];
  var order = 0;
  for (var raw in text.split(RegExp(r'\r\n|\r|\n'))) {
    raw = raw.trim();
    if (raw.startsWith('﻿')) raw = raw.substring(1);
    if (raw.isEmpty) continue;
    final times = <Duration>[];
    var rest = raw;
    for (var m = _timeTag.firstMatch(rest); m != null;) {
      times.add(_tagTime(m));
      rest = rest.substring(m.end).trimLeft();
      m = _timeTag.firstMatch(rest);
    }
    if (times.isEmpty) {
      final id = _idTag.firstMatch(raw);
      if (id != null && id[1]!.toLowerCase() == 'offset') {
        final ms = int.tryParse(id[2]!.trim().replaceFirst('+', ''));
        if (ms != null) offset = Duration(milliseconds: ms);
      }
      continue;
    }
    final words = rest.replaceAll(_wordTag, '').replaceAll(RegExp(r'\s+'), ' ');
    for (final t in times) {
      lines.add((t, order++, words.trim()));
    }
  }
  lines.sort((a, b) {
    final c = a.$1.compareTo(b.$1);
    return c != 0 ? c : a.$2.compareTo(b.$2);
  });
  return [
    for (final (t, _, words) in lines)
      LyricLine(t - offset < Duration.zero ? Duration.zero : t - offset, words),
  ];
}

/// Lyrics from text that may be LRC: synced when it has time tags, else
/// plain (without LRC ID tags). Null for blank text.
Lyrics? lyricsFromText(String? text, LyricsSource source) {
  if (text == null || text.trim().isEmpty) return null;
  final lines = parseLrc(text);
  if (lines.any((l) => l.text.isNotEmpty)) {
    return Lyrics.synced(lines, source: source);
  }
  final plain = text
      .split(RegExp(r'\r\n|\r|\n'))
      .where((l) => !_idTag.hasMatch(l.trim()) && !_timeTag.hasMatch(l.trim()))
      .join('\n')
      .trim();
  return plain.isEmpty ? null : Lyrics.plain(plain, source: source);
}

/// The index of the line sung at [position]: the last line that has
/// started, or -1 before the first one.
int lyricLineAt(List<LyricLine> lines, Duration position) {
  var lo = 0;
  var hi = lines.length - 1;
  var found = -1;
  while (lo <= hi) {
    final mid = (lo + hi) >> 1;
    if (lines[mid].time <= position) {
      found = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return found;
}
