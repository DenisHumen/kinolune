// SPDX-License-Identifier: MPL-2.0
//
// Native Twitch playback (TWI-014, full builds only): what the player
// screen needs from the core, independent of FRB so tests can fake it.
//
// - [TwitchNativeSource.resolve]: the usher variants of a channel or VOD
//   (TWI-004: GQL `PlaybackAccessToken` → usher master playlist), as
//   [TwitchQuality]s: source, lower qualities and "audio only".
// - [TwitchNativeSource.startLive]: the loopback live proxy of TWI-006 in
//   front of one variant; the player opens its URL and the
//   `LivePlaybackController` polls its telemetry.
// - [TwitchVodProgress]: VOD progress under `twitch:vod:<id>` (the same key
//   as the embed of TWI-005, Continue watching and sync between devices)
//   and handoff heartbeats.

import 'package:flutter/foundation.dart';
import 'package:pa_player/pa_player.dart';

import '../integration/library.dart' show Progress;
import '../sync/handoff.dart';
import 'twitch_models.dart';
import 'twitch_service.dart';

/// A loopback address: `127.0.0.0/8`, `::1` or `localhost`.
bool isLoopbackUri(Uri uri) {
  final host = uri.host.toLowerCase();
  return host == 'localhost' ||
      host == '::1' ||
      RegExp(r'^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$').hasMatch(host);
}

/// The player lab origin of `KINOLUNE_TWITCH_LAB` (TWI-014), or null. Never
/// in a release build ([release], default [kReleaseMode]): there native
/// playback needs the signed remote config. Only an http(s) loopback URL:
/// the lab stands in for usher locally and never points at Twitch.
Uri? twitchLabOrigin(String value, {bool release = kReleaseMode}) {
  if (release || value.isEmpty) return null;
  final uri = Uri.tryParse(value);
  if (uri == null || (uri.scheme != 'http' && uri.scheme != 'https')) {
    return null;
  }
  return isLoopbackUri(uri) ? uri : null;
}

/// One usher variant.
@immutable
class TwitchQuality {
  const TwitchQuality({
    required this.id,
    required this.label,
    required this.url,
    this.isSource = false,
    this.audioOnly = false,
    this.height = 0,
    this.frameRate,
    this.bandwidth = 0,
  });

  /// Group id (`chunked`, `720p60`, `audio_only`).
  final String id;

  /// `1080p60`, `audio_only`.
  final String label;

  /// Media playlist URL.
  final String url;
  final bool isSource;
  final bool audioOnly;
  final int height;
  final double? frameRate;
  final int bandwidth;

  @override
  bool operator ==(Object other) =>
      other is TwitchQuality && other.id == id && other.url == url;

  @override
  int get hashCode => Object.hash(id, url);
}

/// The variants of one stream or VOD, best first; "audio only" last.
@immutable
class TwitchNativeStream {
  TwitchNativeStream(List<TwitchQuality> qualities)
    : qualities = List.unmodifiable(sortQualities(qualities));

  final List<TwitchQuality> qualities;

  TwitchQuality? get source =>
      qualities.where((q) => q.isSource).firstOrNull ??
      qualities.where((q) => !q.audioOnly).firstOrNull;

  TwitchQuality? get audioOnly =>
      qualities.where((q) => q.audioOnly).firstOrNull;
}

/// Source first, then by height, frame rate and bandwidth; audio only last.
List<TwitchQuality> sortQualities(List<TwitchQuality> list) {
  int rank(TwitchQuality q) => q.audioOnly ? 0 : (q.isSource ? 2 : 1);
  return [...list]..sort((a, b) {
    final r = rank(b).compareTo(rank(a));
    if (r != 0) return r;
    final h = b.height.compareTo(a.height);
    if (h != 0) return h;
    final f = (b.frameRate ?? 0).compareTo(a.frameRate ?? 0);
    if (f != 0) return f;
    return b.bandwidth.compareTo(a.bandwidth);
  });
}

