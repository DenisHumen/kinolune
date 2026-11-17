// SPDX-License-Identifier: MPL-2.0
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../engine/audio_processing.dart';
import '../engine/player_engine.dart';
import '../l10n/pa_player_strings.dart';
import '../play_request.dart';
import '../subtitles/subtitle_search.dart';
import '../ui/focus_button.dart';
import '../ui/tokens.dart';
import 'osd_controller.dart';
import 'osd_model.dart';
import 'osd_overlays.dart' show OsdSidePanelLabels;

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

/// Right-hand glass panel: audio, subtitles, subtitle file, speed, info.
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
    this.speeds = kPlaybackSpeeds,
  });

  final OsdPanel panel;
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

  /// Rates of the speed panel.
  final List<double> speeds;

  /// Step of the delay buttons.
  static const delayStep = Duration(milliseconds: 100);

  /// Step of the audio delay buttons: lip sync needs finer steps.
  static const audioDelayStep = Duration(milliseconds: 50);

  @override
  Widget build(BuildContext context) {
    final l = PaPlayerStrings.of(context);
    final title = switch (panel) {
      OsdPanel.audio => l.audio,
      OsdPanel.subtitles => l.subtitles,
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
    return Container(
      key: ValueKey('panel-${panel.name}'),
      // Fits a phone in portrait (360 px wide) next to the margins.
      width: MediaQuery.sizeOf(context).width < 428
          ? MediaQuery.sizeOf(context).width - 48
          : 380,
      margin: const EdgeInsets.all(24),
      padding: const EdgeInsets.fromLTRB(12, 16, 12, 12),
      decoration: BoxDecoration(
        color: PaTokens.glassStrong,
        borderRadius: BorderRadius.circular(PaTokens.radius),
      ),
      child: FocusScope(
        autofocus: true,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                Expanded(
                  child: Padding(
                    padding: const EdgeInsets.only(left: 14),
                    child: Text(
                      title,
                      style: const TextStyle(
                        color: PaTokens.text,
                        fontSize: 20,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ),
                ),
                FocusIconButton(
                  icon: Icons.close_rounded,
                  tooltip: l.close,
                  size: 40,
                  onPressed: onClose,
                ),
              ],
            ),
            const SizedBox(height: 8),
            Expanded(
              child: panel == OsdPanel.subtitleSearch
                  ? _subtitleSearch()
                  : ValueListenableBuilder<EngineState>(
                      valueListenable: engine.state,
                      builder: (context, s, _) => ListView(
                        children: switch (panel) {
                          OsdPanel.audio => _audio(s, l),
                          OsdPanel.subtitles => _subtitles(s, l),
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
                                  leading: const Icon(
                                    Icons.query_stats_rounded,
                                  ),
                                  selected: on,
                                  autofocus: true,
                                  onPressed: () => onStats!(!on),
                                ),
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
            ),
          ],
        ),
      ),
    );
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

  List<Widget> _subtitles(EngineState s, PaPlayerStrings l) {
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

  List<Widget> _zoom(PaPlayerStrings l) {
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
                autofocus: z == current,
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
