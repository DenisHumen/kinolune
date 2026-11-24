// SPDX-License-Identifier: MPL-2.0
//
// MUS-006: which lyrics Now Playing gets — synced sources first (SYLT,
// .lrc, LRC in the lyrics tag, LRCLIB), plain text last — LRCLIB lookups
// with the disk cache, and the privacy switch that keeps lookups on the
// device.

import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/features/music/lyrics_service.dart';
import 'package:kinolune/integration/library.dart';
import 'package:pa_player/pa_player.dart';

class _Files implements LibraryLyrics {
  _Files(this.byUri);
  final Map<String, TrackLyricsFiles> byUri;
  final asked = <String>[];
  bool fail = false;

  @override
  Future<TrackLyricsFiles?> trackLyrics(String playbackUri) async {
    asked.add(playbackUri);
    if (fail) throw const SocketException('share offline');
    return byUri[playbackUri] ?? const TrackLyricsFiles();
  }
}

/// LRCLIB answers by path; every request is recorded.
class _Lrclib {
  final requests = <Uri>[];
  final answers = <String, ({int status, String body})>{};
  bool down = false;

  Future<({int status, String body})> get(Uri url) async {
    requests.add(url);
    if (down) throw const SocketException('no network');
    return answers[url.path] ?? (status: 404, body: '{}');
  }
}

const _track = PlayRequest(
  uri: 'http://127.0.0.1:1/s/1/Album/01.flac',
  title: 'Song',
  artist: 'Artist',
  album: 'Album',
  durationMs: 201400,
);

const _lrc = '[00:01.00]From the lrc\n[00:02.00]Second';

