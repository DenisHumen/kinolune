// SPDX-License-Identifier: MPL-2.0
//
// PLAY-010: the stats model, its JSON shared with the core, the mapping
// from libmpv properties, the adaptive buffer, and the overlay with its
// one-press report.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/src/engine/mpv_stats.dart';
import 'package:pa_player/testing.dart';

/// The fixture of `pa_playback` (core/crates/pa-playback/tests): the Rust
/// side reads the same file.
const _fixture =
    '../../../../core/crates/pa-playback/tests/fixtures/playback_stats.json';

Map<String, Object?> _fixtureJson() =>
    jsonDecode(File(_fixture).readAsStringSync()) as Map<String, Object?>;

void main() {
  group('model', () {
    test('reads and writes the JSON of the core unchanged', () {
      final json = _fixtureJson();
      final s = PlaybackStats.fromJson(json);
      expect(s.check(), isNull);
      expect(s.videoCodec, 'hevc');
      expect(s.dynamicRange, DynamicRange.dolbyVision8);
      expect(s.outputRange, DynamicRange.sdr);
      expect(s.audioPath, AudioPath.decoded);
      expect(s.afr, AfrStatus.off);
      expect(s.source, PlaybackSource.network);
      expect(s.videoFps, 23.976);
      // Same key set and values as the Rust model writes.
      expect(s.toJson(), json);
      expect(PlaybackStats.fromJson(s.toJson()), s);
    });

    test('unknown fields stay null', () {
      final s = PlaybackStats.fromJson(const {
        'video_codec': 'h264',
        'dropped_frames': 1,
        'something_new': 3,
      });
      expect(s.videoCodec, 'h264');
      expect(s.droppedFrames, 1);
      expect(s.source, isNull);
      expect(s.check(), isNull);
    });

    test('rules match the core', () {
      expect(
        const PlaybackStats(hwDecoder: 'vaapi').check(),
        contains('hw_decoder'),
      );
      expect(
        const PlaybackStats(
          audioPath: AudioPath.decoded,
          audioPassthrough: true,
        ).check(),
        contains('audio_path'),
      );
      expect(
        const PlaybackStats(
          dynamicRange: DynamicRange.hdr10,
          dvProfile: 8,
        ).check(),
        contains('dv_profile'),
      );
      expect(
        const PlaybackStats(videoWidth: 1920).check(),
        contains('video_width'),
      );
      expect(
        const PlaybackStats(outputFps: double.nan).check(),
        contains('frame rates'),
      );
      expect(
        const PlaybackStats(
          source: PlaybackSource.local,
          networkBps: 1,
        ).check(),
        contains('local'),
      );
      expect(
        const PlaybackStats(
          droppedFrames: 3,
        ).checkProgress(const PlaybackStats()),
        contains('dropped_frames'),
      );
    });

    test('transcode hint only for a slow media server', () {
      const slow = PlaybackStats(
        source: PlaybackSource.server,
        networkBps: 10000000,
        bitrateBps: 20000000,
      );
      expect(slow.headroom, 0.5);
      expect(slow.transcodeAdvised, isTrue);
      const fast = PlaybackStats(
        source: PlaybackSource.server,
        networkBps: 40000000,
        bitrateBps: 20000000,
      );
      expect(fast.transcodeAdvised, isFalse);
      expect(
        const PlaybackStats(source: PlaybackSource.server, stalls: 3)
            .transcodeAdvised,
        isTrue,
      );
      // A share cannot transcode: it gets a bigger buffer instead.
      expect(
        const PlaybackStats(
          source: PlaybackSource.network,
          networkBps: 1,
          bitrateBps: 20000000,
        ).transcodeAdvised,
        isFalse,
      );
    });

    test('report: readable rows and the model JSON', () {
      final s = PlaybackStats.fromJson(_fixtureJson());
      final files = playbackReportFiles(
        s,
        title: 'Film',
        extra: const {'A/V sync': '3 ms'},
      );
      expect(files.keys, ['playback.txt', 'playback.json']);
      final txt = files['playback.txt']!;
      expect(txt, contains('Title: Film'));
      expect(txt, contains('Video: hevc Main 10 · 3840×2160 · 23.976 fps'));
      expect(txt, contains('HDR: Dolby Vision 8 · MaxCLL 1000 · MaxFALL 400'));
      expect(txt, contains('Decoder: Hardware (vaapi)'));
      expect(txt, contains('Output: 1920×1080 · 60 Hz · SDR'));
      expect(txt, contains('Audio: eac3 · 6 → 2 ch · 48 kHz'));
      expect(txt, contains('Network speed: 42.0 Mbps (×2.3)'));
      expect(txt, contains('Buffer: 24.5 / 30.0 s'));
      expect(txt, contains('A/V sync: 3 ms'));
      expect(jsonDecode(files['playback.json']!), _fixtureJson());
    });
  });

  group('contract', () {
    test('the fake engine fills the model', () async {
      final e = FakePlayerEngine();
      await e.open(['/media/a.mkv']);
      expect(await statsContractProblems(e, () async {}), isEmpty);
    });

    test('an engine that breaks the model fails the contract', () async {
      final e = FakePlayerEngine()
        ..fakePlaybackStats = const PlaybackStats(
          videoCodec: 'h264',
          audioCodec: 'aac',
          hwDecoder: 'vaapi',
        );
      await e.open(['/media/a.mkv']);
      final problems = await statsContractProblems(e, () async {});
      expect(problems.single, contains('hw_decoder'));
    });
  });

  group('libmpv mapping', () {
    Future<PlaybackStats> read(
      Map<String, String> props, {
      AdaptiveBuffer? buffer,
    }) => readMpvStats(
      (n) async => props[n],
      buffer: buffer,
      duration: const Duration(seconds: 100),
    );

    test('an HDR10 file decoded in hardware over a share', () async {
      final buffer = AdaptiveBuffer(PlaybackSource.network)
        ..sample(bitrateBps: 20000000, speedBps: 50000000, bufferedSecs: 2);
      final s = await read({
        'mpv-version': 'mpv v0.40.0',
        'file-format': 'matroska,webm',
        'video-format': 'hevc',
        'current-tracks/video/codec-profile': 'Main 10',
        'video-params/w': '3840',
        'video-params/h': '2160',
        'container-fps': '23.976024',
        'video-params/pixelformat': 'vaapi',
        'video-params/hw-pixelformat': 'p010',
        'video-params/gamma': 'pq',
        'video-params/max-cll': '1000',
        'video-params/max-fall': '400',
        'hwdec-current': 'vaapi',
        'frame-drop-count': '2',
        'decoder-frame-drop-count': '0',
        'display-width': '1920',
        'display-height': '1080',
        'display-fps': '60.000000',
        'video-target-params/gamma': 'bt.1886',
        'audio-codec-name': 'eac3',
        'audio-params/channel-count': '6',
        'audio-out-params/channel-count': '2',
        'audio-out-params/format': 'floatp',
        'audio-params/samplerate': '48000',
        'video-bitrate': '18000000',
        'audio-bitrate': '640000',
        'demuxer-cache-duration': '12.5',
      }, buffer: buffer);
      expect(s.check(), isNull);
      expect(s.engine, 'libmpv 0.40.0');
      expect(s.videoBitDepth, 10);
      expect(s.dynamicRange, DynamicRange.hdr10);
      expect((s.maxCll, s.maxFall), (1000, 400));
      expect((s.hwDecoding, s.hwDecoder), (true, 'vaapi'));
      expect((s.outputWidth, s.outputHeight), (1920, 1080));
      expect(s.outputRange, DynamicRange.sdr);
      expect(s.audioPath, AudioPath.decoded);
      expect(s.audioPassthrough, isFalse);
      expect(s.bitrateBps, 18640000);
      expect(s.bufferedMs, 12500);
      expect(s.source, PlaybackSource.network);
      expect(s.networkBps, 50000000);
      expect(s.bufferTargetMs, 30000);
      expect(s.stalls, 0);
      expect(s.afr, AfrStatus.off);
    });

    test('Dolby Vision, passthrough, LPCM, SDR, no bitrate yet', () async {
      final dv = await read({
        'video-format': 'hevc',
        'video-params/gamma': 'pq',
        'current-tracks/video/dolby-vision-profile': '8',
        'audio-codec-name': 'truehd',
        'audio-out-params/format': 'spdif-truehd',
        'hwdec-current': 'no',
      });
      expect(dv.dynamicRange, DynamicRange.dolbyVision8);
      expect(dv.dvProfile, 8);
      expect(dv.audioPath, AudioPath.passthrough);
      expect(dv.audioPassthrough, isTrue);
      expect(dv.hwDecoding, isFalse);
      expect(dv.check(), isNull);
      final pcm = await read({
        'audio-codec-name': 'pcm_s24le',
        'video-params/gamma': 'bt.1886',
        'video-params/max-cll': '1000',
        'file-size': '12500000',
      });
      expect(pcm.audioPath, AudioPath.lpcm);
      expect(pcm.dynamicRange, DynamicRange.sdr);
      expect(pcm.maxCll, isNull, reason: 'no HDR metadata on SDR');
      expect(pcm.bitrateBps, 1000000, reason: 'file size over duration');
      expect(pcm.source, isNull);
      expect(pcm.check(), isNull);
    });

    test('bit depth of pixel formats', () {
      expect(bitDepthOf('yuv420p'), 8);
      expect(bitDepthOf('nv12'), 8);
      expect(bitDepthOf('yuv420p10'), 10);
      expect(bitDepthOf('yuv420p10le'), 10);
      expect(bitDepthOf('p010'), 10);
      expect(bitDepthOf('yuv444p12'), 12);
      expect(bitDepthOf(null), isNull);
    });
  });

  group('adaptive buffer', () {
    test('kind of source from the URI', () {
      expect(sourceOfUri('/media/a.mkv'), PlaybackSource.local);
      expect(sourceOfUri('file:///media/a.mkv'), PlaybackSource.local);
      expect(sourceOfUri(r'C:\Videos\a.mkv'), PlaybackSource.local);
      expect(
        sourceOfUri('http://127.0.0.1:41234/s/abc/a.mkv'),
        PlaybackSource.network,
      );
      expect(
        sourceOfUri('http://127.0.0.1:41234/tok/live.m3u8'),
        PlaybackSource.live,
      );
      expect(
        sourceOfUri('https://jellyfin.example/Videos/1/stream.mkv'),
        PlaybackSource.server,
      );
      expect(
        sourceOfUri(
          partsUri([
            'http://127.0.0.1:1/a.cd1.avi',
            'http://127.0.0.1:1/a.cd2.avi',
          ]),
        ),
        PlaybackSource.network,
      );
      expect(
        sourceOfUri(partsUri(['/m/a.cd1.avi', '/m/a.cd2.avi'])),
        PlaybackSource.local,
      );
    });

    test('local files and live streams keep libmpv defaults', () {
      expect(AdaptiveBuffer(PlaybackSource.local).plan, isNull);
      expect(AdaptiveBuffer(PlaybackSource.live).plan, isNull);
    });

    test('a fast share stays at 30 s; memory follows the bitrate', () {
      final b = AdaptiveBuffer(PlaybackSource.network)
        ..sample(bitrateBps: 40000000, speedBps: 200000000, bufferedSecs: 3);
      final plan = b.plan!;
      expect(plan.level, BufferLevel.normal);
      expect(plan.readAheadSecs, 30);
      // 40 Mbps × 30 s × 1.25 = 187.5 MB.
      expect(plan.maxBytes, 187500000);
      expect(plan.mpvProperties, {
        'cache': 'yes',
        'cache-secs': '30',
        'demuxer-readahead-secs': '30',
        'demuxer-max-bytes': '187500000',
        'cache-pause-wait': '1',
      });
      // A full cache reads slowly: that is not a slow network.
      b.sample(speedBps: 1000, bufferedSecs: 29);
      expect(b.level, BufferLevel.normal);
    });

    test('a barely fast link is cautious, a slow one lossy', () {
      final cautious = AdaptiveBuffer(PlaybackSource.server)
        ..sample(bitrateBps: 20000000, speedBps: 26000000, bufferedSecs: 5);
      expect(cautious.level, BufferLevel.cautious);
      expect(cautious.plan!.readAheadSecs, 60);
      final lossy = AdaptiveBuffer(PlaybackSource.network)
        ..sample(bitrateBps: 20000000, speedBps: 15000000, bufferedSecs: 5);
      expect(lossy.level, BufferLevel.lossy);
      expect(lossy.plan!.resumeWaitSecs, 6);
      // Phones and TV boxes: the memory cap.
      final phone = AdaptiveBuffer(
        PlaybackSource.network,
        maxBytesCap: AdaptiveBuffer.mobileCap,
      )..sample(bitrateBps: 80000000, speedBps: 10000000, bufferedSecs: 1);
      expect(phone.plan!.maxBytes, AdaptiveBuffer.mobileCap);
    });

    test('stalls step up and the level never goes down', () {
      final b = AdaptiveBuffer(PlaybackSource.network);
      expect(b.plan!.maxBytes, AdaptiveBuffer.minBytes);
      b.stall();
      expect((b.level, b.stalls), (BufferLevel.cautious, 1));
      b
        ..stall()
        ..stall();
      expect((b.level, b.stalls), (BufferLevel.lossy, 3));
      b.sample(bitrateBps: 1000000, speedBps: 900000000, bufferedSecs: 1);
      expect(b.level, BufferLevel.lossy);
    });
  });

  group('overlay', () {
    Future<FakePlayerEngine> open(WidgetTester t) async {
      final engine = FakePlayerEngine();
      await t.pumpWidget(
        MaterialApp(
          localizationsDelegates: const [PaPlayerStrings.delegate],
          home: PlayerScreen(
            request: const PlayRequest(uri: '/media/film.mkv', title: 'Film'),
            onProgress: (_, _) {},
            engineFactory: ({bool video = true}) => engine,
          ),
        ),
      );
      await t.pump();
      return engine;
    }

    Future<void> close(WidgetTester t) async {
      await t.pumpWidget(const SizedBox());
      await t.pump(const Duration(seconds: 1));
    }

    tearDown(() => PlayerScreen.statsExporter = null);

    testWidgets('S shows the model rows', (t) async {
      final engine = await open(t);
      engine.fakePlaybackStats = PlaybackStats.fromJson(_fixtureJson());
      await t.sendKeyEvent(LogicalKeyboardKey.keyS);
      await t.pump();
      expect(find.byKey(const ValueKey('osd-stats')), findsOneWidget);
      expect(find.textContaining('Hardware (vaapi)'), findsOneWidget);
      expect(find.textContaining('42.0 Mbps (×2.3)'), findsOneWidget);
      expect(find.textContaining('24.5 / 30.0 s'), findsOneWidget);
      expect(find.textContaining('A/V sync'), findsOneWidget);
      expect(find.byKey(const ValueKey('osd-stats-transcode')), findsNothing);
      await close(t);
    });

    testWidgets('a slow media server shows the transcode hint', (t) async {
      final engine = await open(t);
      engine.fakePlaybackStats = const PlaybackStats(
        videoCodec: 'h264',
        audioCodec: 'aac',
        source: PlaybackSource.server,
        networkBps: 5000000,
        bitrateBps: 20000000,
      );
      await t.sendKeyEvent(LogicalKeyboardKey.keyS);
      await t.pump();
      expect(find.byKey(const ValueKey('osd-stats-transcode')), findsOneWidget);
      await close(t);
    });

    testWidgets('no report row without diagnostics in the app', (t) async {
      await open(t);
      await t.tap(find.byKey(const ValueKey('osd-info')));
      await t.pumpAndSettle();
      expect(find.byKey(const ValueKey('info-stats')), findsOneWidget);
      expect(find.byKey(const ValueKey('info-stats-export')), findsNothing);
      await close(t);
    });

    testWidgets('«Сохранить отчёт» hands the report in one press', (t) async {
      Map<String, String>? saved;
      PlayerScreen.statsExporter = (context, files) async => saved = files;
      final engine = await open(t);
      await t.tap(find.byKey(const ValueKey('osd-info')));
      await t.pumpAndSettle();
      await t.tap(find.byKey(const ValueKey('info-stats-export')));
      await t.pumpAndSettle();
      expect(saved, isNotNull);
      expect(saved!['playback.txt'], contains('Title: Film'));
      expect(saved!['playback.txt'], contains('Video: h264 · 1920×1080'));
      expect(
        PlaybackStats.fromJson(
          jsonDecode(saved!['playback.json']!) as Map<String, Object?>,
        ),
        engine.fakePlaybackStats,
      );
      // A fresh reading, then the overlay's measuring stops again.
      expect(engine.calls, containsAllInOrder(['stats:true', 'stats:false']));
      await close(t);
    });
  });
}
