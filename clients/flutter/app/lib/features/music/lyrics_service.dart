// SPDX-License-Identifier: MPL-2.0
//
// MUS-006: where the lyrics of a track come from. Synced sources first:
// the file's SYLT, the .lrc next to it, LRC text in the lyrics tag, then
// LRCLIB; plain text only when none of them is synced.

import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:pa_player/pa_player.dart'
    show LyricLine, Lyrics, LyricsSource, PlayRequest, lyricsFromText;

import '../../branding.dart';
import '../../integration/library.dart';

/// LRCLIB asks clients to name themselves with a link to the project;
/// nothing about the user is sent.
const kLyricsUserAgent =
    'Kinolune/$kAppVersion (https://github.com/DenisHumen/kinolune)'; // l10n-ignore

/// The LRCLIB API (lrclib.net: free, no account or key).
final kLrclibApi = Uri.parse('https://lrclib.net/api/'); // l10n-ignore

/// An HTTP GET: the status and the body.
typedef LyricsHttpGet =
    Future<({int status, String body})> Function(Uri url);

/// [LyricsHttpGet] with dart:io and [kLyricsUserAgent].
Future<({int status, String body})> lyricsHttpGet(Uri url) async {
  final client = HttpClient()
    ..userAgent = kLyricsUserAgent
    ..connectionTimeout = const Duration(seconds: 10);
  try {
    final req = await client.getUrl(url);
    req.headers.set(HttpHeaders.acceptHeader, 'application/json');
    final res = await req.close().timeout(const Duration(seconds: 15));
    final body = await res
        .transform(utf8.decoder)
        .join()
        .timeout(const Duration(seconds: 15));
    return (status: res.statusCode, body: body);
  } finally {
    client.close(force: true);
  }
}

/// What LRCLIB has for a track.
@immutable
class LrclibResult {
  const LrclibResult({this.synced, this.plain});

  final String? synced;
  final String? plain;

  bool get isEmpty => synced == null && plain == null;

  Map<String, Object?> toJson() => {'synced': synced, 'plain': plain};

  static LrclibResult fromJson(Map<String, Object?> j) => LrclibResult(
    synced: _text(j['synced'] ?? j['syncedLyrics']),
    plain: _text(j['plain'] ?? j['plainLyrics']),
  );

  static String? _text(Object? v) =>
      v is String && v.trim().isNotEmpty ? v : null;
}

/// LRCLIB lookups: `/api/get` by artist, title, album and duration when
/// they are known, else `/api/search` with the closest duration. Answers
/// (also "nothing") are cached on disk, so a track is asked once.
class LrclibClient {
  LrclibClient({
    LyricsHttpGet? get,
    this.cacheDir,
    DateTime Function()? clock,
  }) : _get = get ?? lyricsHttpGet,
       _clock = clock ?? DateTime.now;

  final LyricsHttpGet _get;

  /// Where answers are kept; no disk cache when null.
  final Future<String?> Function()? cacheDir;
  final DateTime Function() _clock;
  final _memory = <String, LrclibResult>{};

  /// How long "nothing found" is remembered: lyrics get added to LRCLIB.
  static const missTtl = Duration(days: 7);

  /// Durations LRCLIB treats as the same track.
  static const durationSlack = 2;

  /// The cache key of a track.
  static String keyOf({
    required String artist,
    required String title,
    String? album,
    int? seconds,
  }) => [
    artist.trim().toLowerCase(),
    title.trim().toLowerCase(),
    (album ?? '').trim().toLowerCase(),
    seconds ?? '',
  ].join('\u0000');

  /// FNV-1a (32 bit) of [key] in hex: the cache file name.
  static String fileNameOf(String key) {
    var h = 0x811c9dc5;
    for (final b in utf8.encode(key)) {
      h ^= b;
      h = (h * 0x01000193) & 0xffffffff;
    }
    return '${h.toRadixString(16).padLeft(8, '0')}.json';
  }

  /// The lyrics LRCLIB has for the track; an empty result when it has
  /// none. Throws on network errors (nothing is cached then).
  Future<LrclibResult> find({
    required String artist,
    required String title,
    String? album,
    Duration? duration,
  }) async {
    final seconds = duration == null || duration <= Duration.zero
        ? null
        : (duration.inMilliseconds / 1000).round();
    final key = keyOf(
      artist: artist,
      title: title,
      album: album,
      seconds: seconds,
    );
    final memo = _memory[key];
    if (memo != null) return memo;
    final cached = await _readCache(key);
    if (cached != null) return _memory[key] = cached;

    LrclibResult? found;
    if (album != null && album.trim().isNotEmpty && seconds != null) {
      found = await _exact(artist, title, album, seconds);
    }
    found ??= await _search(artist, title, seconds);
    final result = found ?? const LrclibResult();
    _memory[key] = result;
    await _writeCache(key, result);
    return result;
  }

  Future<LrclibResult?> _exact(
    String artist,
    String title,
    String album,
    int seconds,
  ) async {
    final res = await _get(
      kLrclibApi.resolve('get').replace(
        queryParameters: {
          'artist_name': artist,
          'track_name': title,
          'album_name': album,
          'duration': '$seconds',
        },
      ),
    );
    if (res.status == 404) return null;
    if (res.status != 200) throw HttpException('LRCLIB: ${res.status}');
    final j = jsonDecode(res.body);
    if (j is! Map<String, Object?>) return null;
    final r = LrclibResult.fromJson(j);
    return r.isEmpty ? null : r;
  }

