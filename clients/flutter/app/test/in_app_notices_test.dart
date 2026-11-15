// SPDX-License-Identifier: MPL-2.0
//
// UI-026: in-app notice banners — the queue, the card over the app (focus
// with a remote, Back, auto-hide, Reduce Motion, screen readers) and the
// events that feed it. With KINOLUNE_SCREENSHOTS_DIR set the phone, desktop
// and Android TV frames go to docs/screenshots/progress/ui-026:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-026 \
//     flutter test test/in_app_notices_test.dart

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/notices/in_app_notices.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/movie_detail_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/sync/device_sync.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));
final en = lookupAppLocalizations(const Locale('en'));

InAppNotice _n(
  String id, {
  InAppNoticeKind kind = InAppNoticeKind.library,
  String? body,
  NoticeAction? onAction,
  VoidCallback? onDismiss,
  Duration duration = InAppNotice.defaultDuration,
}) => InAppNotice(
  id: id,
  kind: kind,
  title: (_) => id,
  body: body == null ? null : (_) => body,
  onAction: onAction,
  onDismiss: onDismiss,
  duration: duration,
);

final _card = find.byKey(const ValueKey('in-app-notice'));
final _action = find.byKey(const ValueKey('notice-action'));
final _dismiss = find.byKey(const ValueKey('notice-dismiss'));

String? get _focusLabel => FocusManager.instance.primaryFocus?.debugLabel;

bool get _onCard =>
    _focusLabel == 'notice action' || _focusLabel == 'notice dismiss';

Future<AppServices> _app(
  WidgetTester t, {
  FormFactor ff = FormFactor.desktop,
  Size size = const Size(1280, 800),
  double dpr = 1,
  Locale locale = const Locale('en'),
  FakeDeviceSync? sync,
}) async {
  setSurface(t, size, dpr: dpr);
  final s = await demoServices(sync: sync);
  s.locale.value = locale;
  await t.pumpWidget(testApp(s, formFactor: ff));
  await settle(t);
  return s;
}

