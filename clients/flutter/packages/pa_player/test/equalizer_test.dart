// SPDX-License-Identifier: MPL-2.0
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';

void main() {
  group('Equalizer (MUS-015)', () {
    test('off or flat: no filter, still bit-perfect', () {
      expect(const Equalizer().mpvFilter, '');
      expect(const Equalizer(enabled: true).mpvFilter, '');
      expect(const AudioProfile().bitPerfectCapable, isTrue);
      final off = const Equalizer().withPreset(EqualizerPreset.rock);
      expect(off.mpvFilter, '', reason: 'a preset alone does not turn it on');
    });

    test('a preset becomes one equalizer per band and a limiter', () {
      final eq = const Equalizer(
        enabled: true,
      ).withPreset(EqualizerPreset.bassBoost);
      expect(
        eq.mpvFilter,
        'lavfi=['
        'equalizer=f=31:t=o:w=1:g=6.0,'
        'equalizer=f=62:t=o:w=1:g=5.0,'
        'equalizer=f=125:t=o:w=1:g=4.0,'
        'equalizer=f=250:t=o:w=1:g=2.0,'
        'alimiter=limit=0.891:level=0]',
      );
      final p = AudioProfile(equalizer: eq);
      expect(p.mpvProperties['af'], eq.mpvFilter);
      expect(p.bitPerfectCapable, isFalse);
    });

    test('preamp goes first; a cut alone needs no limiter', () {
      final eq = const Equalizer(
        enabled: true,
        preampDb: -3,
      ).withBand(9, -4.5).withBand(0, 30);
      expect(eq.preset, EqualizerPreset.custom);
      expect(
        eq.mpvFilter,
        'lavfi=[volume=-3.0dB,'
        'equalizer=f=31:t=o:w=1:g=12.0,'
        'equalizer=f=16000:t=o:w=1:g=-4.5,'
        'alimiter=limit=0.891:level=0]',
        reason: 'gains clamp to ±12 dB; 12 − 3 still boosts',
      );
      final cut = const Equalizer(enabled: true).withBand(4, -6);
      expect(cut.mpvFilter, 'lavfi=[equalizer=f=500:t=o:w=1:g=-6.0]');
      final quiet = const Equalizer(enabled: true, preampDb: -6).withBand(2, 6);
      expect(quiet.mpvFilter, isNot(contains('alimiter')));
    });

    test('kept as JSON per profile', () {
      final custom = const Equalizer(
        enabled: true,
        preampDb: -2,
      ).withBand(3, 5);
      final back = Equalizer.fromJson(custom.toJson());
      expect(back, custom);
      expect(back.mpvFilter, custom.mpvFilter);
      final preset = const Equalizer(
        enabled: true,
      ).withPreset(EqualizerPreset.vocal);
      expect(preset.toJson().containsKey('gains'), isFalse);
      expect(Equalizer.fromJson(preset.toJson()), preset);
      expect(Equalizer.fromJson('garbage'), const Equalizer());
      expect(
        Equalizer.fromJson({'on': true, 'preset': 'nope'}).preset,
        EqualizerPreset.flat,
      );
    });

    test('every preset has ten bands within range', () {
      for (final p in Equalizer.presets.entries) {
        expect(p.value, hasLength(Equalizer.bands.length), reason: '${p.key}');
        for (final g in p.value) {
          expect(g, inInclusiveRange(Equalizer.minGainDb, Equalizer.maxGainDb));
        }
      }
      expect(
        EqualizerPreset.values.toSet().difference(
          Equalizer.presets.keys.toSet(),
        ),
        {EqualizerPreset.custom},
      );
    });
  });
}
