// SPDX-License-Identifier: MPL-2.0
import 'dart:math';

import 'package:flutter/foundation.dart';

import '../engine/player_engine.dart';
import '../play_request.dart';

/// Where the tracks of a queue came from: shown as "Up next from: <label>"
/// (an album, an artist, a playlist…).
@immutable
class QueueSource {
  const QueueSource(this.label, {this.id});

  /// Album, artist or playlist name.
  final String label;

  /// App-defined id of the source (e.g. `album:42`), if any.
  final String? id;

  Map<String, Object?> toJson() => {'label': label, 'id': ?id};

  static QueueSource? fromJson(Object? json) {
    if (json is! Map) return null;
    final label = json['label'];
    if (label is! String || label.isEmpty) return null;
    final id = json['id'];
    return QueueSource(label, id: id is String ? id : null);
  }

  @override
  bool operator ==(Object other) =>
      other is QueueSource && other.label == label && other.id == id;

  @override
  int get hashCode => Object.hash(label, id);
}

/// One track of the queue. [userAdded]: added with Play Next or Add to
/// Queue; such tracks form the "Playing next" block right after the
/// current track, before the rest of the [QueueSource].
@immutable
class QueueEntry {
  const QueueEntry._(this.key, this.request, {this.userAdded = false});

  /// Unique in the queue (the same track may be queued twice).
  final int key;
  final PlayRequest request;
  final bool userAdded;

  QueueEntry _withUserAdded(bool v) =>
      v == userAdded ? this : QueueEntry._(key, request, userAdded: v);
}

/// Music queue: the tracks, the play order (shuffled or not), the current
/// position, the repeat mode, tracks the user queued (Play Next, Add to
/// Queue) and what was played recently. Pure logic, no I/O; [toJson] and
/// [restore] keep it across app restarts.
class MusicQueue {
  MusicQueue({Random? random}) : _random = random ?? Random();

  /// How many recently played tracks [recent] keeps.
  static const recentLimit = 100;

  final Random _random;

  /// Play order.
  List<QueueEntry> _play = const [];

  /// The order to return to when shuffle is turned off; null while not
  /// shuffled.
  List<QueueEntry>? _unshuffled;
  int _position = 0;
  bool _shuffle = false;
  QueueRepeat _repeat = QueueRepeat.off;
  QueueSource? _source;
  List<PlayRequest> _recent = const [];
  int _nextKey = 0;
  int _revision = 0;

  QueueEntry _entry(PlayRequest r, {bool userAdded = false}) =>
      QueueEntry._(_nextKey++, r, userAdded: userAdded);

  /// Changes on every edit of the tracks or their order (not on a plain
  /// move to another track): the app saves the queue when it changes.
  int get revision => _revision;

  /// Tracks in their original (unshuffled) order.
  List<PlayRequest> get items =>
      List.unmodifiable([for (final e in _unshuffled ?? _play) e.request]);

  /// Tracks in play order.
  List<PlayRequest> get ordered =>
      List.unmodifiable([for (final e in _play) e.request]);

  /// Entries in play order.
  List<QueueEntry> get entries => List.unmodifiable(_play);

  /// Index into [ordered] of the current track.
  int get position => _position;
  bool get shuffle => _shuffle;
  QueueRepeat get repeat => _repeat;
  bool get isEmpty => _play.isEmpty;
  int get length => _play.length;

  /// What the queue was started from ("Up next from: …").
  QueueSource? get source => _source;

  PlayRequest? get current => _play.isEmpty ? null : _play[_position].request;

  /// Tracks after the current one, in play order.
  List<PlayRequest> get upNext => [
    for (var i = _position + 1; i < _play.length; i++) _play[i].request,
  ];

  /// Entries after the current one, in play order.
  List<QueueEntry> get upNextEntries =>
      List.unmodifiable(_play.sublist(min(_position + 1, _play.length)));

