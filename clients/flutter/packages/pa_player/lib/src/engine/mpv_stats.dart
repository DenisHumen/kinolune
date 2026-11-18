// SPDX-License-Identifier: MPL-2.0
//
// libmpv → PlaybackStats (PLAY-010): which mpv property fills which field
// of the model. Pure over a property getter, so the mapping is tested
// without libmpv.

import 'adaptive_buffer.dart';
import 'playback_stats.dart';

/// Reads one mpv property; null when it is missing or empty.
typedef MpvProperty = Future<String?> Function(String name);

/// Bitrate of the stream, bits per second: the decoders' own numbers
/// (`video-bitrate` + `audio-bitrate`), else the file size over the
/// duration.
///
/// The decoders' numbers count only after the first second of playback
/// ([played], `time-pos`): until the first frame libmpv reports the size
/// of the first packets (a keyframe), half again the real bitrate on a
/// slow link (QA-007).
int? mpvBitrate({
  double? video,
  double? audio,
  double? fileSize,
  Duration duration = Duration.zero,
  Duration? played,
}) {
  final measured = (video ?? 0) + (audio ?? 0);
  final started = played == null || played >= const Duration(seconds: 1);
  if (measured > 0 && started) return measured.round();
  if (fileSize != null && fileSize > 0 && duration.inMilliseconds > 0) {
    return (fileSize * 8 / (duration.inMilliseconds / 1000)).round();
  }
  return null;
}

/// Bits per sample of a pixel format: `yuv420p10` and `p010` are 10,
/// `nv12` and `yuv420p` 8.
int? bitDepthOf(String? pixelFormat) {
  if (pixelFormat == null) return null;
  final f = pixelFormat.toLowerCase();
  final planar = RegExp(r'p(\d{2})(le|be)?$').firstMatch(f);
  if (planar != null) return int.parse(planar.group(1)!);
  final packed = RegExp(r'^p0(\d{2})').firstMatch(f);
  if (packed != null) return int.parse(packed.group(1)!);
  return 8;
}

/// Dynamic range from the transfer function (`video-params/gamma`) and
/// the Dolby Vision profile of the track.
DynamicRange? rangeOf(String? gamma, int? dvProfile) {
  switch (dvProfile) {
    case 5:
      return DynamicRange.dolbyVision5;
    case 7:
      return DynamicRange.dolbyVision7;
    case 8:
      return DynamicRange.dolbyVision8;
  }
  if (gamma == null) return null;
  return switch (gamma.toLowerCase()) {
    'pq' => DynamicRange.hdr10,
    'hlg' => DynamicRange.hlg,
    _ => DynamicRange.sdr,
  };
}

/// Playback time from mpv's `time-pos`; null when unknown.
Duration? mpvPlayed(String? timePos) {
  final v = double.tryParse(timePos ?? '');
  return v == null || !v.isFinite || v < 0
      ? null
      : Duration(milliseconds: (v * 1000).round());
}

/// `mpv` and `ffmpeg` versions from the `mpv-version` and
/// `ffmpeg-version` properties: `mpv v0.40.0` becomes `0.40.0`, missing
/// values are left out.
Map<String, String> mpvVersions({String? mpv, String? ffmpeg}) => {
  if (mpv != null && mpv.trim().isNotEmpty)
    'mpv': mpv.trim().replaceFirst(RegExp(r'^mpv\s+v?'), ''),
  if (ffmpeg != null && ffmpeg.trim().isNotEmpty) 'ffmpeg': ffmpeg.trim(),
};

