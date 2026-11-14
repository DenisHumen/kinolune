// SPDX-License-Identifier: MPL-2.0
import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Brightness of the player screen, changed by the vertical swipe on the
/// left half of the video (UI-013).
///
/// On Android it is the brightness of the player's window
/// (`WindowManager.LayoutParams.screenBrightness`, through the
/// `app.kinolune/screen` channel of the pa_player plugin): it changes the
/// backlight, needs no permission and ends with the player ([reset]).
/// Elsewhere — and when the channel is missing — the player dims the
/// picture with a black layer instead ([dimsPicture]).
abstract class ScreenBrightness {
  /// Brightness when the player opens, 0..1.
  Future<double> read();

  /// Sets the brightness, 0..1.
  Future<void> write(double value);

  /// Gives the screen back to the system setting.
  Future<void> reset();

  /// True when the player must draw the brightness itself as a dimming
  /// layer over the video.
  bool get dimsPicture;

  /// The implementation for this platform.
  static ScreenBrightness platform() =>
      !kIsWeb && Platform.isAndroid ? WindowBrightness() : DimBrightness();
}

/// Brightness drawn by the player: 1 is the picture as is, lower values
/// put a black layer over it.
class DimBrightness implements ScreenBrightness {
  DimBrightness();

  @override
  bool get dimsPicture => true;

  @override
  Future<double> read() async => 1.0;

  @override
  Future<void> write(double value) async {}

  @override
  Future<void> reset() async {}
}

/// Android window brightness; falls back to [DimBrightness] behaviour when
/// the plugin is not there (tests, other embedders).
class WindowBrightness implements ScreenBrightness {
  WindowBrightness();

  static const MethodChannel _channel = MethodChannel('app.kinolune/screen');
  bool _failed = false;

  @override
  bool get dimsPicture => _failed;

  @override
  Future<double> read() async {
    try {
      final v = await _channel.invokeMethod<double>('brightness');
      return (v ?? 1.0).clamp(0.0, 1.0);
    } on MissingPluginException {
      _failed = true;
    } on PlatformException {
      _failed = true;
    }
    return 1.0;
  }

  @override
  Future<void> write(double value) async {
    if (_failed) return;
    try {
      await _channel.invokeMethod<void>('setBrightness', value.clamp(0, 1));
    } on MissingPluginException {
      _failed = true;
    } on PlatformException {
      _failed = true;
    }
  }

  @override
  Future<void> reset() async {
    if (_failed) return;
    try {
      await _channel.invokeMethod<void>('setBrightness', -1.0);
    } on MissingPluginException {
      _failed = true;
    } on PlatformException {
      _failed = true;
    }
  }
}
