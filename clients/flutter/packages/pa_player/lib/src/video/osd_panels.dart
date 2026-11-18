// SPDX-License-Identifier: MPL-2.0
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../engine/audio_processing.dart';
import '../engine/player_engine.dart';
import '../l10n/pa_player_strings.dart';
import '../play_request.dart';
import '../subtitles/subtitle_search.dart';
import '../subtitles/subtitle_style.dart';
import '../ui/focus_button.dart';
import '../ui/min_tap_target.dart';
import '../ui/tokens.dart';
import 'osd_controller.dart';
import 'osd_model.dart';
import 'osd_overlays.dart' show OsdSidePanelLabels;
import 'track_panel_strings.dart';

/// Playback speeds offered in the speed panel.
const List<double> kPlaybackSpeeds = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

/// Subtitle file extensions shown in the file browser.
const Set<String> kSubtitleExtensions = {
  '.srt',
  '.ass',
  '.ssa',
  '.vtt',
  '.sub',
  '.sup',
  '.idx',
  '.smi',
  '.txt',
};

String formatRate(double r) {
  var s = r.toStringAsFixed(2);
  s = s.replaceFirst(RegExp(r'\.?0+$'), '');
  return '$s×';
}

/// The panel color of a sheet and a TV panel: the panel glass without the
/// see-through, so the OSD controls and the title under it do not show.
const Color _sheetColor = Color(0xFF202029);

/// The OSD panel: the Audio · Subtitles · Video tabs (UI-014), a subtitle
/// file or search, speed, zoom, chapters, info. Beside the picture, as a
/// bottom sheet or across the top ([placement]).
class OsdSidePanel extends StatelessWidget {
  const OsdSidePanel({
    super.key,
    required this.panel,
    required this.engine,
    required this.request,
    required this.osd,
    required this.onClose,
    this.onSelectSubtitle,
    this.onSubtitleFile,
    this.onSubtitleDelay,
    this.onSubtitleDownloaded,
    this.subtitleLanguages = const [],
    this.onSelectAudio,
    this.onAudioDelay,
    this.audioProcessing,
    this.onAudioProcessing,
    this.zoom,
    this.onZoom,
    this.pinchScale,
    this.onResetPinch,
    this.chapters = const [],
    this.onSeek,
    this.statsVisible,
    this.onStats,
    this.onExportStats,
    this.speeds = kPlaybackSpeeds,
    this.placement = OsdPanelPlacement.side,
    this.onSelectVideo,
    this.subtitleSettings,
    this.onSubtitleSettings,
  });

  final OsdPanel panel;

  /// Where the panel opens: beside the picture, as a bottom sheet (phone)
  /// or across the top (TV).
  final OsdPanelPlacement placement;

  /// Picks a video stream (video tab); null: the engine cannot switch them,
  /// the tab only shows the stream.
  final Future<void> Function(MediaTrack track)? onSelectVideo;

  /// The subtitle style and its change (quick style in the subtitles tab,
  /// UI-014); without [onSubtitleSettings] the section is not shown.
  final ValueListenable<SubtitleSettings>? subtitleSettings;
  final ValueChanged<SubtitleSettings>? onSubtitleSettings;
  final PlayerEngine engine;
  final PlayRequest request;
  final OsdController osd;
  final VoidCallback onClose;

  /// Picks a subtitle track (PlayerScreen remembers it for the series);
  /// default: straight to the engine.
  final Future<void> Function(MediaTrack track)? onSelectSubtitle;

  /// Loads a subtitle file picked in the browser.
  final Future<void> Function(String path)? onSubtitleFile;

  /// Sets the subtitle delay (PlayerScreen remembers it for the series).
  final Future<void> Function(Duration delay)? onSubtitleDelay;

  /// Loads and selects a subtitle downloaded by «Найти субтитры»
  /// (PLAY-008); default: straight to the engine.
  final Future<void> Function(ExternalSubtitle subtitle)? onSubtitleDownloaded;

  /// The profile's subtitle languages, best first, for the search.
  final List<String> subtitleLanguages;

  /// Picks an audio track (PlayerScreen remembers it for the series);
  /// default: straight to the engine.
  final Future<void> Function(MediaTrack track)? onSelectAudio;

  /// Sets the audio delay (PLAY-004); null: no delay row (the engine
  /// cannot shift the sound).
  final Future<void> Function(Duration delay)? onAudioDelay;

  /// The profile's sound processing and its change (audio panel quick
  /// toggles, PLAY-004); null: no toggles.
  final ValueListenable<AudioProcessing>? audioProcessing;
  final ValueChanged<AudioProcessing>? onAudioProcessing;

  /// Zoom mode now and its setter (zoom panel).
  final ValueListenable<VideoZoom>? zoom;
  final ValueChanged<VideoZoom>? onZoom;

