// SPDX-License-Identifier: MPL-2.0
//
// [TwitchNativeSource] over the Rust core (TWI-014, full builds):
// `CoreTwitchPlayback` resolves through GQL and usher, `CoreLiveProxy` is
// the loopback live proxy of TWI-006. [LabTwitchNativeSource] reads the
// master playlists of the player lab (`pa-twitch` example `player_lab`)
// instead of usher, for the manual checks without Twitch.

import 'dart:convert';
import 'dart:io';

import 'package:pa_core/pa_core.dart' as core;
import 'package:pa_player/pa_player.dart';

import 'twitch_native.dart';
import 'twitch_service.dart';

TwitchQuality _quality(core.CoreTwitchVariant v) => TwitchQuality(
  id: v.groupId,
  label: v.quality,
  url: v.url,
  isSource: v.isSource,
  audioOnly: v.audioOnly,
  height: v.height ?? 0,
  frameRate: v.frameRate,
  bandwidth: v.bandwidth.toInt(),
);

TwitchNativeError _error(core.CorePlaybackError e) =>
    TwitchNativeError(e.code, fallsBackToEmbed: e.fallsBackToEmbed);

class _CoreLiveSession implements TwitchLiveSession {
  _CoreLiveSession(this._proxy);

  final core.CoreLiveProxy _proxy;

  @override
  late final String playlistUrl = _proxy.playlistUrl();

  @override
  LiveTelemetry telemetry() => HttpLiveTelemetry(
    stats: Uri.parse(_proxy.statsUrl()),
    reset: Uri.parse(_proxy.resetUrl()),
  );

  @override
  void setMode(LiveLatencyMode mode) => _proxy.setMode(mode: _mode(mode));

  @override
  void setVariant(String mediaPlaylistUrl) =>
      _proxy.setMediaPlaylist(url: mediaPlaylistUrl);

  @override
  void close() {
    _proxy.stop();
    _proxy.dispose();
  }
}

core.CoreLiveMode _mode(LiveLatencyMode m) => switch (m) {
  LiveLatencyMode.lowLatency => core.CoreLiveMode.lowLatency,
  LiveLatencyMode.stable => core.CoreLiveMode.stable,
};

TwitchLiveSession _startLive(String url, LiveLatencyMode mode) {
  try {
    return _CoreLiveSession(
      core.CoreLiveProxy.start(mediaPlaylistUrl: url, mode: _mode(mode)),
    );
  } on core.CorePlaybackError catch (e) {
    throw _error(e);
  }
}

/// GQL + usher through the core; only where the signed config allows.
class CoreTwitchNativeSource implements TwitchNativeSource {
  CoreTwitchNativeSource(this._playback);

  final core.CoreTwitchPlayback _playback;

  /// The decision of [config] for this build; null when it keeps native
  /// playback off (store build, flag off, kill-switch, no query hash).
  static CoreTwitchNativeSource? open(core.CoreRemoteConfig config) {
    final p = core.CoreTwitchPlayback.open(config: config);
    if (!p.nativeAllowed()) {
      p.dispose();
      return null;
    }
    return CoreTwitchNativeSource(p);
  }

  @override
  Future<TwitchNativeStream> resolve(TwitchPlayTarget target) async {
    try {
      // H.264 everywhere for now: no reliable AV1/HEVC probe yet.
      final s = target.isLive
          ? await _playback.resolveLive(
              login: target.login!,
              av1: false,
              hevc: false,
            )
          : await _playback.resolveVod(
              videoId: target.videoId!,
              av1: false,
              hevc: false,
            );
      return TwitchNativeStream([for (final v in s.variants) _quality(v)]);
    } on core.CorePlaybackError catch (e) {
      throw _error(e);
    }
  }

  @override
  TwitchLiveSession startLive(String url, LiveLatencyMode mode) =>
      _startLive(url, mode);
}

/// The player lab: `<lab>/live/<login>/master.m3u8` and
/// `<lab>/vod/<id>/master.m3u8` stand in for usher. Built only with
/// `--dart-define=KINOLUNE_TWITCH_LAB=<url>` in full builds.
class LabTwitchNativeSource implements TwitchNativeSource {
  LabTwitchNativeSource(this.lab);

  final Uri lab;

  @override
  Future<TwitchNativeStream> resolve(TwitchPlayTarget target) async {
    final path = target.isLive
        ? 'live/${target.login}/master.m3u8'
        : 'vod/${target.videoId}/master.m3u8';
    final client = HttpClient()..connectionTimeout = const Duration(seconds: 5);
    try {
      final req = await client.getUrl(lab.resolve(path));
      final res = await req.close();
      final text = await res.transform(utf8.decoder).join();
      if (res.statusCode != HttpStatus.ok) {
        throw const TwitchNativeError('unavailable');
      }
      final variants = core.twitchMasterVariants(text: text);
      return TwitchNativeStream([for (final v in variants) _quality(v)]);
    } on core.CorePlaybackError catch (e) {
      throw _error(e);
    } on IOException {
      throw const TwitchNativeError('network');
    } finally {
      client.close(force: true);
    }
  }

  @override
  TwitchLiveSession startLive(String url, LiveLatencyMode mode) =>
      _startLive(url, mode);
}
