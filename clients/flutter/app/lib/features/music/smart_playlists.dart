// SPDX-License-Identifier: MPL-2.0
//
// Smart playlists (MUS-003 [P1], 01 §4.11): a playlist stored as rules
// over genre, year, plays and format, evaluated over the music tracks
// whenever it is opened, so it follows the library. Kept per profile in
// Preferences as JSON (ADR-NNNN).

import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../../integration/library.dart';
import '../../services/preferences.dart';

/// What a rule looks at.
enum SmartField { genre, year, plays, format }

/// How a rule compares.
enum SmartOp { contains, notContains, equals, notEquals, atLeast, atMost }

/// Format rule values besides codec names (`flac`, `mp3`…).
const smartFormatLossless = 'lossless';
const smartFormatHiRes = 'hires';

/// Values offered for a format rule.
const smartFormatValues = [
  smartFormatLossless,
  smartFormatHiRes,
  'flac',
  'alac',
  'mp3',
  'aac',
  'opus',
  'vorbis',
  'pcm',
  'dsd',
];

/// Operators that make sense for [field].
List<SmartOp> smartOpsFor(SmartField field) => switch (field) {
  SmartField.genre => const [
    SmartOp.contains,
    SmartOp.notContains,
    SmartOp.equals,
    SmartOp.notEquals,
  ],
  SmartField.year || SmartField.plays => const [
    SmartOp.equals,
    SmartOp.atLeast,
    SmartOp.atMost,
  ],
  SmartField.format => const [SmartOp.equals, SmartOp.notEquals],
};

/// One condition: `genre contains "jazz"`, `year ≥ 1990`, `plays ≥ 5`,
/// `format is lossless`.
@immutable
class SmartRule {
  const SmartRule(this.field, this.op, this.value);

  final SmartField field;
  final SmartOp op;
  final String value;

  bool matches(MusicTrack t) => switch (field) {
    SmartField.genre => _text(t.genres),
    SmartField.year => _number(t.albumYear),
    SmartField.plays => _number(t.playCount),
    SmartField.format => _format(t),
  };

  bool _text(List<String> genres) {
    final v = value.trim().toLowerCase();
    final g = [for (final x in genres) x.toLowerCase()];
    return switch (op) {
      SmartOp.contains => g.any((x) => x.contains(v)),
      SmartOp.notContains => !g.any((x) => x.contains(v)),
      SmartOp.equals => g.contains(v),
      SmartOp.notEquals => !g.contains(v),
      _ => false,
    };
  }

  bool _number(int? n) {
    final v = int.tryParse(value.trim());
    if (v == null || n == null) return false;
    return switch (op) {
      SmartOp.equals => n == v,
      SmartOp.notEquals => n != v,
      SmartOp.atLeast => n >= v,
      SmartOp.atMost => n <= v,
      _ => false,
    };
  }

  bool _format(MusicTrack t) {
    final v = value.trim().toLowerCase();
    final hit = switch (v) {
      smartFormatLossless => t.isLossless,
      smartFormatHiRes => t.isHiRes,
      _ => t.codec?.toLowerCase() == v,
    };
    return switch (op) {
      SmartOp.equals => hit,
      SmartOp.notEquals => !hit,
      _ => false,
    };
  }

  Map<String, Object?> toJson() => {
    'field': field.name,
    'op': op.name,
    'value': value,
  };

  static SmartRule? fromJson(Object? json) {
    if (json is! Map) return null;
    final field = SmartField.values.asNameMap()[json['field']];
    final op = SmartOp.values.asNameMap()[json['op']];
    final value = json['value'];
    if (field == null || op == null || value is! String) return null;
    if (!smartOpsFor(field).contains(op)) return null;
    return SmartRule(field, op, value);
  }

  @override
  bool operator ==(Object other) =>
      other is SmartRule &&
      other.field == field &&
      other.op == op &&
      other.value == value;

  @override
  int get hashCode => Object.hash(field, op, value);
}

/// Order of a smart playlist.
enum SmartSort { title, mostPlayed, recentlyPlayed, year }

/// A playlist kept as rules.
@immutable
class SmartPlaylist {
  const SmartPlaylist({
    required this.id,
    required this.name,
    this.rules = const [],
    this.matchAll = true,
    this.sort = SmartSort.title,
    this.limit,
  });

  final String id;
  final String name;
  final List<SmartRule> rules;

  /// Every rule must match; `false`: any rule.
  final bool matchAll;
  final SmartSort sort;

  /// At most this many tracks; null: all.
  final int? limit;

  bool matches(MusicTrack t) {
    if (rules.isEmpty) return true;
    return matchAll ? rules.every((r) => r.matches(t)) : rules.any((r) => r.matches(t));
  }

