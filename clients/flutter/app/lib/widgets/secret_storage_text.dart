// SPDX-License-Identifier: MPL-2.0

import '../branding.dart';
import '../l10n/l10n.dart';
import '../services/secret_storage.dart';

/// "is locked", "did not respond"… — the end of "because the system
/// keychain …".
String secretFallbackWhy(AppLocalizations l, SecretFallbackKind? kind) =>
    switch (kind) {
      SecretFallbackKind.unavailable => l.secretsWhyUnavailable,
      SecretFallbackKind.locked => l.secretsWhyLocked,
      SecretFallbackKind.timeout => l.secretsWhyTimeout,
      SecretFallbackKind.failed || null => l.secretsWhyFailed,
    };

/// Product name of a secure backend of the core (not translated).
String secretBackendName(String backend) => switch (backend) {
  'secret-service' => 'Secret Service', // l10n-ignore
  'keychain' => 'Keychain', // l10n-ignore
  'credential-manager' => 'Windows Credential Manager', // l10n-ignore
  'platform' => 'Android Keystore', // l10n-ignore
  _ => backend,
};

/// Settings → Privacy: where secrets are.
String secretStorageText(AppLocalizations l, SecretStorageInfo info) =>
    info.isFallback
    ? l.settingsPrivacySecretsFile(
        secretFallbackWhy(l, info.fallbackKind),
        info.fallbackFile ?? '',
      )
    : l.settingsPrivacySecretsKeychain(secretBackendName(info.backend));

/// The one-time notice of the encrypted-file fallback.
String secretFallbackNotice(AppLocalizations l, SecretStorageInfo info) =>
    l.secretsNoticeFallback(kAppName, secretFallbackWhy(l, info.fallbackKind));
