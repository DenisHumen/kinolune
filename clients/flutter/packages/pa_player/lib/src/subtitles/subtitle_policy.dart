// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/foundation.dart';

import '../engine/player_engine.dart';
import '../tracks/track_policy.dart';
import 'languages.dart';

/// Which subtitle languages the user reads (ISO 639-2/B after
/// [normalizeLanguage]), whether the system asks for captions, and the
/// PLAY-004 policy: when subtitles come on and whether SDH goes first
/// ([TrackPolicy.subtitlePreferences] builds it from a profile's settings).
@immutable
class SubtitlePreferences {
  SubtitlePreferences({
    List<String> languages = const [],
    List<String>? understood,
    this.captionsOn = false,
    this.mode = SubtitleMode.auto,
    this.sdh = SdhPreference.auto,
  }) : languages = _codes(languages),
       understood = _codes(understood ?? languages);

  static List<String> _codes(List<String> list) => List.unmodifiable(<String>{
    for (final l in list) ?normalizeLanguage(l),
  });

  /// Languages to show subtitles in, best first; normalized, without
  /// duplicates.
  final List<String> languages;

  /// Languages the user understands: audio in one of them needs only
  /// forced subtitles ([SubtitleMode.auto]). By default [languages]; with
  /// "Original" subtitles the original language is shown but not
  /// understood.
  final List<String> understood;

  /// The system caption setting is on (Android captioning, Apple
  /// "Closed Captions + SDH"): show full subtitles, SDH first
  /// ([SdhPreference.auto]).
  final bool captionsOn;

  /// When subtitles come on by themselves.
  final SubtitleMode mode;

  /// Whether SDH goes before plain subtitles of one language.
  final SdhPreference sdh;

  /// SDH before plain subtitles.
  bool get sdhFirst =>
      sdh == SdhPreference.prefer || (sdh == SdhPreference.auto && captionsOn);
}

/// A subtitle choice that can be applied to another file: the language and
/// kind of the track, not its id. Kept per series (PLAY-007) so the next
/// episode gets the same subtitles.
@immutable
class SubtitleChoice {
  const SubtitleChoice({
    this.off = false,
    this.language,
    this.forced = false,
    this.hearingImpaired = false,
    this.external = false,
  });

  /// Subtitles switched off.
  static const SubtitleChoice none = SubtitleChoice(off: true);

  final bool off;
  final String? language;
  final bool forced;
  final bool hearingImpaired;
  final bool external;

  /// The choice that [track] stands for (null or "off": [none]).
  factory SubtitleChoice.of(MediaTrack? track) {
    if (track == null || track.isOff) return none;
    return SubtitleChoice(
      language: normalizeLanguage(track.language),
      forced: track.forced,
      hearingImpaired: track.hearingImpaired,
      external: track.external,
    );
  }

  Map<String, Object?> toJson() => off
      ? {'off': true}
      : {
          if (language != null) 'lang': language,
          if (forced) 'forced': true,
          if (hearingImpaired) 'sdh': true,
          if (external) 'external': true,
        };

