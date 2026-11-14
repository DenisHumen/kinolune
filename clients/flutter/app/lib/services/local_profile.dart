// SPDX-License-Identifier: MPL-2.0
//
// The name and avatar colour of the active profile as the first-run setup
// (UI-004) and Settings show them. On the Rust core they live in the
// core's profile (LIB-012, `LibraryProfiles`); a profile the UI-004 setup
// kept in Preferences is moved there once. Libraries without profiles keep
// it in Preferences.

import 'dart:ui' show Color;

import '../integration/library.dart' show LibraryProfile;

/// How the user watches (STRATEGY §1, modes A/B/C).
enum AppMode {
  /// A: the app alone, with folders and network shares.
  app,

  /// B: our own server (`alpha-media`) on the LAN.
  server,

  /// C: an existing Plex, Jellyfin or Emby server.
  mediaServer;

  static AppMode? fromName(String? name) =>
      values.where((m) => m.name == name).firstOrNull;
}

/// Avatar colours of the local profile, by index.
const kProfileAvatarColors = <Color>[
  Color(0xFF7C5CFF),
  Color(0xFF2EA8FF),
  Color(0xFF1FB980),
  Color(0xFFF5A524),
  Color(0xFFFF6B5C),
  Color(0xFFE85AAD),
  Color(0xFF8E9AAF),
  Color(0xFF00B8B8),
];

class LocalProfile {
  const LocalProfile({required this.name, this.avatar = 0});

  final String name;

  /// Index into [kProfileAvatarColors].
  final int avatar;

  Color get color =>
      kProfileAvatarColors[avatar.clamp(0, kProfileAvatarColors.length - 1)];

  /// The first letter of the name (code point), for the avatar circle.
  String get initial {
    final runes = name.trim().runes;
    return runes.isEmpty ? '' : String.fromCharCode(runes.first).toUpperCase();
  }

  Map<String, Object?> toJson() => {'name': name, 'avatar': avatar};

  /// The avatar as the core profile stores it (LIB-012).
  String get avatarRef => 'color:$avatar';

  /// The name and avatar of a core profile; null while it has no name.
  static LocalProfile? fromCore(LibraryProfile p) {
    if (p.name.trim().isEmpty) return null;
    final ref = p.avatar ?? '';
    final index = ref.startsWith('color:')
        ? int.tryParse(ref.substring('color:'.length))
        : null;
    return LocalProfile(name: p.name, avatar: index ?? 0);
  }

  /// Null for anything that is not a stored profile.
  static LocalProfile? fromJson(Object? json) {
    if (json is! Map) return null;
    final name = json['name'];
    final avatar = json['avatar'];
    if (name is! String || name.trim().isEmpty) return null;
    return LocalProfile(name: name, avatar: avatar is int ? avatar : 0);
  }

  @override
  bool operator ==(Object other) =>
      other is LocalProfile && other.name == name && other.avatar == avatar;

  @override
  int get hashCode => Object.hash(name, avatar);
}
