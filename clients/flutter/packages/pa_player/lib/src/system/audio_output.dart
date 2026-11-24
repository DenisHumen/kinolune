// SPDX-License-Identifier: MPL-2.0
import 'dart:io' show Platform, Process;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// The system's audio output picker (MUS-005, «выбор вывода» on Now
/// Playing). The app does not route audio itself: libmpv plays to the
/// system default device, so choosing the output is the system's job and
/// this only opens its picker.
///
/// * Android 14+: the system output switcher
///   (`MediaRouter2.showSystemOutputSwitcher`); 10–13: the volume panel
///   with its «Play on» row; older: the sound settings.
/// * macOS: System Settings → Sound → Output.
/// * Windows: Settings → System → Sound.
/// * Linux, iOS and elsewhere: no picker ([supported] is false). Linux has
///   no common one (PulseAudio/PipeWire mixers differ); iOS needs an
///   `AVRoutePickerView` platform view (MUS-007).
class SystemAudioOutput {
  SystemAudioOutput({
    MethodChannel? channel,
    TargetPlatform? platform,
    Future<void> Function(String executable, List<String> args)? run,
  }) : _channel = channel ?? const MethodChannel(channelName),
       _platform = platform ?? _hostPlatform(),
       _run = run ?? _start;

  static const channelName = 'app.kinolune/audio_output';

  /// The app-wide instance.
  static final SystemAudioOutput instance = SystemAudioOutput();

  final MethodChannel _channel;
  final TargetPlatform? _platform;
  final Future<void> Function(String, List<String>) _run;

  static TargetPlatform? _hostPlatform() {
    if (kIsWeb) return null;
    if (Platform.isAndroid) return TargetPlatform.android;
    if (Platform.isMacOS) return TargetPlatform.macOS;
    if (Platform.isWindows) return TargetPlatform.windows;
    return null;
  }

  static Future<void> _start(String executable, List<String> args) =>
      Process.start(executable, args, mode: ProcessStartMode.detached);

  /// Whether this platform has a picker to open.
  bool get supported => switch (_platform) {
    TargetPlatform.android ||
    TargetPlatform.macOS ||
    TargetPlatform.windows => true,
    _ => false,
  };

  /// Opens the picker; false when it could not.
  Future<bool> showPicker() async {
    try {
      switch (_platform) {
        case TargetPlatform.android:
          return await _channel.invokeMethod<bool>('showPicker') ?? false;
        case TargetPlatform.macOS:
          await _run('open', [
            'x-apple.systempreferences:com.apple.preference.sound?output',
          ]);
          return true;
        case TargetPlatform.windows:
          await _run('explorer.exe', ['ms-settings:sound']);
          return true;
        default:
          return false;
      }
    } on MissingPluginException {
      return false;
    } on PlatformException catch (e) {
      debugPrint('audio output: picker failed: ${e.message}');
      return false;
    } on Exception catch (e) {
      debugPrint('audio output: picker failed: $e');
      return false;
    }
  }
}
