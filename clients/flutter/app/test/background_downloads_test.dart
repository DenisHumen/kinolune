// SPDX-License-Identifier: MPL-2.0
//
// NET-019 (ADR-NNNN): the binding of the download queue to the Android
// host — counts reported over `app.kinolune/downloads`, pause / resume /
// cancel from the notification, and the Battery Saver hold while no UI is
// visible (kept in preferences across a restart).

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_offline.dart';
import 'package:kinolune/offline/background_downloads.dart';
import 'package:kinolune/offline/offline_downloads.dart';
import 'package:kinolune/services/preferences.dart';

const _channel = MethodChannel(BackgroundDownloads.channelName);

OfflineDownload _d(
  int id,
  OfflineState state, {
  String? waitingFor,
  int done = 0,
}) => OfflineDownload(
  jobId: id,
  itemId: 'm$id',
  title: 'Film $id',
  state: state,
  waitingFor: waitingFor,
  bytesDone: done,
  bytesTotal: 1000,
);

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('snapshot', () {
    test('counts downloads by what keeps them from running', () {
      final s = BackgroundDownloadsSnapshot.of([
        _d(1, OfflineState.running, done: 250),
        _d(2, OfflineState.queued),
        _d(3, OfflineState.paused, waitingFor: 'network'),
        _d(4, OfflineState.paused, waitingFor: 'user'),
        _d(5, OfflineState.paused),
        _d(6, OfflineState.paused, waitingFor: 'resources'),
        _d(7, OfflineState.done),
        _d(8, OfflineState.failed),
      ], wifiOnly: true, held: {5});
      expect(s.active, 6);
      expect(s.runnable, 3);
      expect(s.blockedByNetwork, 1);
      expect(s.paused, 1);
      expect(s.held, 1);
      expect(s.current?.jobId, 1);
      expect(s.toMap(), containsPair('bytesDone', 250));
      expect(s.toMap(), containsPair('bytesTotal', 1000));
    });

    test('shows a waiting download when none runs, then a paused one', () {
      expect(
        BackgroundDownloadsSnapshot.of([
          _d(1, OfflineState.paused),
          _d(2, OfflineState.queued, waitingFor: 'network'),
        ], wifiOnly: true).current?.jobId,
        2,
      );
      expect(
        BackgroundDownloadsSnapshot.of([
          _d(1, OfflineState.paused),
        ], wifiOnly: false).current?.jobId,
        1,
      );
      expect(
        BackgroundDownloadsSnapshot.of(const [], wifiOnly: true).active,
        0,
      );
    });
  });

  group('binding', () {
    late FakeOfflineBackend backend;
    late OfflineController offline;
    late MemoryPreferences prefs;
    late List<Map<Object?, Object?>> updates;
    late bool powerSave;
    late BackgroundDownloads binding;

    Future<void> platformCall(String method, [Object? args]) async {
      final data = const StandardMethodCodec().encodeMethodCall(
        MethodCall(method, args),
      );
      await TestDefaultBinaryMessengerBinding
          .instance
          .defaultBinaryMessenger
          .handlePlatformMessage(BackgroundDownloads.channelName, data, (_) {});
    }

    setUp(() async {
      backend = FakeOfflineBackend(titles: {'a': 'Arrival', 'b': 'Dune'});
      offline = OfflineController(backend);
      prefs = MemoryPreferences();
      updates = [];
      powerSave = false;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(_channel, (call) async {
            switch (call.method) {
              case 'ready':
                return {'powerSave': powerSave};
              case 'update':
                updates.add(call.arguments as Map<Object?, Object?>);
            }
            return null;
          });
      // As a headless engine or an app in the background.
      TestWidgetsFlutterBinding.instance.handleAppLifecycleStateChanged(
        AppLifecycleState.hidden,
      );
      await offline.start();
      await offline.download(const OfflineTarget.item('a'));
      await offline.download(const OfflineTarget.item('b'));
      binding = BackgroundDownloads(
        offline,
        preferences: prefs,
        progressInterval: Duration.zero,
      );
    });

    tearDown(() {
      binding.dispose();
      offline.dispose();
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(_channel, null);
    });

    test('reports the queue and its changes', () async {
      await binding.start();
      expect(updates.last['active'], 2);
      expect(updates.last['runnable'], 2);
      backend.advance(0.5);
      await pumpEventQueue();
      expect(updates.last['title'], 'Dune');
      expect(updates.last['bytesDone'], greaterThan(0));
      backend.advance(0.5);
      backend.advance(1);
      await pumpEventQueue();
      await pumpEventQueue();
      expect(updates.last['active'], 0);
    });

    test('pause, resume and cancel from the notification', () async {
      await binding.start();
      await platformCall('command', {'action': 'pause', 'jobId': null});
      await pumpEventQueue();
      expect(
        offline.downloads.every((d) => d.state == OfflineState.paused),
        isTrue,
      );
      expect(updates.last['paused'], 2);
      expect(updates.last['runnable'], 0);

      await platformCall('command', {'action': 'resume', 'jobId': null});
      await pumpEventQueue();
      expect(
        offline.downloads.every((d) => d.state == OfflineState.queued),
        isTrue,
      );

      final first = offline.downloads.first.jobId!;
      await platformCall('command', {'action': 'cancel', 'jobId': first});
      await pumpEventQueue();
      expect(offline.downloads.map((d) => d.jobId), isNot(contains(first)));
      expect(updates.last['active'], 1);
    });

    test('Battery Saver holds the downloads without visible UI', () async {
      powerSave = true;
      // Headless (no lifecycle yet): not visible.
      await binding.start();
      await pumpEventQueue();
      expect(binding.held, hasLength(2));
      expect(updates.last['held'], 2);
      expect(updates.last['runnable'], 0);
      expect(await prefs.read(Preferences.offlineHeldJobs), isNotNull);

      // The app comes to the front: the held downloads go on.
      binding.didChangeAppLifecycleState(AppLifecycleState.resumed);
      await pumpEventQueue();
      expect(binding.held, isEmpty);
      expect(
        offline.downloads.every((d) => d.state == OfflineState.queued),
        isTrue,
      );
      expect(await prefs.read(Preferences.offlineHeldJobs), isNull);

      // Back to the background, then Battery Saver goes off.
      binding.didChangeAppLifecycleState(AppLifecycleState.hidden);
      await pumpEventQueue();
      expect(binding.held, hasLength(2));
      await platformCall('powerSave', false);
      await pumpEventQueue();
      expect(binding.held, isEmpty);
      expect(updates.last['runnable'], 2);
    });

    test('a user pause is not taken over by the hold', () async {
      final first = offline.downloads.first;
      await offline.control(first, OfflineAction.pause);
      powerSave = true;
      await binding.start();
      await pumpEventQueue();
      expect(binding.held, hasLength(1));
      await platformCall('powerSave', false);
      await pumpEventQueue();
      expect(
        offline.downloads.firstWhere((d) => d.jobId == first.jobId).state,
        OfflineState.paused,
      );
    });

    test('held downloads resume after a restart once allowed', () async {
      powerSave = true;
      await binding.start();
      await pumpEventQueue();
      expect(binding.held, hasLength(2));
      binding.dispose();

      // A new process: Battery Saver is off now.
      powerSave = false;
      final again = BackgroundDownloads(
        offline,
        preferences: prefs,
        progressInterval: Duration.zero,
      );
      await again.start();
      await pumpEventQueue();
      expect(again.held, isEmpty);
      expect(
        offline.downloads.every((d) => d.state == OfflineState.queued),
        isTrue,
      );
      again.dispose();
    });

    test('Resume from the notification overrides the hold', () async {
      powerSave = true;
      await binding.start();
      await pumpEventQueue();
      await platformCall('command', {'action': 'resume', 'jobId': null});
      await pumpEventQueue();
      expect(binding.held, isEmpty);
      binding.didChangeAppLifecycleState(AppLifecycleState.paused);
      await pumpEventQueue();
      expect(binding.held, isEmpty);
      expect(updates.last['runnable'], 2);
    });
  });
}
