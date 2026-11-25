// SPDX-License-Identifier: MPL-2.0
//
// Infinite scroll over a Helix list (TWI-034): pages by cursor, drops the
// duplicates of "floating" lists (research 06 §1.4), stops on the last page
// or a cursor seen twice, and keeps an error for "Retry" instead of
// throwing. A reset (a new language) drops answers still on the way.

import 'package:flutter/foundation.dart';

import 'twitch_models.dart';

class TwitchPager<T> extends ChangeNotifier {
  TwitchPager(this._fetch, {required this.keyOf, this.compare});

  final Future<TwitchPage<T>> Function(String? cursor) _fetch;

  /// Stable id of an item: the same id on a later page is a duplicate.
  final String Function(T item) keyOf;

  /// Order of the whole list (e.g. most viewers first); items that compare
  /// equal keep the order they arrived in.
  final int Function(T a, T b)? compare;

  final List<T> _items = [];
  final Map<String, int> _arrival = {};
  final Set<String> _cursors = {};
  String? _cursor;
  bool _started = false;
  bool _loading = false;
  bool _disposed = false;
  int _generation = 0;
  Object? _error;

  List<T> get items => List.unmodifiable(_items);
  bool get loading => _loading;

  /// The first page has not arrived yet.
  bool get initial => !_started;

  /// The last load failed; [loadMore] tries again.
  Object? get error => _error;

  /// More pages may follow.
  bool get hasMore => !_started || _cursor != null;

  /// Loaded and empty.
  bool get isEmpty => _started && _items.isEmpty && _error == null;

  /// Loads the next page; does nothing while loading or after the last.
  Future<void> loadMore() async {
    if (_loading || !hasMore) return;
    final generation = _generation;
    _loading = true;
    _error = null;
    _notify();
    try {
      final page = await _fetch(_cursor);
      if (generation != _generation) return;
      _started = true;
      for (final item in page.items) {
        final key = keyOf(item);
        if (_arrival.containsKey(key)) continue;
        _arrival[key] = _arrival.length;
        _items.add(item);
      }
      final cmp = compare;
      if (cmp != null) {
        _items.sort((a, b) {
          final c = cmp(a, b);
          return c != 0
              ? c
              : _arrival[keyOf(a)]!.compareTo(_arrival[keyOf(b)]!);
        });
      }
      final next = page.hasMore ? page.cursor : null;
      // A cursor seen before or an empty page ends the list: a misbehaving
      // API cannot make us loop.
      _cursor = next != null && page.items.isNotEmpty && _cursors.add(next)
          ? next
          : null;
    } catch (e) {
      if (generation != _generation) return;
      _error = e;
    } finally {
      if (generation == _generation) {
        _loading = false;
        _notify();
      }
    }
  }

  /// Forgets everything and loads the first page again.
  Future<void> reset() {
    _generation++;
    _items.clear();
    _arrival.clear();
    _cursors.clear();
    _cursor = null;
    _started = false;
    _loading = false;
    _error = null;
    return loadMore();
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