  /// Pinch zoom now and its reset (zoom panel).
  final ValueListenable<double>? pinchScale;
  final VoidCallback? onResetPinch;

  /// Chapters of the timeline (chapters panel) and the seek to one.
  final List<PlaybackChapter> chapters;
  final ValueChanged<Duration>? onSeek;

  /// Whether the stats overlay is shown, and its switch (info panel).
  final ValueListenable<bool>? statsVisible;
  final ValueChanged<bool>? onStats;

  /// «Сохранить отчёт» in the info panel (PLAY-010): the playback stats
  /// into a diagnostic report, in one press. Null: no such row (the app
  /// has no diagnostics).
  final VoidCallback? onExportStats;

  /// Rates of the speed panel.
  final List<double> speeds;

  /// Step of the delay buttons.
  static const delayStep = Duration(milliseconds: 100);

  /// Step of the audio delay buttons: lip sync needs finer steps.
  static const audioDelayStep = Duration(milliseconds: 50);

  @override
  Widget build(BuildContext context) {
    final l = PaPlayerStrings.of(context);
    final tl = TrackPanelStrings.of(context);
    final title = switch (panel) {
      OsdPanel.audio => l.audio,
      OsdPanel.subtitles => l.subtitles,
      OsdPanel.video => tl.tabVideo,
      OsdPanel.subtitleFile => l.subtitleFile,
      OsdPanel.subtitleSearch => SubtitleSearchStrings.of(
        context,
      ).findSubtitles,
      OsdPanel.speed => l.playbackSpeed,
      OsdPanel.zoom => l.zoom,
      OsdPanel.chapters => l.chapters,
      OsdPanel.info => l.info,
      OsdPanel.none => '',
    };
    final size = MediaQuery.sizeOf(context);
    final close = FocusIconButton(
      key: const ValueKey('panel-close'),
      icon: Icons.close_rounded,
      tooltip: l.close,
      size: 40,
      onPressed: onClose,
    );
    final header = panel.isTrackTab
        ? Row(
            children: [
              Expanded(
                child: _TrackTabs(
                  current: panel,
                  strings: tl,
                  onSelect: osd.openPanel,
                ),
              ),
              close,
            ],
          )
        : Row(
            children: [
              if (panel.isSubtitlePage)
                FocusIconButton(
                  key: const ValueKey('panel-back'),
                  icon: Icons.arrow_back_rounded,
                  tooltip: l.back,
                  size: 40,
                  onPressed: () => osd.openPanel(OsdPanel.subtitles),
                ),
              Expanded(
                child: Padding(
                  padding: EdgeInsets.only(left: panel.isSubtitlePage ? 6 : 14),
                  child: Text(
                    title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      color: PaTokens.text,
                      fontSize: 20,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                ),
              ),
              close,
            ],
          );
    final body = KeyedSubtree(
      key: ValueKey('panel-${panel.name}'),
      child: panel == OsdPanel.subtitleSearch
          ? _subtitleSearch()
          : ValueListenableBuilder<EngineState>(
              valueListenable: engine.state,
              builder: (context, s, _) => ListView(
                padding: EdgeInsets.zero,
                children: switch (panel) {
                  OsdPanel.audio => _audio(s, l),
                  OsdPanel.subtitles => _subtitles(s, l, tl),
                  OsdPanel.video => _video(s, l, tl),
                  OsdPanel.speed => _speed(s, l),
                  OsdPanel.zoom => _zoom(l),
                  OsdPanel.chapters => _chapters(s, l),
                  OsdPanel.info => [
                    if (statsVisible != null && onStats != null)
                      ValueListenableBuilder<bool>(
                        valueListenable: statsVisible!,
                        builder: (context, on, _) => FocusListRow(
                          key: const ValueKey('info-stats'),
                          title: l.showStatistics,
                          leading: const Icon(Icons.query_stats_rounded),
                          selected: on,
                          autofocus: true,
                          onPressed: () => onStats!(!on),
                        ),
                      ),
                    if (onExportStats != null)
                      FocusListRow(
                        key: const ValueKey('info-stats-export'),
                        title: l.exportStats,
                        leading: const Icon(Icons.save_alt_rounded),
                        onPressed: onExportStats!,
                      ),
                    ..._info(s, l),
                  ],
                  OsdPanel.subtitleFile => [
                    SubtitleFileBrowser(
                      startDir: _dirOf(request.uri),
                      onPicked: (path) {
                        final add = onSubtitleFile;
                        add != null
                            ? add(path)
                            : engine.addExternalSubtitle(path);
                        onClose();
                      },
                    ),
                  ],
                  OsdPanel.subtitleSearch || OsdPanel.none => const [],
                },
              ),
            ),
    );
    final content = FocusScope(
      autofocus: true,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (placement == OsdPanelPlacement.bottom)
            Center(
              child: Container(
                width: 36,
                height: 4,
                margin: const EdgeInsets.only(bottom: 8),
                decoration: BoxDecoration(
                  color: PaTokens.track,
                  borderRadius: BorderRadius.circular(2),
                ),
              ),
            ),
          header,
          const SizedBox(height: 8),
          Expanded(
            child: placement == OsdPanelPlacement.top
                // A TV is wide: rows as long as the screen are hard to read.
                ? Center(
                    child: ConstrainedBox(
                      constraints: const BoxConstraints(maxWidth: 760),
                      child: body,
                    ),
                  )
                : body,
          ),
        ],
      ),
    );
    return switch (placement) {
      OsdPanelPlacement.side => Container(
        key: const ValueKey('osd-panel'),
        // Fits a phone in portrait (360 px wide) next to the margins.
        width: size.width < 428 ? size.width - 48 : 380,
        margin: const EdgeInsets.all(24),
        padding: const EdgeInsets.fromLTRB(12, 16, 12, 12),
        decoration: BoxDecoration(
          color: PaTokens.glassStrong,
          borderRadius: BorderRadius.circular(PaTokens.radius),
        ),
        child: content,
      ),
      // A phone: a sheet from the bottom edge, in reach of the thumb;
      // swiping it down closes it.
      OsdPanelPlacement.bottom => GestureDetector(
        onVerticalDragEnd: (d) {
          if ((d.primaryVelocity ?? 0) > 300) onClose();
        },
        child: Container(
          key: const ValueKey('osd-panel'),
          width: size.width > 640 ? 640 : size.width,
          height: size.height * (size.height < 500 ? 0.85 : 0.6),
          padding: const EdgeInsets.fromLTRB(12, 8, 12, 8),
          decoration: const BoxDecoration(
            color: _sheetColor,
            borderRadius: BorderRadius.vertical(
              top: Radius.circular(PaTokens.radius * 1.5),
            ),
          ),
          child: SafeArea(top: false, child: content),
        ),
      ),
      // A TV: across the top, tabs first; the picture stays visible below.
      OsdPanelPlacement.top => Container(
        key: const ValueKey('osd-panel'),
        width: size.width - 96,
        height: size.height * 0.55,
        margin: const EdgeInsets.fromLTRB(48, 24, 48, 0),
        padding: const EdgeInsets.fromLTRB(16, 16, 16, 12),
        decoration: BoxDecoration(
          color: _sheetColor,
          borderRadius: BorderRadius.circular(PaTokens.radius),
        ),
        child: content,
      ),
    };
  }

  static String? _audioTags(MediaTrack t, PaPlayerStrings l) {
    final tags = [
      ?l.channelsLabel(t.channels),
      if (t.commentary) l.commentaryTrack,
      if (t.audioDescription) l.audioDescriptionTrack,
      if (t.external) l.externalFile,
    ];
    return tags.isEmpty ? null : tags.join(' · ');
  }

  List<Widget> _audio(EngineState s, PaPlayerStrings l) {
    final processing = audioProcessing;
    final change = onAudioProcessing;
    return [
      if (s.audioTracks.isEmpty) _Empty(l.noAudioTracks),
      for (final t in s.audioTracks)
        FocusListRow(
          key: ValueKey('audio-${t.id}'),
          title: l.trackLabel(t),
          subtitle: _audioTags(t, l),
          selected: t == s.audioTrack,
          autofocus: t == s.audioTrack,
          onPressed: () {
            final select = onSelectAudio;
            select != null ? select(t) : engine.setAudioTrack(t);
            onClose();
          },
        ),
      if (onAudioDelay != null) ..._audioDelay(s.audioDelay, l),
      if (processing != null && change != null)
        ValueListenableBuilder<AudioProcessing>(
          valueListenable: processing,
          builder: (context, a, _) => Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: _audioToggles(a, change, l),
          ),
        ),
    ];
  }

  List<Widget> _audioDelay(Duration delay, PaPlayerStrings l) {
    final set = onAudioDelay!;
    return [
      _Caption(l.audioDelay),
      Padding(
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 4),
        child: Row(
          children: [
            FocusIconButton(
              key: const ValueKey('audio-delay-earlier'),
              icon: Icons.remove_rounded,
              tooltip: l.soundEarlier,
              size: 44,
              onPressed: () => set(delay - audioDelayStep),
            ),
            Expanded(
              child: Text(
                l.audioDelayLabel(delay),
                key: const ValueKey('audio-delay-value'),
                textAlign: TextAlign.center,
                style: const TextStyle(color: PaTokens.text, fontSize: 15),
              ),
            ),
            FocusIconButton(
              key: const ValueKey('audio-delay-later'),
              icon: Icons.add_rounded,
              tooltip: l.soundLater,
              size: 44,
              onPressed: () => set(delay + audioDelayStep),
            ),
          ],
        ),
      ),
      if (delay != Duration.zero)
        FocusListRow(
          key: const ValueKey('audio-delay-reset'),
          title: l.resetDelay,
          leading: const Icon(Icons.restart_alt_rounded),
          onPressed: () => set(Duration.zero),
        ),
    ];
  }

  List<Widget> _audioToggles(
    AudioProcessing a,
    ValueChanged<AudioProcessing> change,
    PaPlayerStrings l,
  ) {
    final stereo = a.downmix == DownmixMode.stereo;
    final boost = a.clampedBoost;
    return [
      const SizedBox(height: 8),
      FocusListRow(
        key: const ValueKey('audio-downmix'),
        title: l.stereoDownmix,
        leading: const Icon(Icons.headphones_rounded),
        selected: stereo,
        onPressed: () => change(
          a.copyWith(downmix: stereo ? DownmixMode.auto : DownmixMode.stereo),
        ),
      ),
      FocusListRow(
        key: const ValueKey('audio-lfe'),
        title: l.lfeToStereo,
        leading: const Icon(Icons.speaker_rounded),
        selected: a.lfeToStereo,
        onPressed: () => change(a.copyWith(lfeToStereo: !a.lfeToStereo)),
      ),
      FocusListRow(
        key: const ValueKey('audio-normalize'),
        title: l.normalizeDownmix,
        leading: const Icon(Icons.compress_rounded),
        selected: a.normalizeDownmix,
        onPressed: () =>
            change(a.copyWith(normalizeDownmix: !a.normalizeDownmix)),
      ),
      _Caption(l.volumeBoost),
      Padding(
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 4),
        child: Row(
          children: [
            FocusIconButton(
              key: const ValueKey('audio-boost-less'),
              icon: Icons.remove_rounded,
              tooltip: l.volumeBoostLess,
              size: 44,
              onPressed: boost <= AudioProcessing.minBoost
                  ? null
                  : () => change(a.copyWith(boost: a.stepBoost(up: false))),
            ),
            Expanded(
              child: Text(
                l.percent(boost),
                key: const ValueKey('audio-boost-value'),
                textAlign: TextAlign.center,
                style: const TextStyle(color: PaTokens.text, fontSize: 15),
              ),
            ),
            FocusIconButton(
              key: const ValueKey('audio-boost-more'),
              icon: Icons.add_rounded,
              tooltip: l.volumeBoostMore,
              size: 44,
              onPressed: boost >= AudioProcessing.maxBoost
                  ? null
                  : () => change(a.copyWith(boost: a.stepBoost(up: true))),
            ),
          ],
        ),
      ),
    ];
  }

  void _selectSubtitle(MediaTrack t) {
    final select = onSelectSubtitle;
    select != null ? select(t) : engine.setSubtitleTrack(t);
  }

  void _setDelay(Duration d) {
    final set = onSubtitleDelay;
    set != null ? set(d) : engine.setSubtitleDelay(d);
  }

  static String? _tags(MediaTrack t, PaPlayerStrings l) {
    final tags = [
      if (t.forced) l.forcedTrack,
      if (t.hearingImpaired) l.sdhTrack,
      if (t.external) l.externalFile,
    ];
    return tags.isEmpty ? null : tags.join(' · ');
  }

  List<Widget> _subtitles(
    EngineState s,
    PaPlayerStrings l,
    TrackPanelStrings tl,
  ) {
    final off = s.subtitleTrack == null || s.subtitleTrack!.isOff;
    final delay = s.subtitleDelay;
    return [
      FocusListRow(
        key: const ValueKey('sub-off'),
        title: l.off,
        selected: off,
        autofocus: off,
        onPressed: () {
          _selectSubtitle(MediaTrack.off);
          onClose();
        },
      ),
      for (final t in s.subtitleTracks)
        FocusListRow(
          key: ValueKey('sub-${t.id}'),
          title: l.trackLabel(t),
          subtitle: _tags(t, l),
          selected: t == s.subtitleTrack,
          autofocus: t == s.subtitleTrack,
          onPressed: () {
            _selectSubtitle(t);
            onClose();
          },
        ),
      _Caption(l.subtitleDelay),
      Padding(
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 4),
        child: Row(
          children: [
            FocusIconButton(
              key: const ValueKey('sub-delay-earlier'),
              icon: Icons.remove_rounded,
              tooltip: l.subtitlesEarlier,
              size: 44,
              onPressed: () => _setDelay(delay - delayStep),
            ),
            Expanded(
              child: Text(
                l.delayLabel(delay),
                key: const ValueKey('sub-delay-value'),
                textAlign: TextAlign.center,
                style: const TextStyle(color: PaTokens.text, fontSize: 15),
              ),
            ),
            FocusIconButton(
              key: const ValueKey('sub-delay-later'),
              icon: Icons.add_rounded,
              tooltip: l.subtitlesLater,
              size: 44,
              onPressed: () => _setDelay(delay + delayStep),
            ),
          ],
        ),
      ),
      if (delay != Duration.zero)
        FocusListRow(
          key: const ValueKey('sub-delay-reset'),
          title: l.resetDelay,
          leading: const Icon(Icons.restart_alt_rounded),
          onPressed: () => _setDelay(Duration.zero),
        ),
      if (request.subtitleSearch != null)
        Builder(
          builder: (context) => FocusListRow(
            key: const ValueKey('sub-search'),
            title: SubtitleSearchStrings.of(context).findSubtitles,
            leading: const Icon(Icons.travel_explore_rounded),
            onPressed: () => osd.openPanel(OsdPanel.subtitleSearch),
          ),
        ),
      FocusListRow(
        key: const ValueKey('sub-file'),
        title: l.loadFromFile,
        leading: const Icon(Icons.folder_open_rounded),
        onPressed: () => osd.openPanel(OsdPanel.subtitleFile),
      ),
      if (subtitleSettings != null && onSubtitleSettings != null)
        ValueListenableBuilder<SubtitleSettings>(
          valueListenable: subtitleSettings!,
          builder: (context, st, _) => Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: _subtitleStyle(st, onSubtitleSettings!, tl),
          ),
        ),
    ];
  }

  /// Quick subtitle style (UI-014): size, height and background, the
  /// changes people make while watching. The rest stays in Settings →
  /// Subtitles. Size and background leave the system style (the system
  /// has its own); the height applies to both.
  List<Widget> _subtitleStyle(
    SubtitleSettings st,
    ValueChanged<SubtitleSettings> change,
    TrackPanelStrings tl,
  ) {
    const scales = SubtitleSettings.scales;
    var i = scales.indexOf(st.scale);
    if (i < 0) i = scales.indexOf(1.0);
    final own = !st.useSystemStyle;
    void size(int step) {
      // From the system style the first step starts at the normal size.
      final next = own ? (i + step).clamp(0, scales.length - 1) : i;
      change(st.copyWith(useSystemStyle: false, scale: scales[next]));
    }

    const backgrounds = SubtitleBackground.values;
    final bg = own ? st.background : SubtitleBackground.none;
    return [
      _Caption(tl.subtitleStyle),
      _Stepper(
        lessKey: const ValueKey('sub-size-less'),
        valueKey: const ValueKey('sub-size-value'),
        moreKey: const ValueKey('sub-size-more'),
        label: tl.textSize,
        value: own ? tl.percent(st.scale * 100) : tl.systemSize,
        lessTooltip: tl.smaller,
        moreTooltip: tl.larger,
        onLess: own && i == 0 ? null : () => size(-1),
        onMore: own && i == scales.length - 1 ? null : () => size(1),
      ),
      _Stepper(
        lessKey: const ValueKey('sub-height-less'),
        valueKey: const ValueKey('sub-height-value'),
        moreKey: const ValueKey('sub-height-more'),
        label: tl.height,
        value: tl.percent(st.position * 100),
        lessTooltip: tl.lower,
        moreTooltip: tl.higher,
        onLess: st.position <= SubtitleSettings.minPosition
            ? null
            : () => change(
                st.copyWith(
                  position: st.position - SubtitleSettings.positionStep,
                ),
              ),
        onMore: st.position >= SubtitleSettings.maxPosition - 1e-9
            ? null
            : () => change(
                st.copyWith(
                  position: st.position + SubtitleSettings.positionStep,
                ),
              ),
      ),
      FocusListRow(
        key: const ValueKey('sub-background'),
        title: tl.background(bg),
        leading: const Icon(Icons.format_color_fill_rounded),
        onPressed: () => change(
          st.copyWith(
            useSystemStyle: false,
            background: backgrounds[(bg.index + 1) % backgrounds.length],
          ),
        ),
      ),
    ];
  }

  /// Video tab (UI-014): the video streams when a file has several, the
  /// stream playing, and the zoom modes.
  List<Widget> _video(EngineState s, PaPlayerStrings l, TrackPanelStrings tl) {
    final tracks = s.videoTracks;
    final select = onSelectVideo;
    final current = s.videoTrack ?? tracks.firstOrNull;
    final choose = tracks.length > 1 && select != null;
    return [
      _Caption(tl.videoTrack),
      if (choose)
        for (final t in tracks)
          FocusListRow(
            key: ValueKey('video-${t.id}'),
            title: tl.videoLabel(t),
            selected: t == current,
            autofocus: t == current,
            onPressed: () {
              select(t);
              onClose();
            },
          )
      else
        Padding(
          key: const ValueKey('video-current'),
          padding: const EdgeInsets.fromLTRB(14, 6, 14, 6),
          child: Text(
            current != null
                ? tl.videoLabel(current)
                : s.videoWidth != null && s.videoHeight != null
                ? '${s.videoWidth}×${s.videoHeight}'
                : tl.noVideoInfo,
            style: const TextStyle(color: PaTokens.text, fontSize: 15),
          ),
        ),
      if (zoom != null) ...[_Caption(l.zoom), ..._zoom(l, autofocus: !choose)],
    ];
  }

  Widget _subtitleSearch() {
    final search = request.subtitleSearch;
    if (search == null) return const SizedBox.shrink();
    return SubtitleSearchView(
      key: ValueKey('subsearch-${request.uri}'),
      search: search,
      languages: subtitleLanguages,
      onDownloaded: (sub) async {
        final add = onSubtitleDownloaded;
        await (add != null ? add(sub) : engine.addSubtitle(sub, select: true));
        onClose();
      },
    );
  }

  List<Widget> _speed(EngineState s, PaPlayerStrings l) => [
    for (final r in speeds)
      FocusListRow(
        key: ValueKey('speed-$r'),
        title: r == 1.0 ? l.normalSpeed : formatRate(r),
        selected: (s.rate - r).abs() < 0.001,
        autofocus: (s.rate - r).abs() < 0.001,
        onPressed: () {
          engine.setRate(r);
          onClose();
        },
      ),
  ];

  /// Name of a zoom mode in the zoom panel.
  static String zoomLabel(VideoZoom z, PaPlayerStrings l) =>
      OsdSidePanelLabels.zoom(z, l);

  List<Widget> _zoom(PaPlayerStrings l, {bool autofocus = true}) {
    final zoom = this.zoom;
    if (zoom == null) return const [];
    return [
      ValueListenableBuilder<VideoZoom>(
        valueListenable: zoom,
        builder: (context, current, _) => Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final z in VideoZoom.values)
              FocusListRow(
                key: ValueKey('zoom-${z.name}'),
                title: zoomLabel(z, l),
                selected: z == current,
                autofocus: autofocus && z == current,
                onPressed: () {
                  onZoom?.call(z);
                  onClose();
                },
              ),
          ],
        ),
      ),
      if (pinchScale != null)
        ValueListenableBuilder<double>(
          valueListenable: pinchScale!,
          builder: (context, scale, _) => scale > 1.001
              ? FocusListRow(
                  key: const ValueKey('zoom-reset-pinch'),
                  title: '${l.resetPinch} (${scale.toStringAsFixed(1)}×)',
                  leading: const Icon(Icons.zoom_out_map_rounded),
                  onPressed: () {
                    onResetPinch?.call();
                    onClose();
                  },
                )
              : const SizedBox.shrink(),
        ),
    ];
  }

  List<Widget> _chapters(EngineState s, PaPlayerStrings l) {
    final current = chapterIndexAt(chapters, s.position);
    return [
      for (var i = 0; i < chapters.length; i++)
        FocusListRow(
          key: ValueKey('chapter-$i'),
          title: chapterTitle(chapters, i, l),
          subtitle: formatDuration(chapters[i].start),
          selected: i == current,
          autofocus: i == current || (current < 0 && i == 0),
          onPressed: () {
            onSeek?.call(chapters[i].start);
            onClose();
          },
        ),
    ];
  }

  List<Widget> _info(EngineState s, PaPlayerStrings l) {
    final rows = <String, String>{
      l.infoTitle: request.title,
      l.infoPosition: '${_fmt(s.position)} / ${_fmt(s.duration)}',
      l.speed: formatRate(s.rate),
      if (s.audioTrack != null) l.audio: l.trackLabel(s.audioTrack!),
      if (s.subtitleTrack != null) l.subtitles: l.trackLabel(s.subtitleTrack!),
      for (final e in s.info.entries) l.engineInfoKey(e.key): e.value,
    };
    return [
      for (final e in rows.entries)
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 6),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              SizedBox(
                width: 120,
                child: Text(
                  e.key,
                  style: const TextStyle(
                    color: PaTokens.textSecondary,
                    fontSize: 14,
                  ),
                ),
              ),
              Expanded(
                child: Text(
                  e.value,
                  style: const TextStyle(color: PaTokens.text, fontSize: 14),
                ),
              ),
            ],
          ),
        ),
    ];
  }

  static String _fmt(Duration d) => formatDuration(d);

  static String? _dirOf(String uri) {
    String? path;
    if (uri.startsWith('/')) {
      path = uri;
    } else if (uri.startsWith('file://')) {
      path = Uri.parse(uri).toFilePath();
    }
    if (path == null) return null;
    return File(path).parent.path;
  }
}

