// SPDX-License-Identifier: MPL-2.0
//
// PLAY-010 in the app: the player's stats overlay (the one model every
// engine fills) and «Сохранить отчёт о воспроизведении» in the info panel,
// which writes the CORE-008 diagnostic report with the stats in `extra/`
// in one press (ROADMAP «done when»: the report is exported from
// CORE-008). With KINOLUNE_SCREENSHOTS_DIR set the frames are written:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/play-010 \
//     flutter test test/playback_stats_test.dart

import 'dart:convert';

import 'package:flutter/services.dart' show LogicalKeyboardKey;
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/diagnostics/diagnostics.dart';
import 'package:kinolune/diagnostics/diagnostics_section.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/playback.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

const _movie = 'show-moonlit-bureau-s2e4';

/// A 4K Dolby Vision file decoded in hardware from a share over Wi-Fi.
const _stats = PlaybackStats(
  engine: 'libmpv 0.40.0',
  container: 'matroska',
  videoCodec: 'hevc',
  videoProfile: 'Main 10',
  videoWidth: 3840,
  videoHeight: 2160,
  videoFps: 23.976,
  videoBitDepth: 10,
  dynamicRange: DynamicRange.dolbyVision8,
  maxCll: 1000,
  maxFall: 400,
  dvProfile: 8,
  hwDecoding: true,
  hwDecoder: 'vaapi',
  outputWidth: 1920,
  outputHeight: 1080,
  outputFps: 60,
  outputRange: DynamicRange.sdr,
  afr: AfrStatus.off,
  droppedFrames: 2,
  decoderDroppedFrames: 0,
  audioCodec: 'eac3',
  audioChannels: 6,
  audioOutputChannels: 2,
  audioSampleRate: 48000,
  audioPath: AudioPath.decoded,
  audioPassthrough: false,
  bitrateBps: 18500000,
  source: PlaybackSource.network,
  networkBps: 26000000,
  bufferedMs: 41500,
  bufferTargetMs: 60000,
  stalls: 1,
);

class _Backend implements DiagnosticsBackend {
  Map<String, String>? attachments;

  @override
  DiagnosticsState status() => const DiagnosticsState();

  @override
  DiagnosticsState setEnabled(bool on) => const DiagnosticsState();

  @override
  DiagnosticsState clear() => const DiagnosticsState();

  @override
  void log(String level, String target, String message) {}

  @override
  bool recordCrash(String kind, String message, String stack) => false;

  @override
  Future<String> export(
    String outPath,
    Map<String, String> versions,
    Map<String, String> config, {
    Map<String, String> attachments = const {},
  }) async {
    this.attachments = attachments;
    return outPath;
  }

  @override
  String issueUrl(String repoUrl, Map<String, String> versions, String name) =>
      '$repoUrl/issues/new?logs=$name';
}

class _SceneEngine extends FakePlayerEngine {
  _SceneEngine() : super(loadedDuration: const Duration(minutes: 52)) {
    fakePlaybackStats = _stats;
  }

  @override
  Widget buildVideo(BuildContext context, {BoxFit fit = BoxFit.contain}) =>
      const DecoratedBox(
        decoration: BoxDecoration(
          gradient: LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: [Color(0xFF1E2A44), Color(0xFF3B2F4A), Color(0xFF6A4B3A)],
          ),
        ),
        child: SizedBox.expand(),
      );
}

Future<(AppServices, _SceneEngine)> _services() async {
  final engine = _SceneEngine();
  final services = await demoServices(
    engineFactory: ({bool video = true}) => engine,
  );
  return (services, engine);
}

Future<void> _play(WidgetTester t, AppServices services) async {
  final item = (await services.library.item(_movie))!;
  final context = t.element(find.byType(Scaffold).first);
  playVideo(context, item);
  await settle(t, 30);
  expect(find.byType(PlayerScreen), findsOneWidget);
}

