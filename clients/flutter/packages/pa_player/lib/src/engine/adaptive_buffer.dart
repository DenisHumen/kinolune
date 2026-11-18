// SPDX-License-Identifier: MPL-2.0
//
// Adaptive read-ahead for slow or lossy sources (PLAY-010, ADR-0120): SMB
// or WebDAV over Wi-Fi, a media server far away. Pure logic; the engine
// feeds it samples and applies the plan to libmpv.

import 'package:flutter/foundation.dart';

import 'playback_stats.dart';

/// How careful the buffer is. It only goes up while one item plays: a
/// source that stalled once is not trusted again until the next file.
enum BufferLevel {
  /// 30 s ahead; resume 1 s after a stall.
  normal(30, 1),

  /// The network is barely faster than the stream: 60 s ahead, resume
  /// after 3 s.
  cautious(60, 3),

  /// It stalled, or the network is slower than the stream: 120 s ahead,
  /// resume after 6 s (fewer, longer stops instead of a stutter).
  lossy(120, 6);

  const BufferLevel(this.readAheadSecs, this.resumeWaitSecs);
  final int readAheadSecs;
  final int resumeWaitSecs;
}

/// What the engine sets for one level.
@immutable
class BufferPlan {
  const BufferPlan({
    required this.level,
    required this.readAheadSecs,
    required this.maxBytes,
    required this.resumeWaitSecs,
  });

  final BufferLevel level;
  final int readAheadSecs;

  /// Memory the demuxer cache may use.
  final int maxBytes;
  final int resumeWaitSecs;

  /// libmpv properties of the plan.
  Map<String, String> get mpvProperties => {
    'cache': 'yes',
    'cache-secs': '$readAheadSecs',
    'demuxer-readahead-secs': '$readAheadSecs',
    'demuxer-max-bytes': '$maxBytes',
    'cache-pause-wait': '$resumeWaitSecs',
  };

  @override
  bool operator ==(Object other) =>
      other is BufferPlan &&
      other.level == level &&
      other.readAheadSecs == readAheadSecs &&
      other.maxBytes == maxBytes &&
      other.resumeWaitSecs == resumeWaitSecs;

  @override
  int get hashCode =>
      Object.hash(level, readAheadSecs, maxBytes, resumeWaitSecs);

  @override
  String toString() =>
      'BufferPlan(${level.name}, ${readAheadSecs}s, ${maxBytes >> 20} MiB)';
}

/// The kind of source behind [uri], as the player opens it: a loopback
/// URL is the core's stream proxy over a share (or the Twitch live proxy
/// for an HLS playlist), any other http(s) URL a media server, the rest a
/// local file.
PlaybackSource sourceOfUri(String uri) {
  final lower = uri.toLowerCase();
  if (lower.startsWith('edl://')) {
    return lower.contains('http://') || lower.contains('https://')
        ? sourceOfUri(lower.substring(lower.indexOf('http')))
        : PlaybackSource.local;
  }
  final u = Uri.tryParse(uri);
  final scheme = u?.scheme.toLowerCase() ?? '';
  if (scheme == 'http' || scheme == 'https') {
    final host = u!.host.toLowerCase();
    final loopback =
        host == '127.0.0.1' || host == 'localhost' || host == '::1';
    if (!loopback) return PlaybackSource.server;
    return u.path.toLowerCase().endsWith('.m3u8')
        ? PlaybackSource.live
        : PlaybackSource.network;
  }
  if (const {
    'smb',
    'webdav',
    'davs',
    'dav',
    'sftp',
    'ftp',
    'ftps',
    'nfs',
  }.contains(scheme)) {
    return PlaybackSource.network;
  }
  return PlaybackSource.local;
}

