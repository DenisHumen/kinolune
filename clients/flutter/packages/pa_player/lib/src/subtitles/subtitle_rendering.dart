// SPDX-License-Identifier: MPL-2.0
import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../engine/player_engine.dart' show MediaTrack;
import 'languages.dart';

/// Who draws a subtitle track (PLAY-006, ADR-NNNN).
enum SubtitleRenderer {
  /// The player's [SubtitleOverlay] in the user's style (PLAY-007): plain
  /// text lines. Also "nothing to draw" (subtitles off).
  overlay,

  /// libmpv itself, into the video picture: libass for ASS/SSA with their
  /// own styles, positions and karaoke; FFmpeg's bitmap decoders for PGS,
  /// VobSub and DVB.
  engine,
}

/// Bitmap subtitle codecs, as FFmpeg/mpv name them in the track list.
const Set<String> bitmapSubtitleCodecs = {
  'hdmv_pgs_subtitle',
  'pgssub',
  'dvd_subtitle',
  'dvdsub',
  'vobsub',
  'dvb_subtitle',
  'dvbsub',
  'xsub',
};

/// Styled text codecs that libass renders with their own styles.
const Set<String> assSubtitleCodecs = {'ass', 'ssa'};

bool isBitmapSubtitle(String? codec) =>
    codec != null && bitmapSubtitleCodecs.contains(codec.toLowerCase());

bool isAssSubtitle(String? codec) =>
    codec != null && assSubtitleCodecs.contains(codec.toLowerCase());

/// The renderer for [track]: bitmaps always go to the engine (the overlay
/// cannot draw them), ASS/SSA too unless [SubtitleRenderOptions.overrideAssStyle]
/// asks for the user's style, plain text (SRT, WebVTT, mov_text) to the
/// overlay.
SubtitleRenderer subtitleRendererFor(
  MediaTrack? track,
  SubtitleRenderOptions options,
) {
  if (track == null || track.isOff) return SubtitleRenderer.overlay;
  if (isBitmapSubtitle(track.codec)) return SubtitleRenderer.engine;
  if (isAssSubtitle(track.codec) && !options.overrideAssStyle) {
    return SubtitleRenderer.engine;
  }
  return SubtitleRenderer.overlay;
}

/// What the user chose about engine-drawn subtitles (from
/// [SubtitleSettings]).
@immutable
class SubtitleRenderOptions {
  const SubtitleRenderOptions({
    this.overrideAssStyle = false,
    this.hdrPeakNits = defaultHdrPeakNits,
  });

  /// Draw ASS/SSA as plain text in the user's style instead of their own.
  final bool overrideAssStyle;

  /// Brightness of subtitle white over HDR (PQ) video, in nits. 203 is the
  /// reference white of BT.2408; HDR mastering often puts subtitles at
  /// 200–300. Applies where libmpv composes in HDR (gpu-next output); the
  /// media_kit texture is SDR, where subtitles are at SDR white anyway.
  final int hdrPeakNits;

  static const int defaultHdrPeakNits = 203;

  /// Values offered in the settings.
  static const List<int> hdrPeakChoices = [203, 250, 300];

  /// Allowed range (mpv accepts 10..10000).
  static const int minHdrPeakNits = 100;
  static const int maxHdrPeakNits = 1000;

  static int clampHdrPeak(int nits) =>
      nits.clamp(minHdrPeakNits, maxHdrPeakNits).toInt();

  @override
  bool operator ==(Object other) =>
      other is SubtitleRenderOptions &&
      other.overrideAssStyle == overrideAssStyle &&
      other.hdrPeakNits == hdrPeakNits;

  @override
  int get hashCode => Object.hash(overrideAssStyle, hdrPeakNits);

  @override
  String toString() =>
      'SubtitleRenderOptions(overrideAss: $overrideAssStyle, '
      'hdrPeak: $hdrPeakNits)';
}

