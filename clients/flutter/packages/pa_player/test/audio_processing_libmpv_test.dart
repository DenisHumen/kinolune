// SPDX-License-Identifier: MPL-2.0
//
// PLAY-004: the sound processing on real libmpv, measured with test tones.
// mpv writes what it would play to a WAV file (`ao=pcm`, 32-bit float so
// nothing clips on the way); the test reads the peak and RMS level of each
// output channel:
// - Volume Boost 200 % / 300 % raises a quiet tone by +6.0 / +9.5 dB;
// - with a loud tone the limiter holds every peak at −1 dBFS;
// - "Stereo downmix" turns 5.1 into 2 channels; without normalization the
//   limiter catches the overshoot, with it the downmix stays at the source
//   level;
// - an LFE-only tone is dropped by a stereo downmix unless "LFE in
//   stereo" is on.
//
// Needs libmpv, so it is skipped unless PA_LIBMPV_TESTS=1:
//   PA_LIBMPV_TESTS=1 LIBMPV_LIBRARY_PATH=<dir>/libmpv.so.2 \
//     flutter test test/audio_processing_libmpv_test.dart
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';

final _enabled = Platform.environment['PA_LIBMPV_TESTS'] == '1';
const _rate = 48000;

/// WAVE_FORMAT_EXTENSIBLE with the channel mask of [channels] (2: FL FR;
/// 6: FL FR FC LFE BL BR), 16-bit, one second of a sine at [freq] with
/// [amps] per channel.
void _writeTone(String path, List<double> amps, {double freq = 440}) {
  final ch = amps.length;
  const frames = _rate;
  final data = ByteData(frames * ch * 2);
  for (var i = 0; i < frames; i++) {
    final s = sin(2 * pi * freq * i / _rate);
    for (var c = 0; c < ch; c++) {
      data.setInt16(
        (i * ch + c) * 2,
        (amps[c] * s * 32767).round(),
        Endian.little,
      );
    }
  }
  final fmt = ByteData(40)
    ..setUint16(0, 0xFFFE, Endian.little)
    ..setUint16(2, ch, Endian.little)
    ..setUint32(4, _rate, Endian.little)
    ..setUint32(8, _rate * ch * 2, Endian.little)
    ..setUint16(12, ch * 2, Endian.little)
    ..setUint16(14, 16, Endian.little)
    ..setUint16(16, 22, Endian.little)
    ..setUint16(18, 16, Endian.little)
    ..setUint32(20, ch == 6 ? 0x3F : 0x3, Endian.little);
  // KSDATAFORMAT_SUBTYPE_PCM.
  const guid = [1, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113];
  for (var i = 0; i < 16; i++) {
    fmt.setUint8(24 + i, guid[i]);
  }
  final b = BytesBuilder();
  void tag(String s) => b.add(s.codeUnits);
  void u32(int v) =>
      b.add((ByteData(4)..setUint32(0, v, Endian.little)).buffer.asUint8List());
  tag('RIFF');
  u32(4 + 8 + 40 + 8 + data.lengthInBytes);
  tag('WAVE');
  tag('fmt ');
  u32(40);
  b.add(fmt.buffer.asUint8List());
  tag('data');
  u32(data.lengthInBytes);
  b.add(data.buffer.asUint8List());
  File(path).writeAsBytesSync(b.takeBytes());
}

/// Peak and RMS (dBFS) per channel of a WAV mpv wrote.
class _Levels {
  _Levels(this.peaks, this.rmsDb);

  final List<double> peaks;
  final List<double> rmsDb;

  int get channels => peaks.length;
  double get peak => peaks.reduce(max);

  @override
  String toString() =>
      '$channels ch, peaks ${[for (final p in peaks) p.toStringAsFixed(3)]}, '
      'RMS ${[for (final r in rmsDb) r.toStringAsFixed(1)]} dBFS';

  static _Levels read(String path) {
    final b = File(path).readAsBytesSync();
    final d = ByteData.sublistView(b);
    var at = 12;
    var format = 1, bits = 16, ch = 2;
    while (at + 8 <= b.length) {
      final id = String.fromCharCodes(b.sublist(at, at + 4));
      var size = d.getUint32(at + 4, Endian.little);
      if (id == 'fmt ') {
        format = d.getUint16(at + 8, Endian.little);
        ch = d.getUint16(at + 10, Endian.little);
        bits = d.getUint16(at + 22, Endian.little);
        if (format == 0xFFFE) format = d.getUint16(at + 32, Endian.little);
      } else if (id == 'data') {
        final start = at + 8;
        if (size == 0 || start + size > b.length) size = b.length - start;
        final n = size ~/ (bits ~/ 8);
        final peaks = List.filled(ch, 0.0);
        final sums = List.filled(ch, 0.0);
        // Skip the first 100 ms: the limiter's attack.
        const skip = _rate ~/ 10;
        var count = 0;
        for (var i = skip * ch; i < n; i++) {
          final v = format == 3 && bits == 32
              ? d.getFloat32(start + i * 4, Endian.little)
              : d.getInt16(start + i * 2, Endian.little) / 32768;
          final c = i % ch;
          peaks[c] = max(peaks[c], v.abs());
          sums[c] += v * v;
          if (c == 0) count++;
        }
        return _Levels(peaks, [
          for (final s in sums)
            s == 0 ? -999.0 : 10 * log(s / max(1, count)) / ln10,
        ]);
      }
      at += 8 + size + (size & 1);
    }
    throw StateError('$path: no data chunk');
  }
}

