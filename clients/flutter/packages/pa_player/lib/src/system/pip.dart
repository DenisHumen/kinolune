// SPDX-License-Identifier: MPL-2.0
import 'dart:async';
import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Picture-in-picture of the video player (PLAY-AND-008): Android
/// `enterPictureInPictureMode`, entered by a button or by itself when the
/// user leaves the app while a video plays (auto-enter on Android 12+,
/// `onUserLeaveHint` on 8–11).
///
/// The activity handles orientation and size changes itself
/// (`configChanges`), so the PiP window, the engine and its surface live
/// through a rotation. Elsewhere (and in tests, unless [enabled]) nothing
/// happens and [supported] is false.
class SystemPip {
  SystemPip({MethodChannel? channel, bool? enabled})
    : _channel = channel ?? const MethodChannel(channelName),
      enabled = enabled ?? (!kIsWeb && Platform.isAndroid) {
    if (this.enabled) _channel.setMethodCallHandler(_onCall);
  }

  static const channelName = 'app.kinolune/pip';

  /// The app-wide instance.
  static final SystemPip instance = SystemPip();

  final MethodChannel _channel;
  final bool enabled;

  /// The app is in a PiP window now: the player shows only the picture.
  final ValueNotifier<bool> active = ValueNotifier(false);

  /// The user closed the PiP window (not expanded it back): the video
  /// should pause.
  final ValueNotifier<int> dismissals = ValueNotifier(0);

  bool? _supported;
  String? _configured;

  /// The device has PiP and the user allows it for the app.
  Future<bool> supported() async {
    if (!enabled) return false;
    return _supported ??= await _call<bool>('isSupported') ?? false;
  }

  /// Lets the system enter PiP by itself when the user leaves the app
  /// ([autoEnter]: a video is playing); [width] / [height] give the
  /// window's shape.
  Future<void> configure({
    required bool autoEnter,
    int? width,
    int? height,
  }) async {
    if (!enabled) return;
    final key = '$autoEnter:$width:$height';
    if (key == _configured) return;
    _configured = key;
    await _call<void>('configure', {
      'autoEnter': autoEnter,
      'width': width,
      'height': height,
    });
  }

  /// Enters PiP now; false when the system refused.
  Future<bool> enter({int? width, int? height}) async {
    if (!enabled) return false;
    return await _call<bool>('enter', {'width': width, 'height': height}) ??
        false;
  }

  Future<T?> _call<T>(String method, [Object? args]) async {
    try {
      return await _channel.invokeMethod<T>(method, args);
    } on MissingPluginException {
      return null;
    } on PlatformException catch (e) {
      debugPrint('pip: $method failed: ${e.message}');
      return null;
    }
  }

  Future<Object?> _onCall(MethodCall call) async {
    if (call.method != 'changed') return null;
    final args = (call.arguments as Map?)?.cast<String, Object?>() ?? const {};
    onChanged(
      inPip: args['inPip'] == true,
      dismissed: args['dismissed'] == true,
    );
    return null;
  }

  /// The PiP mode changed (the native side, tests).
  @visibleForTesting
  void onChanged({required bool inPip, bool dismissed = false}) {
    active.value = inPip;
    if (dismissed) dismissals.value++;
  }
}
