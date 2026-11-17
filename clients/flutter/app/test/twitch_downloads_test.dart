// SPDX-License-Identifier: MPL-2.0
//
// TWI-017: the Twitch downloads list and its entry point. A fake queue
// stands in for the core's; no network. The entry point exists only in
// full builds:
//   flutter test test/twitch_downloads_test.dart
//   flutter test test/twitch_downloads_test.dart --dart-define=KINOLUNE_FLAVOR=full

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/flavor.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/twitch/twitch_downloads_screen.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/twitch/twitch_downloads.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

class _FakeDownloads implements TwitchDownloads {
  final items = <TwitchDownloadItem>[
    const TwitchDownloadItem(
      jobId: 1,
      videoId: '777',
      state: TwitchDownloadState.running,
      progress: .4,
      bytesDone: 3 * 1024 * 1024,
    ),
    const TwitchDownloadItem(
      jobId: 2,
      videoId: '555',
      state: TwitchDownloadState.failed,
      error: 'usher_403: Twitch refuses this VOD (vod_manifest_restricted)',
    ),
    const TwitchDownloadItem(
      jobId: 3,
      videoId: '888',
      partLabel: 'Hades II',
      state: TwitchDownloadState.done,
      file: '2026-11-12 Practice run — Hades II [v888].mkv',
    ),
  ];
  final actions = <(int, TwitchDownloadAction)>[];
  final queued = <String>[];

  @override
  Future<int> download({
    required String videoId,
    Duration? duration,
    Duration? start,
    Duration? end,
    String? partLabel,
  }) async {
    queued.add(videoId);
    return 9;
  }

  @override
  Future<List<TwitchDownloadItem>> list() async => items;

  @override
  Future<void> control(int jobId, TwitchDownloadAction action) async =>
      actions.add((jobId, action));
}

void main() {
  setUpAll(loadAppFonts);

  testWidgets('the list shows states, errors and actions', (t) async {
    setSurface(t, const Size(1280, 800));
    final downloads = _FakeDownloads();
    final services = await demoServices(twitchDownloads: downloads);
    await t.pumpWidget(testApp(services, section: Section.twitch));
    await settle(t);
    unawaited(
      t
          .state<NavigatorState>(find.byType(Navigator).first)
          .push(
            MaterialPageRoute<void>(
              builder: (_) =>
                  const TwitchDownloadsScreen(refresh: Duration(hours: 1)),
            ),
          ),
    );
    await settle(t);
    expect(find.byKey(const ValueKey('twitch-downloads-page')), findsOneWidget);
    await capture(t, 'twi-017/01-downloads-desktop');
    expect(find.textContaining('Downloading'), findsOneWidget);
    expect(find.textContaining('3.0 MB'), findsOneWidget);
    expect(find.textContaining('vod_manifest_restricted'), findsOneWidget);
    expect(
      find.text('2026-11-12 Practice run — Hades II [v888].mkv'),
      findsOneWidget,
    );

    await t.tap(find.byKey(const ValueKey('twitch-download-1-pause')));
    await t.tap(find.byKey(const ValueKey('twitch-download-2-retry')));
    await settle(t);
    expect(downloads.actions, [
      (1, TwitchDownloadAction.pause),
      (2, TwitchDownloadAction.retry),
    ]);

    // QA-015: «Убрать из списка» goes at once with «Отменить»; undone, the
    // entry is back and the queue keeps it.
    await t.tap(find.byKey(const ValueKey('twitch-download-3-remove')));
    await settle(t, 4);
    expect(
      find.byKey(const ValueKey('twitch-download-3-remove')),
      findsNothing,
    );
    expect(find.text('Removed from the list'), findsOneWidget);
    await t.tap(find.byKey(const ValueKey('snack-undo')));
    await settle(t, 6);
    expect(find.byKey(const ValueKey('twitch-download-3-remove')), findsOne);
    expect(downloads.actions, hasLength(2));
    // Not undone: the queue forgets the job when the message goes.
    await t.tap(find.byKey(const ValueKey('twitch-download-3-remove')));
    await settle(t, 4);
    await t.pump(const Duration(seconds: 9));
    await settle(t, 6);
    expect(downloads.actions.last, (3, TwitchDownloadAction.remove));
    await t.pumpWidget(const SizedBox());
  });

  testWidgets('the entry point is in full builds only', (t) async {
    setSurface(t, const Size(1280, 800));
    final services = await demoServices(twitchDownloads: _FakeDownloads());
    await services.setShowTwitch(true);
    await t.pumpWidget(testApp(services, section: Section.twitch));
    await settle(t);
    final entry = find.byKey(const ValueKey('twitch-downloads'));
    if (!AppFlavor.twitchArchive) {
      expect(entry, findsNothing);
      return;
    }
    expect(entry, findsOneWidget);
    await capture(t, 'twi-017/02-streams-downloads-entry');
    await t.tap(entry);
    await settle(t);
    expect(find.byKey(const ValueKey('twitch-downloads-page')), findsOneWidget);
    await t.pumpWidget(const SizedBox());
  });

  testWidgets('the list on a phone', (t) async {
    setSurface(t, const Size(390, 844), dpr: 2);
    final services = await demoServices(twitchDownloads: _FakeDownloads());
    await t.pumpWidget(
      testApp(services, section: Section.twitch, formFactor: FormFactor.phone),
    );
    await settle(t);
    unawaited(
      t
          .state<NavigatorState>(find.byType(Navigator).first)
          .push(
            MaterialPageRoute<void>(
              builder: (_) =>
                  const TwitchDownloadsScreen(refresh: Duration(hours: 1)),
            ),
          ),
    );
    await settle(t);
    expect(
      find.byKey(const ValueKey('twitch-download-1-pause')),
      findsOneWidget,
    );
    await capture(t, 'twi-017/03-downloads-phone');
    await t.pumpWidget(const SizedBox());
  });

  test('states map from the core', () {
    expect(
      TwitchDownloadState.parse('waiting_resources'),
      TwitchDownloadState.waiting,
    );
    expect(TwitchDownloadState.parse('running').isActive, isTrue);
    expect(
      const TwitchDownloadItem(
        jobId: 1,
        videoId: '1',
        state: TwitchDownloadState.paused,
      ).actions,
      [TwitchDownloadAction.resume, TwitchDownloadAction.cancel],
    );
  });
}
