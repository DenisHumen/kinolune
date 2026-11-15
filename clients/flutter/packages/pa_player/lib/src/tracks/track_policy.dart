// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/foundation.dart';

import '../engine/player_engine.dart';
import '../subtitles/languages.dart';
import '../subtitles/subtitle_policy.dart';

/// Where the preferred languages of a track kind come from (PLAY-004,
/// design 01 §1.4 "Language").
enum LanguageMode {
  /// Audio: the interface language, else the original language of the
  /// title. Subtitles: the system caption language and the interface
  /// language (PLAY-007).
  auto,

  /// The original language of the title (TMDB `original_language`), then
  /// what [auto] would pick when the title has no track in it.
  original,

  /// The user's own list of languages, best first.
  custom,
}

/// The preferred languages of one track kind (audio or subtitles).
@immutable
class LanguagePreference {
  LanguagePreference({
    this.mode = LanguageMode.auto,
    List<String> languages = const [],
  }) : languages = List.unmodifiable(<String>{
         for (final l in languages) ?normalizeLanguage(l),
       });

  /// Auto, no list.
  static final LanguagePreference automatic = LanguagePreference();

  final LanguageMode mode;

  /// [LanguageMode.custom]: ISO 639-2/B codes, best first, no duplicates.
  /// Kept when the mode changes, so switching back restores the list.
  final List<String> languages;

  LanguagePreference copyWith({LanguageMode? mode, List<String>? languages}) =>
      LanguagePreference(
        mode: mode ?? this.mode,
        languages: languages ?? this.languages,
      );

  Map<String, Object?> toJson() => {
    'mode': mode.name,
    if (languages.isNotEmpty) 'langs': languages,
  };

  factory LanguagePreference.fromJson(Object? json) {
    if (json is! Map) return automatic;
    final langs = json['langs'];
    return LanguagePreference(
      mode: LanguageMode.values.asNameMap()[json['mode']] ?? LanguageMode.auto,
      languages: langs is List ? [for (final l in langs) '$l'] : const [],
    );
  }

  @override
  bool operator ==(Object other) =>
      other is LanguagePreference &&
      other.mode == mode &&
      listEquals(other.languages, languages);

  @override
  int get hashCode => Object.hash(mode, Object.hashAll(languages));

  @override
  String toString() => 'LanguagePreference(${mode.name}, $languages)';
}

/// When subtitles come on by themselves.
enum SubtitleMode {
  /// PLAY-007: full subtitles for audio in a language the user does not
  /// read (or with system captions on); for audio the user understands
  /// only forced subtitles, else none.
  auto,

  /// Full subtitles in a preferred language whenever there are some.
  always,

  /// Only forced subtitles (the foreign lines), never full ones.
  forcedOnly,

  /// Subtitles stay off unless the user picks a track.
  off,
}

/// Whether full subtitles for the deaf and hard of hearing (SDH, CC) are
/// picked before plain ones of the same language.
enum SdhPreference {
  /// SDH first only while the system caption setting is on.
  auto,

  /// SDH first always.
  prefer,

  /// Plain subtitles first, even with system captions on.
  avoid,
}

/// What the player picks by itself when a file opens (PLAY-004), per
/// profile: the preferred audio and subtitle languages, when subtitles come
/// on and whether SDH goes first. A choice the user made for a series
/// ([TrackMemory]) wins over all of it.
@immutable
class TrackPolicy {
  TrackPolicy({
    LanguagePreference? audio,
    LanguagePreference? subtitles,
    this.subtitleMode = SubtitleMode.auto,
    this.sdh = SdhPreference.auto,
  }) : audio = audio ?? LanguagePreference.automatic,
       subtitles = subtitles ?? LanguagePreference.automatic;

  final LanguagePreference audio;
  final LanguagePreference subtitles;
  final SubtitleMode subtitleMode;
  final SdhPreference sdh;

  TrackPolicy copyWith({
    LanguagePreference? audio,
    LanguagePreference? subtitles,
    SubtitleMode? subtitleMode,
    SdhPreference? sdh,
  }) => TrackPolicy(
    audio: audio ?? this.audio,
    subtitles: subtitles ?? this.subtitles,
    subtitleMode: subtitleMode ?? this.subtitleMode,
    sdh: sdh ?? this.sdh,
  );

  /// Audio languages to look for, best first. [interface] are the
  /// languages of the user (the UI language), [original] the original
  /// language of the title, if known.
  ///
  /// Auto: [interface], then [original] (the original beats a dub in a
  /// third language). Original: [original], then [interface] (a release
  /// without the original track gets the user's language). Custom: the
  /// list; empty list: as Auto.
  List<String> audioLanguages({
    required List<String> interface,
    String? original,
  }) {
    final orig = normalizeLanguage(original);
    return _unique(switch (audio.mode) {
      LanguageMode.auto => [...interface, ?orig],
      LanguageMode.original => [?orig, ...interface],
      LanguageMode.custom =>
        audio.languages.isEmpty ? [...interface, ?orig] : audio.languages,
    });
  }

