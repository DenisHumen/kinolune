// SPDX-License-Identifier: MPL-2.0

// A movie split over several files (LIB-020: the VOBs of a DVD title set,
// `cd1`/`cd2` parts) plays as one timeline through an mpv EDL URI
// (https://mpv.io/manual/stable/#edl): mpv opens the parts as segments of
// one virtual file with a common duration and seeking across the parts,
// and makes a chapter of each part. media_kit passes a URI with a scheme it
// does not know (`edl://`) to libmpv unchanged (media_kit 1.2.6,
// `Media.normalizeURI`), so the app needs no access to the `mpv_handle`.

import 'dart:convert';

/// Scheme of an mpv EDL URI.
const edlScheme = 'edl://';

/// One URI that plays [parts] (local paths or URLs) back to back: the part
/// itself if there is only one, else an `edl://` URI. Each part is written
/// as `%<length in UTF-8 bytes>%<part>`, so any character in a path or URL
/// (`;`, `,`, `%`, newlines) is safe. No segment start or length is given:
/// mpv takes each part whole, from its first timestamp (a VOB of a DVD title
/// set does not start at 0).
String partsUri(List<String> parts) {
  if (parts.isEmpty) {
    throw ArgumentError.value(parts, 'parts', 'nothing to play');
  }
  if (parts.length == 1) return parts.single;
  return edlScheme + parts.map((p) => '%${utf8.encode(p).length}%$p').join(';');
}

/// The parts of an EDL URI made by [partsUri]; a plain URI is its own only
/// part. Null for a malformed EDL.
List<String>? partsOf(String uri) {
  if (!uri.startsWith(edlScheme)) return [uri];
  final bytes = utf8.encode(uri.substring(edlScheme.length));
  final out = <String>[];
  var i = 0;
  while (i < bytes.length) {
    if (bytes[i] != 0x25) return null; // '%'
    final end = bytes.indexOf(0x25, i + 1);
    if (end < 0) return null;
    final len = int.tryParse(ascii.decode(bytes.sublist(i + 1, end)));
    if (len == null || end + 1 + len > bytes.length) return null;
    out.add(utf8.decode(bytes.sublist(end + 1, end + 1 + len)));
    i = end + 1 + len;
    if (i < bytes.length) {
      if (bytes[i] != 0x3b) return null; // ';'
      i++;
    }
  }
  return out.isEmpty ? null : out;
}

/// File name of a played URI for the info panel: the last path segment,
/// percent-decoded where possible; for an EDL URI the name of its first
/// part and the number of parts (`Movie.cd1.avi (+1)`).
String mediaDisplayName(String uri) {
  final parts = partsOf(uri);
  if (parts == null) return 'EDL';
  final first = parts.first.split('/').last;
  String name;
  try {
    name = Uri.decodeFull(first);
  } on ArgumentError {
    name = first;
  } on FormatException {
    name = first;
  }
  return parts.length > 1 ? '$name (+${parts.length - 1})' : name;
}

/// Lengths of the parts of an EDL timeline made by [partsUri] (LIB-021),
/// from mpv's `chapter-list`: mpv makes a chapter at the start of each part
/// titled with the part as written in the EDL (the chapters of the files
/// themselves keep their own titles). [chapters] are `(title, start)`
/// pairs; [total] is the duration of the timeline. Null if a part has no
/// chapter or the starts do not increase from zero.
List<Duration>? partLengths(
  List<String> parts,
  List<(String, Duration)> chapters,
  Duration total,
) {
  if (parts.isEmpty) return null;
  final starts = <Duration>[];
  var from = 0;
  for (final part in parts) {
    final i = chapters.indexWhere((c) => c.$1 == part, from);
    if (i < 0) return null;
    final start = chapters[i].$2;
    if (starts.isNotEmpty && start <= starts.last) return null;
    starts.add(start);
    from = i + 1;
  }
  if (starts.first != Duration.zero || total <= starts.last) return null;
  return [
    for (var i = 0; i < starts.length; i++)
      (i + 1 < starts.length ? starts[i + 1] : total) - starts[i],
  ];
}

String _seconds(Duration d) => (d.inMicroseconds / 1e6).toStringAsFixed(3);

/// One subtitle track for a timeline of several parts (LIB-021):
/// [subtitles] holds the subtitle file of each part (local path or URL;
/// null where a part has none), [lengths] the length of each part (see
/// [partLengths]).
///
/// The result is an mpv EDL like the video's: each file is a segment as
/// long as its part, so mpv shows the lines of `Movie.cd2.srt` shifted by
/// the length of part 1. A part without subtitles widens the segment of the
/// next file to the left with a negative start (`start=-<gap>`), so the
/// file's timestamps still begin where its part does; parts without
/// subtitles at the end are left out. mpv opens every segment with its own
/// demuxer and codepage detection, so any format and encoding mpv reads
/// works; the track takes the codec of the first file, so the files should
/// be of one format. Null if no part has subtitles; the file itself if only
/// part 1 has one.
String? partSubtitlesUri(List<String?> subtitles, List<Duration> lengths) {
  if (subtitles.length != lengths.length) {
    throw ArgumentError('one subtitle entry per part');
  }
  final present = [
    for (var i = 0; i < subtitles.length; i++)
      if (subtitles[i] != null) i,
  ];
  if (present.isEmpty) return null;
  if (present.length == 1 && present.single == 0) return subtitles[0];
  final segments = <String>[];
  var covered = Duration.zero;
  var start = Duration.zero;
  for (var i = 0; i < subtitles.length; i++) {
    final file = subtitles[i];
    if (file != null) {
      final gap = start - covered;
      final params = [
        if (gap > Duration.zero) 'start=-${_seconds(gap)}',
        'length=${_seconds(gap + lengths[i])}',
      ].join(',');
      segments.add('%${utf8.encode(file).length}%$file,$params');
      covered = start + lengths[i];
    }
    start += lengths[i];
  }
  return edlScheme + segments.join(';');
}
