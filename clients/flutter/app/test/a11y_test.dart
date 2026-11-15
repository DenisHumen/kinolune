// SPDX-License-Identifier: MPL-2.0
//
// Automated accessibility audit of the key screens (UI-021): flutter_test's
// guidelines for text contrast, tap target size (Android 48 dp, iOS 44 pt)
// and labelled tap targets on phone and desktop, in the flavor the test is
// run with (`--dart-define=KINOLUNE_FLAVOR=store|full`). A screen reader on
// a real device is still checked by hand: docs/qa/ui-021-a11y-audit.md.

import 'dart:math' as math;
import 'dart:ui' show Tristate;

import 'package:flutter/semantics.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/theme/kinolune_theme.dart';
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
  'sources': (Section.settings, (t, l) => _tap(t, find.text(l.sourcesManage))),
  'twitch': (Section.twitch, null),
  'twitch-channel': (
    Section.twitch,
    (t, _) => _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron'))),
  ),
};

const _surfaces = <String, (FormFactor, Size, double)>{
  'phone': (FormFactor.phone, Size(390, 844), 2),
  'desktop': (FormFactor.desktop, Size(1280, 800), 1),
};

/// Contrast findings that are artefacts of the pixel measurement, not of the
/// colours: the guideline takes the most frequent light and dark pixels in
/// the text's box, and the 11 px caption below at 1x is anti-aliased so
/// thin that no pixel reaches its colour (textSecondary, 8.2:1 on the
/// background; the token tests below check the colours themselves). Listed
/// in docs/qa/ui-021-a11y-audit.md.
const _contrastArtefacts = {
  // Onboarding, "Devices" step on desktop (bodySmall, 11 px).
  '2 paired devices',
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
    if (result.passed) continue;
    var blocks = result.reason!.split(RegExp(r'(?=SemanticsNode#)'));
    if (name == 'contrast') {
      blocks = [
        for (final b in blocks)
          if (!_contrastArtefacts.any((a) => b.contains('label: "$a"'))) b,
      ];
    }
    if (blocks.any((b) => b.trim().isNotEmpty)) {
      findings.add('[$name] ${blocks.join()}');
    }
  }
  expect(findings, isEmpty, reason: findings.join('\n'));
}

/// WCAG 2.x contrast ratio of [fg] (composited over [bg]) on [bg].
double _contrast(Color fg, Color bg) {
  final c = Color.alphaBlend(fg, bg);
  final a = c.computeLuminance(), b = bg.computeLuminance();
  return (math.max(a, b) + .05) / (math.min(a, b) + .05);
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
          await capture(t, 'a11y-$surface-$name');
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
        await capture(t, 'a11y-$surface-onboarding');
        await _audit(t, touch: touch);
        for (var i = 0; i < 4; i++) {
          await t.tap(find.byKey(const ValueKey('onboarding-next')));
          await settle(t, 6);
          await capture(t, 'a11y-$surface-onboarding-$i');
          await _audit(t, touch: touch);
        }
        semantics.dispose();
      });
    });
  }

  // Every navigation item is a labelled button for screen readers on every
  // layout (bottom bar, rail, sidebar, TV menu), with the current section
  // marked selected.
  group('navigation semantics', () {
    const layouts = <String, (FormFactor, Size, double)>{
      'phone tabs': (FormFactor.phone, Size(390, 844), 2),
      'tablet rail': (FormFactor.phone, Size(820, 1180), 2),
      'desktop sidebar': (FormFactor.desktop, Size(1280, 800), 1),
      'tv menu': (FormFactor.tv, Size(960, 540), 2),
    };
    for (final MapEntry(key: name, value: (ff, size, dpr))
        in layouts.entries) {
      testWidgets(name, (t) async {
        final semantics = t.ensureSemantics();
        setSurface(t, size, dpr: dpr);
        final services = await _services(Section.movies);
        await t.pumpWidget(
          testApp(services, formFactor: ff, section: Section.movies),
        );
        await settle(t, 10);
        // Settings has no tab on phones (it sits behind the avatar).
        final phone = name == 'phone tabs';
        for (final label in [
          l.navHome,
          l.navMusic,
          phone ? l.navSearch : l.navSettings,
        ]) {
          expect(
            find.semantics.byPredicate(
              (n) =>
                  n.label.split('\n').contains(label) &&
                  n.getSemanticsData().hasAction(SemanticsAction.tap),
            ),
            findsWidgets,
            reason: '"$label" is not a button for screen readers',
          );
        }
        expect(
          find.semantics.byPredicate(
            (n) =>
                n.label
                    .split('\n')
                    .contains(phone ? l.navLibrary : l.navMovies) &&
                n.getSemanticsData().flagsCollection.isSelected ==
                    Tristate.isTrue,
          ),
          findsWidgets,
          reason: 'the current section is not marked selected',
        );
        semantics.dispose();
      });
    }
  });

  // The colour tokens used for text, on every background they sit on: the
  // pixel audit above only sees what the demo screens show.
  group('text colour tokens', () {
    final backgrounds = {
      'bgBase': KColors.bgBase,
      'bgElevated': KColors.bgElevated,
      'bgElevated2': KColors.bgElevated2,
    };
    final text = {
      'textPrimary': KColors.textPrimary,
      'textSecondary': KColors.textSecondary,
      'textSecondaryOnArt': KColors.textSecondaryOnArt,
      'textTertiary': KColors.textTertiary,
      'accentSoft': KColors.accentSoft,
    };
    for (final MapEntry(key: tn, value: tc) in text.entries) {
      for (final MapEntry(key: bn, value: bc) in backgrounds.entries) {
        test('$tn on $bn is at least 4.5:1', () {
          expect(_contrast(tc, bc), greaterThanOrEqualTo(4.5));
        });
      }
    }
    test('focused button label on the focus fill is at least 4.5:1', () {
      expect(
        _contrast(KColors.focusLabel, KColors.focusFill),
        greaterThanOrEqualTo(4.5),
      );
    });
    test('white label on the accent (primary buttons) is at least 4.5:1', () {
      expect(
        _contrast(Colors.white, KColors.accent),
        greaterThanOrEqualTo(4.5),
      );
    });
  });

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
