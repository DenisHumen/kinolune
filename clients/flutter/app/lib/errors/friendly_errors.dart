// SPDX-License-Identifier: MPL-2.0
//
// Plain-language errors (UI-044, ADR-0121): every error the core returns
// carries a stable code (`pa_runtime::error_codes`, CORE-009); the app
// shows one sentence for it and one «Исправить» action. The code itself and
// the core's English message are only under «Подробнее» and in the report
// (UI-017). `test/friendly_errors_test.dart` checks that every common code
// of the core has ru/en text and an action here.

import 'package:pa_core/pa_core.dart';

import '../l10n/l10n.dart';
import '../offline/offline_downloads.dart' show OfflineException;
import '../sources/source_manager.dart';
import '../sync/device_sync.dart' show SyncException;

/// The one thing the «Исправить» button does.
enum ErrorFix {
  /// Try the same again (network, busy, keychain unlocked meanwhile).
  retry,

  /// The source's form: address, folder, user name and password.
  editSource,

  /// The list of sources.
  openSources,

  /// Downloads and storage: delete what is not needed.
  freeSpace,

  /// Settings.
  openSettings,

  /// Devices and sync.
  openSync,

  /// Updates (About).
  openUpdates,

  /// «Состояние» (UI-017): save the report for an issue.
  report,
}

/// The action for each common code of `pa_runtime::error_codes::common`.
/// A released code keeps its meaning, so a code never changes its action.
const commonErrorFixes = <String, ErrorFix>{
  'not_found': ErrorFix.openSources, // l10n-ignore
  'invalid': ErrorFix.openSettings, // l10n-ignore
  'denied': ErrorFix.openSources, // l10n-ignore
  'network': ErrorFix.retry, // l10n-ignore
  'unreachable': ErrorFix.openSources, // l10n-ignore
  'io': ErrorFix.retry, // l10n-ignore
  'database': ErrorFix.report, // l10n-ignore
  'secrets': ErrorFix.retry, // l10n-ignore
  'no_space': ErrorFix.freeSpace, // l10n-ignore
  'busy': ErrorFix.retry, // l10n-ignore
  'disabled': ErrorFix.openSettings, // l10n-ignore
  'not_initialized': ErrorFix.retry, // l10n-ignore
  'sync': ErrorFix.openSync, // l10n-ignore
  'provider': ErrorFix.retry, // l10n-ignore
  'internal': ErrorFix.report, // l10n-ignore
};

/// Domain codes that mean one of the common ones (file operations, the
/// offline folder, Twitch): the same text and action.
const _aliases = <String, String>{
  'full': 'no_space', // l10n-ignore
  'unavailable': 'unreachable', // l10n-ignore
  'timeout': 'network', // l10n-ignore
  'timed_out': 'network', // l10n-ignore
  'read_only': 'denied', // l10n-ignore
  'database_version': 'database', // l10n-ignore
  'foreign_database': 'database', // l10n-ignore
  'other': 'internal', // l10n-ignore
};

/// The common code [code] stands for, or [code] itself.
String commonErrorCode(String code) => _aliases[code] ?? code;

/// The action for [code]; codes the app does not know yet (a newer core)
/// lead to the report.
ErrorFix errorFixFor(String code) =>
    commonErrorFixes[commonErrorCode(code)] ?? ErrorFix.report;

/// The sentence for a known code, or null.
String? knownErrorText(AppLocalizations l, String code) =>
    switch (commonErrorCode(code)) {
      'not_found' => l.errorNotFound, // l10n-ignore
      'invalid' => l.errorInvalid, // l10n-ignore
      'denied' => l.errorDenied, // l10n-ignore
      'network' => l.errorNetwork, // l10n-ignore
      'unreachable' => l.errorUnreachable, // l10n-ignore
      'io' => l.errorIo, // l10n-ignore
      'database' => l.errorDatabase, // l10n-ignore
      'secrets' => l.errorSecrets, // l10n-ignore
      'no_space' => l.errorNoSpace, // l10n-ignore
      'busy' => l.errorBusy, // l10n-ignore
      'disabled' => l.errorDisabled, // l10n-ignore
      'not_initialized' => l.errorNotInitialized, // l10n-ignore
      'sync' => l.errorSync, // l10n-ignore
      'provider' => l.errorProvider, // l10n-ignore
      'internal' => l.errorInternal, // l10n-ignore
      _ => null,
    };

