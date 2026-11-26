// SPDX-License-Identifier: MPL-2.0
//
// TWI-035: «● Записать» on the live screens (full builds only). The button
// shows the ToS warning once and calls `recordNow`; while recording it
// shows the time and size and «Остановить», which stops the recording and
// keeps it; the end of the broadcast gives a notice and a system
// notification «Запись эфира <канал> готова» and the recording with its
// chat in the archive (a full disk says so). FakeTwitchArchive stands in
// for the core; no network, no Twitch. Store builds have no button:
//   flutter test test/twitch_record_live_test.dart --dart-define=KINOLUNE_FLAVOR=full
//   flutter test test/twitch_record_live_test.dart --dart-define=KINOLUNE_FLAVOR=store
// Screenshots with KINOLUNE_SCREENSHOTS_DIR set.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_twitch_archive.dart';
import 'package:kinolune/flavor.dart';
import 'package:kinolune/notices/notice_feeds.dart' show twitchQueuePoll;
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/twitch/twitch_archive_screens.dart';
import 'package:kinolune/screens/twitch/twitch_embed_screen.dart';
import 'package:kinolune/screens/twitch/twitch_record_live.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/twitch/live_notifications.dart';
import 'package:kinolune/twitch/twitch_archive.dart';
import 'package:kinolune/twitch/twitch_downloads.dart';
import 'package:kinolune/twitch/twitch_embed.dart' show TwitchEmbedEngine;
import 'package:kinolune/twitch/twitch_live_recordings.dart';
import 'package:kinolune/twitch/twitch_service.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

const _desktop = Size(1280, 800);
const _phone = Size(390, 844);
const _gib = 1 << 30;

class _Notifier implements LiveNotifier {
  final shown = <(String, String?)>[];

  @override
  Future<bool> init() async => true;

  @override
  Future<void> show(
    LiveAlertInfo alert, {
    required String title,
    String? body,
  }) async => shown.add((title, body));
}

class _Surface implements TwitchEmbedSurface {
  @override
  Widget build(BuildContext context) =>
      const SizedBox.expand(key: ValueKey('fake-embed-webview'));

  @override
  Future<void> runJavaScript(String script) async {}

  @override
  void dispose() {}
}

TwitchEmbedSurface _surface({
  required String html,
  required Uri baseUrl,
  required void Function(String message) onMessage,
  required void Function(Uri url) onExternalLink,
  void Function(bool fullscreen)? onFullscreen,
  VoidCallback? onBack,
}) => _Surface();

/// The app with [page] pushed over it: a black bar like the players'.
Future<void> _open(
  WidgetTester t,
  AppServices services,
  Widget page, {
  FormFactor formFactor = FormFactor.desktop,
}) async {
  await t.pumpWidget(
    testApp(services, section: Section.twitch, formFactor: formFactor),
  );
  await settle(t);
  final nav = t.state<NavigatorState>(find.byType(Navigator).first);
  // Not awaited: the route completes when popped.
  // ignore: unawaited_futures
  nav.push(MaterialPageRoute<void>(builder: (_) => page));
  await settle(t);
}

/// A live screen's top bar with the button, as the players have it.
Widget _bar({bool compact = false}) => Scaffold(
  backgroundColor: Colors.black,
  appBar: AppBar(
    backgroundColor: Colors.black,
    title: const Text('lunaforge'),
    actions: [
      TwitchRecordLiveButton(
        login: 'lunaforge',
        userId: '7002',
        compact: compact,
      ),
    ],
  ),
  body: const SizedBox.expand(),
);

/// Moves the recording on: 1:23:45 in, 2.1 GiB written.
void _progress(FakeTwitchArchive archive, int jobId) {
  archive.update(
    TwitchRecordingItem(
      jobId: jobId,
      userId: '7002',
      login: 'lunaforge',
      state: TwitchDownloadState.running,
      bytesDone: (2.1 * _gib).round(),
      createdAt: DateTime.now().subtract(
        const Duration(hours: 1, minutes: 23, seconds: 45),
      ),
    ),
  );
}

