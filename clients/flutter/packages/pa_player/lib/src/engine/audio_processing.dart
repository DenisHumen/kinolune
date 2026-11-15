// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/foundation.dart';

/// How many channels the video's sound is played with.
enum DownmixMode {
  /// As the output device takes it: mpv downmixes only for a device with
  /// fewer channels than the file.
  auto,

  /// Always stereo (headphones, TV speakers): the downmix is done before
  /// the limiter, so Volume Boost cannot clip it.
  stereo,
}

/// Sound processing of the video player (PLAY-004, design 01 §1.4
/// "Audio"), per profile. Engine-agnostic like [AudioProfile]:
/// [MediaKitEngine] maps it to mpv options and LGPL FFmpeg filters with
/// [mpvProperties].
///
/// Measured on our libmpv (mpv 0.42, FFmpeg 9.0) with test tones, see
/// `test/audio_processing_libmpv_test.dart`:
/// - a 5.1 tone at 0.5 on every channel but LFE peaks at 1.21 in a plain
///   stereo downmix and at 0.5 with [normalizeDownmix];
/// - an LFE-only tone is silent in stereo by default and comes out at
///   −6 dB per channel with [lfeToStereo];
/// - Volume Boost 200 % / 300 % adds +6.0 / +9.5 dB, and the limiter keeps
///   every peak at [limiterCeiling] (−1 dBFS).
@immutable
class AudioProcessing {
  const AudioProcessing({
    this.downmix = DownmixMode.auto,
    this.normalizeDownmix = false,
    this.lfeToStereo = false,
    this.boost = 100,
  });

  final DownmixMode downmix;

  /// Scales the downmix so it cannot clip (mpv `audio-normalize-downmix`):
  /// quieter, but without distortion. Off by default, as in mpv.
  final bool normalizeDownmix;

  /// Mixes the subwoofer channel (LFE) into a stereo downmix at
  /// [lfeLevel]; by default a downmix drops it.
  final bool lfeToStereo;

  /// Volume Boost, percent: [minBoost]..[maxBoost]. Above 100 a limiter
  /// keeps the peaks at [limiterCeiling].
  final int boost;

  static const int minBoost = 100;
  static const int maxBoost = 300;

  /// Values of the Settings choice and the OSD buttons.
  static const List<int> boostSteps = [100, 125, 150, 200, 250, 300];

  /// Limiter ceiling, linear: −1 dBFS.
  static const double limiterCeiling = 0.891;

  /// Level of the LFE channel in a stereo downmix (libswresample
  /// `lfe_mix_level`), linear: −3 dB.
  static const double lfeLevel = 0.707;

  /// [boost] within [minBoost]..[maxBoost].
  int get clampedBoost => boost.clamp(minBoost, maxBoost);

  /// The next step of [boostSteps] up (`up`) or down from [boost].
  int stepBoost({required bool up}) {
    final b = clampedBoost;
    if (up) {
      return boostSteps.firstWhere((s) => s > b, orElse: () => maxBoost);
    }
    return boostSteps.lastWhere((s) => s < b, orElse: () => minBoost);
  }

  /// A limiter runs when a gain or a downmix could clip.
  bool get limits =>
      clampedBoost > 100 ||
      (downmix == DownmixMode.stereo && !normalizeDownmix);

  /// The mpv audio filter chain (`af`), empty for none. The stereo
  /// conversion is mpv's own `format` filter (it uses libswresample with
  /// [normalizeDownmix] and [lfeToStereo]); gain and limiter are FFmpeg's
  /// `volume` and `alimiter` (LGPL). `level=0` turns off the limiter's
  /// auto-gain, which would make quiet sound louder.
  String get mpvFilter {
    final parts = <String>[
      if (downmix == DownmixMode.stereo) 'format=channels=stereo',
      if (limits)
        'lavfi=[${clampedBoost > 100 ? 'volume=${_gain()},' : ''}'
            'alimiter=limit=$limiterCeiling:level=0]',
    ];
    return parts.join(',');
  }

  String _gain() => (clampedBoost / 100).toStringAsFixed(2);

  /// mpv properties; all of them can change while a file plays.
  Map<String, String> get mpvProperties => {
    'audio-normalize-downmix': normalizeDownmix ? 'yes' : 'no',
    'audio-swresample-o': lfeToStereo ? 'lfe_mix_level=$lfeLevel' : '',
    'af': mpvFilter,
  };

  AudioProcessing copyWith({
    DownmixMode? downmix,
    bool? normalizeDownmix,
    bool? lfeToStereo,
    int? boost,
  }) => AudioProcessing(
    downmix: downmix ?? this.downmix,
    normalizeDownmix: normalizeDownmix ?? this.normalizeDownmix,
    lfeToStereo: lfeToStereo ?? this.lfeToStereo,
    boost: boost ?? this.boost,
  );

  Map<String, Object?> toJson() => {
    'downmix': downmix.name,
    if (normalizeDownmix) 'normalize': true,
    if (lfeToStereo) 'lfe': true,
    if (clampedBoost != 100) 'boost': clampedBoost,
  };

  factory AudioProcessing.fromJson(Object? json) {
    if (json is! Map) return const AudioProcessing();
    final boost = json['boost'];
    return AudioProcessing(
      downmix:
          DownmixMode.values.asNameMap()[json['downmix']] ?? DownmixMode.auto,
      normalizeDownmix: json['normalize'] == true,
      lfeToStereo: json['lfe'] == true,
      boost: boost is int ? boost.clamp(minBoost, maxBoost) : 100,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is AudioProcessing &&
      other.downmix == downmix &&
      other.normalizeDownmix == normalizeDownmix &&
      other.lfeToStereo == lfeToStereo &&
      other.clampedBoost == clampedBoost;

  @override
  int get hashCode =>
      Object.hash(downmix, normalizeDownmix, lfeToStereo, clampedBoost);

  @override
  String toString() => 'AudioProcessing($mpvProperties)';
}