  /// How many of [upNextEntries] (from the start) the user queued.
  int get queuedCount {
    var n = 0;
    for (var i = _position + 1; i < _play.length && _play[i].userAdded; i++) {
      n++;
    }
    return n;
  }

  /// Already played tracks of this queue, in play order.
  List<PlayRequest> get history => [
    for (var i = 0; i < _position; i++) _play[i].request,
  ];

  /// Recently played tracks across queues, most recent first (the
  /// "History" of Now Playing).
  List<PlayRequest> get recent => List.unmodifiable(_recent);

  /// Notes that [r] started playing ([recent]); a track already at the
  /// top is not added twice.
  void recordPlayed(PlayRequest r) {
    if (_recent.isNotEmpty && _same(_recent.first, r)) return;
    _recent = [
      r,
      ..._recent.where((x) => !_same(x, r)),
    ].take(recentLimit).toList();
    _revision++;
  }

  static bool _same(PlayRequest a, PlayRequest b) =>
      a.itemId != null ? a.itemId == b.itemId : a.uri == b.uri;

  /// Forgets [recent].
  void clearRecent() {
    _recent = const [];
    _revision++;
  }

  /// Replaces the queue and starts at [index] (index into [items]).
  void replace(List<PlayRequest> items, {int index = 0, QueueSource? source}) {
    _source = source;
    _revision++;
    final list = [for (final r in items) _entry(r)];
    if (list.isEmpty) {
      _play = const [];
      _unshuffled = null;
      _position = 0;
      return;
    }
    index = index.clamp(0, list.length - 1);
    if (_shuffle) {
      _unshuffled = list;
      _play = [
        list[index],
        ..._shuffledExcept(list, {list[index].key}),
      ];
      _position = 0;
    } else {
      _unshuffled = null;
      _play = list;
      _position = index;
    }
  }

  /// Empties the queue (keeps [recent], shuffle and repeat).
  void clear() => replace(const []);

  List<QueueEntry> _shuffledExcept(List<QueueEntry> list, Set<int> keys) => [
    for (final e in list)
      if (!keys.contains(e.key)) e,
  ]..shuffle(_random);

  /// Turns shuffle on/off, keeping the current track playing and the
  /// tracks the user queued right after it.
  void setShuffle(bool on) {
    if (on == _shuffle) return;
    _shuffle = on;
    _revision++;
    if (_play.isEmpty) {
      _unshuffled = null;
      return;
    }
    final cur = _play[_position];
    final queued = _play.sublist(_position + 1, _position + 1 + queuedCount);
    final keep = {cur.key, for (final e in queued) e.key};
    if (on) {
      _unshuffled = _play;
      _play = [cur, ...queued, ..._shuffledExcept(_play, keep)];
      _position = 0;
    } else {
      final rest = [
        for (final e in _unshuffled ?? _play)
          if (!keep.contains(e.key) || e.key == cur.key) e,
      ];
      final at = rest.indexWhere((e) => e.key == cur.key);
      _play = [...rest.sublist(0, at + 1), ...queued, ...rest.sublist(at + 1)];
      _position = at;
      _unshuffled = null;
    }
  }

  void setRepeat(QueueRepeat mode) {
    _repeat = mode;
    _revision++;
  }

  /// off → all → one → off.
  QueueRepeat cycleRepeat() {
    setRepeat(
      QueueRepeat.values[(_repeat.index + 1) % QueueRepeat.values.length],
    );
    return _repeat;
  }

  /// Sets the current position (index into [ordered]), e.g. from the engine.
  void moveTo(int position) {
    if (_play.isEmpty) return;
    _position = position.clamp(0, _play.length - 1);
  }

  /// Position of the next track for a user "next" press, or `null` at the
  /// end without repeat. (Repeat-one does not trap the user on "next".)
  int? nextPosition() {
    if (_play.isEmpty) return null;
    if (_position + 1 < _play.length) return _position + 1;
    return _repeat == QueueRepeat.off ? null : 0;
  }

