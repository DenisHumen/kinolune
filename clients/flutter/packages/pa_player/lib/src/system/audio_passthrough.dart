// SPDX-License-Identifier: MPL-2.0
import 'dart:async';
import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../engine/player_engine.dart';

/// «Звук» (PLAY-AND-004): [auto] sends Dolby (AC-3, E-AC-3, TrueHD with
/// Atmos) and DTS untouched to a receiver or TV that takes them over HDMI
/// (passthrough); [decode] always decodes to PCM — for a receiver that
/// misbehaves, a soundbar without DTS, or to keep the app's volume and
/// audio processing (they do not apply to a bitstream).
enum AudioOutputMode {
  auto,
  decode;

  /// `auto` / `decode`; anything else is [auto].
  static AudioOutputMode fromName(String? name) =>
      name == decode.name ? decode : auto;
}

/// Passthrough for one video (PLAY-AND-004): [prepare] asks Android which
/// codecs the current output (HDMI EDID of the TV or receiver, the
/// system's surround setting) takes untouched, and [wrap] gives that list
/// to every video engine (mpv `audio-spdif`). Android only; elsewhere (and
/// in tests, unless [enabled]) the engine keeps decoding everything.
class AudioPassthrough {
  AudioPassthrough({required this.mode, MethodChannel? channel, bool? enabled})
    : _channel = channel ?? const MethodChannel(channelName),
      enabled = enabled ?? (!kIsWeb && Platform.isAndroid);

  static const channelName = 'app.kinolune/audio_passthrough';

  final AudioOutputMode mode;
  final MethodChannel _channel;

  /// False off Android.
  final bool enabled;

  /// mpv's `audio-spdif` after [prepare]: `ac3,eac3,dts,dts-hd,truehd` or a
  /// part of it, empty when everything is decoded.
  String get codecs => _codecs;
  String _codecs = '';

  /// Reads the output's capabilities; call before the player opens, so the
  /// first audio already starts the right way (no re-init a second in).
  Future<String> prepare() async {
    if (!enabled || mode == AudioOutputMode.decode) return _codecs = '';
    try {
      _codecs =
          await _channel.invokeMethod<String>('spdif', {'mode': mode.name}) ??
          '';
    } on PlatformException catch (e) {
      debugPrint('pa_player: audio passthrough capabilities failed: $e');
      _codecs = '';
    } on MissingPluginException {
      // No Android side (tests, other hosts).
      _codecs = '';
    }
    return _codecs;
  }

  /// Wraps [base] so every video engine it makes gets [codecs]. Music and
  /// books stay decoded (gapless, volume, no receiver re-sync per track).
  PlayerEngineFactory wrap(PlayerEngineFactory base) => ({bool video = true}) {
    final engine = base(video: video);
    if (engine case final AudioPassthroughEngine e when enabled && video) {
      unawaited(e.setAudioPassthrough(_codecs));
    }
    return engine;
  };
}
