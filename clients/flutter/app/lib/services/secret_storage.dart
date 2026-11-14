// SPDX-License-Identifier: MPL-2.0

/// Why the core keeps secrets in its encrypted file instead of the OS
/// keychain (`CoreSecretStoreInfo.fallbackKind`, CORE-007).
enum SecretFallbackKind {
  /// No keychain: no D-Bus session, no Secret Service (headless, minimal
  /// window managers, sandboxes).
  unavailable,

  /// The keychain is locked and unlocking it would need a prompt.
  locked,

  /// The keychain did not answer in time.
  timeout,

  /// The keychain answered with an error.
  failed;

  static SecretFallbackKind fromName(String? name) => switch (name) {
    'unavailable' => unavailable,
    'locked' => locked,
    'timeout' => timeout,
    _ => failed,
  };
}

/// Where the core keeps passwords and tokens: shown in Settings → Privacy
/// and, once, as a notice when it is the encrypted fallback file.
class SecretStorageInfo {
  const SecretStorageInfo({
    required this.backend,
    this.fallbackKind,
    this.fallbackReason,
    this.fallbackFile,
  });

  /// No core secret store (demo, v0 fallback, or it could not be opened).
  const SecretStorageInfo.unknown()
    : backend = '',
      fallbackKind = null,
      fallbackReason = null,
      fallbackFile = null;

  /// The core's backend name: `secret-service`, `keychain`,
  /// `credential-manager`, `platform`, `encrypted-file`; empty = unknown.
  final String backend;

  /// Set when [isFallback].
  final SecretFallbackKind? fallbackKind;

  /// Technical reason (for "Details"); no secrets.
  final String? fallbackReason;

  /// Path of the encrypted file when [isFallback].
  final String? fallbackFile;

  bool get known => backend.isNotEmpty;

  /// Secrets are in the encrypted file in the app data folder.
  bool get isFallback => backend == 'encrypted-file';
}
