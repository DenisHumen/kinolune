// SPDX-License-Identifier: MPL-2.0
//
// MUS-003: the queue model (Play Next, Add to Queue, reorder, remove,
// shuffle with queued tracks, history, JSON), its engine edits, restore
// after a restart, the video interruption and the Now Playing queue panel.
import 'dart:convert';
import 'dart:math';

import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

List<PlayRequest> _tracks(int n, {String prefix = 'Track', String? album}) => [
  for (var i = 0; i < n; i++)
    PlayRequest(
      uri: '/music/$prefix$i.flac',
      itemId: '$prefix-$i',
      title: '$prefix $i',
      artist: 'Artist',
      album: album ?? 'Album',
    ),
];

List<String> _titles(List<PlayRequest> l) => [for (final r in l) r.title];

void main() {
  group('MusicQueue (MUS-003)', () {
    test('Play Next goes right after the current track', () {
      final q = MusicQueue()
        ..replace(_tracks(4), index: 1, source: const QueueSource('Album'));
      final at = q.playNext(_tracks(2, prefix: 'N'));
      expect(at, 2);
      expect(_titles(q.upNext), ['N 0', 'N 1', 'Track 2', 'Track 3']);
      expect(q.queuedCount, 2);
      expect(q.source?.label, 'Album');
      // A second Play Next goes before the first.
      q.playNext(_tracks(1, prefix: 'M'));
      expect(_titles(q.upNext).take(3), ['M 0', 'N 0', 'N 1']);
      expect(q.queuedCount, 3);
    });

    test('Add to Queue goes after the queued tracks, before the source', () {
      final q = MusicQueue()..replace(_tracks(3));
      q.addToQueue(_tracks(1, prefix: 'A'));
      q.addToQueue(_tracks(1, prefix: 'B'));
      q.playNext(_tracks(1, prefix: 'N'));
      expect(_titles(q.upNext), ['N 0', 'A 0', 'B 0', 'Track 1', 'Track 2']);
      expect(q.queuedCount, 3);
    });

    test('an empty queue becomes the added tracks', () {
      final q = MusicQueue();
      expect(q.addToQueue(_tracks(2)), 0);
      expect(q.current!.title, 'Track 0');
      expect(q.queuedCount, 0);
    });

    test('reorder within and across the queued block', () {
      final q = MusicQueue()..replace(_tracks(4));
      q.addToQueue(_tracks(2, prefix: 'A'));
      // up next: A0 A1 T1 T2 T3
      q.moveUpNext(0, 1);
      expect(_titles(q.upNext), ['A 1', 'A 0', 'Track 1', 'Track 2', 'Track 3']);
      expect(q.queuedCount, 2);
      // A source track dropped among the queued ones becomes queued.
      q.moveUpNext(3, 0);
      expect(_titles(q.upNext).first, 'Track 2');
      expect(q.queuedCount, 3);
      // A queued track dropped below them leaves the block.
      q.moveUpNext(0, 4);
      expect(_titles(q.upNext).last, 'Track 2');
      expect(q.queuedCount, 2);
      // Out of range is ignored.
      final before = q.revision;
      q.moveUpNext(0, 9);
      expect(q.revision, before);
    });

    test('remove and clear keep the current track', () {
      final q = MusicQueue()..replace(_tracks(5), index: 1);
      q.removeUpNext(1);
      expect(_titles(q.upNext), ['Track 2', 'Track 4']);
      q.clearUpNext();
      expect(q.upNext, isEmpty);
      expect(q.current!.title, 'Track 1');
      expect(_titles(q.history), ['Track 0']);
    });

    test('shuffle keeps queued tracks next and puts them back after', () {
      final q = MusicQueue(random: Random(3))..replace(_tracks(8), index: 2);
      q.playNext(_tracks(2, prefix: 'N'));
      q.setShuffle(true);
      expect(q.current!.title, 'Track 2');
      expect(_titles(q.upNext).take(2), ['N 0', 'N 1']);
      expect(q.length, 10);
      q.setShuffle(false);
      expect(q.current!.title, 'Track 2');
      expect(_titles(q.upNext), [
        'N 0',
        'N 1',
        for (var i = 3; i < 8; i++) 'Track $i',
      ]);
      expect(_titles(q.history), ['Track 0', 'Track 1']);
    });

    test('tracks added while shuffled stay when shuffle is off', () {
      final q = MusicQueue(random: Random(5))..replace(_tracks(5));
      q.setShuffle(true);
      q.addToQueue(_tracks(1, prefix: 'A'));
      q.removeUpNext(1);
      q.setShuffle(false);
      expect(q.ordered.where((r) => r.title == 'A 0'), hasLength(1));
      expect(q.length, 5);
    });

    test('recent: most recent first, no duplicates, capped', () {
      final q = MusicQueue();
      final t = _tracks(3);
      q
        ..recordPlayed(t[0])
        ..recordPlayed(t[1])
        ..recordPlayed(t[1])
        ..recordPlayed(t[0]);
      expect(_titles(q.recent), ['Track 0', 'Track 1']);
      for (final r in _tracks(MusicQueue.recentLimit + 5, prefix: 'R')) {
        q.recordPlayed(r);
      }
      expect(q.recent, hasLength(MusicQueue.recentLimit));
    });

    test('JSON round trip keeps order, shuffle, modes and source', () {
      final q = MusicQueue(random: Random(7))
        ..replace(_tracks(6), index: 1, source: const QueueSource('Album'));
      q.playNext(_tracks(1, prefix: 'N'));
      q.setShuffle(true);
      q.setRepeat(QueueRepeat.all);
      q.moveTo(2);
      q.recordPlayed(q.current!);
      final json =
          jsonDecode(jsonEncode(q.toJson())) as Map<String, Object?>;

      final r = MusicQueue();
      expect(r.restore(json), isTrue);
      expect(_titles(r.ordered), _titles(q.ordered));
      expect(r.position, 2);
      expect(r.shuffle, isTrue);
      expect(r.repeat, QueueRepeat.all);
      expect(r.source, const QueueSource('Album'));
      expect(r.queuedCount, q.queuedCount);
      expect(_titles(r.recent), _titles(q.recent));
      expect(r.current!.itemId, q.current!.itemId);
      r.setShuffle(false);
      q.setShuffle(false);
      expect(_titles(r.ordered), _titles(q.ordered));
    });

    test('restore drops tracks the resolver rejects', () {
      final q = MusicQueue()..replace(_tracks(4), index: 2);
      final r = MusicQueue();
      expect(
        r.restore(
          q.toJson(),
          resolve: (t) => t.itemId == 'Track-2' || t.itemId == 'Track-0'
              ? null
              : PlayRequest(uri: '/new${t.uri}', title: t.title),
        ),
        isTrue,
      );
      expect(_titles(r.ordered), ['Track 1', 'Track 3']);
      expect(r.current!.title, 'Track 3', reason: 'continues after it');
      expect(r.current!.uri, '/new/music/Track3.flac');
      expect(r.restore(const {'v': 99}), isFalse);
    });
  });

  group('MusicPlayerController queue (MUS-003)', () {
    late FakePlayerEngine engine;
    late MusicPlayerController c;

    setUp(() {
      engine = FakePlayerEngine(loadedDuration: const Duration(minutes: 3));
      c = MusicPlayerController(engineFactory: ({bool video = true}) => engine);
    });

    test('Play Next and Add to Queue edit the engine playlist in place', () async {
      await c.play(_tracks(3));
      engine.calls.clear();
      await c.playNext(_tracks(1, prefix: 'N'));
      await c.addToQueue(_tracks(2, prefix: 'A'));
      expect(engine.calls.where((e) => e.startsWith('open')), isEmpty);
      expect(engine.calls, ['insert:1', 'insert:2', 'insert:3']);
      expect(engine.uris, [for (final t in c.queue.ordered) t.uri]);
      expect(c.current!.title, 'Track 0');
      c.next();
      expect(c.current!.title, 'N 0');
    });

    test('reorder and remove reach the engine', () async {
      await c.play(_tracks(5), index: 1);
      await c.moveUpNext(0, 2);
      expect(engine.uris, [for (final t in c.queue.ordered) t.uri]);
      await c.removeUpNext(1);
      expect(engine.uris, [for (final t in c.queue.ordered) t.uri]);
      await c.clearUpNext();
      expect(engine.uris, ['/music/Track0.flac', '/music/Track1.flac']);
      expect(c.current!.title, 'Track 1');
    });

    test('history records what played', () async {
      await c.play(_tracks(3));
      engine.emit((s) => s.copyWith(playlistIndex: 1));
      c.next();
      expect(_titles(c.recent), ['Track 2', 'Track 1', 'Track 0']);
      await c.playNow(c.recent.last);
      expect(c.current!.title, 'Track 0');
      expect(_titles(c.queue.ordered).last, 'Track 0');
    });

    test('a restored queue waits paused and resumes where it stopped', () async {
      await c.play(_tracks(4), index: 2, source: const QueueSource('Album'));
      await c.addToQueue(_tracks(1, prefix: 'A'));
      await c.setRepeat(QueueRepeat.all);
      engine.advance(const Duration(seconds: 42));
      final saved = jsonEncode(c.snapshot());

      // "Restart": a new controller with a new engine.
      final engine2 = FakePlayerEngine();
      final c2 = MusicPlayerController(
        engineFactory: ({bool video = true}) => engine2,
      );
      expect(
        await c2.restore(jsonDecode(saved) as Map<String, Object?>),
        isTrue,
      );
      expect(c2.current!.title, 'Track 2');
      expect(c2.playing, isFalse);
      expect(c2.position, const Duration(seconds: 42));
      expect(c2.source?.label, 'Album');
      expect(c2.queue.queuedCount, 1);
      expect(c2.repeat, QueueRepeat.all);
      expect(engine2.calls.where((e) => e.startsWith('open')), isEmpty);

      c2.resume();
      await Future<void>.delayed(Duration.zero);
      expect(engine2.calls, contains('open:5:2:42000:true'));
      expect(c2.playing, isTrue);
    });

    test('next on a restored queue opens it at that track', () async {
      await c.play(_tracks(3));
      final c2 = MusicPlayerController(
        engineFactory: ({bool video = true}) => engine,
      );
      await c2.restore(c.snapshot());
      engine.calls.clear();
      c2.next();
      await Future<void>.delayed(Duration.zero);
      expect(engine.calls, contains('open:3:1:0:true'));
    });

    test('a video pauses the music and offers to resume it', () async {
      await c.play(_tracks(2));
      expect(c.pauseForVideo(), isTrue);
      expect(c.playing, isFalse);
      expect(c.interruptedByVideo, isTrue);
      c.resumeAfterVideo();
      expect(c.playing, isTrue);
      expect(c.interruptedByVideo, isFalse);

      c.pause();
      expect(c.pauseForVideo(), isFalse, reason: 'it was not playing');
      expect(c.interruptedByVideo, isFalse);
    });

    test('stop empties the queue (another profile)', () async {
      await c.play(_tracks(2));
      await c.stop();
      expect(c.hasQueue, isFalse);
      expect(c.recent, isEmpty);
      expect(engine.calls.last, 'pause');
    });
  });

  group('Now Playing queue panel (MUS-003)', () {
    late FakePlayerEngine engine;
    late MusicPlayerController c;

    setUp(() {
      engine = FakePlayerEngine();
      c = MusicPlayerController(engineFactory: ({bool video = true}) => engine);
    });

    Future<void> pump(WidgetTester t, {SaveQueueCallback? onSave}) async {
      t.view.physicalSize = const Size(1600, 1000);
      t.view.devicePixelRatio = 1;
      addTearDown(t.view.reset);
      await t.pumpWidget(
        MaterialApp(
          home: NowPlayingScreen(controller: c, onSaveQueue: onSave),
        ),
      );
      await t.pump(const Duration(milliseconds: 500));
    }

    testWidgets('sections: playing next and up next from the source', (
      t,
    ) async {
      await c.play(_tracks(4), source: const QueueSource('Abbey Road'));
      await c.playNext(_tracks(1, prefix: 'N'));
      await pump(t);
      expect(find.text('Playing next'), findsOneWidget);
      expect(find.text('Up next from: Abbey Road'), findsOneWidget);
      expect(find.text('N 0'), findsOneWidget);
    });

    testWidgets('remove, clear and save as playlist', (t) async {
      List<PlayRequest>? saved;
      await c.play(_tracks(4));
      await pump(
        t,
        onSave: (context, tracks) async => saved = tracks,
      );
      await t.tap(find.byKey(const ValueKey('queue-remove-2')));
      await t.pump();
      expect(_titles(c.queue.upNext), ['Track 1', 'Track 3']);

      await t.tap(find.byKey(const ValueKey('np-save')));
      await t.pump();
      expect(_titles(saved!), ['Track 0', 'Track 1', 'Track 3']);

      await t.tap(find.byKey(const ValueKey('np-clear')));
      await t.pump();
      expect(c.queue.upNext, isEmpty);
      expect(find.text('End of queue'), findsOneWidget);
      await t.pump(const Duration(milliseconds: 500));
    });

    testWidgets('swipe removes a track', (t) async {
      await c.play(_tracks(3));
      await pump(t);
      await t.drag(find.byKey(const ValueKey('queue-1')), const Offset(-600, 0));
      await t.pumpAndSettle();
      expect(_titles(c.queue.upNext), ['Track 2']);
    });

    testWidgets('drag to reorder', (t) async {
      await c.play(_tracks(5));
      await pump(t);
      final from = t.getCenter(find.byKey(const ValueKey('queue-drag-1')));
      final row = t.getSize(find.byKey(const ValueKey('queue-1'))).height;
      final g = await t.startGesture(from);
      await t.pump(const Duration(milliseconds: 50));
      for (var i = 0; i < 10; i++) {
        await g.moveBy(Offset(0, row * 2.5 / 10));
        await t.pump(const Duration(milliseconds: 16));
      }
      await g.up();
      await t.pumpAndSettle();
      expect(_titles(c.queue.upNext), ['Track 2', 'Track 3', 'Track 1', 'Track 4']);
      expect(engine.uris, [for (final r in c.queue.ordered) r.uri]);
    });

    testWidgets('history tab plays a track again', (t) async {
      await c.play(_tracks(3));
      c.next();
      await pump(t);
      await t.tap(find.byKey(const ValueKey('np-tab-history')));
      await t.pump();
      expect(find.byKey(const ValueKey('history-1')), findsOneWidget);
      await t.tap(find.byKey(const ValueKey('history-1')));
      await t.pump();
      expect(c.current!.title, 'Track 0');
      await t.pump(const Duration(milliseconds: 500));
    });

    testWidgets('Russian strings', (t) async {
      await c.play(_tracks(3), source: const QueueSource('Альбом'));
      t.view.physicalSize = const Size(400, 900);
      t.view.devicePixelRatio = 1;
      addTearDown(t.view.reset);
      await t.pumpWidget(
        MaterialApp(
          locale: const Locale('ru'),
          supportedLocales: PaPlayerStrings.supportedLocales,
          localizationsDelegates: const [
            PaPlayerStrings.delegate,
            ...GlobalMaterialLocalizations.delegates,
          ],
          home: NowPlayingScreen(controller: c),
        ),
      );
      await t.pump(const Duration(milliseconds: 500));
      expect(find.text('Далее из: Альбом'), findsOneWidget);
      expect(find.text('История'), findsOneWidget);
      expect(t.takeException(), isNull);
    });
  });
}
