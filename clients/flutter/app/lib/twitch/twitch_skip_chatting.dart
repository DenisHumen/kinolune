// SPDX-License-Identifier: MPL-2.0
//
// «Пропустить болтовню» (TWI-027): where the game starts in a VOD whose
// first chapter (TWI-008) is talk — Just Chatting and the like — and where
// a game's part of a VOD starts.
//
// Which categories are "not a game" is a short fixed list, matched by the
// Twitch category id or, for imports and stores without the id, by the
// English name Helix returns. Decided for the simplest client: no setting,
// no remote list; a category missing here only means the button is not
// offered. Everything else counts as a game.

import 'twitch_models.dart';

/// Twitch ids of the talk categories in [kNonGameCategoryNames].
const kNonGameCategoryIds = {
  '509658', // Just Chatting
  '417752', // Talk Shows & Podcasts
  '498566', // I'm Only Sleeping
  '26936', // Music
  '509659', // ASMR
  '509660', // Art
};

/// Lower-case English names of the categories that are not a game.
const kNonGameCategoryNames = {
  'just chatting',
  'talk shows & podcasts',
  "i'm only sleeping",
  'music',
  'asmr',
  'art',
  'special events',
  'food & drink',
  'travel & outdoors',
  'pools, hot tubs, and beaches',
};

/// The chapter is talk, not a game.
bool isNonGameChapter(TwitchChapter c) => isNonGameCategory(c.gameId, c.gameName);

/// The category ([id], English [name]) is talk, not a game.
bool isNonGameCategory(String? id, String? name) =>
    kNonGameCategoryIds.contains(id) ||
    kNonGameCategoryNames.contains(name?.trim().toLowerCase());

/// Where «К игре» seeks: the start of the first game chapter after a talk
/// opening. Null when the VOD does not open with talk, has no game after
/// it, or its chapters are only the channel's current category
/// ([TwitchChapter.approximate], not a timeline): the button is hidden.
TwitchChapter? firstGameAfterTalk(List<TwitchChapter> chapters) {
  if (chapters.length < 2 || chapters.any((c) => c.approximate)) return null;
  final sorted = [...chapters]..sort((a, b) => a.start.compareTo(b.start));
  if (!isNonGameChapter(sorted.first)) return null;
  for (final c in sorted.skip(1)) {
    if (!isNonGameChapter(c)) return c.start > Duration.zero ? c : null;
  }
  return null;
}

/// The start of [gameId]'s first chapter: a playthrough part opened from
/// the game's category starts with the game. Null: no such chapter, or it
/// is the start of the VOD anyway.
Duration? gameStartIn(List<TwitchChapter> chapters, String gameId) {
  if (chapters.any((c) => c.approximate)) return null;
  Duration? start;
  for (final c in chapters) {
    if (c.gameId != gameId) continue;
    if (start == null || c.start < start) start = c.start;
  }
  return start == null || start <= Duration.zero ? null : start;
}