  Future<LrclibResult?> _search(
    String artist,
    String title,
    int? seconds,
  ) async {
    final res = await _get(
      kLrclibApi.resolve('search').replace(
        queryParameters: {'artist_name': artist, 'track_name': title},
      ),
    );
    if (res.status == 404) return null;
    if (res.status != 200) throw HttpException('LRCLIB: ${res.status}');
    final list = jsonDecode(res.body);
    if (list is! List) return null;
    LrclibResult? plainOnly;
    for (final e in list) {
      if (e is! Map<String, Object?>) continue;
      final d = e['duration'];
      if (seconds != null &&
          d is num &&
          (d - seconds).abs() > durationSlack) {
        continue;
      }
      final r = LrclibResult.fromJson(e);
      if (r.synced != null) return r;
      if (r.plain != null) plainOnly ??= r;
    }
    return plainOnly;
  }

  Future<File?> _file(String key) async {
    final dir = await cacheDir?.call();
    if (dir == null) return null;
    return File('$dir${Platform.pathSeparator}${fileNameOf(key)}');
  }

  Future<LrclibResult?> _readCache(String key) async {
    try {
      final f = await _file(key);
      if (f == null || !await f.exists()) return null;
      final j = jsonDecode(await f.readAsString());
      if (j is! Map<String, Object?> || j['key'] != key) return null;
      final r = LrclibResult.fromJson(j);
      final at = j['at'];
      if (r.isEmpty &&
          (at is! int ||
              _clock().difference(DateTime.fromMillisecondsSinceEpoch(at)) >
                  missTtl)) {
        return null;
      }
      return r;
    } catch (e) {
      debugPrint('Lyrics cache not read: $e');
      return null;
    }
  }

  Future<void> _writeCache(String key, LrclibResult r) async {
    try {
      final f = await _file(key);
      if (f == null) return;
      await f.parent.create(recursive: true);
      await f.writeAsString(
        jsonEncode({
          'key': key,
          'at': _clock().millisecondsSinceEpoch,
          ...r.toJson(),
        }),
      );
    } catch (e) {
      debugPrint('Lyrics cache not written: $e');
    }
  }

  /// Forgets everything (Settings → Storage could call it).
  Future<void> clear() async {
    _memory.clear();
    final dir = await cacheDir?.call();
    if (dir == null) return;
    final d = Directory(dir);
    if (await d.exists()) await d.delete(recursive: true);
  }
}

/// The app's [LyricsLoader] for Now Playing.
class LyricsService {
  LyricsService({
    required this.files,
    required this.online,
    LrclibClient? lrclib,
  }) : lrclib = lrclib ?? LrclibClient();

  /// The library's reader of tags and `.lrc` files; null without the core
  /// (then only a `.lrc` next to a local file is found).
  final LibraryLyrics? files;

  /// Whether LRCLIB may be asked (Settings → Music, privacy).
  final ValueListenable<bool> online;
  final LrclibClient lrclib;

  /// The best lyrics of [track]: synced from the file, its `.lrc`, its
  /// lyrics tag or LRCLIB, in that order; else plain text; null when no
  /// source has any.
  Future<Lyrics?> load(PlayRequest track) async {
    final files = await _files(track);
    if (files != null && files.synced.isNotEmpty) {
      return Lyrics.synced([
        for (final (t, text) in files.synced) LyricLine(t, text),
      ], source: LyricsSource.embeddedSynced);
    }
    final lrc = lyricsFromText(files?.lrc, LyricsSource.lrcFile);
    if (lrc != null && lrc.synced) return lrc;
    final tag = lyricsFromText(files?.plain, LyricsSource.embedded);
    if (tag != null && tag.synced) return tag;
    final net = await _online(track);
    if (net != null && net.synced) return net;
    return lrc ?? tag ?? net;
  }

  /// Tags and `.lrc` through the core; for a file outside the library,
  /// the `.lrc` next to it on this device.
  Future<TrackLyricsFiles?> _files(PlayRequest track) async {
    try {
      final found = await files?.trackLyrics(track.uri);
      if (found != null) return found;
      return await _localLrc(track.uri);
    } catch (e) {
      debugPrint('Lyrics of ${track.title} not read from the file: $e');
      return null;
    }
  }

  static Future<TrackLyricsFiles?> _localLrc(String uri) async {
    String path = uri;
    if (path.startsWith('file://')) path = Uri.parse(path).toFilePath();
    final track = File(path);
    if (!track.isAbsolute) return null;
    final dir = track.parent;
    if (!await dir.exists()) return null;
    final name = _baseName(path);
    final dot = name.lastIndexOf('.');
    final want = '${dot > 0 ? name.substring(0, dot) : name}.lrc'.toLowerCase();
    await for (final e in dir.list(followLinks: false)) {
      if (e is File && _baseName(e.path).toLowerCase() == want) {
        if (await e.length() > 1024 * 1024) return null;
        final bytes = await e.readAsBytes();
        return TrackLyricsFiles(lrc: utf8.decode(bytes, allowMalformed: true));
      }
    }
    return null;
  }

  static String _baseName(String path) => path.split(RegExp(r'[/\\]')).last;

  Future<Lyrics?> _online(PlayRequest track) async {
    final artist = track.artist?.trim();
    if (!online.value || artist == null || artist.isEmpty) return null;
    try {
      final r = await lrclib.find(
        artist: artist,
        title: track.title,
        album: track.album,
        duration: track.durationMs == null
            ? null
            : Duration(milliseconds: track.durationMs!),
      );
      return lyricsFromText(r.synced, LyricsSource.lrclib) ??
          lyricsFromText(r.plain, LyricsSource.lrclib);
    } catch (e) {
      debugPrint('LRCLIB lookup of ${track.title} failed: $e');
      return null;
    }
  }
}
