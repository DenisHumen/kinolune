// SPDX-License-Identifier: MPL-2.0

// Sidecar subtitles of a played file (PLAY-007). RustLibrary finds them in
// the core's folder groups (LIB-007); the v0 library has none (the player
// then lets libmpv pick up subtitles next to local files by itself). The
// subtitles of the parts of a split movie are joined across the parts
// (LIB-021).

import 'models.dart';
import 'play_parts.dart' show LibraryParts;

/// A subtitle file next to a video.
class SidecarSubtitle {
  /// What to hand to the player: a local path, or the loopback stream URL
  /// of a file on a network source (ADR-0038).
  final String uri;

  /// File name, for the track list.
  final String name;

  /// ISO 639-2 code (`rus`, `eng`) if the file name tells.
  final String? language;

  /// Forced subtitles: only the lines in a foreign language.
  final bool forced;

  /// SDH / hearing impaired / closed captions.
  final bool hearingImpaired;

  /// Marked as the default track in its name.
  final bool isDefault;

  /// A movie of several parts (LIB-021): the file of this subtitle for each
  /// part in play order (`Movie.cd1.srt`, `Movie.cd2.srt`), null where a
  /// part has none; [uri] is the first of them. The player joins them into
  /// one track on the common timeline. Null for a subtitle of one file or
  /// of the whole movie.
  final List<String?>? parts;

  const SidecarSubtitle({
    required this.uri,
    required this.name,
    this.language,
    this.forced = false,
    this.hearingImpaired = false,
    this.isDefault = false,
    this.parts,
  });

  @override
  bool operator ==(Object other) =>
      other is SidecarSubtitle &&
      other.uri == uri &&
      other.name == name &&
      other.language == language &&
      other.forced == forced &&
      other.hearingImpaired == hearingImpaired &&
      other.isDefault == isDefault &&
      _sameParts(other.parts, parts);

  static bool _sameParts(List<String?>? a, List<String?>? b) {
    if (a == null || b == null) return a == b;
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) return false;
    }
    return true;
  }

  @override
  int get hashCode => Object.hash(
    uri,
    name,
    language,
    forced,
    hearingImpaired,
    isDefault,
    parts == null ? null : Object.hashAll(parts!),
  );

  @override
  String toString() =>
      'SidecarSubtitle($name, $language${forced ? ', forced' : ''}'
      '${hearingImpaired ? ', sdh' : ''}'
      '${parts == null ? '' : ', parts $parts'})';
}

/// The subtitles of a movie of several parts (LIB-021) from the sidecars
/// of each part ([perPart], in play order, as [LibrarySubtitles] gives them
/// for one file).
///
/// A file listed for every part belongs to the whole movie (a subtitle of
/// its folder) and is kept as it is, after the others. The rest belong to
/// one part each and are joined across the parts into tracks: the files of
/// one kind — extension, language, forced and SDH — are one track, the
/// first such file of each part with the first of the others, the second
/// with the second, and so on (`Movie.cd1.en.srt` + `Movie.cd2.en.srt`,
/// `Movie.cd1.ru.srt` + `Movie.cd2.ru.srt`). A track gets the name, uri and
/// flags of its first file and [SidecarSubtitle.parts]; one that only part
/// 1 has stays a plain subtitle. Tracks come in the order their first files
/// do.
List<SidecarSubtitle> joinPartSubtitles(List<List<SidecarSubtitle>> perPart) {
  if (perPart.isEmpty) return const [];
  if (perPart.length == 1) return perPart.single;
  final whole = [
    for (final s in perPart.first)
      if (perPart.every((part) => part.any((o) => o.uri == s.uri))) s,
  ];
  final wholeUris = {for (final s in whole) s.uri};
  String ext(SidecarSubtitle s) {
    final name = s.name.toLowerCase();
    final dot = name.lastIndexOf('.');
    return dot < 0 ? '' : name.substring(dot + 1);
  }

  final tracks = <String, List<SidecarSubtitle?>>{};
  for (var i = 0; i < perPart.length; i++) {
    final seen = <String, int>{};
    for (final s in perPart[i]) {
      if (wholeUris.contains(s.uri)) continue;
      final kind =
          '${ext(s)}|${s.language}|${s.forced}|${s.hearingImpaired}';
      final n = seen[kind] = (seen[kind] ?? 0) + 1;
      final files = tracks.putIfAbsent(
        '$kind|$n',
        () => List.filled(perPart.length, null),
      );
      files[i] = s;
    }
  }
  return [
    for (final files in tracks.values)
      if (files.skip(1).every((f) => f == null))
        files.first!
      else
        _joined(files),
    ...whole,
  ];
}

SidecarSubtitle _joined(List<SidecarSubtitle?> files) {
  final first = files.firstWhere((f) => f != null)!;
  return SidecarSubtitle(
    uri: first.uri,
    name: first.name,
    language: first.language,
    forced: first.forced,
    hearingImpaired: first.hearingImpaired,
    isDefault: files.any((f) => f?.isDefault ?? false),
    parts: [for (final f in files) f?.uri],
  );
}

/// A library that knows the subtitle files of its videos.
abstract interface class LibrarySubtitles {
  /// Subtitle files of the file [item] plays ([LibraryItem.playbackUri]:
  /// the item itself or a version picked with [LibraryItem.withFile]):
  /// those named after it first, then those of its movie folder. For a
  /// movie of several parts ([LibraryParts]), the subtitles of the parts
  /// joined with [joinPartSubtitles] (LIB-021). Empty if the file is not in
  /// the library.
  Future<List<SidecarSubtitle>> sidecarSubtitles(LibraryItem item);
}
