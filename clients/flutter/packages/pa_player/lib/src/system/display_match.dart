// SPDX-License-Identifier: MPL-2.0
import 'dart:async';
import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:media_kit/media_kit.dart' as mk;

import '../engine/media_kit_engine.dart';
import '../engine/player_engine.dart';

/// «Частота кадров» (PLAY-AND-005): the display follows the video's frame
/// rate, or is never touched.
enum FrameRateMatch { auto, off }

/// What the player may do with the TV (PLAY-AND-005, ADR-0147): settings
/// «Частота кадров», «Разрешение как у видео» and «Совпадение динамического
/// диапазона». The defaults are what a TV owner expects: 23.976 fps films at
/// 23.976 Hz, the resolution kept (the TV upscales better than the box),
/// Dolby Vision as Dolby Vision.
@immutable
class DisplayMatchSettings {
  const DisplayMatchSettings({
    this.frameRate = FrameRateMatch.auto,
    this.resolution = false,
    this.dynamicRange = true,
  });

  final FrameRateMatch frameRate;

  /// Also switch the resolution to the video's (off by default).
  final bool resolution;

  /// Dolby Vision goes to the TV as Dolby Vision where the box and the TV
  /// can; off plays the HDR10 base layer (the native Android player only:
  /// the media_kit picture is always SDR).
  final bool dynamicRange;

  DisplayMatchSettings copyWith({
    FrameRateMatch? frameRate,
    bool? resolution,
    bool? dynamicRange,
  }) => DisplayMatchSettings(
    frameRate: frameRate ?? this.frameRate,
    resolution: resolution ?? this.resolution,
    dynamicRange: dynamicRange ?? this.dynamicRange,
  );

  /// The map the Android side reads (`DisplayMatchOptions.fromMap`).
  Map<String, Object?> toJson() => {
    'frame_rate': frameRate.name,
    'resolution': resolution,
    'dynamic_range': dynamicRange,
  };

  /// Unknown or missing keys keep their defaults.
  factory DisplayMatchSettings.fromJson(Map<String, Object?>? json) {
    if (json == null) return const DisplayMatchSettings();
    return DisplayMatchSettings(
      frameRate: json['frame_rate'] == 'off'
          ? FrameRateMatch.off
          : FrameRateMatch.auto,
      resolution: json['resolution'] == true,
      dynamicRange: json['dynamic_range'] != false,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is DisplayMatchSettings &&
      other.frameRate == frameRate &&
      other.resolution == resolution &&
      other.dynamicRange == dynamicRange;

  @override
  int get hashCode => Object.hash(frameRate, resolution, dynamicRange);
}

/// Auto frame rate for one video (PLAY-AND-005): once the engine knows the
/// video's frame rate, the TV is switched to the matching mode (23.976 Hz
/// for a 23.976 fps film, 50 Hz for 25 fps…) through
/// `app.kinolune/display`; [restore] puts the previous mode back when the
/// player closes. Android only; elsewhere (and in tests, unless [enabled])
/// nothing happens.
class DisplayMatcher {
  DisplayMatcher({
    required this.settings,
    MethodChannel? channel,
    bool? enabled,
  }) : _channel = channel ?? const MethodChannel(channelName),
       enabled =
           (enabled ?? (!kIsWeb && Platform.isAndroid)) &&
           settings.frameRate == FrameRateMatch.auto;

  static const channelName = 'app.kinolune/display';

  final DisplayMatchSettings settings;
  final MethodChannel _channel;

  /// False off Android or with «Частота кадров: Выкл».
  final bool enabled;

  /// The display mode after the last switch (`3840x2160@23.976`), null
  /// while the display was not touched.
  final ValueNotifier<String?> mode = ValueNotifier(null);

  ({double fps, int width, int height})? _matched;
  bool _switched = false;

  /// Wraps [base] so every engine it makes reports its video to this
  /// matcher.
  PlayerEngineFactory wrap(PlayerEngineFactory base) => ({bool video = true}) {
    final engine = base(video: video);
    if (enabled && video) {
      engine.state.addListener(() => _onState(engine));
    }
    return engine;
  };

  void _onState(PlayerEngine engine) {
    final s = engine.state.value;
    final track = s.videoTrack;
    final fps = track?.fps;
    if (fps == null || fps <= 1) return;
    final video = (
      fps: fps,
      width: s.videoWidth ?? track?.width ?? 0,
      height: s.videoHeight ?? track?.height ?? 0,
    );
    if (video == _matched) return;
    _matched = video;
    unawaited(_match(engine, video));
  }

  Future<void> _match(
    PlayerEngine engine,
    ({double fps, int width, int height}) video,
  ) async {
    try {
      final result = await _channel.invokeMethod<String>('match', {
        'fps': video.fps,
        'width': video.width,
        'height': video.height,
        'options': settings.toJson(),
      });
      if (result == null) return;
      _switched = true;
      mode.value = result;
      await _tellMpv(engine, result);
    } on PlatformException catch (e) {
      debugPrint('pa_player: display match failed: $e');
    } on MissingPluginException {
      // No Android side (tests, other hosts).
    }
  }

  /// libmpv cannot see the refresh rate on Android: tell it, so its
  /// video sync and the stats overlay («Hz») follow the new mode.
  static Future<void> _tellMpv(PlayerEngine engine, String mode) async {
    final hz = refreshRateOf(mode);
    if (hz == null || engine is! MediaKitEngine) return;
    final p = engine.player.platform;
    if (p is! mk.NativePlayer) return;
    try {
      await p.setProperty('display-fps-override', hz.toStringAsFixed(3));
    } catch (e) {
      debugPrint('pa_player: display-fps-override: $e');
    }
  }

  /// `3840x2160@23.976` → 23.976.
  static double? refreshRateOf(String mode) {
    final at = mode.lastIndexOf('@');
    return at < 0 ? null : double.tryParse(mode.substring(at + 1));
  }

  /// Puts the display mode from before the player back (player closed).
  Future<void> restore() async {
    _matched = null;
    if (!enabled || !_switched) return;
    _switched = false;
    mode.value = null;
    try {
      await _channel.invokeMethod<void>('restore');
    } on PlatformException catch (e) {
      debugPrint('pa_player: display restore failed: $e');
    } on MissingPluginException {
      // No Android side.
    }
  }
}