void main() {
  setUpAll(loadAppFonts);

  testWidgets('Record: warning once, recordNow, time and size, Stop', (
    t,
  ) async {
    setSurface(t, _desktop);
    final archive = FakeTwitchArchive();
    final services = await demoServices(twitchArchive: archive);
    await _open(t, services, _bar());
    final record = find.byKey(const ValueKey('twitch-record-live'));
    if (!AppFlavor.twitchArchive) {
      expect(record, findsNothing);
      await t.pumpWidget(const SizedBox());
      return;
    }
    expect(find.text('Record'), findsOneWidget);
    await capture(t, 'twi-035/01-record-desktop');

    // The first press: the ToS warning; cancelled, nothing starts.
    await t.tap(record);
    await settle(t);
    expect(find.byKey(const ValueKey('twitch-record-warning')), findsOne);
    await t.tap(find.text('Cancel'));
    await settle(t);
    expect(archive.recordCalls, isEmpty);

    // Accepted: recordNow for the channel on screen and the active profile.
    await t.tap(record);
    await settle(t);
    await t.tap(find.byKey(const ValueKey('twitch-record-warning-accept')));
    await settle(t);
    expect(archive.recordCalls, [
      (services.archiveProfileId, '7002', 'lunaforge'),
    ]);
    expect(
      find.text(
        'Recording. It goes on after you close the player, as long as '
        'Kinolune is running.',
      ),
      findsOneWidget,
    );
    final jobId = (await archive.recordings()).first.jobId;

    // The indicator: time since the start and the size so far.
    _progress(archive, jobId);
    await t.pump(TwitchLiveRecordings.defaultPoll);
    await settle(t);
    final status = find.byKey(const ValueKey('twitch-record-live-status'));
    expect(status, findsOneWidget);
    final text = t.widget<Text>(status).data!;
    expect(text, startsWith('Recording 1:23:4'));
    expect(text, endsWith(' · 2.1 GB'));
    expect(record, findsNothing);
    await capture(t, 'twi-035/02-recording-desktop');

    // Stop: confirmed, the recording ends and is kept (not cancelled).
    await t.tap(find.byKey(const ValueKey('twitch-record-live-stop')));
    await settle(t);
    expect(
      find.byKey(const ValueKey('twitch-record-live-stop-dialog')),
      findsOne,
    );
    await t.tap(find.byKey(const ValueKey('twitch-record-live-stop-confirm')));
    await settle(t);
    expect(archive.actions, [(jobId, TwitchDownloadAction.stop)]);
    expect((await archive.recordings()).first.end, 'stopped');
    expect(status, findsNothing);
    expect(record, findsOneWidget);

    // Again: no second warning.
    await t.tap(record);
    await settle(t);
    expect(find.byKey(const ValueKey('twitch-record-warning')), findsNothing);
    expect(archive.recordCalls, hasLength(2));
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });

  testWidgets('Record: no space and archive off say so', (t) async {
    setSurface(t, _desktop);
    final archive = FakeTwitchArchive();
    final services = await demoServices(twitchArchive: archive);
    await services.preferences.write(
      Preferences.twitchRecordWarningAccepted,
      'true',
    );
    await _open(t, services, _bar());
    if (!AppFlavor.twitchArchive) {
      await t.pumpWidget(const SizedBox());
      return;
    }
    final record = find.byKey(const ValueKey('twitch-record-live'));
    archive.recordError = const TwitchRecordError('no_space', 'too large');
    await t.tap(record);
    await settle(t);
    expect(find.textContaining('Not enough space for a recording'), findsOne);
    expect(record, findsOneWidget);
    archive.recordError = const TwitchRecordError('archive_disabled', 'off');
    ScaffoldMessenger.of(t.element(record)).clearSnackBars();
    await settle(t, 4);
    await t.tap(record);
    await settle(t);
    expect(
      find.text('Recording Twitch broadcasts is switched off right now.'),
      findsOne,
    );
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });

  testWidgets('Record on a phone: an icon, then the numbers and Stop', (
    t,
  ) async {
    setSurface(t, _phone);
    final archive = FakeTwitchArchive();
    final services = await demoServices(twitchArchive: archive);
    await services.preferences.write(
      Preferences.twitchRecordWarningAccepted,
      'true',
    );
    await _open(t, services, _bar(compact: true), formFactor: FormFactor.phone);
    final record = find.byKey(const ValueKey('twitch-record-live'));
    if (!AppFlavor.twitchArchive) {
      expect(record, findsNothing);
      await t.pumpWidget(const SizedBox());
      return;
    }
    expect(find.text('Record'), findsNothing);
    await capture(t, 'twi-035/03-record-phone');
    await t.tap(record);
    await settle(t);
    final jobId = (await archive.recordings()).first.jobId;
    _progress(archive, jobId);
    await t.pump(TwitchLiveRecordings.defaultPoll);
    await settle(t);
    final text = t
        .widget<Text>(find.byKey(const ValueKey('twitch-record-live-status')))
        .data!;
    expect(text, startsWith('1:23:4'));
    expect(text, endsWith(' · 2.1 GB'));
    expect(find.byKey(const ValueKey('twitch-record-live-stop')), findsOne);
    expect(t.takeException(), isNull);
    await capture(t, 'twi-035/04-recording-phone');
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });

  testWidgets('Record on a TV: the bar with the indicator', (t) async {
    setSurface(t, const Size(1920, 1080));
    final archive = FakeTwitchArchive();
    final services = await demoServices(twitchArchive: archive);
    await services.preferences.write(
      Preferences.twitchRecordWarningAccepted,
      'true',
    );
    await _open(t, services, _bar(), formFactor: FormFactor.tv);
    final record = find.byKey(const ValueKey('twitch-record-live'));
    if (!AppFlavor.twitchArchive) {
      expect(record, findsNothing);
      await t.pumpWidget(const SizedBox());
      return;
    }
    await t.tap(record);
    await settle(t);
    _progress(archive, (await archive.recordings()).first.jobId);
    await t.pump(TwitchLiveRecordings.defaultPoll);
    await settle(t);
    expect(find.byKey(const ValueKey('twitch-record-live-status')), findsOne);
    await capture(t, 'twi-035/05-recording-tv');
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });

  testWidgets('the embed screen of a live channel has Record (full only)', (
    t,
  ) async {
    setSurface(t, _desktop);
    final services = await twitchServices(twitchArchive: FakeTwitchArchive());
    await _open(
      t,
      services,
      const TwitchEmbedScreen(
        target: TwitchPlayTarget.live('pixelheron'),
        surface: _surface,
        engine: TwitchEmbedEngine.webKitGtk,
      ),
    );
    expect(
      find.byKey(const ValueKey('twitch-record-live')),
      AppFlavor.twitchArchive ? findsOneWidget : findsNothing,
    );
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);

    // A VOD: nothing to record.
    await _open(
      t,
      services,
      const TwitchEmbedScreen(
        target: TwitchPlayTarget.vod('9101', login: 'pixelheron'),
        surface: _surface,
        engine: TwitchEmbedEngine.webKitGtk,
      ),
    );
    expect(find.byKey(const ValueKey('twitch-record-live')), findsNothing);
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });

  testWidgets('without the archive (a kid profile, no queue) no Record', (
    t,
  ) async {
    setSurface(t, _desktop);
    final services = await demoServices();
    await _open(t, services, _bar());
    expect(find.byKey(const ValueKey('twitch-record-live')), findsNothing);
    await t.pumpWidget(const SizedBox());
  });

  testWidgets('the end of the broadcast: notice, notification, archive', (
    t,
  ) async {
    setSurface(t, _desktop);
    final notifier = _Notifier();
    final archive = FakeTwitchArchive(
      recordings: [
        TwitchRecordingItem(
          jobId: 7,
          userId: '7002',
          login: 'lunaforge',
          streamId: '90002',
          state: TwitchDownloadState.running,
          media: const Duration(hours: 2),
          bytesDone: 4 * _gib,
          createdAt: DateTime.now().subtract(const Duration(hours: 2)),
        ),
        TwitchRecordingItem(
          jobId: 8,
          userId: '7004',
          login: 'novamarsh',
          state: TwitchDownloadState.running,
          createdAt: DateTime.now(),
        ),
      ],
    );
    final services = await demoServices(
      twitchArchive: archive,
      liveNotifier: notifier,
    );
    await t.pumpWidget(testApp(services, section: Section.twitch));
    await settle(t);
    // The feeds have seen the recordings running.
    await t.pump(twitchQueuePoll);
    await settle(t);
    if (!AppFlavor.twitchArchive) {
      // Store builds never open the queue; nothing to announce.
      await t.pumpWidget(const SizedBox());
      return;
    }
    archive.finish(7, title: 'Neon Drift any%');
    archive.finish(8, end: 'disk_full');
    await t.pump(twitchQueuePoll);
    await settle(t);
    final channel = services.twitch.channelOf('lunaforge').displayName;
    final other = services.twitch.channelOf('novamarsh').displayName;
    expect(notifier.shown, [
      ('Recording of $channel is ready', 'It is in Recordings and downloads.'),
      (
        'Disk full: recording of $other stopped',
        'What was recorded is kept as is, without conversion (a .ts file).',
      ),
    ]);
    final notices = [?services.notices.current, ...services.notices.queued];
    expect(notices.map((n) => n.id), containsAll(['twitch-r7', 'twitch-r8']));

    // The recording with its chat is in the archive.
    final saved = await archive.saved(kind: TwitchSavedKind.recording);
    final file = saved.firstWhere((f) => f.assetId == 1007);
    expect(file.title, 'Neon Drift any%');
    expect(file.hasChat, isTrue);
    final nav = t.state<NavigatorState>(find.byType(Navigator).first);
    // ignore: unawaited_futures
    nav.push(
      MaterialPageRoute<void>(
        builder: (_) => const TwitchArchiveScreen(refresh: Duration(hours: 1)),
      ),
    );
    await settle(t);
    expect(find.byKey(const ValueKey('twitch-saved-1007')), findsOneWidget);
    expect(find.text('Neon Drift any%'), findsOneWidget);
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });
}