/// Title of chapter [i] of [chapters], or "Chapter N" without one.
String chapterTitle(List<PlaybackChapter> chapters, int i, PaPlayerStrings l) {
  final t = chapters[i].title;
  return t == null || t.isEmpty ? l.chapterNumber(i + 1) : t;
}

class _Empty extends StatelessWidget {
  const _Empty(this.text);
  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.all(14),
    child: Text(
      text,
      style: const TextStyle(color: PaTokens.textSecondary, fontSize: 15),
    ),
  );
}

/// Minimal D-pad-friendly file browser for subtitle files. Starts in the
/// video's folder; no platform file-picker dependency (works on Android TV).
class SubtitleFileBrowser extends StatefulWidget {
  const SubtitleFileBrowser({super.key, this.startDir, required this.onPicked});

  final String? startDir;
  final ValueChanged<String> onPicked;

  @override
  State<SubtitleFileBrowser> createState() => _SubtitleFileBrowserState();
}

class _SubtitleFileBrowserState extends State<SubtitleFileBrowser> {
  late Directory _dir;
  List<FileSystemEntity> _entries = const [];
  bool _failed = false;

  @override
  void initState() {
    super.initState();
    _dir = Directory(
      widget.startDir ??
          Platform.environment['HOME'] ??
          (Platform.isAndroid ? '/storage/emulated/0' : '/'),
    );
    _load();
  }

