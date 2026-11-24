// SPDX-License-Identifier: MPL-2.0
//
// A new device set up from a phone (UI-046, ADR-NNNN): which app settings
// travel, how the phone collects them and the new device applies them,
// and the setup QR code the new device shows. Sources with their
// passwords are handled by the core (pa-sources `setup`), profiles with
// their PIN hashes by sync (SYNC-009).

import '../services/preferences.dart';

/// The setup QR code a new device shows: `kinolune://setup?v=1&c=…&n=…`.
class SetupCode {
  const SetupCode(this.code, {this.deviceName});

  /// The 6 digits.
  final String code;

  /// The new device's name, if the QR code has it.
  final String? deviceName;

  static const prefix = 'kinolune://setup?';

  /// Whether [text] (a scanned QR code) is a setup code, not a pairing one.
  static bool isSetupQr(String text) => text.trim().startsWith(prefix);

  /// A scanned QR code or 6 typed digits; null if it is neither.
  static SetupCode? parse(String text) {
    final t = text.trim();
    if (!t.startsWith(prefix)) {
      final digits = t.replaceAll(RegExp(r'[\s-]'), '');
      return RegExp(r'^\d{6}$').hasMatch(digits) ? SetupCode(digits) : null;
    }
    final Map<String, String> q;
    try {
      q = Uri.splitQueryString(t.substring(prefix.length));
    } on FormatException {
      return null;
    }
    final code = q['c'];
    if (q['v'] != '1' || code == null || !RegExp(r'^\d{6}$').hasMatch(code)) {
      return null;
    }
    final name = q['n']?.trim();
    return SetupCode(
      code,
      deviceName: name == null || name.isEmpty ? null : name,
    );
  }

  @override
  String toString() => 'SetupCode(***)';
}

/// Settings a new device takes over from the phone: how the user watches
/// and listens, not what belongs to one device (cache size, downloads,
/// what was dismissed, the search history) and no consents (sending
/// frames to trace.moe, the recording warning), which the user gives on
/// each device.
final Set<String> setupSettingKeys = {
  Preferences.locale,
  Preferences.appMode,
  Preferences.takeoverPause,
  Preferences.showTwitch,
  Preferences.twitchChatPauseAnimations,
  Preferences.hideSpoilers,
  Preferences.settingsAdvanced,
  Preferences.browserShowFileNames,
  Preferences.sshRsaKeys,
  Preferences.continuousPlayback,
  Preferences.skipSeconds,
  Preferences.skipMode,
  Preferences.trickplayGenerate,
  Preferences.animeDataUpdates,
  Preferences.defaultEpisodeOrder,
  Preferences.metadataLanguage,
  Preferences.musicGapless,
  Preferences.musicReplayGain,
  Preferences.subtitleSettings,
  Preferences.profilesPickerOnStart,
  Preferences.profilesAppLock,
  Preferences.profilesSearchLock,
  Preferences.twitchLiveNotifications,
  Preferences.twitchQuality,
  Preferences.twitchLatencyMode,
  Preferences.trackPolicy('default'),
  Preferences.audioProcessing('default'),
};

/// Longest value taken over, in characters.
const _maxValueLength = 16 * 1024;

/// The phone: the settings it hands a new device.
Future<Map<String, String>> collectSetupSettings(Preferences prefs) async {
  final out = <String, String>{};
  for (final key in setupSettingKeys) {
    final value = await prefs.read(key);
    if (value != null && value.length <= _maxValueLength) out[key] = value;
  }
  return out;
}

/// The new device: writes the phone's [settings] it knows; returns the
/// keys written. Anything else in the bundle is ignored.
Future<List<String>> applySetupSettings(
  Preferences prefs,
  Map<String, String> settings,
) async {
  final written = <String>[];
  for (final e in settings.entries) {
    if (!setupSettingKeys.contains(e.key) || e.value.length > _maxValueLength) {
      continue;
    }
    await prefs.write(e.key, e.value);
    written.add(e.key);
  }
  return written;
}