  factory SubtitleChoice.fromJson(Object? json) {
    if (json is! Map) return none;
    if (json['off'] == true) return none;
    final lang = json['lang'];
    return SubtitleChoice(
      language: lang is String ? normalizeLanguage(lang) : null,
      forced: json['forced'] == true,
      hearingImpaired: json['sdh'] == true,
      external: json['external'] == true,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is SubtitleChoice &&
      other.off == off &&
      other.language == language &&
      other.forced == forced &&
      other.hearingImpaired == hearingImpaired &&
      other.external == external;

  @override
  int get hashCode =>
      Object.hash(off, language, forced, hearingImpaired, external);

  @override
  String toString() => off
      ? 'SubtitleChoice(off)'
      : 'SubtitleChoice($language${forced ? ', forced' : ''}'
            '${hearingImpaired ? ', sdh' : ''}${external ? ', external' : ''})';
}

/// Picks the subtitle track to show when a file opens.
///
/// 1. A [remembered] choice of the series wins: "off" stays off; otherwise
///    the track of the same language closest to it (forced, SDH, external,
///    commentary).
/// 2. [SubtitleMode.off]: off. [SubtitleMode.forcedOnly]: forced subtitles
///    (in the audio language first, then a preferred one), else off.
///    [SubtitleMode.always]: full subtitles in the first preferred language
///    that has them, else forced ones.
/// 3. [SubtitleMode.auto] (PLAY-007): with captions on in the system, full
///    subtitles in the first preferred language that has them. Audio in a
///    language the user understands (or of unknown language): forced
///    subtitles — they translate only the foreign lines — in the audio
///    language first, then a preferred one. Audio in another language:
///    full subtitles in a preferred language, else forced ones.
///
/// SDH go before plain subtitles of one language as
/// [SubtitlePreferences.sdhFirst] says. Commentary tracks are never picked
/// by themselves.
///
/// Returns [MediaTrack.off] to switch subtitles off, or null to leave the
/// engine's own choice (nothing in the user's languages, audio of unknown
/// language without forced subtitles).
MediaTrack? pickSubtitle(
  List<MediaTrack> tracks, {
  required SubtitlePreferences preferences,
  SubtitleChoice? remembered,
  String? audioLanguage,
}) {
  final real = [
    for (final t in tracks)
      if (!t.isOff) t,
  ];
  String? lang(MediaTrack t) => normalizeLanguage(t.language);

  if (remembered != null) {
    if (remembered.off) return MediaTrack.off;
    final same = [
      for (final t in real)
        if (lang(t) == remembered.language) t,
    ];
    if (same.isNotEmpty) {
      int score(MediaTrack t) =>
          (t.commentary ? 0 : 8) +
          (t.forced == remembered.forced ? 4 : 0) +
          (t.hearingImpaired == remembered.hearingImpaired ? 2 : 0) +
          (t.external == remembered.external ? 1 : 0);
      return _best(same, score);
    }
  }

  final mode = preferences.mode;
  if (mode == SubtitleMode.off) return MediaTrack.off;
  final prefs = preferences.languages;
  final candidates = [
    for (final t in real)
      if (!t.commentary) t,
  ];
  final full = [
    for (final t in candidates)
      if (!t.forced) t,
  ];
  final forced = [
    for (final t in candidates)
      if (t.forced) t,
  ];
  final audio = normalizeLanguage(audioLanguage);

  MediaTrack? inLanguages(
    List<MediaTrack> list,
    Iterable<String> languages,
    int Function(MediaTrack) s,
  ) {
    for (final l in languages) {
      final match = [
        for (final t in list)
          if (lang(t) == l) t,
      ];
      if (match.isNotEmpty) return _best(match, s);
    }
    return null;
  }

  final sdhFirst = preferences.sdhFirst;
  int fullScore(MediaTrack t) =>
      ((t.hearingImpaired == sdhFirst) ? 2 : 0) + (t.isDefault ? 1 : 0);
  int byDefault(MediaTrack t) => t.isDefault ? 1 : 0;
  MediaTrack? fullPick() => inLanguages(full, prefs, fullScore);
  // Forced subtitles translate the foreign lines into the language of the
  // audio: that language first.
  MediaTrack? forcedPick() =>
      inLanguages(forced, [?audio, ...prefs], byDefault);

  switch (mode) {
    case SubtitleMode.off:
      return MediaTrack.off;
    case SubtitleMode.forcedOnly:
      return forcedPick() ?? MediaTrack.off;
    case SubtitleMode.always:
      if (prefs.isEmpty || candidates.isEmpty) return null;
      return fullPick() ?? inLanguages(forced, prefs, byDefault);
    case SubtitleMode.auto:
      break;
  }

  if (prefs.isEmpty || real.isEmpty) return null;
  if (preferences.captionsOn) {
    final pick = fullPick() ?? inLanguages(forced, prefs, byDefault);
    if (pick != null) return pick;
  }

  final foreignAudio =
      audio != null && !preferences.understood.contains(audio);
  if (foreignAudio) {
    return fullPick() ?? inLanguages(forced, prefs, byDefault);
  }
  // Audio in the user's language: only the foreign lines.
  final pick = forcedPick();
  if (pick != null) return pick;
  return audio == null ? null : MediaTrack.off;
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

/// Forced / SDH flags of an embedded track from its title, when the
/// container has no flags for them ("Forced", "Надписи", "SDH", "CC").
({bool forced, bool hearingImpaired}) flagsFromTitle(String? title) {
  final t = (title ?? '').toLowerCase();
  final words = t.split(RegExp(r'[^\p{L}\p{N}]+', unicode: true));
  final forced =
      words.contains('forced') ||
      words.contains('foreign') ||
      t.contains('надписи') ||
      t.contains('форсир');
  final sdh =
      words.contains('sdh') ||
      words.contains('cc') ||
      words.contains('hi') ||
      t.contains('hearing impaired') ||
      t.contains('для слабослышащих');
  return (forced: forced, hearingImpaired: sdh);
}