void main() {
  late _Backend backend;
  setUp(() {
    backend = _Backend();
    Diagnostics.instance = Diagnostics(backend);
    Diagnostics.reportDir = () async => '~/Downloads';
    PlayerScreen.statsExporter = exportPlaybackReport;
  });
  tearDown(() {
    Diagnostics.instance = Diagnostics.unavailable();
    PlayerScreen.statsExporter = null;
  });

  // ROADMAP PLAY-010 «done when»: the report is exported from CORE-008.
  testWidgets('the stats go into the diagnostic report in one press', (
    t,
  ) async {
    setSurface(t, const Size(1280, 800));
    final (services, engine) = await _services();
    await t.pumpWidget(testApp(services));
    await settle(t);
    await _play(t, services);
    await t.tap(find.byKey(const ValueKey('osd-info')));
    await settle(t, 6);
    await t.tap(find.byKey(const ValueKey('info-stats-export')));
    await settle(t, 8);
    expect(find.byKey(const ValueKey('diagnostics-saved')), findsOneWidget);
    final files = backend.attachments!;
    expect(files.keys, containsAll(['playback.txt', 'playback.json']));
    expect(files['playback.txt'], contains('Decoder: Hardware (vaapi)'));
    expect(files['playback.txt'], contains('Stalls: 1'));
    expect(
      PlaybackStats.fromJson(
        jsonDecode(files['playback.json']!) as Map<String, Object?>,
      ),
      _stats,
    );
    expect(engine.statsActive, isFalse, reason: 'measuring stopped again');
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });

  testWidgets('no report row when the app has no diagnostics', (t) async {
    Diagnostics.instance = Diagnostics.unavailable();
    PlayerScreen.statsExporter = null;
    setSurface(t, const Size(1280, 800));
    final (services, _) = await _services();
    await t.pumpWidget(testApp(services));
    await settle(t);
    await _play(t, services);
    await t.tap(find.byKey(const ValueKey('osd-info')));
    await settle(t, 6);
    expect(find.byKey(const ValueKey('info-stats')), findsOneWidget);
    expect(find.byKey(const ValueKey('info-stats-export')), findsNothing);
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });

  group('screenshots', () {
    setUpAll(loadAppFonts);

    for (final (name, ff, size, dpr) in [
      ('tv', FormFactor.tv, const Size(960, 540), 2.0),
      ('phone', FormFactor.phone, const Size(844, 390), 2.0),
      ('desktop', FormFactor.desktop, const Size(1280, 800), 1.0),
    ]) {
      testWidgets('stats overlay on $name', (t) async {
        setSurface(t, size, dpr: dpr);
        final (services, _) = await _services();
        services.locale.value = const Locale('ru');
        await t.pumpWidget(testApp(services, formFactor: ff));
        await settle(t);
        await _play(t, services);
        await t.sendKeyEvent(LogicalKeyboardKey.keyS);
        await settle(t, 6);
        expect(find.byKey(const ValueKey('osd-stats')), findsOneWidget);
        expect(t.takeException(), isNull);
        await capture(t, '01-$name-stats');
        final player = t.state<PlayerScreenState>(find.byType(PlayerScreen));
        player.osd.openPanel(OsdPanel.info);
        await settle(t, 8);
        expect(t.takeException(), isNull);
        await capture(t, '02-$name-info-panel');
        await t.pumpWidget(const SizedBox());
        await settle(t, 2);
      });
    }

    testWidgets('report saved from the player', (t) async {
      setSurface(t, const Size(1280, 800));
      final (services, _) = await _services();
      services.locale.value = const Locale('ru');
      await t.pumpWidget(testApp(services));
      await settle(t);
      await _play(t, services);
      t.state<PlayerScreenState>(find.byType(PlayerScreen)).osd.openPanel(
        OsdPanel.info,
      );
      await settle(t, 6);
      await t.tap(find.byKey(const ValueKey('info-stats-export')));
      await settle(t, 8);
      expect(find.byKey(const ValueKey('diagnostics-saved')), findsOneWidget);
      await capture(t, '03-desktop-report-saved');
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('transcode hint for a slow media server', (t) async {
      setSurface(t, const Size(1280, 800));
      final (services, engine) = await _services();
      engine.fakePlaybackStats = const PlaybackStats(
        engine: 'libmpv 0.40.0',
        videoCodec: 'h264',
        videoWidth: 1920,
        videoHeight: 1080,
        audioCodec: 'aac',
        hwDecoding: true,
        hwDecoder: 'mediacodec',
        source: PlaybackSource.server,
        bitrateBps: 20000000,
        networkBps: 9000000,
        bufferedMs: 3200,
        bufferTargetMs: 120000,
        stalls: 3,
      );
      services.locale.value = const Locale('ru');
      await t.pumpWidget(testApp(services));
      await settle(t);
      await _play(t, services);
      await t.sendKeyEvent(LogicalKeyboardKey.keyS);
      await settle(t, 6);
      expect(find.byKey(const ValueKey('osd-stats-transcode')), findsOneWidget);
      await capture(t, '04-desktop-transcode-hint');
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  });
}
