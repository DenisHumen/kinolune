// SPDX-License-Identifier: MPL-2.0
//
// UI-025: the «Личные медиа» section. A locked library opens only with its
// PIN, its videos filter by stars and tags, a movie of the main library
// moves in and back, every library locks and the screen is covered when
// the app leaves the foreground. With KINOLUNE_SCREENSHOTS_DIR set the
// frames are written (in Russian) to docs/screenshots/progress/ui-025:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-025 \
//     flutter test test/personal_media_test.dart

import 'package:flutter/gestures.dart' show kSecondaryMouseButton;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_personal_media.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/widgets/focusable.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

Future<void> _typePin(WidgetTester t, String pin) async {
  for (final c in pin.split('')) {
    await t.sendKeyEvent(
      LogicalKeyboardKey.findKeyByKeyId(0x30 + int.parse(c))!,
    );
  }
  await t.sendKeyEvent(LogicalKeyboardKey.enter);
  await settle(t, 4);
}

Finder _key(String k) => find.byKey(ValueKey(k));

void main() {
  setUpAll(loadAppFonts);

  testWidgets('a locked library opens only with its PIN', (t) async {
    setSurface(t, const Size(1280, 800));
    final personal = FakePersonalMedia.demo();
    final s = await demoServices(personal: personal);
    await t.pumpWidget(testApp(s, section: Section.personal));
    await settle(t);
    expect(_key('personal-library-1'), findsOneWidget);
    expect(find.text('Locked'), findsOneWidget);

    await t.tap(_key('personal-library-1'));
    await settle(t, 4);
    expect(_key('pin-dialog'), findsOneWidget);
    await _typePin(t, '1111');
    // Wrong: the pad stays, the library stays closed.
    expect(_key('pin-dialog'), findsOneWidget);
    expect(_key('personal-library-page-1'), findsNothing);
    await _typePin(t, '2468');
    expect(_key('pin-dialog'), findsNothing);
    expect(_key('personal-library-page-1'), findsOneWidget);
    expect(find.text('Дача, июль'), findsOneWidget);
    // UI-033: also in the library's «Продолжить» row.
    expect(find.text('Выпускной'), findsWidgets);

    // Five stars only, then the tag.
    await t.tap(_key('personal-stars-5'));
    await settle(t, 4);
    expect(find.text('Дача, июль'), findsOneWidget);
    expect(find.text('Аня — 5 лет'), findsOneWidget);
    expect(find.text('Море 2025'), findsNothing);
    await t.tap(_key('personal-tag-аня'));
    await settle(t, 4);
    expect(find.text('Аня — 5 лет'), findsOneWidget);
    expect(find.text('Дача, июль'), findsNothing);

    // "Lock" closes it at once.
    await t.tap(_key('personal-lock-now'));
    await settle(t, 4);
    expect(_key('personal-library-page-1'), findsNothing);
    expect(find.text('Locked'), findsOneWidget);
  });

  testWidgets('going to the background locks and covers', (t) async {
    setSurface(t, const Size(1280, 800));
    final personal = FakePersonalMedia.demo();
    final s = await demoServices(personal: personal);
    await t.pumpWidget(testApp(s, section: Section.personal));
    await settle(t);
    await t.tap(_key('personal-library-1'));
    await settle(t, 4);
    await _typePin(t, '2468');
    expect(_key('personal-library-page-1'), findsOneWidget);

    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    await settle(t, 2);
    expect(_key('personal-privacy-cover'), findsOneWidget);
    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
    await settle(t, 4);
    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await settle(t, 4);
    expect(_key('personal-privacy-cover'), findsNothing);
    // Locked again: the library page is gone, the list says so.
    expect(_key('personal-library-page-1'), findsNothing);
    expect((await personal.libraries()).first.locked, isTrue);
  });

  testWidgets('a movie moves into personal media and back', (t) async {
    setSurface(t, const Size(1280, 800));
    final moves = <(String, bool)>[];
    final personal = FakePersonalMedia.demo(
      onMove: (id, to) => moves.add((id, to)),
    );
    final s = await demoServices(personal: personal);
    final movie = (await s.library.movies().first).first;
    personal.titleOf = (_) => movie.title;
    await t.pumpWidget(testApp(s, section: Section.movies));
    await settle(t);
    final poster = find.descendant(
      of: find.byKey(ValueKey(movie.id)),
      matching: find.byType(KFocusable),
    );
    await t.tap(poster.first, buttons: kSecondaryMouseButton);
    await settle(t, 4);
    await t.tap(_key('item-menu-personal'));
    await settle(t, 4);
    // Two libraries: choose the one without a lock.
    await t.tap(_key('personal-target-2'));
    await settle(t, 4);
    expect(moves, [(movie.id, true)]);
    expect(find.text('Moved to “Домашнее видео”'), findsOneWidget);
    // Let the snack bar go before the bottom sheet opens over it.
    await settle(t, 60);

    final shell = t.state<AppShellState>(find.byType(AppShell));
    shell.select(Section.personal);
    await settle(t);
    await t.tap(_key('personal-library-2'));
    await settle(t, 4);
    final video = (await personal.videos(2)).firstWhere((v) => v.moved);
    expect(find.text(movie.title), findsWidgets);
    await t.tap(_key('personal-video-more-${video.fileId}'));
    await settle(t, 4);
    await t.ensureVisible(_key('personal-video-move-back'));
    await settle(t, 2);
    await t.tap(_key('personal-video-move-back'));
    await settle(t, 4);
    expect(moves, [(movie.id, true), (movie.id, false)]);
    expect((await personal.videos(2)).where((v) => v.moved), isEmpty);
  });

  testWidgets('a new source can be added as personal media', (t) async {
    setSurface(t, const Size(1280, 800));
    final personal = FakePersonalMedia.demo();
    final s = await demoServices(personal: personal);
    await t.pumpWidget(testApp(s, section: Section.settings));
    await settle(t);
    await t.tap(find.text('Add source').first);
    await settle(t, 6);
    await t.tap(find.text('Folder on this device').first);
    await settle(t, 6);
    expect(_key('source-content'), findsOneWidget);
    await t.tap(find.text('Personal media').last);
    await settle(t, 4);
    expect(
      find.text(
        'Files are not recognised and never go to TMDB or any other service.',
      ),
      findsOneWidget,
    );
    // Only "Домашнее видео" is open (no lock): it is offered.
    expect(_key('source-content-library'), findsOneWidget);
  });

  for (final (name, ff, size, dpr) in <(String, FormFactor, Size, double)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800), 1),
    ('phone', FormFactor.phone, const Size(390, 844), 2),
  ]) {
    testWidgets('screenshots on $name', (t) async {
      setSurface(t, size, dpr: dpr);
      final personal = FakePersonalMedia.demo();
      final s = await demoServices(personal: personal);
      s.locale.value = const Locale('ru');
      await t.pumpWidget(testApp(s, formFactor: ff, section: Section.personal));
      await settle(t);
      expect(find.text(ru.personalTitle), findsWidgets);
      await capture(t, '01-libraries-$name');

      await t.tap(_key('personal-library-1'));
      await settle(t, 4);
      await capture(t, '02-pin-$name');
      await _typePin(t, '2468');
      await settle(t, 4);
      await capture(t, '03-library-$name');

      final first = (await personal.videos(1)).first;
      await t.tap(_key('personal-video-more-${first.fileId}'));
      await settle(t, 4);
      await capture(t, '04-video-actions-$name');
      await t.tapAt(const Offset(5, 5));
      await settle(t, 10);

      if (name == 'desktop') {
        await t.ensureVisible(_key('personal-stars-5'));
        await settle(t, 2);
        await t.tap(_key('personal-stars-5'));
        await settle(t, 4);
        await capture(t, '05-filter-stars-$name');
        await t.tap(_key('personal-menu'));
        await settle(t, 4);
        await t.tap(find.text(ru.personalChangeLock));
        await settle(t, 4);
        await t.tap(find.text(ru.personalLockPassword));
        await settle(t, 4);
        await capture(t, '06-lock-password-$name');
      }
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
