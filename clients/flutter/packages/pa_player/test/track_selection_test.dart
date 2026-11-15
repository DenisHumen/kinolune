// SPDX-License-Identifier: MPL-2.0
//
// PLAY-004 "done when": the table of track-selection cases passes on the
// mock engine. Every row opens a file in the real PlayerScreen on
// FakePlayerEngine with the row's tracks, profile policy, original
// language and series memory, and checks which audio and subtitle tracks
// the player picked by itself: original against dubbed, forced, SDH,
// commentary, several tracks of one language, missing languages.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

// ---- audio tracks ----------------------------------------------------------

const aEn = MediaTrack(
  id: 'a-en',
  language: 'eng',
  title: 'English',
  isDefault: true,
  channels: 6,
);
const aEnPlain = MediaTrack(id: 'a-en', language: 'eng');
const aRu = MediaTrack(id: 'a-ru', language: 'rus', title: 'Дубляж');
const aRuMvo = MediaTrack(id: 'a-ru-mvo', language: 'rus', title: 'MVO');
const aRuMvoDefault = MediaTrack(
  id: 'a-ru-mvo',
  language: 'rus',
  title: 'MVO',
  isDefault: true,
);
const aJa = MediaTrack(id: 'a-ja', language: 'jpn');
const aEnCommentary = MediaTrack(
  id: 'a-en-comm',
  language: 'eng',
  title: "Director's commentary",
  commentary: true,
  isDefault: true,
);
const aUnd = MediaTrack(id: 'a-und');
const aRuAd = MediaTrack(
  id: 'a-ru-ad',
  language: 'rus',
  title: 'Тифлокомментарий',
  audioDescription: true,
);

// ---- subtitle tracks -------------------------------------------------------

const sRu = MediaTrack(id: 's-ru', language: 'rus');
const sRuForced = MediaTrack(id: 's-ru-forced', language: 'rus', forced: true);
const sRuSdh = MediaTrack(id: 's-ru-sdh', language: 'rus', hearingImpaired: true);
const sRuCommentary = MediaTrack(
  id: 's-ru-comm',
  language: 'rus',
  title: 'Комментарии режиссёра',
  commentary: true,
);
const sEn = MediaTrack(id: 's-en', language: 'eng');
const sEnForced = MediaTrack(id: 's-en-forced', language: 'eng', forced: true);
const sJa = MediaTrack(id: 's-ja', language: 'jpn');
const sUnd = MediaTrack(id: 's-und');

/// One row of the table.
class Case {
  const Case(
    this.name, {
    required this.audio,
    this.subs = const [],
    this.policy,
    this.original,
    this.interface = const ['ru'],
    this.memory,
    this.captionsOn = false,
    required this.wantAudio,
    this.wantSub,
  });

  final String name;
  final List<MediaTrack> audio;
  final List<MediaTrack> subs;
  final TrackPolicy? policy;
  final String? original;
  final List<String> interface;
  final TrackMemory? memory;
  final bool captionsOn;

  /// Id of the audio track that plays.
  final String wantAudio;

  /// Id of the subtitle track the player selects, `no` for off; null: the
  /// player leaves the engine's choice (no selection at all).
  final String? wantSub;
}

TrackPolicy _audio(LanguageMode mode, [List<String> langs = const []]) =>
    TrackPolicy(
      audio: LanguagePreference(mode: mode, languages: langs),
    );

TrackPolicy _subs({
  SubtitleMode mode = SubtitleMode.auto,
  SdhPreference sdh = SdhPreference.auto,
  LanguagePreference? languages,
}) => TrackPolicy(subtitleMode: mode, sdh: sdh, subtitles: languages);

