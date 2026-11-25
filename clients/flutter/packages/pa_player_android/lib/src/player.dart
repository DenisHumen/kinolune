// SPDX-License-Identifier: MPL-2.0
import 'dart:convert';

import 'package:flutter/services.dart';

/// One playlist item (`MediaSource` of the contract).
class PlayerItem {
  const PlayerItem(this.uri, {this.title});

  /// `file://` URI or path, or `http(s)://` (including the loopback pa-proxy).
  final String uri;
  final String? title;

  Map<String, Object?> toJson() => {'uri': uri, 'title': ?title};
}

/// Starts the native player from the app.
class PaPlayerAndroid {
  PaPlayerAndroid._();

  /// Channel to the plugin; replaceable in tests.
  static MethodChannel channel = const MethodChannel('pa_player_android');

  /// Opens the player Activity with [items] (`OpenRequest` of the contract).
  ///
  /// Completes when the player closes, with the last engine state as JSON
  /// (`position_ms`, `duration_ms`, `completed`, `playlist_index`…), or
  /// `null` if the Activity ended without one. [nativeOsd] uses the Media3
  /// controller instead of the Flutter OSD (fallback for devices where a
  /// Flutter layer above HDR video misbehaves, ADR-0008).
  ///
  /// [display] (PLAY-AND-005): what the player may do with the display —
  /// `frame_rate` (`auto`/`off`), `resolution` and `dynamic_range` (bools),
  /// as `DisplayMatchSettings.toJson()` of pa_player gives them. Without it:
  /// frame rate on, resolution kept, native Dolby Vision where possible.
  ///
  /// [audioOutput] (PLAY-AND-004): `auto` passes Dolby and DTS untouched to
  /// a receiver that takes them, `decode` always decodes to PCM (as
  /// `AudioOutputMode.name` of pa_player). Without it: `auto`.
  static Future<Map<String, Object?>?> open(
    List<PlayerItem> items, {
    int index = 0,
    int startPositionMs = 0,
    bool play = true,
    bool nativeOsd = false,
    String osdEntrypoint = 'osdMain',
    Map<String, Object?>? display,
    String? audioOutput,
  }) async {
    if (items.isEmpty) throw ArgumentError.value(items, 'items', 'empty');
    final request = {
      'items': [for (final i in items) i.toJson()],
      'index': index,
      'start_position_ms': startPositionMs,
      'play': play,
      'display': ?display,
      'audio_output': ?audioOutput,
    };
    final state = await channel.invokeMethod<String>('open', {
      'request': jsonEncode(request),
      'native_osd': nativeOsd,
      'osd_entrypoint': osdEntrypoint,
    });
    return state == null ? null : (jsonDecode(state) as Map).cast<String, Object?>();
  }

  /// Starts the OSD engine in advance so opening the player does not wait
  /// for it (ADR-0008: its start blocks the main thread).
  static Future<void> prewarmOsd({String osdEntrypoint = 'osdMain'}) =>
      channel.invokeMethod<void>('prewarmOsd', {'osd_entrypoint': osdEntrypoint});
}
