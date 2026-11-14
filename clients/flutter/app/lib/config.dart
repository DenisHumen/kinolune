// SPDX-License-Identifier: MPL-2.0

import 'platform/form_factor.dart';

/// Compile-time switches (`--dart-define=NAME=value`).
abstract final class AppConfig {
  /// Start with the built-in demo library (fictional titles, art drawn in
  /// code). Useful for screenshots and for trying the UI without media.
  static const demo = bool.fromEnvironment('KINOLUNE_DEMO');

  /// Run the library on the Rust core (`pa_core`, UI-024; the default). With
  /// `false` the app uses the Dart v0 implementation, which is also the
  /// fallback when the native core is missing (`openAppLibrary`).
  static const rustCore = bool.fromEnvironment(
    'KINOLUNE_RUST_CORE',
    defaultValue: true,
  );

  /// Public Twitch client id for Helix and Device Code Flow sign-in
  /// (TWI-013). The project has none registered yet; empty = the Streams
  /// section works on local favourites only (DEC-006).
  static const twitchClientId = String.fromEnvironment(
    'KINOLUNE_TWITCH_CLIENT_ID',
  );

  /// The player lab of the manual checks (TWI-014): base URL of the
  /// `pa-twitch` example `player_lab`, which stands in for usher with
  /// local media. Full builds only; empty in every release build. With it
  /// the native player is used without the remote config's permission.
  static const twitchLab = String.fromEnvironment('KINOLUNE_TWITCH_LAB');

  /// `tv`, `phone` or `desktop`; empty = auto-detect.
  static const _formFactor = String.fromEnvironment('KINOLUNE_FORM_FACTOR');

  static FormFactor? get forcedFormFactor => switch (_formFactor) {
    'tv' => FormFactor.tv,
    'phone' => FormFactor.phone,
    'desktop' => FormFactor.desktop,
    _ => null,
  };
}
