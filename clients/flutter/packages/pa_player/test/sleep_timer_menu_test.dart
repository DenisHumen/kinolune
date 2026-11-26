// SPDX-License-Identifier: MPL-2.0
//
// PLAY-022: the sleep timer in the player menu on a phone, a computer and
// a TV, in Russian, English and Ukrainian; without «в конце серии» for a
// live stream and for the Twitch player.
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

enum _Device {
  phone(Size(390, 844), isTv: false),
  computer(Size(1600, 900), isTv: false),
  tv(Size(1920, 1080), isTv: true);

  const _Device(this.size, {required this.isTv});
  final Size size;
  final bool isTv;
}

const _texts = {
  'ru': (
    title: 'Таймер сна',
    end: 'В конце этой серии или фильма',
    m30: '30 мин',
    ask: 'Спрашивать после серий подряд',
    three: '3 серии',
  ),
  'en': (
    title: 'Sleep timer',
    end: 'At the end of this episode or movie',
    m30: '30 min',
    ask: 'Ask after episodes in a row',
    three: '3 episodes',
  ),
  'uk': (
    title: 'Таймер сну',
    end: 'Наприкінці цієї серії або фільму',
    m30: '30 хв',
    ask: 'Питати після серій поспіль',
    three: '3 серії',
  ),
};

Future<(FakePlayerEngine, SleepTimerController)> _open(
  WidgetTester t,
  _Device device,
  String lang, {
  bool live = false,
  bool endOfItem = true,
}) async {
  t.view.devicePixelRatio = 1;
  t.view.physicalSize = device.size;
  addTearDown(t.view.reset);
  final engine = FakePlayerEngine(loadedDuration: const Duration(minutes: 40));
  final sleep = SleepTimerController();
  addTearDown(sleep.dispose);
  await t.pumpWidget(
    MaterialApp(
      locale: Locale(lang),
      supportedLocales: const [Locale('en'), Locale('ru'), Locale('uk')],
      localizationsDelegates: const [
        PaPlayerStrings.delegate,
        GlobalMaterialLocalizations.delegate,
        GlobalWidgetsLocalizations.delegate,
        GlobalCupertinoLocalizations.delegate,
      ],
      home: PlayerScreen(
        request: const PlayRequest(uri: '/tv/e1.mkv', title: 'E1'),
        onProgress: (_, _) {},
        engineFactory: ({bool video = true}) => engine,
        loadNext: live
            ? null
            : () async => UpNextItem(
                request: const PlayRequest(uri: '/tv/e2.mkv', title: 'E2'),
                onProgress: (_, _) {},
              ),
        liveBar: live ? (_) => const SizedBox(height: 48) : null,
        sleepTimer: sleep,
        sleepEndOfItem: endOfItem,
        tvLayout: device.isTv,
      ),
    ),
  );
  await t.pump();
  await t.pump(const Duration(seconds: 1));
  return (engine, sleep);
}

void main() {
  for (final device in _Device.values) {
    for (final lang in _texts.keys) {
      final x = _texts[lang]!;
      testWidgets('${device.name} ($lang): menu, choice, «Вы ещё смотрите?»', (
        t,
      ) async {
        final (_, sleep) = await _open(t, device, lang);
        expect(find.byKey(const ValueKey('osd-sleep')), findsOneWidget);
        await t.tap(find.byKey(const ValueKey('osd-sleep')));
        await t.pumpAndSettle();
        expect(find.byKey(const ValueKey('osd-panel')), findsOneWidget);
        expect(find.text(x.title), findsWidgets);
        expect(find.text(x.end), findsOneWidget);
        await t.ensureVisible(find.text(x.three));
        expect(find.text(x.ask), findsOneWidget);
        // The setting is stored through the controller.
        await t.tap(find.byKey(const ValueKey('sleep-ask-more')));
        await t.pump();
        expect(sleep.stillWatchingAfter, 4);
        await t.ensureVisible(find.text(x.m30));
        await t.tap(find.text(x.m30));
        await t.pump();
        await t.pump(const Duration(milliseconds: 500));
        expect(sleep.option, SleepTimerOption.minutes30);
        // The panel closed; the moon is lit.
        expect(find.byKey(const ValueKey('osd-panel')), findsNothing);
        expect(t.takeException(), isNull);
        sleep.cancel();
      });
    }
  }

  testWidgets('TV remote: OK on a row of the panel sets the timer', (t) async {
    final (_, sleep) = await _open(t, _Device.tv, 'ru');
    final s = t.state<PlayerScreenState>(find.byType(PlayerScreen));
    s.osd.openPanel(OsdPanel.sleepTimer);
    await t.pumpAndSettle();
    // «Выкл.» has the focus; Down twice: «в конце серии», 15 мин.
    await t.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await t.pumpAndSettle();
    await t.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await t.pumpAndSettle();
    await t.sendKeyEvent(LogicalKeyboardKey.select);
    await t.pump();
    expect(sleep.option, SleepTimerOption.minutes15);
    sleep.cancel();
  });

  for (final device in _Device.values) {
    testWidgets('${device.name}: a live stream has no «в конце серии»', (
      t,
    ) async {
      final (_, sleep) = await _open(t, device, 'ru', live: true);
      await t.tap(find.byKey(const ValueKey('osd-sleep')));
      await t.pumpAndSettle();
      expect(find.text('15 мин'), findsOneWidget);
      expect(find.text('В конце этой серии или фильма'), findsNothing);
      // No Up Next on a live stream: no «Вы ещё смотрите?» setting.
      expect(find.byKey(const ValueKey('sleep-ask-more')), findsNothing);
      expect(t.takeException(), isNull);
      expect(sleep.option, SleepTimerOption.off);
    });
  }

  testWidgets('Twitch VOD (sleepEndOfItem off): no «в конце серии»', (t) async {
    await _open(t, _Device.computer, 'uk', endOfItem: false);
    await t.tap(find.byKey(const ValueKey('osd-sleep')));
    await t.pumpAndSettle();
    expect(find.text('90 хв'), findsOneWidget);
    expect(find.text('Наприкінці цієї серії або фільму'), findsNothing);
  });

  testWidgets('the chosen row shows the time left', (t) async {
    final (_, sleep) = await _open(t, _Device.computer, 'ru');
    sleep.set(SleepTimerOption.minutes60);
    await t.tap(find.byKey(const ValueKey('osd-sleep')));
    await t.pump();
    await t.pump(const Duration(milliseconds: 500));
    expect(find.textContaining('Пауза через '), findsOneWidget);
    sleep.cancel();
  });

  for (final lang in _texts.keys) {
    testWidgets('warning and question fit a phone ($lang)', (t) async {
      final (engine, sleep) = await _open(t, _Device.phone, lang);
      sleep
        ..restoreStillWatchingAfter(1)
        ..autoAdvanced();
      await t.pump();
      expect(find.byKey(const ValueKey('still-watching')), findsOneWidget);
      sleep
        ..stillWatching()
        ..set(SleepTimerOption.endOfItem);
      await engine.seek(const Duration(minutes: 39, seconds: 40));
      await t.pump();
      expect(find.byKey(const ValueKey('sleep-warning')), findsOneWidget);
      expect(t.takeException(), isNull);
      sleep.cancel();
    });
  }
}