final cases = <Case>[
  // Original against dubbed.
  const Case(
    'auto: the dub in the interface language, forced subtitles for it',
    audio: [aEn, aRu],
    subs: [sRuForced, sRu, sEn],
    original: 'en',
    wantAudio: 'a-ru',
    wantSub: 's-ru-forced',
  ),
  const Case(
    'auto: no dub, the original beats a third language',
    audio: [aEn, aJa],
    subs: [sRu, sEn],
    original: 'ja',
    wantAudio: 'a-ja',
    wantSub: 's-ru',
  ),
  Case(
    'original: the original track, full subtitles in the user language',
    audio: const [aEn, aRu],
    subs: const [sRuForced, sRu],
    policy: _audio(LanguageMode.original),
    original: 'en',
    wantAudio: 'a-en',
    wantSub: 's-ru',
  ),
  Case(
    'original: unknown original language falls back to the interface',
    audio: const [aEn, aRu],
    policy: _audio(LanguageMode.original),
    wantAudio: 'a-ru',
  ),
  Case(
    'original: no original track in the file, the user language',
    audio: const [aEn, aRu],
    policy: _audio(LanguageMode.original),
    original: 'ja',
    wantAudio: 'a-ru',
  ),
  Case(
    'custom list: its first language that the file has',
    audio: const [aRu, aEnPlain],
    subs: const [sRu, sEn],
    policy: _audio(LanguageMode.custom, ['de', 'en', 'ru']),
    wantAudio: 'a-en',
    wantSub: 's-ru',
  ),
  // Missing languages.
  Case(
    'custom list without a match: the default track',
    audio: const [aRu, aEn],
    policy: _audio(LanguageMode.custom, ['de']),
    wantAudio: 'a-en',
  ),
  const Case(
    'no language matches and no default: the first track',
    audio: [aJa, aEnPlain],
    wantAudio: 'a-ja',
  ),
  const Case(
    'untagged tracks never match a language: the default one',
    audio: [aUnd, aEn],
    original: 'ru',
    wantAudio: 'a-en',
  ),
  const Case(
    'subtitles only in other or unknown languages: the engine decides',
    audio: [aEn],
    subs: [sUnd, sEn],
    wantAudio: 'a-en',
    wantSub: null,
  ),
  // Commentary and audio description.
  Case(
    'a commentary track is never picked by itself, even as default',
    audio: const [aEnCommentary, aEnPlain],
    policy: _audio(LanguageMode.original),
    original: 'en',
    wantAudio: 'a-en',
  ),
  const Case(
    'audio description is never picked by itself',
    audio: [aRuAd, aRu],
    wantAudio: 'a-ru',
  ),
  const Case(
    'commentary subtitles are never picked by themselves',
    audio: [aEn],
    subs: [sRuCommentary, sRu],
    original: 'en',
    interface: ['ru'],
    wantAudio: 'a-en',
    wantSub: 's-ru',
  ),
  // Several tracks of one language.
  const Case(
    'two tracks of one language: the default one',
    audio: [aRu, aRuMvoDefault],
    wantAudio: 'a-ru-mvo',
  ),
  const Case(
    'two tracks of one language without a default: the first one',
    audio: [aRu, aRuMvo],
    wantAudio: 'a-ru',
  ),
  // The series memory.
  const Case(
    'remembered audio by title: the same voice-over in the next episode',
    audio: [aRu, aRuMvo],
    memory: TrackMemory(
      audio: AudioChoice(language: 'rus', title: 'MVO'),
    ),
    wantAudio: 'a-ru-mvo',
  ),
  const Case(
    'remembered commentary stays commentary',
    audio: [aEnPlain, aEnCommentary],
    memory: TrackMemory(
      audio: AudioChoice(language: 'eng', commentary: true),
    ),
    wantAudio: 'a-en-comm',
  ),
  const Case(
    'remembered audio language missing in this episode: the policy',
    audio: [aEn, aRu],
    memory: TrackMemory(audio: AudioChoice(language: 'jpn')),
    wantAudio: 'a-ru',
  ),
  const Case(
    'remembered original audio: subtitles follow the picked audio',
    audio: [aRu, aEnPlain],
    subs: [sRuForced, sRu],
    memory: TrackMemory(audio: AudioChoice(language: 'eng')),
    wantAudio: 'a-en',
    wantSub: 's-ru',
  ),
  Case(
    'remembered "subtitles off" beats "always"',
    audio: const [aEn],
    subs: const [sRu],
    policy: _subs(mode: SubtitleMode.always),
    memory: const TrackMemory(choice: SubtitleChoice.none),
    wantAudio: 'a-en',
    wantSub: 'no',
  ),
  // Forced.
  Case(
    'forced only: forced in the audio language first',
    audio: const [aEnPlain],
    subs: const [sRu, sRuForced, sEnForced],
    policy: _subs(mode: SubtitleMode.forcedOnly),
    wantAudio: 'a-en',
    wantSub: 's-en-forced',
  ),
  Case(
    'forced only without forced subtitles: off',
    audio: const [aEnPlain],
    subs: const [sRu, sEn],
    policy: _subs(mode: SubtitleMode.forcedOnly),
    wantAudio: 'a-en',
    wantSub: 'no',
  ),
  Case(
    'always: full subtitles even for audio the user understands',
    audio: const [aRu],
    subs: const [sRuForced, sRu],
    policy: _subs(mode: SubtitleMode.always),
    wantAudio: 'a-ru',
    wantSub: 's-ru',
  ),
  Case(
    'subtitles off: off even for foreign audio',
    audio: const [aEnPlain],
    subs: const [sRu],
    policy: _subs(mode: SubtitleMode.off),
    wantAudio: 'a-en',
    wantSub: 'no',
  ),
  // SDH.
  Case(
    'SDH preferred: SDH before plain subtitles',
    audio: const [aEnPlain],
    subs: const [sRu, sRuSdh],
    policy: _subs(sdh: SdhPreference.prefer),
    wantAudio: 'a-en',
    wantSub: 's-ru-sdh',
  ),
  const Case(
    'SDH auto with system captions on: SDH first',
    audio: [aEnPlain],
    subs: [sRu, sRuSdh],
    captionsOn: true,
    wantAudio: 'a-en',
    wantSub: 's-ru-sdh',
  ),
  Case(
    'SDH avoided: plain subtitles even with system captions on',
    audio: const [aEnPlain],
    subs: const [sRuSdh, sRu],
    policy: _subs(sdh: SdhPreference.avoid),
    captionsOn: true,
    wantAudio: 'a-en',
    wantSub: 's-ru',
  ),
  // Original subtitles.
  Case(
    'original subtitles: in the original language for foreign audio',
    audio: const [aJa],
    subs: const [sRu, sJa],
    original: 'ja',
    policy: _subs(
      languages: LanguagePreference(mode: LanguageMode.original),
    ),
    wantAudio: 'a-ja',
    wantSub: 's-ja',
  ),
];