  /// Position of the previous track, or `null` at the start without repeat.
  int? previousPosition() {
    if (_play.isEmpty) return null;
    if (_position > 0) return _position - 1;
    return _repeat == QueueRepeat.off ? null : _play.length - 1;
  }

  void _insertUnshuffled(List<QueueEntry> added) {
    final u = _unshuffled;
    if (u == null) return;
    final cur = _play[_position].key;
    final at = u.indexWhere((e) => e.key == cur);
    _unshuffled = [...u.sublist(0, at + 1), ...added, ...u.sublist(at + 1)];
  }

  /// Play Next: [tracks] play right after the current one. Returns the
  /// play-order index of the first inserted track (an empty queue just
  /// becomes [tracks], index 0).
  int playNext(List<PlayRequest> tracks) {
    if (tracks.isEmpty) return _position + 1;
    if (_play.isEmpty) {
      replace(tracks);
      return 0;
    }
    final added = [for (final r in tracks) _entry(r, userAdded: true)];
    final at = _position + 1;
    _play = [..._play.sublist(0, at), ...added, ..._play.sublist(at)];
    _insertUnshuffled(added);
    _revision++;
    return at;
  }

  /// Add to Queue: [tracks] play after the tracks already queued by the
  /// user, before the rest of the source. Returns the play-order index of
  /// the first inserted track.
  int addToQueue(List<PlayRequest> tracks) {
    if (tracks.isEmpty) return _position + 1 + queuedCount;
    if (_play.isEmpty) {
      replace(tracks);
      return 0;
    }
    final added = [for (final r in tracks) _entry(r, userAdded: true)];
    final at = _position + 1 + queuedCount;
    _play = [..._play.sublist(0, at), ...added, ..._play.sublist(at)];
    _insertUnshuffled(added);
    _revision++;
    return at;
  }

  /// Moves up-next entry [from] to index [to] (both indices into
  /// [upNextEntries]). A track dropped among the queued ones becomes
  /// queued, one dropped below them becomes part of the source.
  void moveUpNext(int from, int to) {
    final n = _play.length - _position - 1;
    if (from < 0 || from >= n || to < 0 || to >= n || from == to) return;
    final base = _position + 1;
    final up = _play.sublist(base);
    var e = up.removeAt(from);
    var queued = 0;
    while (queued < up.length && up[queued].userAdded) {
      queued++;
    }
    if (to < queued) e = e._withUserAdded(true);
    if (to > queued) e = e._withUserAdded(false);
    up.insert(to, e);
    _play = [..._play.sublist(0, base), ...up];
    final u = _unshuffled;
    if (u != null) {
      _unshuffled = [for (final x in u) x.key == e.key ? e : x];
    }
    _revision++;
  }

  /// Removes up-next entry [index] (index into [upNextEntries]).
  void removeUpNext(int index) {
    final at = _position + 1 + index;
    if (index < 0 || at >= _play.length) return;
    final key = _play[at].key;
    _play = [..._play]..removeAt(at);
    final u = _unshuffled;
    if (u != null) _unshuffled = [...u.where((e) => e.key != key)];
    _revision++;
  }

  /// Removes every track after the current one ("Clear queue").
  void clearUpNext() {
    if (_play.length <= _position + 1) return;
    final gone = {for (final e in _play.sublist(_position + 1)) e.key};
    _play = _play.sublist(0, _position + 1);
    final u = _unshuffled;
    if (u != null) _unshuffled = [...u.where((e) => !gone.contains(e.key))];
    _revision++;
  }

  // -- persistence ------------------------------------------------------------

  /// Version of [toJson].
  static const jsonVersion = 1;

