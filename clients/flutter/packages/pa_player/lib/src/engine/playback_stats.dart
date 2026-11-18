// SPDX-License-Identifier: MPL-2.0
//
// The stats overlay model (PLAY-010, ADR-NNNN): the Dart side of
// `pa_playback::PlaybackStats` in the core. Same fields, same JSON
// (snake_case, null for unknown): the fixture
// core/crates/pa-playback/tests/fixtures/playback_stats.json is read by the
// tests of both sides, and the diagnostic report (CORE-008) carries this
// JSON as `playback.json`.

import 'dart:convert' show JsonEncoder;

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart' show BuildContext;

/// Dynamic range of a video stream or of the display output.
enum DynamicRange {
  sdr('sdr', 'SDR'),
  hdr10('hdr10', 'HDR10'),
  hdr10Plus('hdr10_plus', 'HDR10+'),
  hlg('hlg', 'HLG'),
  dolbyVision5('dolby_vision5', 'Dolby Vision 5'),
  dolbyVision7('dolby_vision7', 'Dolby Vision 7'),
  dolbyVision8('dolby_vision8', 'Dolby Vision 8');

  const DynamicRange(this.wire, this.label);

  /// Name in the JSON of the core.
  final String wire;

  /// Technical name shown as is (not translated).
  final String label;

  bool get isDolbyVision =>
      this == dolbyVision5 || this == dolbyVision7 || this == dolbyVision8;
}

/// Where the bytes come from; drives the adaptive buffer and the
/// transcode hint.
enum PlaybackSource {
  /// A file on this device.
  local('local', 'Local file'),

  /// A share read through the core (SMB, WebDAV, SFTP, FTP): the loopback
  /// proxy.
  network('network', 'Network share'),

  /// A media server stream (Jellyfin, Emby, Plex): it could transcode.
  server('server', 'Media server'),

  /// A live stream (Twitch): the buffer stays short on purpose.
  live('live', 'Live stream');

  const PlaybackSource(this.wire, this.label);
  final String wire;

  /// English label; the overlay translates it.
  final String label;
}

/// How the sound leaves the player.
enum AudioPath {
  passthrough('passthrough', 'Passthrough'),
  decoded('decoded', 'Decoded to PCM'),
  lpcm('lpcm', 'LPCM');

  const AudioPath(this.wire, this.label);
  final String wire;
  final String label;
}

/// Automatic frame rate matching of the display.
enum AfrStatus {
  off('off', 'Off'),
  switched('switched', 'Switched'),
  matched('matched', 'Matched'),
  unsupported('unsupported', 'Unsupported');

  const AfrStatus(this.wire, this.label);
  final String wire;
  final String label;
}

/// Network speed per stream bitrate below which a server stream is better
/// transcoded (`TRANSCODE_HEADROOM` in the core).
const transcodeHeadroom = 1.2;

/// Stalls after which a server stream is better transcoded
/// (`TRANSCODE_STALLS` in the core).
const transcodeStalls = 3;

T? _enum<T extends Enum>(List<T> values, String Function(T) wire, Object? v) {
  if (v is! String) return null;
  for (final e in values) {
    if (wire(e) == v) return e;
  }
  return null;
}

int? _int(Object? v) => v is num ? v.toInt() : null;
double? _double(Object? v) => v is num ? v.toDouble() : null;
String? _str(Object? v) => v is String && v.isNotEmpty ? v : null;
bool? _bool(Object? v) => v is bool ? v : null;

/// Technical numbers of the playing item: input, output, decoder, frames,
/// buffer against the network, AFR and the audio path. Every engine fills
/// what it can measure; `null` is "unknown". [check] holds the rules every
/// engine follows (the contract test, `stats_contract.dart`).
@immutable
class PlaybackStats {
  const PlaybackStats({
    this.videoCodec,
    this.audioCodec,
    this.hwDecoding,
    this.audioPassthrough,
    this.droppedFrames = 0,
    this.bitrateBps,
    this.bufferedMs = 0,
    this.engine,
    this.container,
    this.videoProfile,
    this.videoWidth,
    this.videoHeight,
    this.videoFps,
    this.videoBitDepth,
    this.dynamicRange,
    this.maxCll,
    this.maxFall,
    this.dvProfile,
    this.hwDecoder,
    this.decoderDroppedFrames,
    this.outputWidth,
    this.outputHeight,
    this.outputFps,
    this.outputRange,
    this.afr,
    this.audioPath,
    this.audioChannels,
    this.audioOutputChannels,
    this.audioSampleRate,
    this.source,
    this.networkBps,
    this.bufferTargetMs,
    this.stalls,
  });

