// SPDX-License-Identifier: MPL-2.0
//
// PLAY-004: the profile's track policy and sound processing as data — the
// languages each mode looks for, JSON kept across versions (PLAY-007
// records still read), track flags from titles, and the mpv options and
// FFmpeg filters of the sound processing.
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';

void main() {
  group('languages to look for', () {
    test('auto: the interface, then the original', () {
      expect(TrackPolicy().audioLanguages(interface: ['ru'], original: 'ja'), [
        'rus',
        'jpn',
      ]);
    });

    test('original: the original, then the interface', () {
      final p = TrackPolicy(
        audio: LanguagePreference(mode: LanguageMode.original),
      );
      expect(p.audioLanguages(interface: ['ru'], original: 'en'), [
        'eng',
        'rus',
      ]);
      expect(p.audioLanguages(interface: ['ru']), ['rus']);
    });

    test('custom: the list; an empty list behaves as auto', () {
      final p = TrackPolicy(
        audio: LanguagePreference(
          mode: LanguageMode.custom,
          languages: ['en-US', 'eng', 'de', 'xx'],
        ),
      );
      expect(p.audioLanguages(interface: ['ru']), ['eng', 'ger']);
      final empty = TrackPolicy(
        audio: LanguagePreference(mode: LanguageMode.custom),
      );
      expect(empty.audioLanguages(interface: ['ru'], original: 'fr'), [
        'rus',
        'fre',
      ]);
    });

    test('original subtitles are shown, not understood', () {
      final prefs = TrackPolicy(
        subtitles: LanguagePreference(mode: LanguageMode.original),
      ).subtitlePreferences(interface: ['ru'], original: 'ja');
      expect(prefs.languages, ['jpn', 'rus']);
      expect(prefs.understood, ['rus']);
    });
  });

  group('JSON', () {
    test('a policy round-trips', () {
      final p = TrackPolicy(
        audio: LanguagePreference(
          mode: LanguageMode.custom,
          languages: ['ja', 'en'],
        ),
        subtitles: LanguagePreference(mode: LanguageMode.original),
        subtitleMode: SubtitleMode.forcedOnly,
        sdh: SdhPreference.prefer,
      );
      expect(TrackPolicy.fromJson(p.toJson()), p);
      expect(TrackPolicy.fromJson(null), TrackPolicy());
      expect(TrackPolicy.fromJson({'subtitleMode': 'nonsense'}), TrackPolicy());
    });

    test('a PLAY-007 record still reads, and gains audio fields', () {
      final old = TrackMemory.fromJson({
        'choice': {'lang': 'rus', 'forced': true},
        'delayMs': 300,
      });
      expect(old.choice, const SubtitleChoice(language: 'rus', forced: true));
      expect(old.delay, const Duration(milliseconds: 300));
      expect(old.audio, isNull);
      expect(old.audioDelay, Duration.zero);

      final both = old.copyWith(
        audio: const AudioChoice(language: 'eng', title: 'MVO'),
        audioDelay: const Duration(milliseconds: -150),
      );
      final json = both.toJson();
      expect(json['delayMs'], 300, reason: 'PLAY-007 keys unchanged');
      expect(TrackMemory.fromJson(json), both);
    });

    test('delays are clamped', () {
      final m = TrackMemory.fromJson({
        'delayMs': 99999999,
        'audioDelayMs': -99999,
      });
      expect(m.delay.inMilliseconds, TrackMemory.maxDelayMs);
      expect(m.audioDelay.inMilliseconds, -TrackMemory.maxAudioDelayMs);
    });

    test('sound processing round-trips and clamps the boost', () {
      const a = AudioProcessing(
        downmix: DownmixMode.stereo,
        normalizeDownmix: true,
        lfeToStereo: true,
        boost: 250,
      );
      expect(AudioProcessing.fromJson(a.toJson()), a);
      expect(AudioProcessing.fromJson({'boost': 900}).boost, 300);
      expect(AudioProcessing.fromJson('x'), const AudioProcessing());
    });
  });

  group('flags from titles', () {
    test('commentary and audio description', () {
      expect(audioFlagsFromTitle("Director's Commentary").commentary, isTrue);
      expect(audioFlagsFromTitle('Комментарий режиссёра').commentary, isTrue);
      final ad = audioFlagsFromTitle('Тифлокомментарий');
      expect(ad.audioDescription, isTrue);
      expect(ad.commentary, isFalse);
      expect(
        audioFlagsFromTitle('English (Audio Description)').audioDescription,
        isTrue,
      );
      expect(audioFlagsFromTitle('Дубляж').commentary, isFalse);
      expect(audioFlagsFromTitle(null).commentary, isFalse);
    });
  });

  group('mpv options of the sound processing', () {
    test('defaults change nothing', () {
      expect(const AudioProcessing().mpvProperties, {
        'audio-normalize-downmix': 'no',
        'audio-swresample-o': '',
        'af': '',
      });
    });

    test('Volume Boost: gain, then a limiter at -1 dBFS', () {
      expect(
        const AudioProcessing(boost: 300).mpvFilter,
        'lavfi=[volume=3.00,alimiter=limit=0.891:level=0]',
      );
      expect(const AudioProcessing(boost: 500).clampedBoost, 300);
      expect(const AudioProcessing(boost: 10).mpvFilter, '');
    });

    test('stereo downmix before the limiter, LFE and normalization', () {
      const a = AudioProcessing(
        downmix: DownmixMode.stereo,
        lfeToStereo: true,
        boost: 150,
      );
      expect(a.mpvProperties, {
        'audio-normalize-downmix': 'no',
        'audio-swresample-o': 'lfe_mix_level=0.707',
        'af':
            'format=channels=stereo,'
            'lavfi=[volume=1.50,alimiter=limit=0.891:level=0]',
      });
      // A normalized downmix cannot clip: no limiter without a boost.
      expect(
        const AudioProcessing(
          downmix: DownmixMode.stereo,
          normalizeDownmix: true,
        ).mpvFilter,
        'format=channels=stereo',
      );
      // A plain downmix can: the limiter catches it.
      expect(
        const AudioProcessing(downmix: DownmixMode.stereo).mpvFilter,
        'format=channels=stereo,lavfi=[alimiter=limit=0.891:level=0]',
      );
    });

    test('boost steps', () {
      const a = AudioProcessing(boost: 160);
      expect(a.stepBoost(up: true), 200);
      expect(a.stepBoost(up: false), 150);
      expect(const AudioProcessing().stepBoost(up: false), 100);
      expect(const AudioProcessing(boost: 300).stepBoost(up: true), 300);
    });
  });

  test('delay labels say which way the sound moves', () {
    const en = PaPlayerStrings.en;
    const ru = PaPlayerStrings.ru;
    expect(
      en.audioDelayLabel(const Duration(milliseconds: 250)),
      'Sound 0.25 s later',
    );
    expect(
      ru.audioDelayLabel(const Duration(milliseconds: -100)),
      'Звук раньше на 0,1 с',
    );
    expect(en.audioDelayLabel(Duration.zero), en.audioDelayNone);
    expect(ru.percent(150), '150 %');
    expect(en.channelsLabel(6), '5.1');
    expect(ru.channelsLabel(2), 'Стерео');
    expect(en.channelsLabel(null), isNull);
  });
}