/// Picks the read-ahead for one item from what the engine measures.
///
/// - Local files and live streams get no plan: libmpv's defaults are
///   right for them (live keeps its short cache on purpose, ADR-0052).
/// - A share or a server starts at [BufferLevel.normal].
/// - A stall (playback stopped to refill, not after a seek) moves one
///   level up.
/// - The network speed per stream bitrate, measured while the cache is
///   filling: below 1.5 at least [BufferLevel.cautious], below 1.1
///   [BufferLevel.lossy].
/// - The level never goes down for the same item, so the buffer does not
///   flap; the next file starts over.
/// - Memory: the read-ahead time at the stream bitrate plus a quarter,
///   between 32 MiB and [maxBytesCap] (512 MiB on desktops, 192 MiB on
///   phones and TV boxes).
class AdaptiveBuffer {
  AdaptiveBuffer(this.source, {this.maxBytesCap = desktopCap});

  static const int minBytes = 32 << 20;
  static const int desktopCap = 512 << 20;
  static const int mobileCap = 192 << 20;

  /// Speed below this many times the bitrate: cautious; below
  /// [lossyHeadroom]: lossy.
  /// The thresholds of the indicator «сеть против битрейта» (QA-007).
  static const double cautiousHeadroom = networkEnoughHeadroom;
  static const double lossyHeadroom = networkShortHeadroom;

  final PlaybackSource source;
  final int maxBytesCap;

  BufferLevel _level = BufferLevel.normal;
  int _stalls = 0;
  double? _networkBps;
  int? _bitrateBps;

  bool get enabled =>
      source == PlaybackSource.network || source == PlaybackSource.server;

  BufferLevel get level => _level;
  int get stalls => _stalls;

  /// Smoothed read speed of the source, bits per second.
  int? get networkBps => _networkBps?.round();
  int? get bitrateBps => _bitrateBps;

  /// The plan for now, or null when libmpv's defaults stay.
  BufferPlan? get plan {
    if (!enabled) return null;
    final secs = _level.readAheadSecs;
    final rate = _bitrateBps;
    var bytes = rate == null ? minBytes : (rate / 8 * secs * 1.25).round();
    if (bytes < minBytes) bytes = minBytes;
    if (bytes > maxBytesCap) bytes = maxBytesCap;
    return BufferPlan(
      level: _level,
      readAheadSecs: secs,
      maxBytes: bytes,
      resumeWaitSecs: _level.resumeWaitSecs,
    );
  }

  /// One measurement: the stream bitrate, the cache read speed (bits per
  /// second), and how much is buffered. Speeds count only while the cache
  /// is filling: a full cache stops reading and would look like a slow
  /// network.
  ///
  /// [idle] is libmpv's `demuxer-cache-idle`: the cache is not reading
  /// (the whole file is in, or it is full). Its speed is a one-second
  /// window that decays to zero after reading stops, so it does not count
  /// either; on a 2× link it dragged the speed to ×1.4 at the end of a
  /// film and moved the buffer up for nothing (QA-007).
  void sample({
    int? bitrateBps,
    int? speedBps,
    double? bufferedSecs,
    bool? idle,
  }) {
    if (bitrateBps != null && bitrateBps > 0) _bitrateBps = bitrateBps;
    if (!enabled) return;
    final target = _level.readAheadSecs;
    final filling =
        idle != true && (bufferedSecs == null || bufferedSecs < target * 0.9);
    if (speedBps != null && speedBps > 0 && filling) {
      final prev = _networkBps;
      _networkBps = prev == null
          ? speedBps.toDouble()
          : prev * 0.7 + speedBps * 0.3;
    }
    final n = _networkBps, b = _bitrateBps;
    if (n != null && b != null && b > 0) {
      final headroom = n / b;
      if (headroom < lossyHeadroom) {
        _raise(BufferLevel.lossy);
      } else if (headroom < cautiousHeadroom) {
        _raise(BufferLevel.cautious);
      }
    }
  }

  /// Playback stopped to refill the buffer.
  void stall() {
    _stalls++;
    if (!enabled) return;
    _raise(
      BufferLevel.values[(_level.index + 1).clamp(
        0,
        BufferLevel.values.length - 1,
      )],
    );
  }

  void _raise(BufferLevel to) {
    if (to.index > _level.index) _level = to;
  }
}