/// The model from libmpv properties. [buffer] adds the source, the
/// measured network speed, the read-ahead target and the stalls.
Future<PlaybackStats> readMpvStats(
  MpvProperty prop, {
  AdaptiveBuffer? buffer,
  Duration duration = Duration.zero,
}) async {
  Future<int?> i(String n) async {
    final v = double.tryParse(await prop(n) ?? '');
    return v == null || !v.isFinite ? null : v.round();
  }

  Future<double?> d(String n) async {
    final v = double.tryParse(await prop(n) ?? '');
    return v == null || !v.isFinite || v <= 0 ? null : v;
  }

  int? positive(int? v) => v == null || v <= 0 ? null : v;

  final version = await prop('mpv-version');
  final hwdec = await prop('hwdec-current');
  final hw = hwdec != null && hwdec != 'no';
  final dv = positive(await i('current-tracks/video/dolby-vision-profile'));
  final range = rangeOf(await prop('video-params/gamma'), dv);
  final hdr = range != null && range != DynamicRange.sdr;
  final width = positive(await i('video-params/w'));
  final height = positive(await i('video-params/h'));
  final outWidth = positive(await i('display-width'));
  final outHeight = positive(await i('display-height'));
  final target = await prop('video-target-params/gamma');
  final audioCodec = await prop('audio-codec-name');
  final outFormat = await prop('audio-out-params/format');
  final passthrough = outFormat?.startsWith('spdif') ?? false;
  final AudioPath? path = audioCodec == null && outFormat == null
      ? null
      : passthrough
      ? AudioPath.passthrough
      : (audioCodec?.startsWith('pcm') ?? false)
      ? AudioPath.lpcm
      : AudioPath.decoded;
  final bitrate = mpvBitrate(
    video: await d('video-bitrate'),
    audio: await d('audio-bitrate'),
    fileSize: await d('file-size'),
    duration: duration,
    played: mpvPlayed(await prop('time-pos')),
  );
  final cached = await d('demuxer-cache-duration');
  final source = buffer?.source;
  final network = source == null || source == PlaybackSource.local
      ? null
      : buffer!.networkBps ??
            positive(((await d('cache-speed')) ?? 0).round() * 8);
  final plan = buffer?.plan;
  return PlaybackStats(
    videoCodec: await prop('video-format'),
    audioCodec: audioCodec,
    hwDecoding: hw,
    audioPassthrough: path == null ? null : passthrough,
    droppedFrames: (await i('frame-drop-count')) ?? 0,
    bitrateBps: bitrate,
    bufferedMs: cached == null ? 0 : (cached * 1000).round(),
    engine: version == null
        ? 'libmpv'
        : 'libmpv ${version.replaceFirst(RegExp(r'^mpv\s+v?'), '')}',
    container: await prop('file-format'),
    videoProfile: await prop('current-tracks/video/codec-profile'),
    videoWidth: width != null && height != null ? width : null,
    videoHeight: width != null && height != null ? height : null,
    videoFps: await d('container-fps') ?? await d('estimated-vf-fps'),
    videoBitDepth: bitDepthOf(
      await prop('video-params/hw-pixelformat') ??
          await prop('video-params/pixelformat'),
    ),
    dynamicRange: range,
    maxCll: hdr ? positive(await i('video-params/max-cll')) : null,
    maxFall: hdr ? positive(await i('video-params/max-fall')) : null,
    dvProfile: dv,
    hwDecoder: hw ? hwdec : null,
    decoderDroppedFrames: await i('decoder-frame-drop-count'),
    outputWidth: outWidth != null && outHeight != null ? outWidth : null,
    outputHeight: outWidth != null && outHeight != null ? outHeight : null,
    outputFps: await d('display-fps'),
    outputRange: target == null ? null : rangeOf(target, null),
    // The Flutter player does not switch display modes yet (PLAY-DSK-008,
    // the Android TV task): AFR is off.
    afr: AfrStatus.off,
    audioPath: path,
    audioChannels: positive(await i('audio-params/channel-count')),
    audioOutputChannels: positive(await i('audio-out-params/channel-count')),
    audioSampleRate: positive(await i('audio-params/samplerate')),
    source: source,
    networkBps: network,
    bufferTargetMs: plan == null ? null : plan.readAheadSecs * 1000,
    stalls: buffer?.stalls,
  );
}
