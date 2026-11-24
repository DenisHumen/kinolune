// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  const channel = MethodChannel('test/display');
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
  late List<MethodCall> calls;
  String? answer;

  setUp(() {
    calls = [];
    answer = '3840x2160@23.976';
    messenger.setMockMethodCallHandler(channel, (call) async {
      calls.add(call);
      return call.method == 'match' ? answer : null;
    });
  });
  tearDown(() => messenger.setMockMethodCallHandler(channel, null));

  DisplayMatcher matcher([
    DisplayMatchSettings settings = const DisplayMatchSettings(),
  ]) => DisplayMatcher(settings: settings, channel: channel, enabled: true);

  FakePlayerEngine engineOf(DisplayMatcher m, {bool video = true}) =>
      m.wrap(({bool video = true}) => FakePlayerEngine())(video: video)
          as FakePlayerEngine;

  void playVideo(
    FakePlayerEngine e,
    double fps, {
    int w = 3840,
    int h = 2160,
  }) => e.emit(
    (s) => s.copyWith(
      videoTrack: MediaTrack(id: '1', width: w, height: h, fps: fps),
      videoWidth: w,
      videoHeight: h,
    ),
  );

  group('DisplayMatchSettings', () {
    test('defaults: frame rate on, resolution kept, Dolby Vision native', () {
      const s = DisplayMatchSettings();
      expect(s.frameRate, FrameRateMatch.auto);
      expect(s.resolution, isFalse);
      expect(s.dynamicRange, isTrue);
      expect(DisplayMatchSettings.fromJson(null), s);
      expect(DisplayMatchSettings.fromJson(const {}), s);
    });

    test('round-trips through the map the Android side reads', () {
      const s = DisplayMatchSettings(
        frameRate: FrameRateMatch.off,
        resolution: true,
        dynamicRange: false,
      );
      expect(s.toJson(), {
        'frame_rate': 'off',
        'resolution': true,
        'dynamic_range': false,
      });
      expect(DisplayMatchSettings.fromJson(s.toJson()), s);
      expect(
        s.copyWith(frameRate: FrameRateMatch.auto).frameRate,
        FrameRateMatch.auto,
      );
    });
  });

  group('DisplayMatcher', () {
    test('switches once the frame rate is known, then restores', () async {
      final m = matcher();
      final e = engineOf(m);
      e.emit((s) => s.copyWith(position: const Duration(seconds: 1)));
      await pumpEventQueue();
      expect(calls, isEmpty, reason: 'no video track yet');

      playVideo(e, 23.976);
      await pumpEventQueue();
      expect(calls.single.method, 'match');
      expect(calls.single.arguments, {
        'fps': 23.976,
        'width': 3840,
        'height': 2160,
        'options': const DisplayMatchSettings().toJson(),
      });
      expect(m.mode.value, '3840x2160@23.976');

      // Position ticks of the same video do not switch again.
      e.emit((s) => s.copyWith(position: const Duration(seconds: 2)));
      await pumpEventQueue();
      expect(calls, hasLength(1));

      // The next episode at 25 fps switches again.
      playVideo(e, 25, w: 1920, h: 1080);
      await pumpEventQueue();
      expect(calls.last.method, 'match');
      expect((calls.last.arguments as Map)['fps'], 25.0);
      expect((calls.last.arguments as Map)['width'], 1920);

      await m.restore();
      expect(calls.last.method, 'restore');
      expect(m.mode.value, isNull);
    });

    test('nothing to restore when the display was not touched', () async {
      answer = null;
      final m = matcher();
      playVideo(engineOf(m), 60);
      await pumpEventQueue();
      expect(calls.single.method, 'match');
      await m.restore();
      expect(calls, hasLength(1));
    });

    test('«Частота кадров: Выкл» never touches the display', () async {
      final m = matcher(
        const DisplayMatchSettings(frameRate: FrameRateMatch.off),
      );
      expect(m.enabled, isFalse);
      playVideo(engineOf(m), 24);
      await pumpEventQueue();
      await m.restore();
      expect(calls, isEmpty);
    });

    test('audio engines are left alone', () async {
      final m = matcher();
      playVideo(engineOf(m, video: false), 24);
      await pumpEventQueue();
      expect(calls, isEmpty);
    });

    test('refresh rate of a mode', () {
      expect(DisplayMatcher.refreshRateOf('3840x2160@23.976'), 23.976);
      expect(DisplayMatcher.refreshRateOf('1920x1080@50.000'), 50);
      expect(DisplayMatcher.refreshRateOf('nonsense'), isNull);
    });
  });
}
