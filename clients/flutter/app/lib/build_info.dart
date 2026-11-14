// SPDX-License-Identifier: MPL-2.0

import 'branding.dart';
import 'flavor.dart';

/// What About shows of this build (UI-016): the app version, the Dart
/// flavor and capabilities (ADR-0041) and, when the Rust core is loaded,
/// its version, flavor and capabilities (`coreVersion()`, `buildFlavor()`,
/// `buildCapabilities()` from pa_core).
class BuildInfo {
  const BuildInfo({
    this.appVersion = kAppVersion,
    this.flavor = AppFlavor.name,
    this.capabilities = AppFlavor.capabilities,
    this.coreVersion,
    this.coreFlavor,
    this.coreCapabilities,
  });

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
