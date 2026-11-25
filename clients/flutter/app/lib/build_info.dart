// SPDX-License-Identifier: MPL-2.0

import 'branding.dart';
import 'flavor.dart';

const _builtVersion = String.fromEnvironment(
  'KINOLUNE_VERSION',
  defaultValue: kAppVersion,
);
const _buildNumber = String.fromEnvironment('KINOLUNE_BUILD_NUMBER');

/// The full version of this build (BUG-024): `KINOLUNE_VERSION` of the
/// release workflow with its pre-release (`0.1.0-alpha.19`) and the build
/// number (`KINOLUNE_BUILD_NUMBER`, the workflow run) after `+`:
/// `0.1.0-alpha.19+50`. [kAppVersion] in local and test builds.
const kBuildVersion = _buildNumber == ''
    ? _builtVersion
    : '$_builtVersion+$_buildNumber';

/// What About shows of this build (UI-016): the app version, the Dart
/// flavor and capabilities (ADR-0041) and, when the Rust core is loaded,
/// its version, flavor and capabilities (`coreVersion()`, `buildFlavor()`,
/// `buildCapabilities()` from pa_core).
class BuildInfo {
  const BuildInfo({
    this.appVersion = kBuildVersion,
    this.flavor = AppFlavor.name,
    this.capabilities = AppFlavor.capabilities,
    this.coreVersion,
    this.coreFlavor,
    this.coreCapabilities,
  });

  /// [kBuildVersion]: what About, «Состояние» and the diagnostic report
  /// show.
  final String appVersion;

  /// `store` or `full` (`KINOLUNE_FLAVOR`).
  final String flavor;
  final List<String> capabilities;

  /// Null when the app runs without the native core (v0 fallback, tests).
  final String? coreVersion;
  final String? coreFlavor;
  final List<String>? coreCapabilities;

  /// The Dart half disagrees with the core (a packaging error, CORE-013).
  bool get mismatch => coreFlavor != null && coreFlavor != flavor;
}
