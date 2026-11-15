// SPDX-License-Identifier: MPL-2.0
//
// TWI-019: the recordings and downloads library (active / paused / gap
// indicators), the record rule of a favourite channel (ToS warning,
// quality, chat, keep N / X days, categories), the storage settings
// (templates with preview, quota) and import. FakeTwitchArchive stands in
// for the core; no network, no Twitch. Entry points exist only in full
// builds:
//   flutter test test/twitch_archive_test.dart --dart-define=KINOLUNE_FLAVOR=full
// Screenshots with KINOLUNE_SCREENSHOTS_DIR set.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_twitch_archive.dart';
import 'package:kinolune/flavor.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/twitch/twitch_archive_screens.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/twitch/twitch_archive.dart';
import 'package:kinolune/twitch/twitch_downloads.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

Future<void> _push(WidgetTester t, Widget page) async {
  final nav = t.state<NavigatorState>(find.byType(Navigator).first);
  // Not awaited: the route completes when popped.
  // ignore: unawaited_futures
  nav.push(MaterialPageRoute<void>(builder: (_) => page));
  await settle(t);
}

void main() {
  setUpAll(loadAppFonts);

  testWidgets('the library shows active, paused and gap indicators', (
    t,
  ) async {
    setSurface(t, const Size(1280, 800));
    final archive = FakeTwitchArchive.demo(demoNow);
    final services = await demoServices(twitchArchive: archive);
    await t.pumpWidget(testApp(services, section: Section.twitch));
    await settle(t);
    await _push(t, const TwitchArchiveScreen(refresh: Duration(hours: 1)));
    expect(find.byKey(const ValueKey('twitch-archive-page')), findsOneWidget);
    await capture(t, 'twi-019/02-recordings-desktop');

    // Running: "Recording"; paused by the space monitor, with a gap.
    expect(
      find.byKey(const ValueKey('twitch-recording-41-active')),
      findsOneWidget,
    );
    expect(
      find.byKey(const ValueKey('twitch-recording-40-paused')),
      findsOneWidget,
    );
    expect(find.byKey(const ValueKey('twitch-recording-40-gaps')), findsOne);
    expect(find.text('1 gap'), findsOneWidget);
    // The finished job is in the saved list, not in "In progress".
    expect(find.byKey(const ValueKey('twitch-recording-38')), findsNothing);
    expect(find.byKey(const ValueKey('twitch-saved-3')), findsOneWidget);
    expect(find.text('Ads marked'), findsOneWidget);

    await t.tap(find.byKey(const ValueKey('twitch-recording-41-pause')));
    await t.tap(find.byKey(const ValueKey('twitch-recording-40-resume')));
    await settle(t);
    expect(archive.actions, [
      (41, TwitchDownloadAction.pause),
      (40, TwitchDownloadAction.resume),
    ]);

    // Downloads and imports.
    await t.tap(find.byKey(const ValueKey('twitch-archive-tab-downloads')));
    await settle(t);
    expect(find.text('Any% practice — Neon Drift'), findsOneWidget);
    expect(find.text('3 chapters'), findsOneWidget);
    expect(find.textContaining('Imported'), findsOneWidget);
    await capture(t, 'twi-019/03-downloads-desktop');
    await t.pumpWidget(const SizedBox());
  });

  testWidgets('a record rule: warning once, then saved with its settings', (
    t,
  ) async {
    setSurface(t, const Size(1280, 900));
    final archive = FakeTwitchArchive.demo(demoNow);
    final services = await demoServices(twitchArchive: archive);
    await t.pumpWidget(testApp(services, section: Section.twitch));
    await settle(t);
    await _push(
      t,
      const TwitchRecordRuleScreen(
        userId: '7002',
        displayName: 'LunaForge',
        categories: [('504', 'Neon Drift'), ('501', 'Starfall Tactics')],
      ),
    );
    expect(find.byKey(const ValueKey('twitch-rule-page-7002')), findsOne);

    // Turning it on asks once (06 §2.7); cancelling keeps it off.
    await t.tap(find.byKey(const ValueKey('twitch-rule-enabled')));
    await settle(t);
    expect(find.byKey(const ValueKey('twitch-record-warning')), findsOne);
    await t.tap(find.text('Cancel'));
    await settle(t);
    expect(await archive.rules(services.archiveProfileId), isEmpty);

    await t.tap(find.byKey(const ValueKey('twitch-rule-enabled')));
    await settle(t);
    await t.tap(find.byKey(const ValueKey('twitch-record-warning-accept')));
    await settle(t);
    expect(
      await services.preferences.read(
        Preferences.twitchRecordWarningAccepted,
      ),
      'true',
    );
    var rule = (await archive.rules(services.archiveProfileId)).single;
    expect(rule.enabled, isTrue);
    // Defaults: 10 broadcasts or 14 days, chat on, best quality.
    expect((rule.keepLast, rule.keepDays, rule.withChat, rule.quality), (
      10,
      14,
      true,
      'best',
    ));
    // The live watcher now runs for this channel.
    expect(services.liveNotifications.recordingChannels, {'7002'});

    // Only one category, no chat, keep 5.
    await t.tap(find.byKey(const ValueKey('twitch-rule-category-504')));
    await settle(t);
    await t.tap(find.byKey(const ValueKey('twitch-rule-chat')));
    await settle(t);
    await t.tap(find.byKey(const ValueKey('twitch-rule-keep-last')));
    await settle(t);
    await t.enterText(find.byKey(const ValueKey('twitch-rule-number')), '5');
    await t.tap(find.byKey(const ValueKey('twitch-rule-number-ok')));
    await settle(t);
    rule = (await archive.rules(services.archiveProfileId)).single;
    expect(rule.categories, {'504'});
    expect(rule.withChat, isFalse);
    expect(rule.keepLast, 5);
    await capture(t, 'twi-019/01-record-rule-desktop');

    // Off again: no warning, the watcher stops caring.
    await t.tap(find.byKey(const ValueKey('twitch-rule-enabled')));
    await settle(t);
    expect(find.byKey(const ValueKey('twitch-record-warning')), findsNothing);
    expect(services.liveNotifications.recordingChannels, isEmpty);
    await t.pumpWidget(const SizedBox());
  });

  testWidgets('the rule on a phone', (t) async {
    setSurface(t, const Size(390, 844), dpr: 2);
    final archive = FakeTwitchArchive.demo(demoNow);
    final services = await demoServices(twitchArchive: archive);
    await archive.setRule(
      services.archiveProfileId,
      const TwitchRecordRule(userId: '7001', categories: {'501'}),
    );
    await t.pumpWidget(
      testApp(services, section: Section.twitch, formFactor: FormFactor.phone),
    );
    await settle(t);
    await _push(
      t,
      const TwitchRecordRuleScreen(
        userId: '7001',
        displayName: 'PixelHeron',
        categories: [('501', 'Starfall Tactics'), ('503', 'Talk & Tea')],
      ),
    );
    expect(find.text('Only 1 category'), findsOneWidget);
    await capture(t, 'twi-019/04-record-rule-phone');
    await t.pumpWidget(const SizedBox());
  });

  testWidgets('storage settings: template preview, errors, quota, import', (
    t,
  ) async {
    setSurface(t, const Size(1280, 1000));
    final archive = FakeTwitchArchive.demo(demoNow);
    final services = await demoServices(twitchArchive: archive);
    await t.pumpWidget(testApp(services, section: Section.twitch));
    await settle(t);
    await _push(t, const TwitchArchiveSettingsScreen());
    expect(
      find.byKey(const ValueKey('twitch-archive-settings-page')),
      findsOneWidget,
    );
    expect(
      find.textContaining('2026-11-12 Elden Ring, Hades II — Title here'),
      findsWidgets,
    );
    await capture(t, 'twi-019/05-storage-settings-desktop');

    await t.enterText(
      find.byKey(const ValueKey('twitch-archive-template-vod')),
      '{date} {nope}',
    );
    await settle(t);
    expect(
      find.byKey(const ValueKey('twitch-archive-template-error')),
      findsOne,
    );
    await t.enterText(
      find.byKey(const ValueKey('twitch-archive-template-vod')),
      '{date} {title}',
    );
    await settle(t);
    await t.ensureVisible(
      find.byKey(const ValueKey('twitch-archive-template-save')),
    );
    await t.tap(find.byKey(const ValueKey('twitch-archive-template-save')));
    await settle(t);
    expect((await archive.settings()).vodTemplate, '{date} {title}');
    expect(
      await services.preferences.read(Preferences.twitchNameTemplates),
      contains('{date} {title}'),
    );

    await t.tap(find.byKey(const ValueKey('twitch-archive-quota-vod')));
    await settle(t);
    await t.tap(find.byKey(const ValueKey('twitch-archive-quota-100')));
    await settle(t);
    expect((await archive.settings()).downloadsQuotaBytes, 100 << 30);

    // Import with a chosen folder (the picker is the platform's).
    final ctx = t.element(
      find.byKey(const ValueKey('twitch-archive-settings-page')),
    );
    final r = await importTwitchFolder(ctx, path: '/media/archive');
    await settle(t);
    expect(r?.added, 3);
    expect(archive.imported, ['/media/archive']);
    expect(find.textContaining('Added 3'), findsOneWidget);
    await t.pumpWidget(const SizedBox());
  });

  testWidgets('entry points: full builds only, never without the queue', (
    t,
  ) async {
    setSurface(t, const Size(1280, 800));
    final services = await demoServices(
      twitchArchive: FakeTwitchArchive.demo(demoNow),
    );
    await services.setShowTwitch(true);
    await t.pumpWidget(testApp(services, section: Section.twitch));
    await settle(t);
    final entry = find.byKey(const ValueKey('twitch-archive-card'));
    if (!AppFlavor.twitchArchive) {
      expect(entry, findsNothing);
      expect(find.byKey(const ValueKey('twitch-archive')), findsNothing);
      return;
    }
    await t.scrollUntilVisible(
      entry,
      300,
      scrollable: find.byType(Scrollable).first,
    );
    expect(entry, findsOneWidget);
    await t.tap(find.byKey(const ValueKey('twitch-archive-open')));
    await settle(t);
    expect(find.byKey(const ValueKey('twitch-archive-page')), findsOneWidget);
    await t.pumpWidget(const SizedBox());
  });
}