  final String? videoCodec;
  final String? audioCodec;

  /// Hardware decoding is active.
  final bool? hwDecoding;

  /// Audio goes to the receiver as a bitstream.
  final bool? audioPassthrough;

  /// Frames dropped on output (late for the display) since open.
  final int droppedFrames;

  /// Bitrate of the stream (video + audio), bits per second.
  final int? bitrateBps;

  /// Media buffered ahead of the playhead.
  final int bufferedMs;

  /// Engine name and version ("libmpv 0.40.0").
  final String? engine;

  /// Container format ("matroska", "mp4").
  final String? container;

  /// Codec profile ("Main 10").
  final String? videoProfile;
  final int? videoWidth;
  final int? videoHeight;
  final double? videoFps;
  final int? videoBitDepth;

  /// Dynamic range of the input.
  final DynamicRange? dynamicRange;

  /// HDR10 static metadata, cd/m².
  final int? maxCll;
  final int? maxFall;

  /// Dolby Vision profile (5, 7, 8).
  final int? dvProfile;

  /// Hardware decoder in use ("vaapi", "mediacodec").
  final String? hwDecoder;

  /// Frames the decoder dropped (could not keep up) since open.
  final int? decoderDroppedFrames;

  /// What goes to the display.
  final int? outputWidth;
  final int? outputHeight;

  /// Refresh rate of the display.
  final double? outputFps;

  /// Dynamic range sent to the display (SDR after tone mapping).
  final DynamicRange? outputRange;
  final AfrStatus? afr;
  final AudioPath? audioPath;
  final int? audioChannels;
  final int? audioOutputChannels;
  final int? audioSampleRate;
  final PlaybackSource? source;

  /// Measured read speed of the source, bits per second.
  final int? networkBps;

  /// Read-ahead the adaptive buffer aims for.
  final int? bufferTargetMs;

  /// Playback stops to refill the buffer since open.
  final int? stalls;

  /// Network speed per stream bitrate (2.0: twice as fast as needed).
  double? get headroom {
    final n = networkBps, b = bitrateBps;
    if (n == null || b == null || b <= 0) return null;
    return n / b;
  }

  /// The source is a media server and it does not keep up: a transcoded
  /// (lower bitrate) stream would play without stops. A share cannot
  /// transcode; it gets a bigger buffer instead.
  bool get transcodeAdvised {
    if (source != PlaybackSource.server) return false;
    final h = headroom;
    return (h != null && h < transcodeHeadroom) ||
        (stalls ?? 0) >= transcodeStalls;
  }

  /// The consistency rules (same as `PlaybackStats::check` in the core):
  /// null when the stats follow them, else the broken rule. Unknown
  /// fields never break a rule.
  String? check() {
    bool pair(Object? a, Object? b) => (a == null) == (b == null);
    bool positive(double? v) => v == null || (v.isFinite && v > 0);
    if (!pair(videoWidth, videoHeight)) {
      return 'video_width and video_height come together';
    }
    if (!pair(outputWidth, outputHeight)) {
      return 'output_width and output_height come together';
    }
    if ([
      videoWidth,
      videoHeight,
      outputWidth,
      outputHeight,
    ].any((v) => v != null && v <= 0)) {
      return 'sizes are positive';
    }
    if (!positive(videoFps) || !positive(outputFps)) {
      return 'frame rates are positive and finite';
    }
    if (hwDecoder != null && hwDecoding != true) {
      return 'hw_decoder needs hw_decoding = true';
    }
    final path = audioPath, pass = audioPassthrough;
    if (path != null &&
        pass != null &&
        (path == AudioPath.passthrough) != pass) {
      return 'audio_path and audio_passthrough agree';
    }
    final range = dynamicRange;
    if ((maxCll != null || maxFall != null) &&
        range != null &&
        range == DynamicRange.sdr) {
      return 'MaxCLL/MaxFALL only for HDR';
    }
    if (dvProfile != null && range != null && !range.isDolbyVision) {
      return 'dv_profile only for Dolby Vision';
    }
    if (networkBps != null && source == PlaybackSource.local) {
      return 'no network speed for a local file';
    }
    if (droppedFrames < 0 ||
        bufferedMs < 0 ||
        (decoderDroppedFrames ?? 0) < 0 ||
        (stalls ?? 0) < 0) {
      return 'counters are not negative';
    }
    if (bufferTargetMs == 0 && (bitrateBps ?? 0) > 0) {
      return 'buffer target is positive';
    }
    return null;
  }

