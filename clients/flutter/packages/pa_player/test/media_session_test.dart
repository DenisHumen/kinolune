// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

/// A [SystemMediaSession] / [SystemPip] pair on mocked channels.
class _System {
  _System() {
    final m = TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    m.setMockMethodCallHandler(const MethodChannel('test/session'), (c) async {
      session.add(c);
      return null;
    });
    m.setMockMethodCallHandler(const MethodChannel('test/pip'), (c) async {
      pipCalls.add(c);
      return switch (c.method) {
        'isSupported' => true,
        'enter' => true,
        _ => null,
      };
    });
  }

  final session = <MethodCall>[];
  final pipCalls = <MethodCall>[];
  Duration now = Duration.zero;

  late final media = SystemMediaSession(
    channel: const MethodChannel('test/session'),
    enabled: true,
    clock: () => now,
  );
  late final pip = SystemPip(
    channel: const MethodChannel('test/pip'),
    enabled: true,
  );

  List<Map<Object?, Object?>> get updates => [
    for (final c in session)
      if (c.method == 'update') c.arguments as Map<Object?, Object?>,
  ];

  Map<Object?, Object?> get lastUpdate => updates.last;
}

MediaSessionInfo _info(
  String title, {
  bool playing = true,
  Duration position = Duration.zero,
}) => MediaSessionInfo(
  kind: MediaSessionKind.music,
  title: title,
  duration: const Duration(minutes: 3),
  position: position,
  playing: playing,
);