Future<FakePlayerEngine> _play(WidgetTester t, Case c) async {
  final engine = FakePlayerEngine()
    ..openAudioTracks = c.audio
    ..embeddedSubtitles = c.subs;
  await t.pumpWidget(
    MaterialApp(
      home: PlayerScreen(
        request: PlayRequest(
          uri: '/media/Show/S01E02.mkv',
          title: 'S01E02',
          subtitles: const [],
          seriesKey: 'show',
          subtitleMemory: c.memory,
          originalLanguage: c.original,
        ),
        onProgress: (_, _) {},
        engineFactory: ({bool video = true}) => engine,
        subtitleLanguages: c.interface,
        trackPolicy: c.policy,
        captionStyleLoader: () async =>
            c.captionsOn ? const SystemCaptionStyle(enabled: true) : null,
      ),
    ),
  );
  await t.pump();
  // The subtitle pick waits a moment for the track list.
  await t.pump(const Duration(seconds: 2));
  return engine;
}

void main() {
  test('the table covers the cases of the task', () {
    expect(cases.length, greaterThanOrEqualTo(20));
  });

  for (final c in cases) {
    testWidgets(c.name, (t) async {
      final engine = await _play(t, c);
      final s = engine.state.value;
      expect(s.audioTrack?.id, c.wantAudio, reason: 'audio');
      final subCalls = engine.calls.where((e) => e.startsWith('sub:'));
      if (c.wantSub == null) {
        expect(subCalls, isEmpty, reason: 'subtitles left to the engine');
      } else {
        expect(s.subtitleTrack?.id, c.wantSub, reason: 'subtitles');
      }
      // An automatic pick is not a choice: nothing changes in the memory.
      expect(engine.calls.where((e) => e.startsWith('audiodelay:')), isEmpty);
    });
  }
}