void main() {
  late _Lrclib net;
  late ValueNotifier<bool> online;

  setUp(() {
    net = _Lrclib();
    online = ValueNotifier(true);
  });

  LyricsService service(_Files? files, {String? cacheDir}) => LyricsService(
    files: files,
    online: online,
    lrclib: LrclibClient(
      get: net.get,
      cacheDir: cacheDir == null ? null : () async => cacheDir,
    ),
  );

  group('priority', () {
    test('SYLT beats everything', () async {
      final files = _Files({
        _track.uri: const TrackLyricsFiles(
          synced: [(Duration(milliseconds: 500), 'Embedded')],
          plain: 'Plain tag',
          lrc: _lrc,
        ),
      });
      final l = (await service(files).load(_track))!;
      expect(l.source, LyricsSource.embeddedSynced);
      expect(l.lines.single.text, 'Embedded');
      expect(net.requests, isEmpty, reason: 'nothing asked online');
    });

    test('.lrc next to the track before the lyrics tag', () async {
      final files = _Files({
        _track.uri: const TrackLyricsFiles(plain: '[00:05.00]tag', lrc: _lrc),
      });
      final l = (await service(files).load(_track))!;
      expect(l.source, LyricsSource.lrcFile);
      expect(l.lines.first.text, 'From the lrc');
    });

    test('LRC text in the lyrics tag counts as synced', () async {
      final files = _Files({
        _track.uri: const TrackLyricsFiles(plain: '[00:05.00]tag line'),
      });
      final l = (await service(files).load(_track))!;
      expect(l.source, LyricsSource.embedded);
      expect(l.synced, isTrue);
      expect(net.requests, isEmpty);
    });

    test('plain local lyrics lose to synced LRCLIB', () async {
      net.answers['/api/get'] = (
        status: 200,
        body: jsonEncode({
          'syncedLyrics': '[00:03.00]Online',
          'plainLyrics': 'Online',
        }),
      );
      final files = _Files({
        _track.uri: const TrackLyricsFiles(plain: 'Just the words'),
      });
      final l = (await service(files).load(_track))!;
      expect(l.source, LyricsSource.lrclib);
      expect(l.lines.single, LyricLine(const Duration(seconds: 3), 'Online'));
      final q = net.requests.single.queryParameters;
      expect(q, {
        'artist_name': 'Artist',
        'track_name': 'Song',
        'album_name': 'Album',
        'duration': '201',
      });
      expect(net.requests.single.host, 'lrclib.net');
    });

    test('plain local lyrics when LRCLIB has none', () async {
      final files = _Files({
        _track.uri: const TrackLyricsFiles(plain: 'Just the words'),
      });
      final l = (await service(files).load(_track))!;
      expect(l.synced, isFalse);
      expect(l.plainText, 'Just the words');
      expect(l.source, LyricsSource.embedded);
    });

    test('nothing anywhere: null («Текст не найден»)', () async {
      expect(await service(_Files({})).load(_track), isNull);
    });

    test('an unreachable source still lets LRCLIB answer', () async {
      net.answers['/api/get'] = (
        status: 200,
        body: jsonEncode({'plainLyrics': 'Online words'}),
      );
      final files = _Files({})..fail = true;
      final l = (await service(files).load(_track))!;
      expect(l.plainText, 'Online words');
      expect(l.source, LyricsSource.lrclib);
    });
  });

  group('privacy', () {
    test('online lookup off: LRCLIB is never asked', () async {
      online.value = false;
      expect(await service(_Files({})).load(_track), isNull);
      expect(net.requests, isEmpty);
    });

    test('no artist: no online guess by title alone', () async {
      const noArtist = PlayRequest(uri: '/x.mp3', title: 'Intro');
      expect(await service(_Files({})).load(noArtist), isNull);
      expect(net.requests, isEmpty);
    });

    test('the User-Agent names the project, nothing about the user', () {
      expect(kLyricsUserAgent, startsWith('Kinolune/'));
      expect(kLyricsUserAgent, contains('github.com/DenisHumen/kinolune'));
    });
  });

  group('LRCLIB', () {
    test('search when album or length is unknown; closest length', () async {
      net.answers['/api/search'] = (
        status: 200,
        body: jsonEncode([
          {'duration': 90, 'syncedLyrics': '[00:01.00]Radio edit'},
          {'duration': 200.6, 'plainLyrics': 'Plain only'},
          {'duration': 202, 'syncedLyrics': '[00:01.00]Album version'},
        ]),
      );
      const t = PlayRequest(
        uri: '/x.mp3',
        title: 'Song',
        artist: 'Artist',
        durationMs: 201000,
      );
      final l = (await service(null).load(t))!;
      expect(l.lines.single.text, 'Album version');
      expect(net.requests.single.path, '/api/search');
    });

    test('answers are cached on disk, misses too', () async {
      final dir = await Directory.systemTemp.createTemp('lyrics');
      addTearDown(() => dir.delete(recursive: true));
      net.answers['/api/get'] = (
        status: 200,
        body: jsonEncode({'syncedLyrics': '[00:01.00]Cached'}),
      );
      expect(
        (await service(null, cacheDir: dir.path).load(_track))!
            .lines
            .single
            .text,
        'Cached',
      );
      expect(net.requests, hasLength(1));
      // A new service (next start) reads the disk, not the network.
      net.down = true;
      expect(
        (await service(null, cacheDir: dir.path).load(_track))!
            .lines
            .single
            .text,
        'Cached',
      );
      expect(net.requests, hasLength(1));

      // A miss is remembered as well.
      net.down = false;
      const other = PlayRequest(uri: '/o.mp3', title: 'Other', artist: 'A');
      expect(await service(null, cacheDir: dir.path).load(other), isNull);
      final n = net.requests.length;
      expect(await service(null, cacheDir: dir.path).load(other), isNull);
      expect(net.requests, hasLength(n));
    });

    test('network errors are not cached', () async {
      net.down = true;
      final s = service(null);
      expect(await s.load(_track), isNull);
      net.down = false;
      net.answers['/api/get'] = (
        status: 200,
        body: jsonEncode({'syncedLyrics': '[00:01.00]Back'}),
      );
      expect((await s.load(_track))!.lines.single.text, 'Back');
    });

    test('cache file names are stable', () {
      final key = LrclibClient.keyOf(artist: 'A', title: 'B', seconds: 3);
      expect(LrclibClient.fileNameOf(key), LrclibClient.fileNameOf(key));
      expect(
        LrclibClient.fileNameOf(key),
        matches(RegExp(r'^[0-9a-f]{8}\.json$')),
      );
    });
  });

  test('a loose local file: the .lrc next to it, any case', () async {
    final dir = await Directory.systemTemp.createTemp('loose');
    addTearDown(() => dir.delete(recursive: true));
    final song = File('${dir.path}/Song.mp3')..writeAsBytesSync([0]);
    File('${dir.path}/song.LRC').writeAsStringSync('[00:00.50]Loose');
    online.value = false;
    final l = (await service(null)
        .load(PlayRequest(uri: song.uri.toString(), title: 'Song')))!;
    expect(l.source, LyricsSource.lrcFile);
    expect(l.lines.single.text, 'Loose');
  });
}