/// The remembered quality: `source`, `audio_only` or a height such as
/// `720`. The best variant not above it; unknown: source.
TwitchQuality? pickQuality(TwitchNativeStream stream, String? preference) {
  if (stream.qualities.isEmpty) return null;
  switch (preference) {
    case null || '' || 'source':
      return stream.source ?? stream.qualities.first;
    case 'audio_only':
      return stream.audioOnly ?? stream.source;
  }
  final max = int.tryParse(preference);
  if (max == null) return stream.source;
  final video = stream.qualities.where((q) => !q.audioOnly).toList();
  return video.where((q) => q.height > 0 && q.height <= max).firstOrNull ??
      video.lastOrNull ??
      stream.source;
}

/// How a quality is remembered ([pickQuality]).
String qualityPreference(TwitchQuality q) => q.audioOnly
    ? 'audio_only'
    : q.isSource || q.height == 0
    ? 'source'
    : '${q.height}';

/// Why native playback failed. [fallsBackToEmbed]: the official embed may
/// still play it (sub-only, integrity, config); else it is offline or gone.
class TwitchNativeError implements Exception {
  const TwitchNativeError(this.code, {this.fallsBackToEmbed = false});

  /// `unavailable` (offline channel, deleted VOD), `usher_403`, `network`…
  final String code;
  final bool fallsBackToEmbed;

  bool get offline => code == 'unavailable' || code == 'not_found';

  @override
  String toString() => 'TwitchNativeError($code)';
}

/// A running live proxy.
abstract class TwitchLiveSession {
  /// What the player opens.
  String get playlistUrl;

  /// The proxy's telemetry for `LivePlaybackController`.
  LiveTelemetry telemetry();

  /// "Low latency" / "Stable"; the player reopens [playlistUrl].
  void setMode(LiveLatencyMode mode);

  /// Another variant; the player reopens [playlistUrl].
  void setVariant(String mediaPlaylistUrl);

  void close();
}

/// The core side of the native player.
abstract class TwitchNativeSource {
  /// Variants of [target]. Throws [TwitchNativeError].
  Future<TwitchNativeStream> resolve(TwitchPlayTarget target);

  /// Starts the live proxy for [mediaPlaylistUrl].
  TwitchLiveSession startLive(String mediaPlaylistUrl, LiveLatencyMode mode);
}

/// The player's chapters for Twitch chapters (TWI-008): the category
/// name, "≈" for the channel's current category.
List<PlaybackChapter> playbackChapters(List<TwitchChapter> chapters) => [
  for (final c in chapters)
    PlaybackChapter(
      start: c.start,
      end: c.end,
      title: c.gameName == null
          ? null
          : c.approximate
          ? '≈ ${c.gameName}'
          : c.gameName,
    ),
];

/// VOD progress of the native player under `twitch:vod:<id>`, as the embed
/// does (TWI-005): not before the first real position, watched at the end.
class TwitchVodProgress {
  TwitchVodProgress({
    required this.videoId,
    required this.saveProgress,
    this.handoff,
    DateTime Function()? clock,
  }) : clock = clock ?? DateTime.now;

  final String videoId;
  final Future<void> Function(Progress progress) saveProgress;
  final HandoffController? handoff;
  final DateTime Function() clock;

  String get itemId => 'twitch:vod:$videoId'; // l10n-ignore

  int? _savedMs;
  bool _begun = false;
  bool _ended = false;

  /// The player's report ([PlayerScreen.onProgress]).
  Future<void> report(int positionMs, int durationMs) =>
      _save(positionMs, durationMs, watched: false);

  /// The VOD played to the end.
  Future<void> finished(int durationMs) async {
    if (_ended || durationMs <= 0) return;
    await _save(durationMs, durationMs, watched: true);
    _ended = true;
  }

  Future<void> _save(
    int positionMs,
    int durationMs, {
    required bool watched,
  }) async {
    if (_ended || durationMs <= 0) return;
    if (positionMs <= 0 && !watched) return;
    if (_savedMs == positionMs && !watched) return;
    _savedMs = positionMs;
    if (!_begun) {
      _begun = true;
      handoff?.begin(itemId, positionMs, durationMs: durationMs);
    }
    handoff?.report(itemId, positionMs, durationMs);
    try {
      await saveProgress(
        Progress(
          itemId: itemId,
          positionMs: positionMs,
          durationMs: durationMs,
          updatedAt: clock(),
          watched: watched,
        ),
      );
    } catch (e) {
      debugPrint('Twitch VOD progress not saved: $e');
    }
  }

  /// The player closed.
  void close() {
    if (_begun) handoff?.end();
    _begun = false;
  }
}
