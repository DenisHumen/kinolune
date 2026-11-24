// SPDX-License-Identifier: MPL-2.0
//
// PLAY-012 (ADR-0153): what this build can cast to and what a receiver is
// handed for an item. Pure logic, no platform calls: the tests run it for
// every platform and flavor.

import 'package:flutter/foundation.dart';

import '../integration/library.dart';

/// A way to show a video on another screen.
enum CastTech {
  /// Google Cast (Chromecast, Google TV, TVs with Chromecast built-in),
  /// through the official sender SDK and the Default Media Receiver.
  googleCast,

  /// AirPlay to an Apple TV or an AirPlay TV, through AVPlayer.
  airPlay,
}

/// The casting this build offers on [platform].
///
/// - Google Cast: Android (and iOS once it has a runner) when the build has
///   the Cast SDK ([googleCastBuild] is false for F-Droid, ADR-0007).
/// - AirPlay: iOS and macOS; media_kit/libmpv cannot send video over
///   AirPlay, so the stream goes to an AVPlayer.
/// - A TV is a receiver, not a sender: nothing there. Linux, Windows and the
///   web: nothing (no Cast SDK; ADR-0153 «Не входит»).
List<CastTech> castTechsFor(
  TargetPlatform platform, {
  required bool googleCastBuild,
  bool tv = false,
}) {
  if (tv) return const [];
  return switch (platform) {
    TargetPlatform.android => [if (googleCastBuild) CastTech.googleCast],
    TargetPlatform.iOS => [
      CastTech.airPlay,
      if (googleCastBuild) CastTech.googleCast,
    ],
    TargetPlatform.macOS => const [CastTech.airPlay],
    _ => const [],
  };
}

/// What a receiver plays by itself (no transcode).
@immutable
class ReceiverProfile {
  const ReceiverProfile({
    required this.containers,
    required this.videoCodecs,
    required this.audioCodecs,
    required this.maxHeight,
    this.highBitDepthCodecs = const {},
    this.hdr = const {},
  });

  /// Container ids of [containerOf].
  final Set<String> containers;

  /// FFmpeg codec names.
  final Set<String> videoCodecs;

  /// pa-probe audio slugs of [TechInfo.audioFormats].
  final Set<String> audioCodecs;

  /// Tallest picture, pixels.
  final int maxHeight;

  /// Video codecs it plays at 10 bits and more.
  final Set<String> highBitDepthCodecs;

  /// HDR formats it shows; any other HDR file is not cast directly.
  final Set<HdrFormat> hdr;

  /// The Default Media Receiver on a Chromecast (2nd and 3rd generation)
  /// and most TVs with Chromecast built-in: MP4 and WebM, H.264 up to
  /// 1080p and VP8, AAC, MP3, Opus, Vorbis, FLAC (Google's «Supported
  /// Media for Google Cast»). AC-3 and E-AC-3 only pass through to an
  /// HDMI receiver that takes them, so they count as unsupported.
  static const chromecast = ReceiverProfile(
    containers: {'mp4', 'webm'},
    videoCodecs: {'h264', 'vp8'},
    audioCodecs: {'aac', 'mp3', 'opus', 'vorbis', 'flac'},
    maxHeight: 1080,
  );

  /// Chromecast Ultra and Chromecast with Google TV: also HEVC and VP9 up
  /// to 4K, HDR10 and HLG (Dolby Vision only on some models: not assumed).
  static const chromecastUltra = ReceiverProfile(
    containers: {'mp4', 'webm'},
    videoCodecs: {'h264', 'vp8', 'vp9', 'hevc'},
    audioCodecs: {'aac', 'mp3', 'opus', 'vorbis', 'flac'},
    maxHeight: 2160,
    highBitDepthCodecs: {'hevc', 'vp9'},
    hdr: {HdrFormat.hdr10, HdrFormat.hlg, HdrFormat.hdr10Plus},
  );

