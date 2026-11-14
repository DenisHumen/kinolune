// SPDX-License-Identifier: MPL-2.0
//
// The Streams section (TWI-013): golden images of its Home, the channel
// list, a channel page and search on the four densities, over the fake
// Helix with made-up channels (lib/fakes/fake_twitch.dart). Like
// test/home_golden_test.dart they use the default test font. Refresh
// deliberately:
//   flutter test --update-goldens test/twitch_golden_test.dart

import 'package:flutter_test/flutter_test.dart';
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
};

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
          matchesGoldenFile('goldens/twitch_${screen}_$name.png'),
        );
        await t.pumpWidget(const SizedBox());
        await settle(t, 2);
      });
    }
  }
}
