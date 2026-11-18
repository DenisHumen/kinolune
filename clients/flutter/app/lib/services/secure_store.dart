// SPDX-License-Identifier: MPL-2.0

import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

/// Secrets of the app (the TMDB key) in the platform keystore: Android
/// Keystore-backed storage, libsecret on Linux. If the keystore is not
/// available (e.g. no Secret Service on a headless desktop) or does not
/// answer within [PlatformSecureStore.callTimeout] (a locked keyring waiting
/// for an unlock prompt nobody sees), values are kept in memory for the
/// session only.
abstract class SecureStore {
  Future<String?> read(String key);
  Future<void> write(String key, String? value);

  static const tmdbKey = 'tmdb_api_key';

  /// The user's own TheTVDB key and personal Fanart.tv key (META-012).
  static const tvdbKey = 'tvdb_api_key';
  static const tvdbPin = 'tvdb_pin';
  static const fanartKey = 'fanart_api_key';

  /// Where app versions before LIB-018 kept SMB passwords. Read once to move
  /// them into the core's secret store, then deleted
  /// (`AppServices.migrateLegacySmbPasswords`).
  static String smbPassword(String sourceId) => 'smb_password_$sourceId';
}

class PlatformSecureStore implements SecureStore {
  PlatformSecureStore();

  /// Longest wait for the keystore; libsecret can wait forever for an
  /// unlock prompt (CORE-007).
  static const callTimeout = Duration(seconds: 5);

  /// macOS (PKG-019): the login keychain. The data protection keychain needs
  /// a keychain-access-groups entitlement tied to a Team ID, which the
  /// ad-hoc signed GitHub build does not have.
  final _storage = const FlutterSecureStorage(
    mOptions: MacOsOptions(usesDataProtectionKeychain: false),
  );
  final _memory = <String, String>{};
  bool _broken = false;
  Future<bool>? _probe;

  /// libsecret aborts the process (uncatchable C++ exception) when no Secret
  /// Service is running, so on Linux check D-Bus for one first.
  Future<bool> _available() => _probe ??= _probeSecretService();

  static Future<bool> _probeSecretService() async {
    if (kIsWeb || !Platform.isLinux) return true;
    for (final method in ['ListNames', 'ListActivatableNames']) {
      try {
        final r = await Process.run('dbus-send', [
          '--session',
          '--print-reply',
          '--dest=org.freedesktop.DBus',
          '/org/freedesktop/DBus',
          'org.freedesktop.DBus.$method',
        ]).timeout(const Duration(seconds: 2));
        if (r.exitCode == 0 &&
            '${r.stdout}'.contains('org.freedesktop.secrets')) {
          if (await _defaultKeyringLocked()) {
            debugPrint(
              'The default keyring is locked: secrets are kept for this '
              'session only.',
            );
            return false;
          }
          return true;
        }
      } catch (_) {
        // dbus-send missing or no session bus.
      }
    }
    debugPrint(
      'No Secret Service on D-Bus: secrets are kept for this session only.',
    );
    return false;
  }

  /// libsecret runs on the UI thread and waits for an unlock prompt when
  /// the keyring is locked; on a headless or HTPC session that prompt never
  /// comes and the app freezes (CORE-007). Reading `Locked` never prompts.
  /// A keyring that does not answer counts as locked.
  static Future<bool> _defaultKeyringLocked() async {
    try {
      final r = await Process.run('dbus-send', [
        '--session',
        '--print-reply',
        '--reply-timeout=2000',
        '--dest=org.freedesktop.secrets',
        '/org/freedesktop/secrets/aliases/default',
        'org.freedesktop.DBus.Properties.Get',
        'string:org.freedesktop.Secret.Collection',
        'string:Locked',
      ]).timeout(const Duration(seconds: 3));
      if (r.exitCode != 0) {
        // A daemon that does not answer; else no default keyring yet
        // (libsecret creates it on the first write).
        const noReply = 'Did not receive a reply'; // l10n-ignore
        return '${r.stderr}'.contains('NoReply') ||
            '${r.stderr}'.contains(noReply);
      }
      return '${r.stdout}'.contains('boolean true');
    } on TimeoutException {
      return true;
    } catch (_) {
      return false;
    }
  }

  @override
  Future<String?> read(String key) async {
    if (!_broken && await _available()) {
      try {
        return await _storage.read(key: key).timeout(callTimeout);
      } catch (e) {
        _broken = true;
        debugPrint('SecureStore unavailable, using memory: $e');
      }
    }
    return _memory[key];
  }

  @override
  Future<void> write(String key, String? value) async {
    if (!_broken && await _available()) {
      try {
        if (value == null) {
          await _storage.delete(key: key).timeout(callTimeout);
        } else {
          await _storage.write(key: key, value: value).timeout(callTimeout);
        }
        return;
      } catch (e) {
        _broken = true;
        debugPrint('SecureStore unavailable, using memory: $e');
      }
    }
    if (value == null) {
      _memory.remove(key);
    } else {
      _memory[key] = value;
    }
  }
}

class MemorySecureStore implements SecureStore {
  final values = <String, String>{};

  @override
  Future<String?> read(String key) async => values[key];

  @override
  Future<void> write(String key, String? value) async {
    if (value == null) {
      values.remove(key);
    } else {
      values[key] = value;
    }
  }
}