  /// Null when [later] (read after this, same item) only counts up.
  String? checkProgress(PlaybackStats later) {
    if (later.droppedFrames < droppedFrames) {
      return 'dropped_frames never goes down';
    }
    final a = decoderDroppedFrames, b = later.decoderDroppedFrames;
    if (a != null && b != null && b < a) {
      return 'decoder_dropped_frames never goes down';
    }
    final s = stalls, t = later.stalls;
    if (s != null && t != null && t < s) return 'stalls never go down';
    return null;
  }

  factory PlaybackStats.fromJson(Map<String, Object?> j) => PlaybackStats(
    videoCodec: _str(j['video_codec']),
    audioCodec: _str(j['audio_codec']),
    hwDecoding: _bool(j['hw_decoding']),
    audioPassthrough: _bool(j['audio_passthrough']),
    droppedFrames: _int(j['dropped_frames']) ?? 0,
    bitrateBps: _int(j['bitrate_bps']),
    bufferedMs: _int(j['buffered_ms']) ?? 0,
    engine: _str(j['engine']),
    container: _str(j['container']),
    videoProfile: _str(j['video_profile']),
    videoWidth: _int(j['video_width']),
    videoHeight: _int(j['video_height']),
    videoFps: _double(j['video_fps']),
    videoBitDepth: _int(j['video_bit_depth']),
    dynamicRange: _enum(DynamicRange.values, (e) => e.wire, j['dynamic_range']),
    maxCll: _int(j['max_cll']),
    maxFall: _int(j['max_fall']),
    dvProfile: _int(j['dv_profile']),
    hwDecoder: _str(j['hw_decoder']),
    decoderDroppedFrames: _int(j['decoder_dropped_frames']),
    outputWidth: _int(j['output_width']),
    outputHeight: _int(j['output_height']),
    outputFps: _double(j['output_fps']),
    outputRange: _enum(DynamicRange.values, (e) => e.wire, j['output_range']),
    afr: _enum(AfrStatus.values, (e) => e.wire, j['afr']),
    audioPath: _enum(AudioPath.values, (e) => e.wire, j['audio_path']),
    audioChannels: _int(j['audio_channels']),
    audioOutputChannels: _int(j['audio_output_channels']),
    audioSampleRate: _int(j['audio_sample_rate']),
    source: _enum(PlaybackSource.values, (e) => e.wire, j['source']),
    networkBps: _int(j['network_bps']),
    bufferTargetMs: _int(j['buffer_target_ms']),
    stalls: _int(j['stalls']),
  );

  /// The JSON of the core: every field, null when unknown.
  Map<String, Object?> toJson() => {
    'video_codec': videoCodec,
    'audio_codec': audioCodec,
    'hw_decoding': hwDecoding,
    'audio_passthrough': audioPassthrough,
    'dropped_frames': droppedFrames,
    'bitrate_bps': bitrateBps,
    'buffered_ms': bufferedMs,
    'engine': engine,
    'container': container,
    'video_profile': videoProfile,
    'video_width': videoWidth,
    'video_height': videoHeight,
    'video_fps': videoFps,
    'video_bit_depth': videoBitDepth,
    'dynamic_range': dynamicRange?.wire,
    'max_cll': maxCll,
    'max_fall': maxFall,
    'dv_profile': dvProfile,
    'hw_decoder': hwDecoder,
    'decoder_dropped_frames': decoderDroppedFrames,
    'output_width': outputWidth,
    'output_height': outputHeight,
    'output_fps': outputFps,
    'output_range': outputRange?.wire,
    'afr': afr?.wire,
    'audio_path': audioPath?.wire,
    'audio_channels': audioChannels,
    'audio_output_channels': audioOutputChannels,
    'audio_sample_rate': audioSampleRate,
    'source': source?.wire,
    'network_bps': networkBps,
    'buffer_target_ms': bufferTargetMs,
    'stalls': stalls,
  };