  /// The whole queue as JSON (tracks, order, position, modes, source,
  /// recent): the app keeps it per profile (MUS-003).
  Map<String, Object?> toJson() {
    final index = {for (var i = 0; i < _play.length; i++) _play[i].key: i};
    return {
      'v': jsonVersion,
      'tracks': [
        for (final e in _play)
          {...musicRequestToJson(e.request), if (e.userAdded) 'queued': true},
      ],
      'unshuffled': ?_unshuffled?.map((e) => index[e.key]).toList(),
      'position': _position,
      'shuffle': _shuffle,
      'repeat': _repeat.name,
      'source': ?_source?.toJson(),
      'recent': [for (final r in _recent) musicRequestToJson(r)],
    };
  }

  /// Replaces the state with [json] from [toJson]; [resolve] may update
  /// or drop (null) each track, e.g. a file no longer in the library.
  /// Returns `false` (and leaves the queue as it was) for unreadable
  /// data.
  bool restore(
    Map<String, Object?> json, {
    PlayRequest? Function(PlayRequest)? resolve,
  }) {
    if (json['v'] != jsonVersion) return false;
    final tracks = json['tracks'];
    if (tracks is! List) return false;
    PlayRequest? read(Object? t) {
      final r = musicRequestFromJson(t);
      if (r == null) return null;
      return resolve == null ? r : resolve(r);
    }

    // Entries by stored index; dropped tracks stay null.
    final stored = <QueueEntry?>[
      for (final t in tracks)
        switch (read(t)) {
          final r? => _entry(r, userAdded: t is Map && t['queued'] == true),
          null => null,
        },
    ];
    var position = json['position'] is int ? json['position'] as int : 0;
    position = position.clamp(0, max(0, stored.length - 1));
    // The current track was dropped: continue with the next one left.
    var newPosition = 0;
    for (var i = 0; i < stored.length; i++) {
      if (stored[i] == null) continue;
      if (i < position) newPosition++;
    }
    _play = stored.whereType<QueueEntry>().toList();
    _position = _play.isEmpty ? 0 : min(newPosition, _play.length - 1);
    _shuffle = json['shuffle'] == true;
    final order = json['unshuffled'];
    if (_shuffle && order is List) {
      final u = <QueueEntry>[
        for (final i in order)
          if (i is int && i >= 0 && i < stored.length) ?stored[i],
      ];
      // Anything the stored order missed keeps its play-order place.
      final seen = {for (final e in u) e.key};
      _unshuffled = [...u, ..._play.where((e) => !seen.contains(e.key))];
    } else {
      _unshuffled = _shuffle ? List.of(_play) : null;
    }
    _repeat = QueueRepeat.values.firstWhere(
      (m) => m.name == json['repeat'],
      orElse: () => QueueRepeat.off,
    );
    _source = QueueSource.fromJson(json['source']);
    final recent = json['recent'];
    _recent = [
      if (recent is List)
        for (final t in recent) ?read(t),
    ];
    _revision++;
    return true;
  }
}

/// The music fields of [r] as JSON (no video-only fields).
Map<String, Object?> musicRequestToJson(PlayRequest r) => {
  'uri': r.uri,
  'title': r.title,
  'id': ?r.itemId,
  'artist': ?r.artist,
  'album': ?r.album,
  'albumId': ?r.albumId,
  'art': ?r.artworkUri,
  'durationMs': ?r.durationMs,
};

/// Reads [musicRequestToJson]; null for anything else.
PlayRequest? musicRequestFromJson(Object? json) {
  if (json is! Map) return null;
  final uri = json['uri'];
  final title = json['title'];
  if (uri is! String || uri.isEmpty || title is! String) return null;
  String? s(String k) => json[k] is String ? json[k] as String : null;
  return PlayRequest(
    uri: uri,
    title: title,
    itemId: s('id'),
    artist: s('artist'),
    album: s('album'),
    albumId: s('albumId'),
    artworkUri: s('art'),
    durationMs: json['durationMs'] is int ? json['durationMs'] as int : null,
  );
}