  /// Subtitle preferences for [pickSubtitle]. [interface] are the
  /// languages the user reads (system caption language first, then the
  /// UI language); they also decide whether the audio is "foreign".
  SubtitlePreferences subtitlePreferences({
    required List<String> interface,
    String? original,
    bool captionsOn = false,
  }) {
    final orig = normalizeLanguage(original);
    final show = switch (subtitles.mode) {
      LanguageMode.auto => interface,
      LanguageMode.original => [?orig, ...interface],
      LanguageMode.custom =>
        subtitles.languages.isEmpty ? interface : subtitles.languages,
    };
    return SubtitlePreferences(
      languages: show,
      understood: interface,
      captionsOn: captionsOn,
      mode: subtitleMode,
      sdh: sdh,
    );
  }

  Map<String, Object?> toJson() => {
    'audio': audio.toJson(),
    'subtitles': subtitles.toJson(),
    'subtitleMode': subtitleMode.name,
    'sdh': sdh.name,
  };

  factory TrackPolicy.fromJson(Object? json) {
    if (json is! Map) return TrackPolicy();
    return TrackPolicy(
      audio: LanguagePreference.fromJson(json['audio']),
      subtitles: LanguagePreference.fromJson(json['subtitles']),
      subtitleMode:
          SubtitleMode.values.asNameMap()[json['subtitleMode']] ??
          SubtitleMode.auto,
      sdh: SdhPreference.values.asNameMap()[json['sdh']] ?? SdhPreference.auto,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is TrackPolicy &&
      other.audio == audio &&
      other.subtitles == subtitles &&
      other.subtitleMode == subtitleMode &&
      other.sdh == sdh;

  @override
  int get hashCode => Object.hash(audio, subtitles, subtitleMode, sdh);

  @override
  String toString() =>
      'TrackPolicy(audio: $audio, subtitles: $subtitles, '
      '${subtitleMode.name}, sdh: ${sdh.name})';
}

List<String> _unique(List<String?> list) => List.unmodifiable(<String>{
  for (final l in list) ?normalizeLanguage(l),
});

/// An audio choice that can be applied to another file: the language and
/// kind of the track, not its id (ids differ between episodes).
@immutable
class AudioChoice {
  const AudioChoice({
    this.language,
    this.commentary = false,
    this.audioDescription = false,
    this.title,
  });

  final String? language;
  final bool commentary;
  final bool audioDescription;

  /// Title of the track ("Дубляж", "MVO LostFilm"): tells apart several
  /// tracks of one language.
  final String? title;

  factory AudioChoice.of(MediaTrack track) => AudioChoice(
    language: normalizeLanguage(track.language),
    commentary: track.commentary,
    audioDescription: track.audioDescription,
    title: (track.title ?? '').trim().isEmpty ? null : track.title!.trim(),
  );

  Map<String, Object?> toJson() => {
    if (language != null) 'lang': language,
    if (commentary) 'commentary': true,
    if (audioDescription) 'ad': true,
    if (title != null) 'title': title,
  };

  factory AudioChoice.fromJson(Object? json) {
    if (json is! Map) return const AudioChoice();
    final lang = json['lang'];
    final title = json['title'];
    return AudioChoice(
      language: lang is String ? normalizeLanguage(lang) : null,
      commentary: json['commentary'] == true,
      audioDescription: json['ad'] == true,
      title: title is String && title.isNotEmpty ? title : null,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is AudioChoice &&
      other.language == language &&
      other.commentary == commentary &&
      other.audioDescription == audioDescription &&
      other.title == title;

  @override
  int get hashCode =>
      Object.hash(language, commentary, audioDescription, title);

  @override
  String toString() =>
      'AudioChoice($language${commentary ? ', commentary' : ''}'
      '${audioDescription ? ', ad' : ''}${title == null ? '' : ', $title'})';
}

/// What the player remembers for a series (PLAY-007, PLAY-004): the
/// subtitle and audio choices and both delays. One record per series and
/// profile; the next episode starts with it.
@immutable
class TrackMemory {
  const TrackMemory({
    this.choice,
    this.delay = Duration.zero,
    this.audio,
    this.audioDelay = Duration.zero,
  });

  /// Subtitle choice; null: none made yet (the policy picks).
  final SubtitleChoice? choice;

  /// Subtitle delay; positive: subtitles later (mpv `sub-delay`).
  final Duration delay;

  /// Audio choice; null: none made yet (the policy picks).
  final AudioChoice? audio;

  /// Audio delay; positive: sound later (mpv `audio-delay`).
  final Duration audioDelay;

  TrackMemory copyWith({
    SubtitleChoice? choice,
    Duration? delay,
    AudioChoice? audio,
    Duration? audioDelay,
  }) => TrackMemory(
    choice: choice ?? this.choice,
    delay: delay ?? this.delay,
    audio: audio ?? this.audio,
    audioDelay: audioDelay ?? this.audioDelay,
  );

  /// The PLAY-007 keys (`choice`, `delayMs`) stay as they were, so records
  /// saved before PLAY-004 read the same.
  Map<String, Object?> toJson() => {
    if (choice != null) 'choice': choice!.toJson(),
    if (delay != Duration.zero) 'delayMs': delay.inMilliseconds,
    if (audio != null) 'audio': audio!.toJson(),
    if (audioDelay != Duration.zero) 'audioDelayMs': audioDelay.inMilliseconds,
  };

  factory TrackMemory.fromJson(Object? json) {
    if (json is! Map) return const TrackMemory();
    Duration ms(Object? v, int max) =>
        v is int ? Duration(milliseconds: v.clamp(-max, max)) : Duration.zero;
    return TrackMemory(
      choice: json.containsKey('choice')
          ? SubtitleChoice.fromJson(json['choice'])
          : null,
      delay: ms(json['delayMs'], maxDelayMs),
      audio: json['audio'] is Map ? AudioChoice.fromJson(json['audio']) : null,
      audioDelay: ms(json['audioDelayMs'], maxAudioDelayMs),
    );
  }

  /// Largest subtitle delay the controls allow, either way.
  static const int maxDelayMs = 600000;

  /// Largest audio delay, either way (mpv allows more; a lip-sync fix
  /// needs far less).
  static const int maxAudioDelayMs = 10000;

  @override
  bool operator ==(Object other) =>
      other is TrackMemory &&
      other.choice == choice &&
      other.delay == delay &&
      other.audio == audio &&
      other.audioDelay == audioDelay;

  @override
  int get hashCode => Object.hash(choice, delay, audio, audioDelay);

  @override
  String toString() =>
      'TrackMemory($choice, ${delay.inMilliseconds} ms, $audio, '
      '${audioDelay.inMilliseconds} ms)';
}

/// PLAY-007 name of [TrackMemory].
typedef SubtitleMemory = TrackMemory;

/// Picks the audio track to play when a file opens (PLAY-004).
///
/// 1. A [remembered] choice of the series wins: the track of the same
///    language closest to it (commentary, audio description, title).
/// 2. The first of [languages] (see [TrackPolicy.audioLanguages]) that has
///    a track: of those, the container's default, else the first one.
///    Commentary and audio description tracks are never picked by
///    themselves.
/// 3. Nothing in those languages: the default track, else the first one
///    that is not commentary.
///
/// Returns null when there is nothing to pick (no tracks).
MediaTrack? pickAudio(
  List<MediaTrack> tracks, {
  required List<String> languages,
  AudioChoice? remembered,
}) {
  final real = [
    for (final t in tracks)
      if (!t.isOff) t,
  ];
  if (real.isEmpty) return null;
  String? lang(MediaTrack t) => normalizeLanguage(t.language);

  if (remembered != null && remembered.language != null) {
    final same = [
      for (final t in real)
        if (lang(t) == remembered.language) t,
    ];
    if (same.isNotEmpty) {
      int score(MediaTrack t) =>
          (t.commentary == remembered.commentary ? 8 : 0) +
          (t.audioDescription == remembered.audioDescription ? 4 : 0) +
          (remembered.title != null && t.title?.trim() == remembered.title
              ? 2
              : 0) +
          (t.isDefault ? 1 : 0);
      return _best(same, score);
    }
  }

  // Main tracks only; a file of commentaries alone still plays.
  final main = [
    for (final t in real)
      if (!t.commentary && !t.audioDescription) t,
  ];
  final pool = main.isEmpty ? real : main;
  int byDefault(MediaTrack t) => t.isDefault ? 1 : 0;
  for (final l in languages) {
    final code = normalizeLanguage(l);
    if (code == null) continue;
    final match = [
      for (final t in pool)
        if (lang(t) == code) t,
    ];
    if (match.isNotEmpty) return _best(match, byDefault);
  }
  return _best(pool, byDefault);
}

/// The track with the highest [score]; the first one on a tie.
MediaTrack _best(List<MediaTrack> list, int Function(MediaTrack) score) {
  var best = list.first;
  var bestScore = score(best);
  for (final t in list.skip(1)) {
    final s = score(t);
    if (s > bestScore) {
      best = t;
      bestScore = s;
    }
  }
  return best;
}

/// Commentary and audio description flags of a track from its title, when
/// the container has no flags for them.
({bool commentary, bool audioDescription}) audioFlagsFromTitle(String? title) {
  final t = (title ?? '').toLowerCase();
  final words = t.split(RegExp(r'[^\p{L}\p{N}]+', unicode: true));
  final commentary =
      t.contains('commentary') ||
      t.contains('коммент') ||
      words.contains('comments');
  final ad =
      t.contains('audio description') ||
      t.contains('descriptive') ||
      t.contains('тифлокоммент') ||
      t.contains('тифло') ||
      words.contains('ad');
  return (commentary: commentary && !ad, audioDescription: ad);
}
