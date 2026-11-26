// SPDX-License-Identifier: MPL-2.0
//
// TWI-026: the «Медиатека» tab of the Twitch archive — streamer → game →
// parts — and a playthrough page, on FakeTwitchArchive.series. Full builds
// only:
//   flutter test test/twitch_archive_library_screen_test.dart --dart-define=KINOLUNE_FLAVOR=full
// Screenshots (phone, desktop, TV) with KINOLUNE_SCREENSHOTS_DIR set.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_twitch_archive.dart';
import 'package:kinolune/flavor.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/twitch/twitch_archive_screens.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

Future<void> _push(WidgetTester t, Widget page) async {
  final nav = t.state<NavigatorState>(find.byType(Navigator).first);
  // Not awaited: the route completes when popped.
  // ignore: unawaited_futures
  nav.push(MaterialPageRoute<void>(builder: (_) => page));
  await settle(t);
}

Finder _prefix(String p) => find.byWidgetPredicate((w) {
  final k = w.key;
  return k is ValueKey<String> && k.value.startsWith(p);
});

Future<void> _open(WidgetTester t, FormFactor ff, Size size) async {
  setSurface(t, size);
  final services = await demoServices(
    twitchArchive: FakeTwitchArchive.series(demoNow),
  );
  await t.pumpWidget(testApp(services, section: Section.twitch, formFactor: ff));
  await settle(t);
  await _push(t, const TwitchArchiveScreen(refresh: Duration(hours: 1)));
}

void main() {
  setUpAll(loadAppFonts);
  const full = AppFlavor.twitchArchive;

  testWidgets('streamers, playthroughs and a playthrough of 12 parts', (
    t,
  ) async {
    await _open(t, FormFactor.desktop, const Size(1280, 800));
    // Opens on «Медиатека»: a shelf per streamer.
    expect(find.byKey(const ValueKey('twitch-archive-shelf-7001')), findsOne);
    expect(find.byKey(const ValueKey('twitch-archive-shelf-7002')), findsOne);
    expect(find.text('Starfall Tactics playthrough'), findsWidgets);
    await capture(t, 'twi-026/01-library-desktop');

    // Search by game: only the found parts, with their part numbers.
    await t.enterText(
      find.byKey(const ValueKey('twitch-archive-search')),
      'neon drift',
    );
    await settle(t);
    expect(_prefix('twitch-archive-result-'), findsNWidgets(4));
    await capture(t, 'twi-026/02-search-desktop');
    // By streamer.
    await t.enterText(
      find.byKey(const ValueKey('twitch-archive-search')),
      'lunaforge',
    );
    await settle(t);
    expect(_prefix('twitch-archive-result-'), findsNWidgets(3));
    await t.enterText(find.byKey(const ValueKey('twitch-archive-search')), '');
    await settle(t);

    // The playthrough page: 12 parts in date order.
    await t.tap(_prefix('twitch-playthrough-7001/1159690097').first);
    await settle(t);
    expect(find.byKey(const ValueKey('twitch-playthrough-page')), findsOne);
    expect(
      find.byKey(const ValueKey('twitch-playthrough-continue')),
      findsOne,
    );
    expect(find.text('12 parts'), findsWidgets);
    expect(find.byKey(const ValueKey('twitch-part-100')), findsOne);
    await capture(t, 'twi-026/03-playthrough-desktop');
    await t.pumpWidget(const SizedBox());
  }, skip: !full);

  testWidgets('phone and TV layouts', (t) async {
    await _open(t, FormFactor.phone, const Size(390, 844));
    expect(find.byKey(const ValueKey('twitch-archive-shelf-7001')), findsOne);
    await capture(t, 'twi-026/04-library-phone');
    await t.tap(_prefix('twitch-playthrough-7001/1159690097').first);
    await settle(t);
    expect(find.byKey(const ValueKey('twitch-playthrough-page')), findsOne);
    await capture(t, 'twi-026/05-playthrough-phone');
    await t.pumpWidget(const SizedBox());

    await _open(t, FormFactor.tv, const Size(1920, 1080));
    expect(find.byKey(const ValueKey('twitch-archive-shelf-7001')), findsOne);
    await capture(t, 'twi-026/06-library-tv');
    await t.pumpWidget(const SizedBox());
  }, skip: !full);
}