  /// AVPlayer's own formats (what it streams over AirPlay): MP4 and
  /// QuickTime, H.264 and HEVC, AAC, AC-3, E-AC-3, MP3, ALAC; HDR10, HLG
  /// and Dolby Vision on an Apple TV 4K.
  static const airPlay = ReceiverProfile(
    containers: {'mp4', 'mov'},
    videoCodecs: {'h264', 'hevc'},
    audioCodecs: {'aac', 'ac3', 'eac3', 'mp3', 'alac'},
    maxHeight: 2160,
    highBitDepthCodecs: {'hevc'},
    hdr: {
      HdrFormat.hdr10,
      HdrFormat.hlg,
      HdrFormat.dolbyVision,
      HdrFormat.hdr10Plus,
    },
  );

  /// The Google Cast profile of a device by its model name: the Ultra,
  /// Google TV devices and Android TVs play 4K HEVC; anything else gets the
  /// conservative [chromecast] profile (the user can still «try anyway»).
  static ReceiverProfile forCastModel(String? model) {
    final m = (model ?? '').toLowerCase();
    const uhd = ['ultra', 'google tv', '4k', 'android tv', 'shield', 'bravia'];
    return uhd.any(m.contains) ? chromecastUltra : chromecast;
  }
}

/// Container id of a file: from the probe's demuxer name ([container]),
/// else from the file name's extension. Null when neither tells.
String? containerOf(String? container, String fileName) {
  final ext = _extension(fileName);
  switch (container?.toLowerCase()) {
    case final c? when c.startsWith('mov,mp4'):
      // One demuxer for MP4 and QuickTime: the extension tells which.
      return ext == 'mov' ? 'mov' : 'mp4';
    case 'matroska,webm':
      return ext == 'webm' ? 'webm' : 'mkv';
    case final c? when c.isNotEmpty:
      return c.split(',').first;
  }
  return switch (ext) {
    'mp4' || 'm4v' => 'mp4',
    'mov' => 'mov',
    'webm' => 'webm',
    'mkv' => 'mkv',
    'avi' => 'avi',
    'ts' || 'm2ts' || 'mts' => 'mpegts',
    'm3u8' => 'hls',
    _ => null,
  };
}

String _extension(String name) {
  final path = Uri.tryParse(name)?.path ?? name;
  final slash = path.lastIndexOf('/');
  final base = slash < 0 ? path : path.substring(slash + 1);
  final dot = base.lastIndexOf('.');
  return dot < 0 ? '' : base.substring(dot + 1).toLowerCase();
}

/// MIME type a receiver is told for a container id.
String mimeOf(String container) => switch (container) {
  'mp4' => 'video/mp4',
  'mov' => 'video/quicktime',
  'webm' => 'video/webm',
  'mkv' => 'video/x-matroska',
  'hls' => hlsMime,
  _ => 'video/mp4',
};

/// MIME type of an HLS playlist.
const hlsMime = 'application/x-mpegURL';

/// Why a receiver cannot play a file as it is.
enum CastProblem {
  /// The container (MKV, AVI, TS…).
  container,

  /// The video codec.
  videoCodec,

  /// The first audio track's codec.
  audioCodec,

  /// Taller than the receiver shows, or 10-bit where it plays 8 only.
  picture,

  /// An HDR format it does not show.
  hdr,

  /// A movie of several files (LIB-020) is one timeline only for libmpv.
  parts,

  /// The file has not been probed and its name says nothing.
  unknown,
}

/// What a receiver gets for an item.
@immutable
sealed class CastPlan {
  const CastPlan();
}

/// The file as it is, through the LAN relay ([CastRelay]).
final class CastDirect extends CastPlan {
  const CastDirect(this.mime);
  final String mime;
}

/// The media server's HLS stream (Jellyfin, Emby, Plex transcode).
final class CastServerStream extends CastPlan {
  const CastServerStream([this.problem]);