  void _load() {
    try {
      final all = _dir.listSync(followLinks: false)
        ..sort((a, b) => a.path.toLowerCase().compareTo(b.path.toLowerCase()));
      _entries = [
        ...all.whereType<Directory>().where((d) => !_name(d).startsWith('.')),
        ...all.whereType<File>().where((f) {
          final n = _name(f).toLowerCase();
          final dot = n.lastIndexOf('.');
          return dot > 0 && kSubtitleExtensions.contains(n.substring(dot));
        }),
      ];
      _failed = false;
    } catch (e) {
      _entries = const [];
      _failed = true;
    }
  }

  static String _name(FileSystemEntity e) =>
      e.uri.pathSegments.where((s) => s.isNotEmpty).lastOrNull ?? e.path;

  void _open(Directory d) => setState(() {
    _dir = d;
    _load();
  });

  @override
  Widget build(BuildContext context) {
    final parent = _dir.parent;
    final l = PaPlayerStrings.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(14, 0, 14, 8),
          child: Text(
            _dir.path,
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: const TextStyle(color: PaTokens.textSecondary, fontSize: 13),
          ),
        ),
        if (parent.path != _dir.path)
          FocusListRow(
            title: '..',
            leading: const Icon(Icons.arrow_upward_rounded),
            autofocus: _entries.isEmpty,
            onPressed: () => _open(parent),
          ),
        if (_failed) _Empty(l.cannotOpenFolder),
        if (!_failed && _entries.isEmpty) _Empty(l.noSubtitleFiles),
        for (var i = 0; i < _entries.length; i++)
          FocusListRow(
            key: ValueKey(_entries[i].path),
            title: _name(_entries[i]),
            autofocus: i == 0,
            leading: Icon(
              _entries[i] is Directory
                  ? Icons.folder_rounded
                  : Icons.subtitles_rounded,
            ),
            onPressed: () {
              final e = _entries[i];
              if (e is Directory) {
                _open(e);
              } else {
                widget.onPicked(e.path);
              }
            },
          ),
      ],
    );
  }
}

