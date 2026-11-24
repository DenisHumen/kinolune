// SPDX-License-Identifier: MPL-2.0
//
// What the unified search (UI-045) asks, each on its own:
// * the core index (pa-catalog, LIB-008): movies, shows, episodes, tracks —
//   filtered by the core for the active profile (LIB-012: ratings, folder
//   white lists), under 100 ms on 50 000 items;
// * the music index the Music section builds (MUS-004): artists and albums,
//   matched by name in memory; left out when music is hidden for the
//   profile (UI-015);
// * the audiobooks (MUS-018), already in memory;
// * personal media (UI-025): only libraries that are open now. A locked
//   library is never asked, so nothing of it can show up in the results;
// * Twitch channels (TWI-013): only while the Streams section is shown
//   (not for restricted profiles), a little later than the rest — it may
//   go to the network.

import 'dart:async';

import 'package:flutter/foundation.dart' show debugPrint;

import '../integration/library.dart';
import '../services/app_services.dart';
import 'search_model.dart';

/// Pause before Twitch is asked, on top of the debounce: typing a word does
/// not send a request per letter.
const twitchSearchDelay = Duration(milliseconds: 300);

final _spaces = RegExp(r'\s+');

/// Lower case, single spaces (as the music index groups names).
String foldName(String s) => s.trim().toLowerCase().replaceAll(_spaces, ' ');

/// The words of a query, folded.
List<String> searchWords(String query) =>
    foldName(query).split(' ')..removeWhere((w) => w.isEmpty);

/// Whether every word is in one of [texts].
bool matchesWords(List<String> words, Iterable<String?> texts) {
  final hay = [
    for (final t in texts)
      if (t != null && t.isNotEmpty) foldName(t),
  ].join('\n');
  return words.isNotEmpty && words.every(hay.contains);
}

/// [items] whose [name] starts with the query first, the order kept
/// otherwise.
List<T> startsFirst<T>(List<T> items, String query, String Function(T) name) {
  final q = foldName(query);
  return [
    ...items.where((i) => foldName(name(i)).startsWith(q)),
    ...items.where((i) => !foldName(name(i)).startsWith(q)),
  ];
}

/// The section of a core search group.
SearchSection sectionOfKind(ItemKind kind) => switch (kind) {
  ItemKind.movie => SearchSection.movies,
  ItemKind.show => SearchSection.shows,
  ItemKind.episode => SearchSection.episodes,
  ItemKind.track => SearchSection.tracks,
};

/// Music is shown to the active profile (UI-015), asked per query.
typedef MusicShown = bool Function();

bool _always() => true;

/// The core index: movies, shows, episodes and tracks.
SearchSource librarySearchSource(
  Library library, {
  MusicShown music = _always,
}) => SearchSource(
  sections: {
    SearchSection.movies,
    SearchSection.shows,
    SearchSection.episodes,
    SearchSection.tracks,
  },
  search: (q, limit) async {
    // One more than shown tells whether «Все» has more.
    final groups = await library.searchGrouped(q, perGroup: limit + 1);
    final tracks = music();
    return [
      for (final g in groups)
        if (tracks || g.kind != ItemKind.track)
          cutHits(sectionOfKind(g.kind), g.items, limit),
    ];
  },
);

/// The music index of the library, kept fresh while the search is open
/// (the tracks arrive while the user starts typing) and rebuilt only when
/// the tracks changed, not per keystroke.
class MusicSearchIndex {
  MusicSearchIndex(LibraryMusic music) {
    _sub = music.musicTracks().listen(
      (t) => _tracks = t,
      onError: (Object e) => debugPrint('Music search index: $e'),
    );
  }

  StreamSubscription<List<MusicTrack>>? _sub;
  List<MusicTrack> _tracks = const [];
  List<MusicTrack>? _indexed;
  MusicIndex _index = MusicIndex.empty;

  /// The index of the latest tracks (audiobook parts left out, as in the
  /// Music section); empty until the tracks arrived.
  Future<MusicIndex> index() async {
    if (!identical(_indexed, _tracks)) {
      _indexed = _tracks;
      _index = MusicIndex([
        for (final t in _tracks)
          if (!t.audiobook) t,
      ]);
    }
    return _index;
  }

  void dispose() {
    unawaited(_sub?.cancel());
    _sub = null;
  }
}

/// Artists and albums of the music index.
SearchSource musicSearchSource(
  MusicSearchIndex music, {
  MusicShown shown = _always,
}) => SearchSource(
  sections: {SearchSection.artists, SearchSection.albums},
  search: (q, limit) async {
    final words = searchWords(q);
    if (words.isEmpty || !shown()) return const [];
    final index = await music.index();
    final artists = startsFirst(
      [
        for (final a in index.artists)
          if (matchesWords(words, [a.name])) a,
      ],
      q,
      (a) => a.name,
    );
    final albums = startsFirst(
      [
        for (final a in index.albums)
          if (a.title != null && matchesWords(words, [a.title, a.artist])) a,
      ],
      q,
      (a) => a.title ?? '',
    );
    return [
      cutHits(SearchSection.artists, artists, limit, music: index),
      cutHits(SearchSection.albums, albums, limit, music: index),
    ];
  },
);

/// The audiobooks: title, author, narrator, series.
SearchSource audiobookSearchSource(List<Audiobook> Function() books) =>
    SearchSource(
      sections: {SearchSection.audiobooks},
      search: (q, limit) async {
        final words = searchWords(q);
        if (words.isEmpty) return const [];
        final found = startsFirst(
          [
            for (final b in books())
              if (matchesWords(words, [
                b.title,
                b.author,
                b.narrator,
                b.series,
              ]))
                b,
          ],
          q,
          (b) => b.title,
        );
        return [cutHits(SearchSection.audiobooks, found, limit)];
      },
    );

/// Videos of the personal libraries that are open now. Locked libraries
/// are skipped before anything is asked of them.
SearchSource personalSearchSource(PersonalMedia personal) => SearchSource(
  sections: {SearchSection.personal},
  search: (q, limit) async {
    final found = <PersonalHit>[];
    for (final lib in await personal.libraries()) {
      if (lib.locked) continue;
      try {
        final videos = await personal.videos(
          lib.id,
          filter: PersonalFilter(text: q),
          sort: PersonalSort.title,
        );
        for (final v in videos) {
          found.add(PersonalHit(lib, v));
          if (found.length > limit) break;
        }
      } on PersonalLocked {
        // Locked meanwhile (the app went to the background): skip it.
      }
      if (found.length > limit) break;
    }
    return [cutHits(SearchSection.personal, found, limit)];
  },
);

/// Twitch channels: the favourites, the cache and, signed in, Helix.
SearchSource twitchSearchSource(AppServices services) => SearchSource(
  sections: {SearchSection.channels},
  delay: twitchSearchDelay,
  search: (q, limit) async {
    if (!services.twitchVisible) return const [];
    final r = await services.twitch.search(q);
    return [cutHits(SearchSection.channels, r.channels, limit)];
  },
);

/// Every source of the unified search for the active profile.
List<SearchSource> searchSources(
  AppServices services, {
  MusicSearchIndex? music,
}) {
  bool musicShown() => !services.profileControl.musicHidden;
  final personal = services.personal;
  return [
    librarySearchSource(services.library, music: musicShown),
    if (music != null) musicSearchSource(music, shown: musicShown),
    audiobookSearchSource(() => services.audiobooks.books),
    if (personal != null) personalSearchSource(personal),
    twitchSearchSource(services),
  ];
}
