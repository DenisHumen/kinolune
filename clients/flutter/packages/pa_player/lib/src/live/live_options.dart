// SPDX-License-Identifier: MPL-2.0
//
// libmpv options for live HLS (TWI-006, research 06 §6.4): "Низкая задержка"
// and "Стабильно". The low-latency stream itself (prefetch segments played as
// ordinary segments, ad markers, latency telemetry) comes from the loopback
// live proxy of the Rust core (`pa_twitch::playback::live`, full builds);
// these options make the player start next to the live edge and keep a
// small buffer.

/// Latency mode of a live stream.
enum LiveLatencyMode {
  /// Start at the first prefetch segment, small buffer, resume quickly
  /// after a stall. Target ≈ 3 s behind the broadcaster.
  lowLatency,

  /// Start three complete segments before the edge, larger buffer.
  /// Target ≈ 8 s.
  stable,
}

/// mpv properties for a live engine in [mode]. Pass them as
/// `MediaKitEngine(mpvOptions: …)`: they must be set before the stream is
/// opened.
///
/// - `live_start_index` (FFmpeg HLS demuxer): which playlist entry to
///   start at, counted from the end. The proxy lists Twitch's two prefetch
///   segments last, so −2 is the segment being encoded right now.
/// - `demuxer-lavf-analyzeduration`: probe less before the first frame.
/// - `cache-pause-wait`: how long to buffer after a stall.
/// - `demuxer-seekable-cache` + `demuxer-max-back-bytes`: seeking back
///   within what was already played (DVR inside the cache).
/// - `video-latency-hacks`: mpv's own low-latency tweaks of the video path.
///
/// The playback rate is not touched here: catch-up
/// ([LivePlaybackController]) speeds up by at most 10 % and mpv keeps the
/// pitch (`audio-pitch-correction`, on by default).
Map<String, String> liveMpvOptions(LiveLatencyMode mode) => switch (mode) {
  LiveLatencyMode.lowLatency => const {
    'demuxer-lavf-o': 'live_start_index=-2',
    'demuxer-lavf-analyzeduration': '0.5',
    'cache': 'yes',
    'cache-pause-initial': 'no',
    'cache-pause-wait': '0.3',
    'demuxer-seekable-cache': 'yes',
    'demuxer-max-back-bytes': '64MiB',
    'video-latency-hacks': 'yes',
  },
  LiveLatencyMode.stable => const {
    'demuxer-lavf-o': 'live_start_index=-3',
    'cache': 'yes',
    'cache-pause-initial': 'no',
    'cache-pause-wait': '1',
    'demuxer-seekable-cache': 'yes',
    'demuxer-max-back-bytes': '128MiB',
  },
};
