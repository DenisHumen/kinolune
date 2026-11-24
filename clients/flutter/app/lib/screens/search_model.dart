// SPDX-License-Identifier: MPL-2.0
//
// State of the Search screen (UI-011): debounced as-you-type search and the
// local search history.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../integration/library.dart';
import '../services/preferences.dart';

/// Pause after the last keystroke before searching. With the core query
/// under 50 ms on 50 000 items (pa-catalog `tests/search.rs`), results
/// appear within 100 ms of the input.
const searchDebounce = Duration(milliseconds: 50);

/// Grouped search function, e.g. [Library.searchGrouped].
typedef GroupedSearch = Future<List<SearchGroup>> Function(String query);

/// Debounced as-you-type search. [update] on every edit; listeners are told
/// when [groups] (or [busy], [error]) change. A result of an older query is
/// dropped when a newer one was asked meanwhile.
class LibrarySearch extends ChangeNotifier {
  LibrarySearch(this._search, {this.debounce = searchDebounce});

  final GroupedSearch _search;
  final Duration debounce;

  Timer? _timer;
  int _generation = 0;
  bool _disposed = false;

  /// The trimmed query the shown [groups] belong to.
  String get query => _query;
  String _query = '';

  /// Results of [query]; empty when it is empty or nothing matched.
  List<SearchGroup> get groups => _groups;
  List<SearchGroup> _groups = const [];

  /// A search is pending or running.
  bool get busy => _busy;
  bool _busy = false;

  /// Why the last search failed, if it did.
  Object? get error => _error;
  Object? _error;

  /// Whether [query] was searched and nothing matched.
  bool get isEmptyResult =>
      _query.isNotEmpty && !_busy && _error == null && _groups.isEmpty;

  /// The text of the field changed: search after [debounce]. An empty
  /// text clears the results at once.
  void update(String text) {
    final q = text.trim();
    _timer?.cancel();
    if (q.isEmpty) {
      _generation++;
      _set(q, const [], busy: false, error: null);
      return;
    }
    if (q == _query && !_busy && _error == null) return;
    // A search still running for an older text is dropped.
    _generation++;
    _busy = true;
    _timer = Timer(debounce, () => _run(q));
    notifyListeners();
  }

  /// Searches [text] now (Enter, a history entry).
  Future<void> submit(String text) {
    _timer?.cancel();
    final q = text.trim();
    if (q.isEmpty) {
      update(q);
      return Future.value();
    }
    return _run(q);
  }

  Future<void> _run(String q) async {
    final gen = ++_generation;
    _busy = true;
    try {
      final groups = await _search(q);
      if (gen != _generation || _disposed) return;
      _set(q, groups, busy: false, error: null);
    } catch (e) {
      if (gen != _generation || _disposed) return;
      _set(q, const [], busy: false, error: e);
    }
  }

  void _set(
    String q,
    List<SearchGroup> groups, {
    required bool busy,
    required Object? error,
  }) {
    _query = q;
    _groups = groups;
    _busy = busy;
    _error = error;
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _timer?.cancel();
    super.dispose();
  }
}

/// Recent search queries, newest first, kept in [Preferences] on this
/// device only.
class SearchHistory extends ValueNotifier<List<String>> {
  SearchHistory(
    this._prefs, {
    this.max = 10,
    this.key = Preferences.searchHistory,
  }) : super(const []);

  final Preferences _prefs;

  /// Preferences key: each profile has its own history (UI-015).
  final String key;

  /// Most entries kept.
  final int max;

  /// Reads the saved history.
  Future<void> load() async {
    try {
      final raw = await _prefs.read(key);
      if (raw == null) return;
      final data = jsonDecode(raw);
      if (data is List) {
        value = data.whereType<String>().take(max).toList(growable: false);
      }
    } on FormatException {
      value = const [];
    }
  }

  /// Puts [query] first; an entry differing only in case is replaced.
  Future<void> add(String query) async {
    final q = query.trim();
    if (q.isEmpty) return;
    final lower = q.toLowerCase();
    await _save(
      [
        q,
        ...value.where((e) => e.toLowerCase() != lower),
      ].take(max).toList(growable: false),
    );
  }

  Future<void> remove(String query) =>
      _save(value.where((e) => e != query).toList(growable: false));

  Future<void> clear() => _save(const []);

  Future<void> _save(List<String> list) async {
    value = list;
    await _prefs.write(key, list.isEmpty ? null : jsonEncode(list));
  }
}

// ---------------------------------------------------------------- UI-045

/// Result groups of the unified search (UI-045), in screen order.
enum SearchSection {
  movies,
  shows,
  episodes,
  artists,
  albums,
  tracks,
  audiobooks,
  personal,
  channels,
}

/// Results of one [SearchSection], best first.
class SearchHits {
  const SearchHits(this.section, this.items, {this.more = false, this.music});

  final SearchSection section;

  /// [LibraryItem] for movies, shows, episodes and tracks; [MusicArtist],
  /// [MusicAlbum], [Audiobook], [PersonalHit] or a Twitch channel for the
  /// other sections.
  final List<Object> items;