void main() {
  setUpAll(loadAppFonts);

  group('queue', () {
    test('one at a time, in order; the same id replaces its text', () {
      final q = InAppNotices();
      expect(q.post(_n('a')), isTrue);
      q.post(_n('b'));
      q.post(_n('c', body: 'one'));
      q.post(_n('c', body: 'two'));
      expect(q.current!.id, 'a');
      expect(q.queued.map((n) => n.id), ['b', 'c']);
      expect(q.queued.last.body!(en), 'two');
      final serial = q.serial;
      q.post(_n('a', body: 'new text'));
      expect(q.serial, serial, reason: 'same notice, new text');
      expect(q.current!.body!(en), 'new text');
      q.expire();
      expect(q.current!.id, 'b');
      q.expire();
      q.expire();
      expect(q.current, isNull);
      expect(q.contains('a'), isFalse);
    });

    test('a long queue drops the oldest waiting notice', () {
      final q = InAppNotices(maxQueued: 2);
      for (final id in ['a', 'b', 'c', 'd']) {
        q.post(_n(id));
      }
      expect(q.current!.id, 'a');
      expect(q.queued.map((n) => n.id), ['c', 'd']);
    });

    test('dismiss runs onDismiss, expiry does not; remove and clear', () {
      var dismissed = 0;
      final q = InAppNotices()
        ..post(_n('a', onDismiss: () => dismissed++))
        ..post(_n('b', onDismiss: () => dismissed++))
        ..post(_n('c'));
      q.expire();
      expect(dismissed, 0);
      q.dismiss();
      expect(dismissed, 1);
      expect(q.current!.id, 'c');
      q.post(_n('d'));
      q.remove('d');
      expect(q.queued, isEmpty);
      q.clear();
      expect(q.current, isNull);
    });

    test('the filter hook refuses notices (UI-025 protected libraries)', () {
      final q = InAppNotices()..filter = (n) => n.itemId != 'secret';
      expect(
        q.post(
          const InAppNotice(
            id: 'x',
            kind: InAppNoticeKind.library,
            title: _title,
            itemId: 'secret',
          ),
        ),
        isFalse,
      );
      expect(q.current, isNull);
      expect(q.post(_n('ok')), isTrue);
    });
  });

  group('card', () {
    testWidgets('shows over the app without taking focus and hides by itself', (
      t,
    ) async {
      final s = await _app(t, ff: FormFactor.tv, size: const Size(1920, 1080));
      final before = FocusManager.instance.primaryFocus;
      expect(before, isNotNull);
      s.notices
        ..post(_n('First', body: 'body one'))
        ..post(_n('Second'));
      await settle(t, 4);
      expect(_card, findsOneWidget);
      expect(find.text('First'), findsOneWidget);
      expect(find.text('Second'), findsNothing);
      expect(find.text(en.noticeMore(1)), findsOneWidget);
      expect(FocusManager.instance.primaryFocus, same(before));
      // Top right on TV.
      final box = t.getRect(_card);
      expect(box.right, greaterThan(1920 - 120));
      expect(box.top, lessThan(120));

      await t.pump(InAppNotice.defaultDuration);
      await settle(t, 4);
      expect(find.text('First'), findsNothing);
      expect(find.text('Second'), findsOneWidget);
      await t.pump(InAppNotice.defaultDuration);
      await settle(t, 4);
      expect(_card, findsNothing);
      expect(FocusManager.instance.primaryFocus, same(before));
    });

    testWidgets('remote: Up at the top reaches the card, Down returns, '
        'Back closes', (t) async {
      final s = await _app(t, ff: FormFactor.tv, size: const Size(1920, 1080));
      var dismissed = 0;
      s.notices.post(
        _n(
          'Severance',
          body: 'New episode',
          onAction: (_) {},
          onDismiss: () => dismissed++,
        ),
      );
      await settle(t, 4);
      // Up until the page has nothing above: then the card.
      for (var i = 0; i < 12 && !_onCard; i++) {
        final was = FocusManager.instance.primaryFocus;
        await pressKey(t, LogicalKeyboardKey.arrowUp);
        if (_onCard) {
          expect(_focusLabel, 'notice action');
          // Down: back where the focus was before.
          await pressKey(t, LogicalKeyboardKey.arrowDown);
          expect(FocusManager.instance.primaryFocus, same(was));
          await pressKey(t, LogicalKeyboardKey.arrowUp);
          break;
        }
      }
      expect(_onCard, isTrue);
      // Left and right move between the buttons.
      await pressKey(t, LogicalKeyboardKey.arrowLeft);
      expect(_focusLabel, 'notice dismiss');
      await pressKey(t, LogicalKeyboardKey.arrowRight);
      expect(_focusLabel, 'notice action');
      // The card stays while it has the focus.
      await t.pump(InAppNotice.defaultDuration * 3);
      await settle(t, 2);
      expect(_card, findsOneWidget);
      await capture(t, '03-tv-focused');

      // Back closes it; the focus is back on the page.
      await pressKey(t, LogicalKeyboardKey.goBack);
      expect(_card, findsNothing);
      expect(dismissed, 1);
      expect(_onCard, isFalse);
      expect(FocusManager.instance.primaryFocus, isNotNull);
    });

    testWidgets('F6 moves the focus to the card on desktop; Escape closes', (
      t,
    ) async {
      final s = await _app(t);
      s.notices.post(_n('Arrival', onAction: (_) {}));
      await settle(t, 4);
      await pressKey(t, LogicalKeyboardKey.f6);
      expect(_focusLabel, 'notice action');
      await pressKey(t, LogicalKeyboardKey.escape);
      expect(_card, findsNothing);
      // A mouse closes it with its button.
      s.notices.post(_n('Dune'));
      await settle(t, 4);
      await t.tap(_dismiss);
      await settle(t, 4);
      expect(_card, findsNothing);
    });

    testWidgets('the action runs under the app navigator', (t) async {
      final s = await _app(t);
      final movie = (await s.library.movies().first).first;
      s.notices.post(
        InAppNotice(
          id: 'm',
          kind: InAppNoticeKind.library,
          title: (_) => movie.title,
          body: (l) => l.noticeNewMovie,
          onAction: (ctx) => Navigator.of(ctx).push(
            MaterialPageRoute<void>(
              builder: (_) => MovieDetailScreen(item: movie),
            ),
          ),
        ),
      );
      await settle(t, 4);
      expect(find.text(en.noticeOpen), findsOneWidget);
      await t.tap(_action);
      await settle(t, 8);
      expect(_card, findsNothing);
      expect(find.byType(MovieDetailScreen), findsOneWidget);
    });

    testWidgets('phone: across the top, swipe up closes', (t) async {
      final s = await _app(
        t,
        ff: FormFactor.phone,
        size: const Size(390, 844),
        dpr: 2,
      );
      s.notices.post(_n('Dune', body: 'New movie in the library'));
      await settle(t, 4);
      final box = t.getRect(_card);
      expect(box.left, lessThan(20));
      expect(box.right, greaterThan(370));
      await t.drag(_card, const Offset(0, -80));
      await settle(t, 4);
      expect(_card, findsNothing);
    });

    testWidgets('Reduce Motion: no slide, the card is there at once', (
      t,
    ) async {
      t.platformDispatcher.accessibilityFeaturesTestValue =
          const FakeAccessibilityFeatures(disableAnimations: true);
      addTearDown(t.platformDispatcher.clearAccessibilityFeaturesTestValue);
      final s = await _app(t);
      s.notices.post(_n('Instant'));
      await t.pump();
      await t.pump();
      final fades = t.widgetList<FadeTransition>(
        find.ancestor(of: _card, matching: find.byType(FadeTransition)),
      );
      expect(fades, isNotEmpty);
      expect(fades.first.opacity.value, 1);
      final slides = t.widgetList<SlideTransition>(
        find.ancestor(of: _card, matching: find.byType(SlideTransition)),
      );
      expect(slides.first.position.value, Offset.zero);
    });

    testWidgets('screen readers: a live region with a dismiss action, an '
        'announcement off Android, and more time', (t) async {
      final semantics = t.ensureSemantics();
      final s = await _app(t);
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      t.platformDispatcher.accessibilityFeaturesTestValue =
          const FakeAccessibilityFeatures(accessibleNavigation: true);
      try {
        s.notices.post(_n('Arrival', body: 'Downloaded'));
        await settle(t, 4);
        expect(
          t.getSemantics(_card),
          isSemantics(isLiveRegion: true, hasDismissAction: true),
        );
        final said = t.takeAnnouncements();
        expect(said.map((a) => a.message), contains('Arrival. Downloaded'));
        // Three times the usual time.
        await t.pump(InAppNotice.defaultDuration * 2);
        await settle(t, 2);
        expect(_card, findsOneWidget);
        await t.pump(InAppNotice.defaultDuration);
        await settle(t, 4);
        expect(_card, findsNothing);
      } finally {
        debugDefaultTargetPlatformOverride = null;
        t.platformDispatcher.clearAccessibilityFeaturesTestValue();
        semantics.dispose();
      }
    });
  });

  group('events', () {
    testWidgets('another device starts a title: Resume here', (t) async {
      final sync = FakeDeviceSync.demo();
      final s = await _app(t, sync: sync, locale: const Locale('ru'));
      final movie = (await s.library.movies().first).first;
      final tv = sync.devices().firstWhere((d) => d.kind == SyncDeviceKind.tv);
      sync.receiveTakeover(deviceId: tv.id, itemId: movie.id);
      await settle(t, 4);
      expect(_card, findsOneWidget);
      expect(find.text(movie.title), findsWidgets);
      expect(find.text(ru.noticeHandoffBody(tv.name)), findsOneWidget);
      expect(find.text(ru.noticeResumeHere), findsOneWidget);
    });

    testWidgets('new titles after a scan; nothing for what was there', (
      t,
    ) async {
      setSurface(t, const Size(1280, 800));
      final s = await demoServices(empty: true);
      await s.completeOnboarding();
      await t.pumpWidget(testApp(s));
      await settle(t);
      expect(_card, findsNothing);
      await s.library.addSource(
        const MediaSource(
          id: 'src-new',
          kind: SourceKind.localFolder,
          name: 'Films',
          path: '/media',
        ),
      );
      await t.runAsync(() => s.library.scan(sourceId: 'src-new').drain<void>());
      await settle(t, 6);
      expect(_card, findsOneWidget);
      expect(find.text(en.noticeNewInLibrary), findsOneWidget);
      // The same scan again finds nothing new.
      s.notices.clear();
      await t.runAsync(() => s.library.scan(sourceId: 'src-new').drain<void>());
      await settle(t, 6);
      expect(_card, findsNothing);
    });
  });

  group('screenshots', () {
    InAppNotice episode(AppLocalizations l) => InAppNotice(
      id: 'shot-episode',
      kind: InAppNoticeKind.library,
      title: (_) => 'Severance', // l10n-ignore
      body: (l) => l.noticeNewEpisode('S2 E7 Chikhai Bardo'), // l10n-ignore
      onAction: (_) {},
    );

    testWidgets('phone', (t) async {
      final s = await _app(
        t,
        ff: FormFactor.phone,
        size: const Size(390, 844),
        dpr: 2,
        locale: const Locale('ru'),
      );
      s.notices.post(episode(ru));
      await settle(t, 6);
      expect(_card, findsOneWidget);
      await capture(t, '01-phone-new-episode');
    });

    testWidgets('desktop', (t) async {
      final s = await _app(t, locale: const Locale('ru'));
      s.notices
        ..post(
          InAppNotice(
            id: 'shot-handoff',
            kind: InAppNoticeKind.handoff,
            title: (_) => 'Arrival', // l10n-ignore
            body: (l) => l.noticeHandoffBody('Living room TV'), // l10n-ignore
            actionLabel: (l) => l.noticeResumeHere,
            onAction: (_) {},
          ),
        )
        ..post(episode(ru));
      await settle(t, 6);
      expect(_card, findsOneWidget);
      await capture(t, '02-desktop-handoff');
    });

    testWidgets('Android TV', (t) async {
      final s = await _app(
        t,
        ff: FormFactor.tv,
        size: const Size(1920, 1080),
        locale: const Locale('ru'),
      );
      s.notices.post(
        InAppNotice(
          id: 'shot-live',
          kind: InAppNoticeKind.live,
          title: (l) => l.twitchLiveNotificationTitle('Northern Lights'),
          body: (_) => 'Speedrun night · Celeste', // l10n-ignore
          actionLabel: (l) => l.noticeWatch,
          onAction: (_) {},
        ),
      );
      await settle(t, 6);
      expect(_card, findsOneWidget);
      await capture(t, '04-tv-live');
    });
  });
}

String _title(AppLocalizations l) => 'x';