List<PlayRequest> _tracks(int n) => [
  for (var i = 0; i < n; i++)
    PlayRequest(
      uri: '/music/$i.flac',
      title: 'Track $i',
      artist: 'Artist',
      album: 'Album',
      artworkUri: '/music/cover.jpg',
      itemId: 't$i',
    ),
];

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('SystemMediaSession', () {
    test('the newest claim is shown; releasing it brings the older back', () {
      final s = _System();
      final music = s.media.claim((_, _) {});
      music.update(_info('Song'));
      expect(s.lastUpdate['title'], 'Song');
      final video = s.media.claim((_, _) {});
      video.update(_info('Movie'));
      expect(s.lastUpdate['title'], 'Movie');
      // Updates of a covered claim are kept, not shown.
      music.update(_info('Song 2'));
      expect(s.lastUpdate['title'], 'Movie');
      video.release();
      expect(s.lastUpdate['title'], 'Song 2');
      music.release();
      expect(s.session.last.method, 'clear');
    });

    test('position ticks are sent only when the system would drift', () {
      final s = _System();
      final c = s.media.claim((_, _) {});
      c.update(_info('Song'));
      s.now = const Duration(seconds: 1);
      c.update(_info('Song', position: const Duration(seconds: 1)));
      expect(s.updates, hasLength(1), reason: 'extrapolated by the system');
      s.now = const Duration(seconds: 2);
      c.update(_info('Song', position: const Duration(seconds: 30))); // seek
      expect(s.updates, hasLength(2));
      expect(s.lastUpdate['positionMs'], 30000);
      c.update(_info('Song', playing: false, position: const Duration(seconds: 30)));
      expect(s.updates, hasLength(3));
      expect(s.lastUpdate['playing'], false);
    });

    test('live streams are never seekable', () {
      const info = MediaSessionInfo(
        kind: MediaSessionKind.live,
        title: 'Stream',
        duration: Duration(hours: 1),
      );
      expect(info.toMap()['seekable'], false);
    });

    test('commands from the platform go to the shown claim', () async {
      final s = _System();
      final got = <String>[];
      s.media.claim((c, p) => got.add('a:${c.name}')).update(_info('A'));
      s.media
          .claim((c, p) => got.add('b:${c.name}:${p?.inSeconds}'))
          .update(_info('B'));
      final codec = const StandardMethodCodec();
      await TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .handlePlatformMessage(
            'test/session',
            codec.encodeMethodCall(
              const MethodCall('command', {
                'action': 'seek',
                'positionMs': 42000,
              }),
            ),
            (_) {},
          );
      expect(got, ['b:seek:42']);
    });
  });

  group('music in the session', () {
    test('appears once it plays, follows the queue, goes on stop', () async {
      final s = _System();
      final engine = FakePlayerEngine(
        loadedDuration: const Duration(minutes: 3),
      );
      final music = MusicPlayerController(
        engineFactory: ({bool video = true}) => engine,
        mediaSession: s.media,
      );
      await music.play(_tracks(3), index: 1);
      final u = s.lastUpdate;
      expect(u['title'], 'Track 1');
      expect(u['subtitle'], 'Artist');
      expect(u['artworkUri'], '/music/cover.jpg');
      expect(u['hasNext'], true);
      expect(u['hasPrevious'], true);
      expect(u['playing'], true);

      s.media.dispatch(MediaSessionCommand.pause);
      expect(engine.calls.last, 'pause');
      s.media.dispatch(MediaSessionCommand.next);
      await Future<void>.delayed(Duration.zero);
      expect(engine.calls, contains('jump:2'));
      s.media.dispatch(MediaSessionCommand.duck);
      await Future<void>.delayed(Duration.zero);
      expect(engine.calls.last, 'volume:20.0');
      s.media.dispatch(MediaSessionCommand.unduck);
      await Future<void>.delayed(Duration.zero);
      expect(engine.calls.last, 'volume:100.0');

      await music.stop();
      expect(s.session.last.method, 'clear');
      music.dispose();
    });

    test('a restored queue stays out until it plays', () async {
      final s = _System();
      final music = MusicPlayerController(
        engineFactory: ({bool video = true}) => FakePlayerEngine(),
        mediaSession: s.media,
      );
      final other = MusicPlayerController(
        engineFactory: ({bool video = true}) => FakePlayerEngine(),
        mediaSession: SystemMediaSession(enabled: false),
      );
      await other.play(_tracks(2));
      await music.restore(other.snapshot());
      expect(s.updates, isEmpty);
      music.resume();
      await Future<void>.delayed(Duration.zero);
      expect(s.updates, isNotEmpty);
      music.dispose();
      other.dispose();
    });
  });

  group('video player', () {
    Future<(_System, FakePlayerEngine)> open(
      WidgetTester t, {
      Widget Function(BuildContext)? liveBar,
    }) async {
      final s = _System();
      final engine = FakePlayerEngine(
        loadedDuration: const Duration(minutes: 30),
      );
      await t.pumpWidget(
        MaterialApp(
          home: PlayerScreen(
            request: const PlayRequest(
              uri: '/media/movie.mkv',
              title: 'Big Buck Bunny',
              artworkUri: '/media/poster.jpg',
            ),
            onProgress: (_, _) {},
            engineFactory: ({bool video = true}) => engine,
            mediaSession: s.media,
            pip: s.pip,
            liveBar: liveBar,
          ),
        ),
      );
      await t.pump();
      engine.emit((v) => v.copyWith(videoWidth: 1920, videoHeight: 800));
      await t.pump();
      return (s, engine);
    }

    testWidgets('claims the session and enables auto-PiP while playing', (
      t,
    ) async {
      final (s, engine) = await open(t);
      expect(s.lastUpdate['kind'], 'video');
      expect(s.lastUpdate['title'], 'Big Buck Bunny');
      expect(s.lastUpdate['seekable'], true);
      final configure = s.pipCalls.lastWhere((c) => c.method == 'configure');
      expect(configure.arguments, {
        'autoEnter': true,
        'width': 1920,
        'height': 800,
      });
      engine.emit((v) => v.copyWith(playing: false));
      await t.pump();
      expect(
        s.pipCalls.lastWhere((c) => c.method == 'configure').arguments,
        containsPair('autoEnter', false),
      );

      s.media.dispatch(MediaSessionCommand.play);
      expect(engine.calls.last, 'play');
      s.media.dispatch(MediaSessionCommand.seek, const Duration(minutes: 2));
      expect(engine.calls.last, 'seek:120000');

      await t.pumpWidget(const SizedBox());
      expect(s.session.last.method, 'clear');
      expect(
        s.pipCalls.lastWhere((c) => c.method == 'configure').arguments,
        containsPair('autoEnter', false),
      );
    });

    testWidgets('PiP shows only the picture; closing it pauses', (t) async {
      final (s, engine) = await open(t);
      // The OSD button (PiP is supported on this "device").
      expect(find.byKey(const ValueKey('osd-pip')), findsOneWidget);
      await t.tap(find.byKey(const ValueKey('osd-pip')));
      await t.pump();
      expect(s.pipCalls.last.method, 'enter');

      s.pip.onChanged(inPip: true);
      await t.pump();
      expect(find.byKey(const ValueKey('osd')), findsNothing);
      expect(find.byKey(const ValueKey('osd-video')), findsOneWidget);

      s.pip.onChanged(inPip: false);
      await t.pump();
      expect(find.byKey(const ValueKey('osd')), findsOneWidget);
      expect(engine.calls.last, isNot('pause'));

      s.pip.onChanged(inPip: true);
      s.pip.onChanged(inPip: false, dismissed: true);
      await t.pump();
      expect(engine.calls.last, 'pause');
      await t.pumpWidget(const SizedBox());
    });

    testWidgets('a live stream is not seekable in the session', (t) async {
      final (s, engine) = await open(t, liveBar: (_) => const SizedBox());
      expect(s.lastUpdate['kind'], 'live');
      expect(s.lastUpdate['seekable'], false);
      s.media.dispatch(MediaSessionCommand.duck);
      expect(engine.calls.last, 'volume:20.0');
      s.media.dispatch(MediaSessionCommand.unduck);
      expect(engine.calls.last, 'volume:100.0');
      await t.pumpWidget(const SizedBox());
    });
  });
}
