// SPDX-License-Identifier: MPL-2.0
//
// Voice input for Search on the TV (UI-045, ADR-NNNN), channel
// `app.kinolune/voice` (VoiceBridge on the Android side). Android TV: the
// system speech recognizer (`RecognizerIntent.ACTION_RECOGNIZE_SPEECH`)
// opens its own listening screen and gives back the text; the app records
// nothing and needs no microphone permission. Elsewhere (and on a TV
// without a recognizer) there is no voice button. tvOS Siri dictation is a
// follow-up (TV-*): its keyboard already has a microphone.

import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Speech to text for one query.
abstract class VoiceInput {
  const VoiceInput();

  /// The one the app uses; tests put a fake here.
  static VoiceInput instance = const ChannelVoiceInput();

  /// Whether the device can listen (a system recognizer is installed).
  Future<bool> available();

  /// Listens once; the recognized text, or null if the user said nothing
  /// or cancelled. [language] is a BCP 47 tag (`ru-RU`), [prompt] is shown
  /// on the listening screen.
  Future<String?> listen({String? language, String? prompt});
}

/// [VoiceInput] over the platform channel; Android only.
class ChannelVoiceInput extends VoiceInput {
  const ChannelVoiceInput();

  static const _channel = MethodChannel('app.kinolune/voice');

  bool get _supported => !kIsWeb && Platform.isAndroid;

  @override
  Future<bool> available() async {
    if (!_supported) return false;
    try {
      return await _channel.invokeMethod<bool>('available') ?? false;
    } on PlatformException catch (e) {
      debugPrint('Voice input: ${e.code}');
      return false;
    } on MissingPluginException {
      return false;
    }
  }

  @override
  Future<String?> listen({String? language, String? prompt}) async {
    if (!_supported) return null;
    try {
      final text = await _channel.invokeMethod<String>('listen', {
        'language': language,
        'prompt': prompt,
      });
      final t = text?.trim();
      return t == null || t.isEmpty ? null : t;
    } on PlatformException catch (e) {
      debugPrint('Voice input: ${e.code}');
      return null;
    } on MissingPluginException {
      return null;
    }
  }
}
