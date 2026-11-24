// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/foundation.dart';

/// A ready-made curve of the [Equalizer] (MUS-015).
enum EqualizerPreset {
  flat,
  bassBoost,
  trebleBoost,
  vocal,
  rock,
  pop,
  jazz,
  classical,
  electronic,
  loudness,

  /// The user moved a band.
  custom,
}

/// 10-band graphic equalizer of the music player (MUS-015), per profile,
/// off by default. Engine-agnostic like [AudioProfile]: [MediaKitEngine]
/// gets it as the libmpv audio filter [mpvFilter] — FFmpeg's `equalizer`
/// (a peaking biquad per band, `af_biquads`, LGPL) and `volume` for the
/// preamp, with `alimiter` (LGPL) so a boost cannot clip.
///
/// Any processing changes the samples: with the equalizer on, the output
/// is no longer bit-perfect (MUS-009).
@immutable
class Equalizer {
  const Equalizer({
    this.enabled = false,
    this.preset = EqualizerPreset.flat,
    this.gainsDb = flatGains,
    this.preampDb = 0,
  });

  /// Centre frequencies of the bands, Hz (ISO octaves).
  static const List<int> bands = [
    31,
    62,
    125,
    250,
    500,
    1000,
    2000,
    4000,
    8000,
    16000,
  ];

  static const double minGainDb = -12;
  static const double maxGainDb = 12;

  static const List<double> flatGains = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

  /// Limiter ceiling, linear: −1 dBFS (as Volume Boost, PLAY-004).
  static const double limiterCeiling = 0.891;

  /// Gains of every preset but [EqualizerPreset.custom], dB per band.
  static const Map<EqualizerPreset, List<double>> presets = {
    EqualizerPreset.flat: flatGains,
    EqualizerPreset.bassBoost: [6, 5, 4, 2, 0, 0, 0, 0, 0, 0],
    EqualizerPreset.trebleBoost: [0, 0, 0, 0, 0, 1, 2, 4, 5, 6],
    EqualizerPreset.vocal: [-2, -2, -1, 1, 3, 4, 3, 1, 0, -1],
    EqualizerPreset.rock: [5, 3, 2, -1, -2, -1, 2, 3, 4, 4],
    EqualizerPreset.pop: [-1, 1, 3, 4, 3, 0, -1, -1, 1, 2],
    EqualizerPreset.jazz: [3, 2, 1, 2, -1, -1, 0, 1, 2, 3],
    EqualizerPreset.classical: [4, 3, 2, 1, -1, -1, 0, 2, 3, 4],
    EqualizerPreset.electronic: [5, 4, 1, 0, -2, 2, 1, 1, 4, 5],
    EqualizerPreset.loudness: [6, 4, 0, 0, -2, 0, -1, -3, 3, 1],
  };

  final bool enabled;
  final EqualizerPreset preset;

  /// Gain of each of [bands], dB ([minGainDb]..[maxGainDb]).
  final List<double> gainsDb;

  /// Gain before the bands, dB (-12..12): lower it when bands boost.
  final double preampDb;

  /// [preset] applied (its gains; [EqualizerPreset.custom] keeps them).
  Equalizer withPreset(EqualizerPreset p) => Equalizer(
    enabled: enabled,
    preset: p,
    gainsDb: presets[p] ?? gainsDb,
    preampDb: preampDb,
  );

  /// Band [index] set to [db]: the preset becomes [EqualizerPreset.custom].
  Equalizer withBand(int index, double db) {
    if (index < 0 || index >= bands.length) return this;
    final g = List<double>.of(_gains);
    g[index] = db.clamp(minGainDb, maxGainDb).toDouble();
    return Equalizer(
      enabled: enabled,
      preset: EqualizerPreset.custom,
      gainsDb: List.unmodifiable(g),
      preampDb: preampDb,
    );
  }

  Equalizer copyWith({bool? enabled, double? preampDb}) => Equalizer(
    enabled: enabled ?? this.enabled,
    preset: preset,
    gainsDb: gainsDb,
    preampDb: preampDb ?? this.preampDb,
  );

  /// [gainsDb] with exactly one value per band, clamped.
  List<double> get _gains => [
    for (var i = 0; i < bands.length; i++)
      i < gainsDb.length
          ? gainsDb[i].clamp(minGainDb, maxGainDb).toDouble()
          : 0.0,
  ];

  double get _preamp => preampDb.clamp(-12, 12).toDouble();

  /// Whether it changes the sound at all.
  bool get active => enabled && (_preamp != 0 || _gains.any((g) => g != 0));

  /// The libmpv audio filter (`af`); empty when off or flat. One
  /// `equalizer` per band that is not 0 dB (an octave wide), the preamp
  /// as `volume` first, and a limiter at the end when anything boosts.
  String get mpvFilter {
    if (!active) return '';
    final g = _gains;
    final parts = <String>[
      if (_preamp != 0) 'volume=${_db(_preamp)}dB',
      for (var i = 0; i < bands.length; i++)
        if (g[i] != 0) 'equalizer=f=${bands[i]}:t=o:w=1:g=${_db(g[i])}',
    ];
    final peak = _preamp + g.fold<double>(0, (m, v) => v > m ? v : m);
    if (peak > 0) parts.add('alimiter=limit=$limiterCeiling:level=0');
    return 'lavfi=[${parts.join(',')}]';
  }

  static String _db(double v) => v.toStringAsFixed(1);

  Map<String, Object?> toJson() => {
    if (enabled) 'on': true,
    'preset': preset.name,
    if (preset == EqualizerPreset.custom) 'gains': _gains,
    if (_preamp != 0) 'preamp': _preamp,
  };

  factory Equalizer.fromJson(Object? json) {
    if (json is! Map) return const Equalizer();
    final preset =
        EqualizerPreset.values.asNameMap()[json['preset']] ??
        EqualizerPreset.flat;
    final raw = json['gains'];
    final gains = preset == EqualizerPreset.custom && raw is List
        ? [
            for (var i = 0; i < bands.length; i++)
              i < raw.length && raw[i] is num
                  ? (raw[i] as num).toDouble().clamp(minGainDb, maxGainDb)
                  : 0.0,
          ]
        : presets[preset] ?? flatGains;
    final pre = json['preamp'];
    return Equalizer(
      enabled: json['on'] == true,
      preset: preset,
      gainsDb: List.unmodifiable(gains),
      preampDb: pre is num ? pre.toDouble().clamp(-12, 12) : 0,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is Equalizer &&
      other.enabled == enabled &&
      other.preset == preset &&
      other._preamp == _preamp &&
      listEquals(other._gains, _gains);

  @override
  int get hashCode =>
      Object.hash(enabled, preset, _preamp, Object.hashAll(_gains));

  @override
  String toString() => 'Equalizer(${enabled ? mpvFilter : 'off'})';
}