/// A small heading inside a panel ("Subtitle delay", "Volume Boost").
class _Caption extends StatelessWidget {
  const _Caption(this.text);

  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(14, 12, 14, 0),
    child: Text(
      text,
      style: const TextStyle(color: PaTokens.textSecondary, fontSize: 13),
    ),
  );
}

/// "Text size   −  120 %  +": a value with two step buttons; a null step
/// is at its end (disabled).
class _Stepper extends StatelessWidget {
  const _Stepper({
    required this.lessKey,
    required this.valueKey,
    required this.moreKey,
    required this.label,
    required this.value,
    required this.lessTooltip,
    required this.moreTooltip,
    required this.onLess,
    required this.onMore,
  });

  final Key lessKey;
  final Key valueKey;
  final Key moreKey;
  final String label;
  final String value;
  final String lessTooltip;
  final String moreTooltip;
  final VoidCallback? onLess;
  final VoidCallback? onMore;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(14, 2, 4, 2),
    child: Row(
      children: [
        Expanded(
          child: Text(
            label,
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: const TextStyle(color: PaTokens.text, fontSize: 15),
          ),
        ),
        FocusIconButton(
          key: lessKey,
          icon: Icons.remove_rounded,
          tooltip: lessTooltip,
          size: 40,
          onPressed: onLess,
        ),
        SizedBox(
          width: 96,
          child: Text(
            value,
            key: valueKey,
            textAlign: TextAlign.center,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: const TextStyle(color: PaTokens.text, fontSize: 15),
          ),
        ),
        FocusIconButton(
          key: moreKey,
          icon: Icons.add_rounded,
          tooltip: moreTooltip,
          size: 40,
          onPressed: onMore,
        ),
      ],
    ),
  );
}