  /// Why the file is not sent as it is; null when it could not be judged.
  final CastProblem? problem;
}

/// Neither: the user is told why (and may try the file anyway).
final class CastUnsupported extends CastPlan {
  const CastUnsupported(this.problem, {this.detail, this.mime = 'video/mp4'});
  final CastProblem problem;

  /// The format at fault, for the message (`MKV`, `HEVC`, `DTS`, `2160p`).
  final String? detail;

  /// MIME type for «try anyway».
  final String mime;
}

/// Decides what [receiver] gets for the file [playbackUri] (the item's
/// stream URI, [LibraryItem.playbackUri], or an EDL timeline of parts):
///
/// 1. the file itself when [receiver] plays its container, codecs and
///    picture ([tech]; without a probe — by the extension, MP4 and WebM
///    only);
/// 2. else the media server's transcode when the item is on one
///    ([serverItem]);
/// 3. else [CastUnsupported] with the reason.
CastPlan planCast({
  required String playbackUri,
  required String fileName,
  required TechInfo? tech,
  required ReceiverProfile receiver,
  required bool serverItem,
}) {
  CastPlan refuse(CastProblem p, [String? detail, String mime = 'video/mp4']) =>
      serverItem
      ? CastServerStream(p)
      : CastUnsupported(p, detail: detail, mime: mime);

  if (playbackUri.startsWith('edl://')) return refuse(CastProblem.parts);
  final container = containerOf(tech?.container, fileName);
  if (container == 'hls') return const CastDirect(hlsMime);
  if (container == null) return refuse(CastProblem.unknown);
  final mime = mimeOf(container);
  if (!receiver.containers.contains(container)) {
    return refuse(CastProblem.container, container.toUpperCase(), mime);
  }
  if (tech == null) {
    // No probe yet: MP4 and WebM files are nearly always H.264/VP8 + AAC.
    return CastDirect(mime);
  }
  final video = tech.videoCodec?.toLowerCase();
  if (video != null && !receiver.videoCodecs.contains(video)) {
    return refuse(CastProblem.videoCodec, video.toUpperCase(), mime);
  }
  if (tech.height case final h? when h > receiver.maxHeight) {
    return refuse(CastProblem.picture, '${h}p', mime);
  }
  if ((tech.bitDepth ?? 8) > 8 &&
      video != null &&
      !receiver.highBitDepthCodecs.contains(video)) {
    return refuse(CastProblem.picture, '${tech.bitDepth}-bit', mime);
  }
  // Dolby Vision 8.1 carries an HDR10 base layer: shown as HDR10.
  if (tech.hdr.isNotEmpty && !tech.hdr.any(receiver.hdr.contains)) {
    return refuse(CastProblem.hdr, null, mime);
  }
  final audio = tech.audioFormats
      .where((a) => a != 'atmos' && a != 'dts_x')
      .firstOrNull;
  if (audio != null && !receiver.audioCodecs.contains(audio)) {
    return refuse(CastProblem.audioCodec, _audioName(audio), mime);
  }
  return CastDirect(mime);
}

String _audioName(String slug) => switch (slug) {
  'dts_hd_ma' || 'dts_hd_hra' => 'DTS-HD',
  'truehd' => 'TrueHD',
  'eac3' => 'E-AC-3',
  'ac3' => 'AC-3',
  _ => slug.toUpperCase(),
};

/// Whether [url] can be fetched by another device: not a loopback or
/// link-local address. A server stream that points to this device's
/// loopback (a server reached through a tunnel) cannot be cast.
bool reachableFromLan(String url) {
  final host = Uri.tryParse(url)?.host.toLowerCase();
  if (host == null || host.isEmpty) return false;
  if (host == 'localhost' || host == '::1' || host == '[::1]') return false;
  if (host.startsWith('127.')) return false;
  if (host.startsWith('169.254.') || host.startsWith('fe80')) return false;
  return true;
}
