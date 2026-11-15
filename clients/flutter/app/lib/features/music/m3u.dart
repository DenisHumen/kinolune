// SPDX-License-Identifier: MPL-2.0
//
// M3U/M3U8 import and export of audio playlists (MUS-003). Export writes
// UTF-8 extended M3U with paths relative to the playlist file when both are
// under the same root (a folder of music carries its playlists along), and
// absolute paths or URLs otherwise. Import resolves each line against the
// playlist's folder and finds the library track by its path; a playlist made
// on another computer (other root, other drive, Windows separators) still
// matches by the longest common tail of the path.

import 'dart:convert';

import '../../integration/library.dart';

/// One line of a playlist: where the file is and what `#EXTINF` said.
class M3uEntry {
  const M3uEntry(this.location, {this.title, this.durationSeconds});

  /// As written: a relative or absolute path, or a URL.
  final String location;
  final String? title;
  final int? durationSeconds;

  @override
  String toString() => 'M3uEntry($location)';
}

/// Text of a playlist file: UTF-8 (M3U8, and most `.m3u` today), else
/// Latin-1 (old `.m3u`).
String decodeM3u(List<int> bytes) {
  var b = bytes;
  if (b.length >= 3 && b[0] == 0xEF && b[1] == 0xBB && b[2] == 0xBF) {
    b = b.sublist(3);
  }
  try {
    return utf8.decode(b);
  } on FormatException {
    return latin1.decode(b);
  }
}

/// Reads an M3U/M3U8 playlist; comments and directives other than
/// `#EXTINF` are skipped.
List<M3uEntry> parseM3u(String text) {
  final out = <M3uEntry>[];
  String? title;
  int? seconds;
  for (final raw in const LineSplitter().convert(text)) {
    final line = raw.trim();
    if (line.isEmpty) continue;
    if (line.startsWith('#')) {
      if (line.toUpperCase().startsWith('#EXTINF:')) {
        final body = line.substring(8);
        final comma = body.indexOf(',');
        final head = comma < 0 ? body : body.substring(0, comma);
        seconds = int.tryParse(head.trim().split(' ').first);
        if (seconds != null && seconds < 0) seconds = null;
        final t = comma < 0 ? '' : body.substring(comma + 1).trim();
        title = t.isEmpty ? null : t;
      }
      continue;
    }
    out.add(M3uEntry(line, title: title, durationSeconds: seconds));
    title = null;
    seconds = null;
  }
  return out;
}

/// A path split into its root (`smb://nas`, `C:`, `` for `/…`, null for a
/// relative path) and its segments.
class _Path {
  _Path(this.root, this.segments);
  final String? root;
  final List<String> segments;

  static final _scheme = RegExp(r'^([a-zA-Z][a-zA-Z0-9+.-]*)://([^/]*)(.*)$');
  static final _drive = RegExp(r'^([a-zA-Z]:)(.*)$');

  static _Path parse(String s) {
    var p = s.replaceAll('\\', '/');
    String? root;
    final m = _scheme.firstMatch(p);
    if (m != null) {
      final scheme = m.group(1)!.toLowerCase();
      if (scheme == 'file') {
        // file:///C:/Music/a.flac → C: + /Music/a.flac
        p = Uri.decodeFull(m.group(3)!);
        final d = _drive.firstMatch(p.startsWith('/') ? p.substring(1) : p);
        if (d != null) {
          root = d.group(1)!.toUpperCase();
          p = d.group(2)!;
        } else {
          root = '';
        }
      } else {
        root = '$scheme://${m.group(2)!.toLowerCase()}';
        p = Uri.decodeFull(m.group(3)!);
      }
    } else if (_drive.hasMatch(p)) {
      final d = _drive.firstMatch(p)!;
      root = d.group(1)!.toUpperCase();
      p = d.group(2)!;
    } else if (p.startsWith('/')) {
      root = '';
    }
    final segs = <String>[];
    for (final part in p.split('/')) {
      if (part.isEmpty || part == '.') continue;
      if (part == '..') {
        if (segs.isNotEmpty) segs.removeLast();
        continue;
      }
      segs.add(part);
    }
    return _Path(root, segs);
  }

  bool get isAbsolute => root != null;

  /// [rel] resolved against this folder.
  _Path join(_Path rel) =>
      rel.isAbsolute ? rel : parse('${toString()}/${rel.segments.join('/')}');

  _Path get parent => _Path(
    root,
    segments.isEmpty ? segments : segments.sublist(0, segments.length - 1),
  );

  String get key => toString().toLowerCase();

