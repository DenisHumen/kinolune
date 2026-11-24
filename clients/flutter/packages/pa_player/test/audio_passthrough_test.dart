// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  const channel = MethodChannel('test/audio_passthrough');
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
  late List<MethodCall> calls;
  String? answer;

  setUp(() {
    calls = [];
    // A Shield with an Atmos receiver.
    answer = 'ac3,eac3,dts,dts-hd,truehd';
    messenger.setMockMethodCallHandler(channel, (call) async {
      calls.add(call);
      return answer;
    });
  });
  tearDown(() => messenger.setMockMethodCallHandler(channel, null));

  AudioPassthrough passthrough(AudioOutputMode mode, {bool enabled = true}) =>
      AudioPassthrough(mode: mode, channel: channel, enabled: enabled);

  FakePlayerEngine engineOf(AudioPassthrough p, {bool video = true}) =>
      p.wrap(({bool video = true}) => FakePlayerEngine())(video: video)
          as FakePlayerEngine;

  test('mode names: anything unknown is auto', () {
    expect(AudioOutputMode.fromName('decode'), AudioOutputMode.decode);
    expect(AudioOutputMode.fromName('auto'), AudioOutputMode.auto);
    expect(AudioOutputMode.fromName(null), AudioOutputMode.auto);
    expect(AudioOutputMode.fromName('passthrough'), AudioOutputMode.auto);
  });

  test(
    'auto asks Android for the codecs and gives them to video engines',
    () async {
      final p = passthrough(AudioOutputMode.auto);
      expect(await p.prepare(), 'ac3,eac3,dts,dts-hd,truehd');
      expect(calls.single.method, 'spdif');
      expect(calls.single.arguments, {'mode': 'auto'});
      final engine = engineOf(p);
      await Future<void>.delayed(Duration.zero);
      expect(engine.audioPassthrough, 'ac3,eac3,dts,dts-hd,truehd');
    },
  );

  test('a device without passthrough decodes everything', () async {
    answer = '';
    final p = passthrough(AudioOutputMode.auto);
    expect(await p.prepare(), '');
    final engine = engineOf(p);
    await Future<void>.delayed(Duration.zero);
    expect(engine.audioPassthrough, '');
  });

  test('«всегда декодировать» never asks and never passes through', () async {
    final p = passthrough(AudioOutputMode.decode);
    expect(await p.prepare(), '');
    expect(calls, isEmpty);
    final engine = engineOf(p);
    await Future<void>.delayed(Duration.zero);
    expect(engine.audioPassthrough, '');
  });

  test('music engines and other hosts are left alone', () async {
    final p = passthrough(AudioOutputMode.auto);
    await p.prepare();
    final music = engineOf(p, video: false);
    await Future<void>.delayed(Duration.zero);
    expect(music.audioPassthrough, isNull);

    final off = passthrough(AudioOutputMode.auto, enabled: false);
    expect(await off.prepare(), '');
    final engine = engineOf(off);
    await Future<void>.delayed(Duration.zero);
    expect(engine.audioPassthrough, isNull);
  });

  test('a failing or missing Android side falls back to decoding', () async {
    messenger.setMockMethodCallHandler(channel, (call) async {
      throw PlatformException(code: 'audio');
    });
    expect(await passthrough(AudioOutputMode.auto).prepare(), '');
    messenger.setMockMethodCallHandler(channel, null);
    expect(await passthrough(AudioOutputMode.auto).prepare(), '');
  });
}
