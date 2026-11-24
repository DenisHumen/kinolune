// SPDX-License-Identifier: MPL-2.0
//
// The format line of Now Playing (MUS-005, research 01 §4.11):
// «FLAC · 24/96 · bit-perfect». The numbers are the stream's own (the
// library reads them from the file header, MUS-001; the engine names the
// codec it decodes); «bit-perfect» is shown only when the audio path says
// so — the shared libmpv output never is (MUS-009 brings exclusive output).

import 'package:flutter/foundation.dart';

/// What the stream of a track is: codec, sample rate, bit depth.
@immutable
class AudioFormat {
  const AudioFormat({
    this.codec,
    this.sampleRateHz,
    this.bitDepth,
    this.lossless,
    this.bitrateKbps,
  });

  /// `flac`, `alac`, `mp3`, `aac`, `opus`, `vorbis`, `pcm`, `dsd`…
  final String? codec;
  final int? sampleRateHz;
  final int? bitDepth;

  /// From the tags; null: guessed from [codec].
  final bool? lossless;
  final int? bitrateKbps;

  static const _losslessCodecs = {
    'flac',
    'alac',
    'pcm',
    'wav',
    'aiff',
    'ape',
    'wavpack',
    'wv',
    'tta',
  };

  bool get isLossless =>
      lossless ?? _losslessCodecs.contains(codec?.toLowerCase());

  /// Fills what this one does not know from [other].
  AudioFormat orElse(AudioFormat? other) => other == null
      ? this
      : AudioFormat(
          codec: codec ?? other.codec,
          sampleRateHz: sampleRateHz ?? other.sampleRateHz,
          bitDepth: bitDepth ?? other.bitDepth,
          lossless: lossless ?? other.lossless,
          bitrateKbps: bitrateKbps ?? other.bitrateKbps,
        );

  Map<String, Object?> toJson() => {
    'codec': ?codec,
    'rate': ?sampleRateHz,
    'bits': ?bitDepth,
    'lossless': ?lossless,
    'kbps': ?bitrateKbps,
  };

  static AudioFormat? fromJson(Object? json) {
    if (json is! Map) return null;
    int? n(Object? v) => v is int && v > 0 ? v : null;
    final f = AudioFormat(
      codec: json['codec'] is String ? json['codec'] as String : null,
      sampleRateHz: n(json['rate']),
      bitDepth: n(json['bits']),
      lossless: json['lossless'] is bool ? json['lossless'] as bool : null,
      bitrateKbps: n(json['kbps']),
    );
    return f.codec == null && f.sampleRateHz == null ? null : f;
  }

  @override
  bool operator ==(Object other) =>
      other is AudioFormat &&
      other.codec == codec &&
      other.sampleRateHz == sampleRateHz &&
      other.bitDepth == bitDepth &&
      other.lossless == lossless &&
      other.bitrateKbps == bitrateKbps;

  @override
  int get hashCode =>
      Object.hash(codec, sampleRateHz, bitDepth, lossless, bitrateKbps);

  @override
  String toString() => 'AudioFormat(${audioFormatLine(this) ?? '?'})';
}

/// An engine that knows whether its output is bit-perfect: the samples
/// reach the device unchanged (no resampling, mixing, volume or gain).
/// The libmpv engine does not implement it (shared output); MUS-009 does.
abstract interface class BitPerfectEngine {
  bool get bitPerfect;
}

/// The technical term, the same in every language.
const bitPerfectLabel = 'bit-perfect'; // l10n-ignore

/// «FLAC · 24/96 · bit-perfect», «MP3 · 320 kbps», «DSD64»; null when the
/// codec is unknown. [bitPerfect] adds the last part.
String? audioFormatLine(AudioFormat? f, {bool bitPerfect = false}) {
  final codec = f?.codec?.toLowerCase();
  if (f == null || codec == null || codec.isEmpty) return null;
  final parts = <String>[];
  if (codec == 'dsd') {
    final rate = f.sampleRateHz ?? 0;
    parts.add(rate > 0 ? 'DSD${(rate / 44100).round()}' : 'DSD');
  } else {
    parts.add(switch (codec) {
      'pcm' || 'wav' => 'PCM',
      'wavpack' => 'WV',
      'vorbis' => 'OGG',
      _ => codec.toUpperCase(),
    });
    final rate = f.sampleRateHz;
    if (f.isLossless && rate != null && rate > 0) {
      parts.add(
        f.bitDepth != null && f.bitDepth! > 0
            ? '${f.bitDepth}/${_khz(rate)}'
            : '${_khz(rate)} kHz', // l10n-ignore
      );
    } else if (!f.isLossless && f.bitrateKbps != null) {
      parts.add('${f.bitrateKbps} kbps'); // l10n-ignore
    }
  }
  if (bitPerfect) parts.add(bitPerfectLabel);
  return parts.join(' · ');
}

/// 96000 → `96`, 44100 → `44.1`, 352800 → `352.8`.
String _khz(int hz) {
  final k = hz / 1000;
  return k == k.roundToDouble()
      ? k.round().toString()
      : k.toStringAsFixed(1).replaceFirst(RegExp(r'\.0$'), '');
}

/// The library's codec name for what libmpv calls the decoder:
/// `pcm_s24le` is `pcm`, `dsd_lsbf_planar` is `dsd`; null for none.
String? engineCodec(String? mpvCodec) {
  final c = mpvCodec?.trim().toLowerCase();
  if (c == null || c.isEmpty) return null;
  if (c.startsWith('pcm')) return 'pcm';
  if (c.startsWith('dsd')) return 'dsd';
  return c;
}