/// Plays [file] through libmpv with [processing] and measures the output.
Future<_Levels> _play(
  String file,
  String out,
  AudioProcessing processing,
) async {
  final engine = MediaKitEngine(
    video: false,
    mpvOptions: {
      'ao': 'pcm',
      'ao-pcm-file': out,
      'ao-pcm-waveheader': 'yes',
      'audio-format': 'float',
      'audio-samplerate': '$_rate',
    },
  );
  try {
    await engine.setAudioProcessing(processing);
    await engine.open([file]);
    final deadline = DateTime.now().add(const Duration(seconds: 60));
    while (!engine.state.value.completed) {
      final error = engine.state.value.error;
      if (error != null) fail('mpv: $error');
      if (DateTime.now().isAfter(deadline)) fail('playback did not finish');
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
  } finally {
    await engine.dispose(); // closes the audio output, finalises the WAV
  }
  return _Levels.read(out);
}

double _db(double ratio) => 20 * log(ratio) / ln10;

void main() {
  late Directory dir;
  late String quiet, loud, surround, lfe;
  var n = 0;
  String out() => '${dir.path}/out${n++}.wav';

  setUpAll(() {
    if (!_enabled) return;
    ensurePaPlayerInitialized();
    dir = Directory.systemTemp.createTempSync('pa_play004_');
    quiet = '${dir.path}/quiet.wav';
    loud = '${dir.path}/loud.wav';
    surround = '${dir.path}/surround.wav';
    lfe = '${dir.path}/lfe.wav';
    _writeTone(quiet, [0.1, 0.1]);
    _writeTone(loud, [0.5, 0.5]);
    // FL FR FC LFE BL BR.
    _writeTone(surround, [0.5, 0.5, 0.5, 0, 0.5, 0.5]);
    _writeTone(lfe, [0, 0, 0, 0.5, 0, 0], freq: 60);
  });

  tearDownAll(() {
    if (_enabled) dir.deleteSync(recursive: true);
  });

  test('Volume Boost: +6.0 dB at 200 %, +9.5 dB at 300 %', () async {
    final base = await _play(quiet, out(), const AudioProcessing());
    final x2 = await _play(quiet, out(), const AudioProcessing(boost: 200));
    final x3 = await _play(quiet, out(), const AudioProcessing(boost: 300));
    // ignore: avoid_print
    print('boost: 100 % $base\n  200 % $x2\n  300 % $x3');
    expect(base.peak, closeTo(0.1, 0.002));
    expect(_db(x2.peak / base.peak), closeTo(6.02, 0.2));
    expect(_db(x3.peak / base.peak), closeTo(9.54, 0.2));
    expect(x3.rmsDb[0] - base.rmsDb[0], closeTo(9.54, 0.2));
  }, skip: !_enabled);

  test('the limiter holds a boosted loud tone at -1 dBFS', () async {
    final base = await _play(loud, out(), const AudioProcessing());
    final x3 = await _play(loud, out(), const AudioProcessing(boost: 300));
    // ignore: avoid_print
    print('limiter: 100 % $base\n  300 % $x3');
    expect(base.peak, closeTo(0.5, 0.002));
    expect(x3.peak, lessThanOrEqualTo(AudioProcessing.limiterCeiling + 0.005));
    expect(x3.rmsDb[0], greaterThan(base.rmsDb[0] + 3), reason: 'louder');
  }, skip: !_enabled);

  test('stereo downmix: two channels, limited or normalized', () async {
    final auto = await _play(surround, out(), const AudioProcessing());
    final plain = await _play(
      surround,
      out(),
      const AudioProcessing(downmix: DownmixMode.stereo),
    );
    final normalized = await _play(
      surround,
      out(),
      const AudioProcessing(
        downmix: DownmixMode.stereo,
        normalizeDownmix: true,
      ),
    );
    // ignore: avoid_print
    print('downmix: auto $auto\n  stereo $plain\n  normalized $normalized');
    expect(auto.channels, 6, reason: 'auto keeps the layout for this ao');
    expect(plain.channels, 2);
    // FL + 0.707 FC + 0.707 BL would be 1.21: the limiter catches it.
    expect(
      plain.peak,
      lessThanOrEqualTo(AudioProcessing.limiterCeiling + 0.005),
    );
    expect(plain.peak, greaterThan(0.8));
    expect(normalized.channels, 2);
    expect(normalized.peak, closeTo(0.5, 0.02));
  }, skip: !_enabled);

  test('track flags come from mpv: 5.1 has six channels', () async {
    final engine = MediaKitEngine(video: false, mpvOptions: {'ao': 'null'});
    try {
      await engine.open([surround], play: false);
      final deadline = DateTime.now().add(const Duration(seconds: 20));
      while (engine.state.value.audioTracks.firstOrNull?.channels != 6) {
        if (DateTime.now().isAfter(deadline)) {
          fail('channels not read: ${engine.state.value.audioTracks}');
        }
        await Future<void>.delayed(const Duration(milliseconds: 50));
      }
      final track = engine.state.value.audioTracks.single;
      expect(track.commentary, isFalse);
      expect(track.audioDescription, isFalse);
    } finally {
      await engine.dispose();
    }
  }, skip: !_enabled);

  test('LFE goes into the stereo downmix only when asked', () async {
    final dropped = await _play(
      lfe,
      out(),
      const AudioProcessing(downmix: DownmixMode.stereo),
    );
    final mixed = await _play(
      lfe,
      out(),
      const AudioProcessing(downmix: DownmixMode.stereo, lfeToStereo: true),
    );
    // ignore: avoid_print
    print('LFE: dropped $dropped\n  mixed $mixed');
    expect(dropped.channels, 2);
    expect(dropped.peak, lessThan(0.001));
    expect(mixed.channels, 2);
    expect(mixed.peak, greaterThan(0.1));
    expect(mixed.peaks[0], closeTo(mixed.peaks[1], 0.01));
  }, skip: !_enabled);
}