  /// Rows of the overlay and of the report: English label and value.
  /// [t] translates the English words of labels and values (the overlay
  /// passes `PaPlayerStrings.engineInfoKey`; the report keeps English).
  List<(String, String)> rows([String Function(String) t = _same]) {
    String size(int? w, int? h) => w == null || h == null ? '' : '$w×$h';
    String join(Iterable<String?> parts) =>
        parts.whereType<String>().where((s) => s.isNotEmpty).join(' · ');
    final video = join([
      [videoCodec, videoProfile].whereType<String>().join(' '),
      size(videoWidth, videoHeight),
      if (videoFps != null) '${_fps(videoFps!)} fps',
      if (videoBitDepth != null) '$videoBitDepth-bit',
    ]);
    final hdr = join([
      dynamicRange?.label,
      if (maxCll != null) 'MaxCLL $maxCll',
      if (maxFall != null) 'MaxFALL $maxFall',
      if (dvProfile != null && dynamicRange == null) 'DV $dvProfile',
    ]);
    final output = join([
      size(outputWidth, outputHeight),
      if (outputFps != null) '${_fps(outputFps!)} Hz',
      outputRange?.label,
    ]);
    final decoder = switch (hwDecoding) {
      true =>
        hwDecoder == null ? t('Hardware') : '${t('Hardware')} ($hwDecoder)',
      false => t('Software'),
      null => null,
    };
    final audio = join([
      audioCodec,
      if (audioChannels != null)
        audioOutputChannels != null && audioOutputChannels != audioChannels
            ? '$audioChannels → $audioOutputChannels ch'
            : '$audioChannels ch',
      if (audioSampleRate != null) _khz(audioSampleRate!),
    ]);
    final audioRoute =
        audioPath?.label ??
        switch (audioPassthrough) {
          true => AudioPath.passthrough.label,
          _ => null,
        };
    final network = networkBps == null
        ? null
        : headroom == null
        ? _bps(networkBps!)
        : '${_bps(networkBps!)} (×${headroom!.toStringAsFixed(1)})';
    final buffer = bufferTargetMs == null
        ? '${_secs(bufferedMs)} s'
        : '${_secs(bufferedMs)} / ${_secs(bufferTargetMs!)} s';
    return [
      if (engine != null) ('Engine', engine!),
      if (source != null) ('Source', t(source!.label)),
      if (container != null) ('Container', container!),
      if (video.isNotEmpty) ('Video', video),
      if (hdr.isNotEmpty) ('HDR', hdr),
      if (decoder != null) ('Decoder', decoder),
      if (output.isNotEmpty) ('Output', output),
      if (afr != null) ('AFR', t(afr!.label)),
      ('Dropped frames', '$droppedFrames'),
      if (decoderDroppedFrames != null)
        ('Decoder drops', '$decoderDroppedFrames'),
      if (audio.isNotEmpty) ('Audio', audio),
      if (audioRoute != null) ('Audio path', t(audioRoute)),
      if (bitrateBps != null) ('Bitrate', _bps(bitrateBps!)),
      if (network != null) ('Network speed', network),
      ('Buffer', buffer),
      if (stalls != null) ('Stalls', '$stalls'),
    ];
  }

  /// The plain-text report for the diagnostic zip (`playback.txt`):
  /// English rows, then the model JSON.
  String toReport({String? title, Map<String, String> extra = const {}}) {
    final b = StringBuffer('Kinolune playback stats (PLAY-010)\n');
    if (title != null) b.writeln('Title: $title');
    for (final (k, v) in rows()) {
      b.writeln('$k: $v');
    }
    for (final e in extra.entries) {
      b.writeln('${e.key}: ${e.value}');
    }
    if (transcodeAdvised) {
      b.writeln('Hint: the network is slower than the stream; transcode it');
    }
    b
      ..writeln()
      ..writeln(const JsonEncoder.withIndent('  ').convert(toJson()));
    return b.toString();
  }

  @override
  bool operator ==(Object other) =>
      other is PlaybackStats && mapEquals(other.toJson(), toJson());

  @override
  int get hashCode => Object.hashAll(toJson().values);

  @override
  String toString() => 'PlaybackStats(${toJson()})';
}

String _same(String s) => s;

/// What the player hands to the diagnostic report (`extra/` of the zip):
/// the readable stats and the model JSON.
Map<String, String> playbackReportFiles(
  PlaybackStats stats, {
  String? title,
  Map<String, String> extra = const {},
}) => {
  'playback.txt': stats.toReport(title: title, extra: extra),
  'playback.json': const JsonEncoder.withIndent('  ').convert(stats.toJson()),
};

/// Saves the report files (name → text) and tells the user where
/// (PLAY-010); the app writes them into its diagnostic report (CORE-008).
typedef StatsReportExporter =
    Future<void> Function(BuildContext context, Map<String, String> files);

String _fps(double v) =>
    v == v.roundToDouble() ? v.toStringAsFixed(0) : v.toStringAsFixed(3);

/// "48 kHz", "44.1 kHz".
String _khz(int hz) =>
    '${(hz / 1000).toStringAsFixed(hz % 1000 == 0 ? 0 : 1)} kHz';

String _secs(int ms) => (ms / 1000).toStringAsFixed(1);

/// "18.5 Mbps", "320 kbps".
String _bps(int v) => v >= 1000000
    ? '${(v / 1e6).toStringAsFixed(1)} Mbps'
    : '${(v / 1e3).round()} kbps';
