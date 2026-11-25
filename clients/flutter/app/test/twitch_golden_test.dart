// SPDX-License-Identifier: MPL-2.0
//
// The Streams section (TWI-013): golden images of its Home, the channel
// list, a channel page, search, Browse and a category page (TWI-034) on the
// four densities, over the fake
// Helix with made-up channels (lib/fakes/fake_twitch.dart). Like
// test/home_golden_test.dart they use the default test font. Refresh
// deliberately:
//   flutter test --update-goldens test/twitch_golden_test.dart
//   flutter test --update-goldens --dart-define=KINOLUNE_FLAVOR=full \
//     test/twitch_golden_test.dart
//
// Home shows the full-only "Downloaded" row (ADR-0011 §5, TWI-017/019), so
// it has a golden per flavor: `twitch_home_<density>.png` for store and
// `twitch_home_<density>_full.png` for full. The other screens look the
// same in both flavors with the fake services and share one golden.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/flavor.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

Future<void> _tap(WidgetTester t, Finder f) async {
  await t.ensureVisible(f.first);
  await settle(t, 4);
  await t.tap(f.first);
  await settle(t, 10);
}

final _flows = <String, Future<void> Function(WidgetTester)>{
  'home': (_) async {},
  'channels': (t) => _tap(t, find.byKey(const ValueKey('twitch-tab-channels'))),
  'channel': (t) =>
      _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron'))),
  'search': (t) async {
    await _tap(t, find.byKey(const ValueKey('twitch-tab-search')));
    await t.enterText(
      find.byKey(const ValueKey('twitch-search-field')),
      'nova',
    );
    await settle(t, 10);
  },
  // TWI-034: top categories and a category's live streams.
  'browse': (t) => _tap(t, find.byKey(const ValueKey('twitch-tab-browse'))),
  'category': (t) async {
    await _tap(t, find.byKey(const ValueKey('twitch-tab-browse')));
    await _tap(t, find.byKey(const ValueKey('twitch-top-category-501')));
  },
};

/// Screens whose look depends on the build flavor.
const _flavoredScreens = {'home'};

String _golden(String screen, String density) {
  final suffix = AppFlavor.isFull && _flavoredScreens.contains(screen)
      ? '_full'
      : '';
  return 'goldens/twitch_${screen}_$density$suffix.png';
}

void main() {
  final densities = <(String, FormFactor, Size)>[
    ('phone', FormFactor.phone, const Size(390, 844)),
    ('tablet', FormFactor.phone, const Size(820, 1180)),
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('tv', FormFactor.tv, const Size(960, 540)),
  ];
  for (final (name, ff, size) in densities) {
    for (final MapEntry(key: screen, value: flow) in _flows.entries) {
      testWidgets('twitch $screen on $name', (t) async {
        setSurface(t, size);
        final services = await twitchServices();
        await t.pumpWidget(
          testApp(services, formFactor: ff, section: Section.twitch),
        );
        await settle(t);
        await flow(t);
        await expectLater(
          find.byKey(screenshotKey),
          matchesGoldenFile(_golden(screen, name)),
        );
        await t.pumpWidget(const SizedBox());
        await settle(t, 2);
      });
    }
  }
}