  /// The group was cut at the limit: «Все» shows the rest.
  final bool more;

  /// The music index artists and albums belong to (their pages need it).
  final MusicIndex? music;

  @override
  String toString() => 'SearchHits(${section.name}, ${items.length})';
}

/// A video of a personal library (UI-025) found by the search.
class PersonalHit {
  const PersonalHit(this.library, this.video);

  final PersonalLibrary library;
  final PersonalVideo video;
}

/// One kind of content the unified search asks: the core index, the music
/// index, the books, personal media, Twitch.
class SearchSource {
  const SearchSource({
    required this.sections,
    required this.search,
    this.delay = Duration.zero,
  });

  /// Sections its results go to.
  final Set<SearchSection> sections;

  /// Results of a query, at most `limit` per section.
  final Future<List<SearchHits>> Function(String query, int limit) search;

  /// Extra wait after the debounce: a network source (Twitch) is not asked
  /// on every keystroke.
  final Duration delay;
}

/// Cuts [items] at [limit] for a section, telling whether more were found.
SearchHits cutHits(
  SearchSection section,
  List<Object> items,
  int limit, {
  MusicIndex? music,
}) => SearchHits(
  section,
  items.length > limit ? items.sublist(0, limit) : items,
  more: items.length > limit,
  music: music,
);

/// Debounced as-you-type search over every [SearchSource] (UI-045). Each
/// source answers on its own: the library index (under 100 ms on 50 000
/// items) is shown at once, Twitch joins when it answers. A result of an
/// older query is dropped.
class UnifiedSearch extends ChangeNotifier {
  UnifiedSearch(
    this.sources, {
    this.debounce = searchDebounce,
    this.perGroup = searchPerGroupDefault,
  });

  final List<SearchSource> sources;
  final Duration debounce;

  /// Results per group on the search screen.
  final int perGroup;

  Timer? _timer;
  int _generation = 0;
  bool _disposed = false;

  /// The trimmed query the shown [groups] belong to.
  String get query => _query;
  String _query = '';

  final Map<SearchSection, SearchHits> _hits = {};

  /// Non-empty groups of [query] in [SearchSection] order.
  List<SearchHits> get groups => [
    for (final s in SearchSection.values) ?_hits[s],
  ];

  /// A source has not answered yet.
  bool get busy => _busy;
  bool _busy = false;

  /// Why the search failed: set only when every source failed.
  Object? get error => _error;
  Object? _error;

  /// Whether [query] was searched and nothing matched.
  bool get isEmptyResult =>
      _query.isNotEmpty && !_busy && _error == null && _hits.isEmpty;

  /// The text of the field changed: search after [debounce]. An empty
  /// text clears the results at once.
  void update(String text) {
    final q = text.trim();
    _timer?.cancel();
    if (q.isEmpty) {
      _generation++;
      _query = '';
      _hits.clear();
      _busy = false;
      _error = null;
      _notify();
      return;
    }
    if (q == _query && !_busy && _error == null) return;
    _generation++;
    _busy = true;
    _timer = Timer(debounce, () => _run(q));
    _notify();
  }

  /// Searches [text] now (Enter, a history entry, a voice query).
  Future<void> submit(String text) {
    _timer?.cancel();
    final q = text.trim();
    if (q.isEmpty) {
      update(q);
      return Future.value();
    }
    return _run(q);
  }

  /// Everything [section] has for [query], for its «Все» page.
  Future<SearchHits?> all(
    SearchSection section,
    String query, {
    int limit = 500,
  }) async {
    for (final s in sources) {
      if (!s.sections.contains(section)) continue;
      final hits = await s.search(query.trim(), limit);
      for (final h in hits) {
        if (h.section == section) return h;
      }
    }
    return null;
  }

  Future<void> _run(String q) async {
    final gen = ++_generation;
    _busy = true;
    var fresh = true;
    var failed = 0;
    Object? lastError;

    void publish(List<SearchHits> hits) {
      if (gen != _generation || _disposed) return;
      if (fresh) {
        // The first answer of a new query replaces the old results.
        fresh = false;
        _hits.clear();
        _query = q;
        _error = null;
      }
      for (final h in hits) {
        if (h.items.isNotEmpty) _hits[h.section] = h;
      }
      _notify();
    }

    Future<void> one(SearchSource s) async {
      if (s.delay > Duration.zero) {
        await Future<void>.delayed(s.delay);
        if (gen != _generation || _disposed) return;
      }
      try {
        publish(await s.search(q, perGroup));
      } catch (e) {
        debugPrint('Search source failed: ${e.runtimeType}');
        failed++;
        lastError = e;
      }
    }

    await Future.wait([for (final s in sources) one(s)]);
    if (gen != _generation || _disposed) return;
    if (fresh) {
      // Nothing answered or every source failed: no results.
      _hits.clear();
      _query = q;
    }
    _busy = false;
    _error = sources.isNotEmpty && failed == sources.length ? lastError : null;
    _notify();
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _timer?.cancel();
    super.dispose();
  }
}