/// Audio · Subtitles · Video (UI-014). For a remote the row is one focus
/// stop, so Up from any row of the list lands on it whatever its width:
/// the open tab lights up, Left/Right open the tab next to it at once (no
/// extra press), Down goes back into the list, Right past the last tab
/// reaches the close button. A touch, a click or a screen reader opens a
/// tab directly.
class _TrackTabs extends StatefulWidget {
  const _TrackTabs({
    required this.current,
    required this.strings,
    required this.onSelect,
  });

  final OsdPanel current;
  final TrackPanelStrings strings;
  final ValueChanged<OsdPanel> onSelect;

  static const order = [OsdPanel.audio, OsdPanel.subtitles, OsdPanel.video];

  @override
  State<_TrackTabs> createState() => _TrackTabsState();
}

class _TrackTabsState extends State<_TrackTabs> {
  bool _focused = false;

  KeyEventResult _onKey(FocusNode node, KeyEvent e) {
    if (e is KeyUpEvent) return KeyEventResult.ignored;
    final k = e.logicalKey;
    final step = k == LogicalKeyboardKey.arrowRight
        ? 1
        : k == LogicalKeyboardKey.arrowLeft
        ? -1
        : 0;
    if (step == 0) return KeyEventResult.ignored;
    const order = _TrackTabs.order;
    final next = order.indexOf(widget.current) + step;
    // Past the ends the normal traversal goes on (the close button).
    if (next < 0 || next >= order.length) return KeyEventResult.ignored;
    widget.onSelect(order[next]);
    return KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    final s = widget.strings;
    final labels = {
      OsdPanel.audio: s.tabAudio,
      OsdPanel.subtitles: s.tabSubtitles,
      OsdPanel.video: s.tabVideo,
    };
    return Semantics(
      container: true,
      label: s.tracks,
      child: Focus(
        key: const ValueKey('track-tabs'),
        onKeyEvent: _onKey,
        onFocusChange: (v) => setState(() => _focused = v),
        child: SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          child: Row(
            children: [
              for (final tab in _TrackTabs.order)
                _Tab(
                  key: ValueKey('tab-${tab.name}'),
                  label: labels[tab]!,
                  selected: tab == widget.current,
                  focused: _focused && tab == widget.current,
                  onSelect: () {
                    if (tab != widget.current) widget.onSelect(tab);
                  },
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class _Tab extends StatefulWidget {
  const _Tab({
    super.key,
    required this.label,
    required this.selected,
    required this.focused,
    required this.onSelect,
  });

  final String label;
  final bool selected;

  /// The tab row has the focus and this is its open tab.
  final bool focused;
  final VoidCallback onSelect;

  @override
  State<_Tab> createState() => _TabState();
}

class _TabState extends State<_Tab> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final focused = widget.focused;
    final fg = focused
        ? PaTokens.bg
        : widget.selected
        ? PaTokens.text
        : PaTokens.textSecondary;
    return MinTapTarget(
      child: Semantics(
        button: true,
        selected: widget.selected,
        label: widget.label,
        onTap: widget.onSelect,
        excludeSemantics: true,
        child: MouseRegion(
          cursor: SystemMouseCursors.click,
          onEnter: (_) => setState(() => _hovered = true),
          onExit: (_) => setState(() => _hovered = false),
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: widget.onSelect,
            child: AnimatedScale(
              scale: focused ? 1.06 : 1.0,
              duration: const Duration(milliseconds: 150),
              child: AnimatedContainer(
                duration: const Duration(milliseconds: 120),
                margin: const EdgeInsets.only(right: 4),
                padding: const EdgeInsets.symmetric(
                  horizontal: 14,
                  vertical: 8,
                ),
                decoration: BoxDecoration(
                  color: focused
                      ? PaTokens.focusRing
                      : widget.selected
                      ? const Color(0x33FFFFFF)
                      : _hovered
                      ? const Color(0x1FFFFFFF)
                      : const Color(0x00000000),
                  borderRadius: BorderRadius.circular(20),
                ),
                child: Text(
                  widget.label,
                  style: TextStyle(
                    color: fg,
                    fontSize: 17,
                    fontWeight: widget.selected
                        ? FontWeight.w600
                        : FontWeight.w500,
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
