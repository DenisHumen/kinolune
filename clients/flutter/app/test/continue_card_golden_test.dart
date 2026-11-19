// SPDX-License-Identifier: MPL-2.0
//
// BUG-006: the «Продолжить» card (LandscapeCard with progress) on phone,
// desktop and TV, with and without focus, at several watched shares. No
// line crosses the still at the watch position; the bar at the bottom is
// the only position mark. Goldens use the default test font like
// test/home_golden_test.dart. Refresh deliberately:
//   flutter test --update-goldens test/continue_card_golden_test.dart

import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/theme/kinolune_theme.dart';
import 'package:kinolune/widgets/artwork.dart';
import 'package:kinolune/widgets/cards.dart';
import 'package:kinolune/widgets/watched_art.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

const _rowKey = ValueKey('continue-row');
const _shares = [.1, .4, .85];

Widget _app(FormFactor ff, {required int? focused}) {
  return MaterialApp(
    debugShowCheckedModeBanner: false,
    locale: const Locale('ru'),
    supportedLocales: AppLocalizations.supportedLocales,
    localizationsDelegates: kLocalizationsDelegates,
    theme: buildKinoluneTheme(ff),
    home: FormFactorScope(
      formFactor: ff,
      child: Builder(
        builder: (context) {
          final d = KDensity.of(context);
          return Scaffold(
            backgroundColor: KColors.bgBase,
            body: Center(
              child: RepaintBoundary(
                key: _rowKey,
                child: ColoredBox(
                  color: KColors.bgBase,
                  child: Padding(
                    // Room for the focus scale and glow.
                    padding: const EdgeInsets.all(24),
                    child: IntrinsicHeight(
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          for (var i = 0; i < _shares.length; i++) ...[
                            if (i > 0) const SizedBox(width: 24),
                            LandscapeCard(
                              key: ValueKey('card-$i'),
                              seed: 'continue-$i',
                              imageUrl: '${kDemoArtScheme}continue-$i',
                              width: d.landscapeWidth,
                              title: 'Big Buck Bunny',
                              subtitle: 'осталось 7 мин',
                              badge: 'Фильм',
                              progress: _shares[i],
                              remaining: 'ещё 7 мин',
                              autofocus: focused == i,
                              onTap: () {},
                            ),
                          ],
                        ],
                      ),
                    ),
                  ),
                ),
              ),
            ),
          );
        },
      ),
    ),
  );
}

/// Luminance of a pixel of [rgba] (an image [width] wide) at [x], [y].
double _lum(ByteData rgba, int width, int x, int y) {
  final o = (y * width + x) * 4;
  return .2126 * rgba.getUint8(o) +
      .7152 * rgba.getUint8(o + 1) +
      .0722 * rgba.getUint8(o + 2);
}

/// Fails when a bright vertical line crosses the still of card [i] at its
/// watch position: the column there must not be brighter than the columns
/// a few pixels to its left and right.
Future<void> _expectNoSplitLine(WidgetTester t, int i) async {
  final boundary = t.renderObject(find.byKey(_rowKey)) as RenderRepaintBoundary;
  final origin = boundary.localToGlobal(Offset.zero);
  final still = t.getRect(
    find.descendant(
      of: find.byKey(ValueKey('card-$i')),
      matching: find.byType(WatchedStill),
    ),
  );
  final shot = await t.runAsync(() async {
    final ui.Image image = await boundary.toImage();
    final data = await image.toByteData(format: ui.ImageByteFormat.rawRgba);
    image.dispose();
    return data!;
  });
  final rgba = shot!;
  final width = boundary.size.width.round();
  final x = (still.left + still.width * _shares[i] - origin.dx).round();
  // From below the badge to above the bar and the «ещё N мин» chip.
  final top = (still.top - origin.dy + 30).round();
  final bottom = (still.bottom - origin.dy - 40).round();
  var worst = 0.0;
  for (var y = top; y < bottom; y++) {
    final here = [-1, 0, 1].map((dx) => _lum(rgba, width, x + dx, y));
    // A line is brighter than both sides; the sharp/blurred edge of the
    // art itself is only a step.
    final left = _lum(rgba, width, x - 5, y);
    final right = _lum(rgba, width, x + 5, y);
    final diff =
        here.reduce((a, b) => a > b ? a : b) - (left > right ? left : right);
    if (diff > worst) worst = diff;
  }
  expect(
    worst,
    lessThan(24),
    reason: 'a line crosses card $i at ${(_shares[i] * 100).round()} %',
  );
}

void main() {
  final densities = <(String, FormFactor, Size)>[
    ('phone', FormFactor.phone, const Size(1100, 400)),
    ('desktop', FormFactor.desktop, const Size(1600, 500)),
    ('tv', FormFactor.tv, const Size(1600, 500)),
  ];
  for (final (name, ff, size) in densities) {
    for (final focused in [null, 1]) {
      final state = focused == null ? 'plain' : 'focused';
      testWidgets('continue card on $name, $state', (t) async {
        setSurface(t, size);
        await t.pumpWidget(_app(ff, focused: focused));
        await settle(t, 6);
        // The bar stays: one per card.
        expect(find.byType(KProgressBar), findsNWidgets(_shares.length));
        for (var i = 0; i < _shares.length; i++) {
          await _expectNoSplitLine(t, i);
        }
        await expectLater(
          find.byKey(_rowKey),
          matchesGoldenFile('goldens/continue_card_${name}_$state.png'),
        );
      });
    }
  }
}