/// The sentence for [code]; a general one for codes the app does not know.
String errorText(AppLocalizations l, String code) =>
    knownErrorText(l, code) ?? l.errorUnknown;

/// The button's label: «Повторить» when the fix is to try again,
/// «Исправить» otherwise.
String errorFixLabel(AppLocalizations l, ErrorFix fix) =>
    fix == ErrorFix.retry ? l.errorFixRetry : l.errorFix;

/// What the button opens, for screen readers and the tooltip.
String errorFixHint(AppLocalizations l, ErrorFix fix) => switch (fix) {
  ErrorFix.retry => l.errorFixHintRetry,
  ErrorFix.editSource => l.errorFixHintEditSource,
  ErrorFix.openSources => l.errorFixHintSources,
  ErrorFix.freeSpace => l.errorFixHintFreeSpace,
  ErrorFix.openSettings => l.errorFixHintSettings,
  ErrorFix.openSync => l.errorFixHintSync,
  ErrorFix.openUpdates => l.errorFixHintUpdates,
  ErrorFix.report => l.errorFixHintReport,
};

/// The action for a source error (UI-005 texts): a wrong password or
/// folder is fixed in the source's form, a server that is off by trying
/// again once it is on.
ErrorFix sourceErrorFix(SourceErrorCode code) => switch (code) {
  SourceErrorCode.authFailed ||
  SourceErrorCode.permissionDenied ||
  SourceErrorCode.invalidSettings ||
  SourceErrorCode.hostKeyChanged ||
  SourceErrorCode.rsaDisabled ||
  SourceErrorCode.pathNotFound ||
  SourceErrorCode.shareNotFound ||
  SourceErrorCode.shareListRefused ||
  SourceErrorCode.notADirectory ||
  SourceErrorCode.invalidPath ||
  SourceErrorCode.notIndexable => ErrorFix.editSource,
  SourceErrorCode.unreachable ||
  SourceErrorCode.timedOut ||
  SourceErrorCode.secretStore ||
  SourceErrorCode.secretStoreLocked ||
  SourceErrorCode.secretStoreTimeout ||
  SourceErrorCode.other => ErrorFix.retry,
  SourceErrorCode.sourceMissing => ErrorFix.openSources,
  SourceErrorCode.unsupported => ErrorFix.report,
};

/// The stable code of an error of the core (every FRB error has one,
/// CORE-009) or of the app's own wrappers; null for anything else.
String? errorCodeOf(Object error) => switch (error) {
  LibraryError(:final code) ||
  SourcesError(:final code) ||
  DiagnosticsError(:final code) ||
  SecretsError(:final code) ||
  CoreSyncError(:final code) ||
  CoreOfflineError(:final code) ||
  CoreDiscoveryError(:final code) ||
  ImageCacheError(:final code) ||
  SkipError(:final code) ||
  TrickplayError(:final code) ||
  RemoteConfigError(:final code) ||
  UpdateVerifyError(:final code) ||
  CoreDownloadError(:final code) ||
  CoreChatError(:final code) ||
  CorePlaybackError(:final code) ||
  CoreLiveError(:final code) ||
  TwitchApiError(:final code) ||
  OfflineException(:final code) => code,
  SyncException() => 'sync', // l10n-ignore
  _ => null,
};

/// One sentence for any error: by its code when it has one, else the
/// general text (never the raw message).
String errorTextOf(AppLocalizations l, Object error) =>
    errorText(l, errorCodeOf(error) ?? 'internal'); // l10n-ignore

/// The technical part for «Подробнее» and the report: the code and the
/// core's message.
String errorDetailOf(Object error) {
  final code = errorCodeOf(error);
  final text = switch (error) {
    LibraryError(:final message) ||
    SourcesError(:final message) ||
    DiagnosticsError(:final message) ||
    SecretsError(:final message) ||
    CoreSyncError(:final message) ||
    CoreOfflineError(:final message) ||
    OfflineException(:final message) => message,
    _ => '$error',
  };
  return code == null ? text : '$code: $text';
}