  @override
  String toString() {
    final body = segments.join('/');
    return switch (root) {
      null => body,
      '' => '/$body',
      final r when r.endsWith(':') => '$r/$body',
      final r => '$r/$body',
    };
  }
}

/// [path] relative to the folder [dir] when both have the same root, else
/// null.
String? relativePath(String path, String dir) {
  final p = _Path.parse(path);
  final d = _Path.parse(dir);
  if (!p.isAbsolute || p.root != d.root) return null;
  var common = 0;
  while (common < d.segments.length &&
      common < p.segments.length - 1 &&
      d.segments[common] == p.segments[common]) {
    common++;
  }
  return [
    for (var i = common; i < d.segments.length; i++) '..',
    ...p.segments.sublist(common),
  ].join('/');
}

/// Writes [tracks] as an extended M3U8 playlist. With [playlistPath] (where
/// the file goes) paths are written relative to its folder when they can
/// be; otherwise as in the library.
String writeM3u(List<MusicTrack> tracks, {String? playlistPath}) {
  final dir = playlistPath == null
      ? null
      : _Path.parse(playlistPath).parent.toString();
  final b = StringBuffer('#EXTM3U\n');
  for (final t in tracks) {
    final ms = t.item.durationMs;
    final secs = ms == null ? -1 : (ms / 1000).round();
    final who = t.artist;
    final name = who == null || who.isEmpty ? t.title : '$who - ${t.title}';
    b.writeln('#EXTINF:$secs,${name.replaceAll('\n', ' ')}');
    final path = t.item.filePath;
    b.writeln((dir == null ? null : relativePath(path, dir)) ?? path);
  }
  return b.toString();
}

/// What an import found.
class M3uMatch {
  const M3uMatch(this.tracks, this.missing);

  /// Library tracks in playlist order.
  final List<MusicTrack> tracks;

  /// Lines no track was found for.
  final List<String> missing;
}

/// Finds the library tracks of [entries]. Relative lines are resolved
/// against the folder of [playlistPath]; a line matches the track with
/// the same path, else the track whose path shares the longest tail with
/// it (at least the file name).
M3uMatch resolveM3u(
  List<M3uEntry> entries,
  List<MusicTrack> library, {
  String? playlistPath,
}) {
  final byPath = <String, MusicTrack>{};
  final byName = <String, List<(MusicTrack, _Path)>>{};
  for (final t in library) {
    for (final s in {t.item.filePath, t.item.playbackUri}) {
      byPath[_Path.parse(s).key] ??= t;
    }
    final p = _Path.parse(t.item.filePath);
    if (p.segments.isEmpty) continue;
    (byName[p.segments.last.toLowerCase()] ??= []).add((t, p));
  }
  final dir = playlistPath == null ? null : _Path.parse(playlistPath).parent;
  final found = <MusicTrack>[];
  final missing = <String>[];
  for (final e in entries) {
    final raw = _Path.parse(e.location);
    final path = dir != null && !raw.isAbsolute ? dir.join(raw) : raw;
    final exact = byPath[path.key] ?? byPath[raw.key];
    if (exact != null) {
      found.add(exact);
      continue;
    }
    final match = _byTail(path, byName);
    if (match != null) {
      found.add(match);
    } else {
      missing.add(e.location);
    }
  }
  return M3uMatch(found, missing);
}

MusicTrack? _byTail(_Path p, Map<String, List<(MusicTrack, _Path)>> byName) {
  if (p.segments.isEmpty) return null;
  final candidates = byName[p.segments.last.toLowerCase()];
  if (candidates == null) return null;
  MusicTrack? best;
  var bestLen = 0;
  var tie = false;
  for (final (t, tp) in candidates) {
    var n = 0;
    while (n < p.segments.length &&
        n < tp.segments.length &&
        p.segments[p.segments.length - 1 - n].toLowerCase() ==
            tp.segments[tp.segments.length - 1 - n].toLowerCase()) {
      n++;
    }
    if (n > bestLen) {
      best = t;
      bestLen = n;
      tie = false;
    } else if (n == bestLen) {
      tie = true;
    }
  }
  // Only the file name in common and several such files: ambiguous.
  if (tie && bestLen <= 1) return null;
  return best;
}

/// Playlist name from its file name: `Road Trip.m3u8` → `Road Trip`.
String m3uPlaylistName(String fileName) {
  final s = fileName.replaceAll('\\', '/').split('/').last;
  final dot = s.lastIndexOf('.');
  return dot > 0 ? s.substring(0, dot) : s;
}
