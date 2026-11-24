// SPDX-License-Identifier: MPL-2.0
// PLAY-012: the cast button appears only with a device, lists the devices,
// the mini-controller follows the session, and a file the TV cannot play
// is explained (ru).

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/cast/cast_controller.dart';
import 'package:kinolune/cast/cast_plan.dart';
import 'package:kinolune/cast/cast_platform.dart';
import 'package:kinolune/cast/cast_progress.dart';
import 'package:kinolune/cast/cast_ui.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:material_ui/material_ui.dart';

class _Platform implements CastPlatform {
  // Synchronous: the controller subscribes in setUp, outside the test's
  // fake-async zone, where an async event would wait for the real loop.
  final devicesCtl = StreamController<List<CastDevice>>.broadcast(sync: true);
  final calls = <String>[];
  CastStatus? next;

  @override
  Future<bool> available() async => true;
  @override
  Stream<List<CastDevice>> get devices => devicesCtl.stream;
  @override
  Stream<void> get sessionEnded => const Stream.empty();
  @override
  Future<void> startDiscovery() async => calls.add('startDiscovery');
  @override
  Future<void> stopDiscovery() async => calls.add('stopDiscovery');
  @override
  Future<CastConnection?> connect(CastDevice device) async =>
      const CastConnection(model: 'Chromecast');
  @override
  Future<bool> load(CastLoad load) async => true;
  @override
  Future<bool> airPlay(CastLoad load, Rect anchor) async => true;
  @override
  Future<CastStatus?> status() async => next;
  @override
  Future<void> play() async => calls.add('play');
  @override
  Future<void> pause() async => calls.add('pause');
  @override
  Future<void> seek(Duration position) async => calls.add('seek');
  @override
  Future<void> stop() async => calls.add('stop');
}

const _tv = CastDevice(
  id: 'cc1',
  name: 'Гостиная',
  tech: CastTech.googleCast,
  model: 'Chromecast',
);
const _bedroom = CastDevice(
  id: 'cc2',
  name: 'Спальня',
  tech: CastTech.googleCast,
);
const _airPlay = CastDevice(id: 'airplay', name: '', tech: CastTech.airPlay);

Widget _app(Widget child) => MaterialApp(
  locale: const Locale('ru'),
  supportedLocales: AppLocalizations.supportedLocales,
  localizationsDelegates: kLocalizationsDelegates,
  home: Scaffold(body: Center(child: child)),
);

void main() {
  late _Platform platform;
  late CastController controller;

  setUp(() {
    platform = _Platform();
    controller = CastController(
      platform: platform,
      techs: const [CastTech.googleCast, CastTech.airPlay],
      lanAddress: (_) async => null,
      pollInterval: const Duration(hours: 1),
    );
  });
  tearDown(() => controller.dispose());

  testWidgets('cast button: hidden without a device, lists them', (
    tester,
  ) async {
    final picked = <CastDevice>[];
    await tester.pumpWidget(
      _app(
        CastButton(
          controller: controller,
          onPick: (d, _) async => picked.add(d),
        ),
      ),
    );
    await tester.pump();
    expect(platform.calls, ['startDiscovery']);
    expect(find.byKey(const ValueKey('cast-button')), findsNothing);

    platform.devicesCtl.add([_tv, _bedroom]);
    await tester.pump();
    expect(find.byKey(const ValueKey('cast-button')), findsOneWidget);
    expect(find.byTooltip('Смотреть на телевизоре'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('cast-button')));
    await tester.pumpAndSettle();
    expect(find.text('Где смотреть'), findsOneWidget);
    await tester.tap(find.text('Спальня'));
    await tester.pumpAndSettle();
    expect(picked, [_bedroom]);

    await tester.pumpWidget(const SizedBox());
    await tester.pump();
    expect(platform.calls.last, 'stopDiscovery');
  });

  testWidgets('one AirPlay entry opens the system picker directly', (
    tester,
  ) async {
    final picked = <(CastDevice, Rect?)>[];
    await tester.pumpWidget(
      _app(
        CastButton(
          controller: controller,
          onPick: (d, a) async => picked.add((d, a)),
        ),
      ),
    );
    platform.devicesCtl.add([_airPlay]);
    await tester.pump();
    expect(find.byIcon(Icons.airplay_rounded), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('cast-button')));
    await tester.pumpAndSettle();
    expect(find.text('Где смотреть'), findsNothing);
    expect(picked.single.$1, _airPlay);
    expect(picked.single.$2, isNotNull);
  });

  testWidgets('mini-controller while casting: pause and stop', (tester) async {
    await tester.pumpWidget(_app(CastMiniBar(controller: controller)));
    expect(find.byKey(const ValueKey('cast-mini-bar')), findsNothing);

    // A server stream: no relay, no sockets in a widget test.
    final r = await controller.cast(
      CastRequest(
        itemId: 'm1',
        title: 'Дюна',
        playbackUri: '/m/dune.mkv',
        fileName: '/m/dune.mkv',
        tech: const TechInfo(container: 'matroska,webm', videoCodec: 'hevc'),
        durationMs: 9000000,
        serverItem: true,
        serverStream: (_) async => 'http://192.168.1.10:8096/master.m3u8',
        onProgress: (_, _) {},
      ),
      _tv,
    );
    expect(r.outcome, CastOutcome.started);
    platform.next = const CastStatus(
      state: CastPlayerState.playing,
      position: Duration(minutes: 30),
      duration: Duration(minutes: 150),
    );
    await controller.togglePause();
    await tester.pump();
    expect(find.byKey(const ValueKey('cast-mini-bar')), findsOneWidget);
    expect(find.text('Дюна'), findsOneWidget);
    expect(find.text('Играет на «Гостиная»'), findsOneWidget);
    expect(find.byIcon(Icons.pause_rounded), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('cast-mini-play')));
    await tester.pump();
    expect(platform.calls, contains('pause'));

    await tester.tap(find.byKey(const ValueKey('cast-mini-bar')));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('cast-controls')), findsOneWidget);
    expect(find.text('30:00'), findsOneWidget);
    expect(find.text('2:30:00'), findsOneWidget);
    await tester.tap(find.text('Остановить трансляцию'));
    await tester.pumpAndSettle();
    expect(platform.calls.last, 'stop');
    expect(controller.session, isNull);
    expect(find.byKey(const ValueKey('cast-mini-bar')), findsNothing);
  });

  testWidgets('a file the TV cannot play: why, and «try anyway»', (
    tester,
  ) async {
    late BuildContext ctx;
    await tester.pumpWidget(
      _app(
        Builder(
          builder: (context) {
            ctx = context;
            return const SizedBox();
          },
        ),
      ),
    );
    final answer = showCastUnsupported(
      ctx,
      _tv,
      const CastUnsupported(CastProblem.audioCodec, detail: 'DTS-HD'),
    );
    await tester.pumpAndSettle();
    expect(find.text('«Гостиная» не воспроизведёт этот файл'), findsOneWidget);
    expect(
      find.textContaining('Телевизор не воспроизводит звук DTS-HD.'),
      findsOneWidget,
    );
    await tester.tap(find.byKey(const ValueKey('cast-try-anyway')));
    await tester.pumpAndSettle();
    expect(await answer, isTrue);
  });
}