/// mpv properties for [renderer]. The engine path turns libmpv's own
/// subtitle drawing on with full ASS; the overlay path turns it off (the
/// overlay reads `sub-text`, which mpv keeps updating).
Map<String, String> mpvSubtitleProperties(
  SubtitleRenderer renderer,
  SubtitleRenderOptions options,
) {
  final engine = renderer == SubtitleRenderer.engine;
  return {
    'sub-visibility': engine ? 'yes' : 'no',
    'sub-ass': engine ? 'yes' : 'no',
    'sub-hdr-peak': '${options.hdrPeakNits}',
    'image-subs-hdr-peak': '${options.hdrPeakNits}',
  };
}

/// Fonts libass can use on Android: links to system fonts in one directory
/// (`sub-fonts-dir`), made by the pa_player plugin. libass there has no
/// system font provider, so these, the fonts attached to the file and the
/// default family are all it has (ADR-NNNN).
@immutable
class SubtitleFontDir {
  const SubtitleFontDir(this.path, this.files);

  final String path;

  /// File names in [path].
  final Set<String> files;

  static const cjk = 'NotoSansCJK-Regular.ttc';
  static const arabic = 'NotoNaskhArabic-Regular.ttf';
  static const hebrew = 'NotoSansHebrew-Regular.ttf';
  static const thai = 'NotoSansThai-Regular.ttf';

  static const MethodChannel _channel = MethodChannel(
    'app.kinolune/captioning',
  );

  static Future<SubtitleFontDir?>? _android;

  /// The directory on Android (made once per process), or null elsewhere
  /// (Linux: fontconfig; Apple: CoreText; Windows: DirectWrite) or if the
  /// plugin is missing.
  static Future<SubtitleFontDir?> android() {
    if (kIsWeb || !Platform.isAndroid) return Future.value(null);
    return _android ??= _load();
  }

  static Future<SubtitleFontDir?> _load() async {
    try {
      final m = await _channel.invokeMapMethod<String, Object?>(
        'subtitleFonts',
      );
      return m == null ? null : fromMap(m);
    } on MissingPluginException {
      return null;
    } on PlatformException catch (e) {
      debugPrint('pa_player: subtitle fonts not linked: $e');
      return null;
    }
  }

  /// From the plugin's answer: `{dir: path, files: [names]}`.
  static SubtitleFontDir? fromMap(Map<String, Object?> m) {
    final dir = m['dir'];
    final files = m['files'];
    if (dir is! String || dir.isEmpty) return null;
    return SubtitleFontDir(dir, {
      if (files is List)
        for (final f in files)
          if (f is String) f,
    });
  }

  /// The family libass falls back to (`sub-font`) for a track in
  /// [language]: libass takes it for text in a font that is missing and for
  /// every glyph the requested font lacks. Noto Sans CJK has Latin, Greek,
  /// Cyrillic and CJK, so it is the default (in the variant of the
  /// language); Arabic, Hebrew and Thai get their Noto font; Roboto where
  /// the device has no Noto CJK.
  String fallbackFamily(String? language) {
    final tag = language?.trim().toLowerCase() ?? '';
    final lang = normalizeLanguage(tag);
    final base = tag.split(RegExp('[-_]')).first;
    if (files.contains(arabic) &&
        (lang == 'ara' ||
            const {'fa', 'fas', 'per', 'ur', 'urd'}.contains(base))) {
      return 'Noto Naskh Arabic';
    }
    if (files.contains(hebrew) && lang == 'heb') return 'Noto Sans Hebrew';
    if (files.contains(thai) && lang == 'tha') return 'Noto Sans Thai';
    if (!files.contains(cjk)) return 'Roboto';
    if (lang == 'chi') {
      final traditional =
          tag.contains('hant') || tag.contains('tw') || tag.contains('hk');
      return traditional ? 'Noto Sans CJK TC' : 'Noto Sans CJK SC';
    }
    if (lang == 'kor') return 'Noto Sans CJK KR';
    return 'Noto Sans CJK JP';
  }
}
