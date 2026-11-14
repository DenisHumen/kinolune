// SPDX-License-Identifier: MPL-2.0
//
// Viewer chat settings (TWI-024, research 06 §3.3): block list, word
// filter, mention highlighting and "hide bots". Kept per profile in
// Preferences as JSON; the core compiles and applies them to new
// messages (regular expressions run in Rust: linear time, no freezes).

import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../../../services/preferences.dart';

/// One word-filter rule.
@immutable
class ChatWordRule {
  const ChatWordRule(this.pattern, {this.regex = false, this.hide = false});

  /// A word or phrase, or a regular expression when [regex].
  final String pattern;
  final bool regex;

  /// Hide the whole message; `false`: mask the match with `*`.
  final bool hide;

  Map<String, Object?> toJson() => {
    'pattern': pattern,
    if (regex) 'regex': true,
    if (hide) 'hide': true,
  };

  static ChatWordRule? fromJson(Object? json) {
    if (json is! Map) return null;
    final p = json['pattern'];
    if (p is! String || p.trim().isEmpty) return null;
    return ChatWordRule(
      p,
      regex: json['regex'] == true,
      hide: json['hide'] == true,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is ChatWordRule &&
      other.pattern == pattern &&
      other.regex == regex &&
      other.hide == hide;

  @override
  int get hashCode => Object.hash(pattern, regex, hide);
}

/// Normalises a login typed by the user: no `@`, no spaces, lower case.
String chatLoginKey(String s) =>
    s.trim().replaceFirst(RegExp('^@+'), '').toLowerCase();

/// What the viewer configured.
@immutable
class ChatSettings {
  const ChatSettings({
    this.blocked = const [],
    this.words = const [],
    this.highlightOwnName = true,
    this.highlightKeywords = const [],
    this.hideBots = false,
    this.extraBots = const [],
  });

  /// Blocked logins, lower case.
  final List<String> blocked;
  final List<ChatWordRule> words;

  /// Messages naming the signed-in user are highlighted.
  final bool highlightOwnName;

  /// More words that highlight a message.
  final List<String> highlightKeywords;

  /// Hide the bots the core knows plus [extraBots].
  final bool hideBots;
  final List<String> extraBots;

  bool isBlocked(String login) => blocked.contains(chatLoginKey(login));

  ChatSettings copyWith({
    List<String>? blocked,
    List<ChatWordRule>? words,
    bool? highlightOwnName,
    List<String>? highlightKeywords,
    bool? hideBots,
    List<String>? extraBots,
  }) => ChatSettings(
    blocked: blocked ?? this.blocked,
    words: words ?? this.words,
    highlightOwnName: highlightOwnName ?? this.highlightOwnName,
    highlightKeywords: highlightKeywords ?? this.highlightKeywords,
    hideBots: hideBots ?? this.hideBots,
    extraBots: extraBots ?? this.extraBots,
  );

  /// [login] added to the block list (once).
  ChatSettings block(String login) {
    final key = chatLoginKey(login);
    if (key.isEmpty || blocked.contains(key)) return this;
    return copyWith(blocked: [...blocked, key]);
  }

  ChatSettings unblock(String login) {
    final key = chatLoginKey(login);
    return copyWith(blocked: [...blocked.where((b) => b != key)]);
  }

  Map<String, Object?> toJson() => {
    'v': 1,
    'blocked': blocked,
    'words': [for (final w in words) w.toJson()],
    'highlightOwnName': highlightOwnName,
    'highlightKeywords': highlightKeywords,
    'hideBots': hideBots,
    'extraBots': extraBots,
  };

  /// Reads what [toJson] wrote; anything malformed falls back to defaults.
  factory ChatSettings.fromJson(Object? json) {
    if (json is! Map) return const ChatSettings();
    List<String> strings(Object? v) => v is List
        ? [
            for (final s in v)
              if (s is String && s.trim().isNotEmpty) s.trim(),
          ]
        : const [];
    final words = json['words'];
    return ChatSettings(
      blocked: [for (final b in strings(json['blocked'])) chatLoginKey(b)],
      words: words is List
          ? [
              for (final w in words) ?ChatWordRule.fromJson(w),
            ]
          : const [],
      highlightOwnName: json['highlightOwnName'] != false,
      highlightKeywords: strings(json['highlightKeywords']),
      hideBots: json['hideBots'] == true,
      extraBots: [for (final b in strings(json['extraBots'])) chatLoginKey(b)],
    );
  }

  @override
  bool operator ==(Object other) =>
      other is ChatSettings &&
      listEquals(other.blocked, blocked) &&
      listEquals(other.words, words) &&
      other.highlightOwnName == highlightOwnName &&
      listEquals(other.highlightKeywords, highlightKeywords) &&
      other.hideBots == hideBots &&
      listEquals(other.extraBots, extraBots);

  @override
  int get hashCode => Object.hash(
    Object.hashAll(blocked),
    Object.hashAll(words),
    highlightOwnName,
    Object.hashAll(highlightKeywords),
    hideBots,
    Object.hashAll(extraBots),
  );
}

/// Chat settings of the active profile (ADR-0055: state per profile),
/// stored in [Preferences] under one key per profile.
class ChatSettingsStore {
  ChatSettingsStore(this._preferences, this._profileId);

  final Preferences _preferences;

  /// Id of the active profile; null for a library without profiles.
  final Future<int?> Function() _profileId;

  /// Settings of the active profile; listen to follow edits.
  final ValueNotifier<ChatSettings> current = ValueNotifier(
    const ChatSettings(),
  );
  int? _loadedFor;
  var _loaded = false;

  Future<String> _key() async {
    int? id;
    try {
      id = await _profileId();
    } catch (e) {
      debugPrint('Chat settings: no active profile: $e');
    }
    _loadedFor = id;
    return Preferences.twitchChatSettings(id?.toString() ?? 'default');
  }

  /// Loads the settings of the active profile (again after a profile
  /// switch).
  Future<ChatSettings> load() async {
    final raw = await _preferences.read(await _key());
    ChatSettings s;
    try {
      s = raw == null ? const ChatSettings() : ChatSettings.fromJson(
        jsonDecode(raw),
      );
    } on FormatException {
      s = const ChatSettings();
    }
    _loaded = true;
    current.value = s;
    return s;
  }

  /// Loads once; later calls return what is in memory unless the active
  /// profile changed.
  Future<ChatSettings> ensureLoaded() async {
    if (_loaded) {
      int? id;
      try {
        id = await _profileId();
      } catch (_) {}
      if (id == _loadedFor) return current.value;
    }
    return load();
  }

  Future<void> save(ChatSettings settings) async {
    current.value = settings;
    final key = _loaded
        ? Preferences.twitchChatSettings(_loadedFor?.toString() ?? 'default')
        : await _key();
    await _preferences.write(key, jsonEncode(settings.toJson()));
  }

  void dispose() => current.dispose();
}
