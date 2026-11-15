// SPDX-License-Identifier: MPL-2.0
//
// Automated accessibility audit of the key screens (UI-021): flutter_test's
// guidelines for text contrast, tap target size (Android 48 dp, iOS 44 pt)
// and labelled tap targets on phone and desktop, in the flavor the test is
// run with (`--dart-define=KINOLUNE_FLAVOR=store|full`). A screen reader on
// a real device is still checked by hand: docs/qa/ui-021-a11y-audit.md.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart' show NowPlayingBar;

import 'support/test_app.dart';

typedef _Step = Future<void> Function(WidgetTester t, AppLocalizations l);

Future<void> _tap(WidgetTester t, Finder f) async {
  await t.ensureVisible(f.first);
  await settle(t, 4);
  await t.tap(f.first);
  await settle(t, 10);
}

/// The key screens of UI-021 and how to reach them from a section.
final _screens = <String, (Section, _Step?)>{
  'home': (Section.home, null),
  'movies': (Section.movies, null),
  'movie-detail': (
    Section.movies,
    (t, _) => _tap(t, find.text('The Quiet Orbit')),
  ),
  'shows': (Section.shows, null),
  'show-detail': (
    Section.shows,
    (t, _) => _tap(t, find.text('Moonlit Bureau')),
  ),
  'player': (
    Section.movies,
    (t, _) async {
      await _tap(t, find.text('The Quiet Orbit'));
      await _tap(t, find.byIcon(Icons.play_arrow_rounded));
    },
  ),
  'music': (Section.music, null),
  'album': (Section.music, (t, _) => _tap(t, find.text('Nocturne Avenue'))),
  'music-player': (
    Section.music,
    (t, l) async {
      await _tap(t, find.text('Nocturne Avenue'));
      await _tap(t, find.text(l.musicPlay));
      await _tap(t, find.byType(NowPlayingBar));
    },
  ),
  'search': (
    Section.search,
    (t, _) async {
      await t.enterText(find.byType(TextField), 'o');
      await settle(t, 6);
    },
  ),
  'settings': (Section.settings, null),
  'sources': (
    Section.settings,
    (t, l) => _tap(t, find.text(l.sourcesManage)),
  ),
  'twitch': (Section.twitch, null),
  'twitch-channel': (
    Section.twitch,
    (t, _) =>
        _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron'))),
  ),
};

const _surfaces = <String, (FormFactor, Size, double)>{
  'phone': (FormFactor.phone, Size(390, 844), 2),
  'desktop': (FormFactor.desktop, Size(1280, 800), 1),
};

/// Evaluates every guideline (not only up to the first failure) and fails
/// with all findings, one block per guideline.
Future<void> _audit(WidgetTester t, {required bool touch}) async {
  final guidelines = <String, AccessibilityGuideline>{
    'labeled': labeledTapTargetGuideline,
    'contrast': textContrastGuideline,
    if (touch) 'android-tap': androidTapTargetGuideline,
    if (touch) 'ios-tap': iOSTapTargetGuideline,
  };
  final findings = <String>[];
  for (final MapEntry(key: name, value: g) in guidelines.entries) {
    final result = await g.evaluate(t);
    if (!result.passed) findings.add('[$name] ${result.reason}');
  }
  expect(findings, isEmpty, reason: findings.join('\n'));
}

Future<AppServices> _services(Section section, {bool empty = false}) async {
  final s = section == Section.twitch
      ? await twitchServices()
      : await demoServices(empty: empty);
  await s.setLocale(const Locale('en'));
  return s;
}

void main() {
  setUpAll(loadAppFonts);
  final l = lookupAppLocalizations(const Locale('en'));

  for (final MapEntry(key: surface, value: (ff, size, dpr))
      in _surfaces.entries) {
    final touch = ff == FormFactor.phone;
    group('a11y $surface', () {
      for (final MapEntry(key: name, value: (section, step))
          in _screens.entries) {
        testWidgets(name, (t) async {
          final semantics = t.ensureSemantics();
          setSurface(t, size, dpr: dpr);
          final services = await _services(section);
          await t.pumpWidget(
            testApp(services, formFactor: ff, section: section),
          );
          await settle(t, 10);
          await step?.call(t, l);
          await settle(t, 6);
          await _audit(t, touch: touch);
          services.music.pause();
          await t.pumpWidget(const SizedBox());
          await settle(t, 2);
          semantics.dispose();
        });
      }

      testWidgets('onboarding', (t) async {
        final semantics = t.ensureSemantics();
        setSurface(t, size, dpr: dpr);
        final services = await _services(Section.home, empty: true);
        await t.pumpWidget(testApp(services, formFactor: ff));
        await settle(t, 10);
        await _audit(t, touch: touch);
        for (var i = 0; i < 4; i++) {
          await t.tap(find.byKey(const ValueKey('onboarding-next')));
          await settle(t, 6);
          await _audit(t, touch: touch);
        }
        semantics.dispose();
      });
    });
  }

  group('Reduce Motion', () {
    testWidgets('pages crossfade instead of sliding', (t) async {
      setSurface(t, const Size(1280, 800));
      final services = await _services(Section.movies);
      t.platformDispatcher.accessibilityFeaturesTestValue =
          const FakeAccessibilityFeatures(disableAnimations: true);
      addTearDown(t.platformDispatcher.clearAccessibilityFeaturesTestValue);
      await t.pumpWidget(testApp(services, section: Section.movies));
      await settle(t, 10);
      await t.tap(find.text('The Quiet Orbit'));
      await t.pump();
      await t.pump(const Duration(milliseconds: 60));
      // Mid-transition: only fades, no slide or scale of the new page.
      final route = find.byKey(const ValueKey('reduced-motion-transition'));
      expect(route, findsWidgets);
      expect(
        find.descendant(of: route.last, matching: find.byType(SlideTransition)),
        findsNothing,
      );
      await settle(t, 10);
    });
  });
}