  /// The tracks of [tracks] this playlist holds, in its order.
  List<MusicTrack> evaluate(List<MusicTrack> tracks) {
    final hits = tracks.where(matches).toList();
    int byTitle(MusicTrack a, MusicTrack b) =>
        a.title.toLowerCase().compareTo(b.title.toLowerCase());
    switch (sort) {
      case SmartSort.title:
        hits.sort(byTitle);
      case SmartSort.mostPlayed:
        hits.sort((a, b) {
          final c = b.playCount.compareTo(a.playCount);
          return c != 0 ? c : byTitle(a, b);
        });
      case SmartSort.recentlyPlayed:
        hits.sort((a, b) {
          final x = a.lastPlayedAt, y = b.lastPlayedAt;
          if (x == null && y == null) return byTitle(a, b);
          if (x == null) return 1;
          if (y == null) return -1;
          return y.compareTo(x);
        });
      case SmartSort.year:
        hits.sort((a, b) {
          final c = (a.albumYear ?? 1 << 30).compareTo(b.albumYear ?? 1 << 30);
          return c != 0 ? c : byTitle(a, b);
        });
    }
    final n = limit;
    return n != null && n > 0 && hits.length > n ? hits.sublist(0, n) : hits;
  }

  SmartPlaylist copyWith({
    String? name,
    List<SmartRule>? rules,
    bool? matchAll,
    SmartSort? sort,
    int? Function()? limit,
  }) => SmartPlaylist(
    id: id,
    name: name ?? this.name,
    rules: rules ?? this.rules,
    matchAll: matchAll ?? this.matchAll,
    sort: sort ?? this.sort,
    limit: limit == null ? this.limit : limit(),
  );

  Map<String, Object?> toJson() => {
    'id': id,
    'name': name,
    'rules': [for (final r in rules) r.toJson()],
    'matchAll': matchAll,
    'sort': sort.name,
    'limit': ?limit,
  };

  static SmartPlaylist? fromJson(Object? json) {
    if (json is! Map) return null;
    final id = json['id'];
    final name = json['name'];
    if (id is! String || name is! String) return null;
    final rules = json['rules'];
    final limit = json['limit'];
    return SmartPlaylist(
      id: id,
      name: name,
      rules: [
        if (rules is List)
          for (final r in rules) ?SmartRule.fromJson(r),
      ],
      matchAll: json['matchAll'] != false,
      sort: SmartSort.values.asNameMap()[json['sort']] ?? SmartSort.title,
      limit: limit is int && limit > 0 ? limit : null,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is SmartPlaylist &&
      other.id == id &&
      other.name == name &&
      listEquals(other.rules, rules) &&
      other.matchAll == matchAll &&
      other.sort == sort &&
      other.limit == limit;

  @override
  int get hashCode =>
      Object.hash(id, name, Object.hashAll(rules), matchAll, sort, limit);
}

/// Smart playlists of the active profile.
class SmartPlaylistStore {
  SmartPlaylistStore(
    this._preferences,
    this._profileId, {
    DateTime Function()? clock,
  }) : _clock = clock ?? DateTime.now;

  final Preferences _preferences;
  final Future<String?> Function() _profileId;
  final DateTime Function() _clock;

  /// The playlists, by name; listen to follow edits.
  final ValueNotifier<List<SmartPlaylist>> playlists = ValueNotifier(const []);

  Future<String> _key() async {
    String? id;
    try {
      id = await _profileId();
    } catch (e) {
      debugPrint('Smart playlists: no active profile: $e');
    }
    return Preferences.musicSmartPlaylists(id ?? 'default');
  }

  /// Reads the playlists of the active profile (again after a switch).
  Future<void> load() async {
    final raw = await _preferences.read(await _key());
    var list = <SmartPlaylist>[];
    if (raw != null && raw.isNotEmpty) {
      try {
        final json = jsonDecode(raw);
        if (json is List) {
          list = [for (final p in json) ?SmartPlaylist.fromJson(p)];
        }
      } catch (e) {
        debugPrint('Smart playlists not read: $e');
      }
    }
    playlists.value = _sorted(list);
  }

  static List<SmartPlaylist> _sorted(List<SmartPlaylist> l) =>
      [...l]..sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));

  Future<void> _write(List<SmartPlaylist> list) async {
    playlists.value = _sorted(list);
    await _preferences.write(
      await _key(),
      jsonEncode([for (final p in playlists.value) p.toJson()]),
    );
  }

  /// A new id for [create].
  String newId() => 'smart-${_clock().microsecondsSinceEpoch}';

  /// Adds [p] or replaces the playlist with its id.
  Future<void> save(SmartPlaylist p) => _write([
    for (final x in playlists.value)
      if (x.id != p.id) x,
    p,
  ]);

  Future<void> delete(String id) =>
      _write([...playlists.value.where((p) => p.id != id)]);

  void dispose() => playlists.dispose();
}
