// SPDX-License-Identifier: MPL-2.0
//
// Trailer settings (UI-036, ADR-0100): when the banner of a movie or show
// page plays its trailer by itself, and whether internet trailers (TMDB
// videos on YouTube or Vimeo) are used at all. Both are device settings in
// [Preferences]; defaults: autoplay always, internet trailers on.

import '../services/preferences.dart';

/// When the banner plays the trailer by itself (muted). The «Трейлер»
/// button and «вверх» on a TV play it in any mode.
enum TrailerAutoplay {
  always,

  /// Not over mobile data (cellular: the trailer streams from the
  /// internet or the server).
  wifiOnly,
  never;

  static TrailerAutoplay parse(String? name) =>
      values.where((v) => v.name == name).firstOrNull ?? always;
}

/// The trailer settings of this device.
class TrailerSettings {
  const TrailerSettings({
    this.autoplay = TrailerAutoplay.always,
    this.internet = true,
  });

  final TrailerAutoplay autoplay;

  /// TMDB videos (YouTube, Vimeo) may be looked up and played; off = only
  /// the user's own trailer files and the media server's trailers.
  final bool internet;

  /// `Preferences` key of [autoplay], a [TrailerAutoplay] name.
  static const autoplayKey = 'trailers_autoplay';

  /// `Preferences` key of [internet]: `false` = off.
  static const internetKey = 'trailers_internet';

  static Future<TrailerSettings> read(Preferences prefs) async =>
      TrailerSettings(
        autoplay: TrailerAutoplay.parse(await prefs.read(autoplayKey)),
        internet: await prefs.read(internetKey) != 'false',
      );

  static Future<void> writeAutoplay(Preferences prefs, TrailerAutoplay v) =>
      prefs.write(autoplayKey, v == TrailerAutoplay.always ? null : v.name);

  static Future<void> writeInternet(Preferences prefs, bool on) =>
      prefs.write(internetKey, on ? null : 'false');
}
