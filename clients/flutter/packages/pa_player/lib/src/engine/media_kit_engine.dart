// SPDX-License-Identifier: MPL-2.0
import 'dart:async';
import 'dart:io' show File, Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:media_kit/media_kit.dart' as mk;
import 'package:media_kit_video/media_kit_video.dart' as mkv;

import '../play_request.dart';
import '../subtitles/subtitle_policy.dart';
import '../subtitles/subtitle_rendering.dart';
import '../video/osd_model.dart';
import 'audio_profile.dart';
import 'parts.dart';
import 'player_engine.dart';

/// Initialises libmpv bindings. Call once in `main()` before `runApp`.
void ensurePaPlayerInitialized() =>
    mk.MediaKit.ensureInitialized(libmpv: bundledLibmpvPath());

/// The LGPL libmpv shipped inside the Linux bundle (`lib/libmpv.so.2`, put
/// there by tools/mpv/bundle.sh, PLAY-002), or null to let media_kit search
/// (LIBMPV_LIBRARY_PATH, then the system). Opening it by path keeps a system
/// libmpv from being loaded next to the bundled one.
String? bundledLibmpvPath() {
  if (kIsWeb || !Platform.isLinux) return null;
  if (Platform.environment['LIBMPV_LIBRARY_PATH'] != null) return null;
  final path =
      '${File(Platform.resolvedExecutable).parent.path}/lib/libmpv.so.2';
  return File(path).existsSync() ? path : null;
}

/// [PlayerEngine] on media_kit / libmpv (ADR-0012).
class MediaKitEngine
    implements
        PlayerEngine,
        StatsEngine,
        SubtitleRenderingEngine,
        PlaylistEditingEngine {
  MediaKitEngine({bool video = true, this.mpvOptions = const {}})
    : _player = mk.Player(
        configuration: const mk.PlayerConfiguration(
          title: 'Kinolune',
          logLevel: mk.MPVLogLevel.warn,
        ),
      ) {
    if (video) _controller = mkv.VideoController(_player);
    _subscribe();
    _ready = _configure(video: video);
  }

  /// Factory matching [PlayerEngineFactory].
  static PlayerEngine create({bool video = true}) =>
      MediaKitEngine(video: video, mpvOptions: defaultMpvOptions);

  /// Extra mpv properties applied to every engine made by [create]
  /// (diagnostics, e.g. `{'ao': 'null'}` on a headless machine).
  static Map<String, String> defaultMpvOptions = const {};

  /// Extra mpv properties for this engine.
  final Map<String, String> mpvOptions;

  final mk.Player _player;
  mkv.VideoController? _controller;
  final ValueNotifier<EngineState> _state = ValueNotifier(const EngineState());
  final List<StreamSubscription<Object?>> _subs = [];
  bool _disposed = false;
  late final Future<void> _ready;
  int _chapterGeneration = 0;
  Timer? _statsTimer;

  /// Subtitles added with [addSubtitle], by track title: their flags are
  /// not in mpv's track list.
  final Map<String, ExternalSubtitle> _external = {};

  /// The URI last opened with [open]: an `edl://` timeline of parts gets
  /// the subtitles of its later parts shifted onto it ([addSubtitle]).
  String? _opened;

  /// How subtitles are drawn (PLAY-006): set by the screen from the user's
  /// settings, applied on every track change.
  SubtitleRenderOptions _renderOptions = const SubtitleRenderOptions();

  /// Android: system fonts for libass (`sub-fonts-dir`), see
  /// [SubtitleFontDir].
  SubtitleFontDir? _fonts;

  /// Subtitle properties last set on mpv, to skip repeats.
  final Map<String, String> _subProps = {};

  /// Direct access for advanced use (diagnostics, app-specific mpv options).
  mk.Player get player => _player;

  @override
  ValueListenable<EngineState> get state => _state;

  void _set(EngineState s) {
    if (!_disposed) _state.value = s;
  }

  Future<void> _configure({required bool video}) async {
    final p = _player.platform;
    if (p is! mk.NativePlayer) return;
    try {
      // Gapless playback for music queues (mpv: "weak" by default).
      await p.setProperty('gapless-audio', 'yes');
      await p.setProperty('prefetch-playlist', 'yes');
      // External subtitles next to the file are picked up automatically,
      // unless the library passes them (open(autoLoadSubtitles: false)).
      await p.setProperty('sub-auto', 'fuzzy');
      if (!video) await p.setProperty('vid', 'no');
      for (final e in mpvOptions.entries) {
        await p.setProperty(e.key, e.value);
      }
    } catch (e) {
      debugPrint('pa_player: mpv option failed: $e');
    }
    if (!video) return;
    // libass on Android has no system font provider: give it the system
    // fonts through a directory of links (PLAY-006).
    final fonts = _fonts = await SubtitleFontDir.android();
    if (fonts != null && !mpvOptions.containsKey('sub-fonts-dir')) {
      try {
        await p.setProperty('sub-fonts-dir', fonts.path);
        // Before any track is set up; per-language later.
        final family = fonts.fallbackFamily(null);
        await p.setProperty('sub-font', family);
        _subProps['sub-font'] = family;
      } catch (e) {
        debugPrint('pa_player: subtitle fonts failed: $e');
      }
    }
    // mpv's own selection is the truth: media_kit does not see tracks
    // selected by mpv or by `sub-add … select`.
    // Not awaited: media_kit waits for the video controller first.
    unawaited(
      p
          .observeProperty('sid', (_) async => _applySubtitleRendering())
          .catchError((Object e) => debugPrint('pa_player: sid: $e')),
    );
  }

  @override
  Future<void> setSubtitleRendering(SubtitleRenderOptions options) async {
    _renderOptions = options;
    await _ready;
    await _applySubtitleRendering();
  }

  /// Lets libmpv draw the selected subtitle track (styled ASS, bitmaps) or
  /// leaves it to the overlay, and publishes the choice in
  /// [EngineState.subtitleRenderer].
  Future<void> _applySubtitleRendering() =>
      _subtitleApply = _subtitleApply.then((_) => _applySubtitleRenderingNow());

  /// Serialises [_applySubtitleRendering] calls (track changes, settings).
  Future<void> _subtitleApply = Future.value();

  /// The subtitle track mpv shows now, with codec and language.
  Future<MediaTrack?> _selectedSubtitle() async {
    final p = _player.platform;
    if (!_disposed && p is mk.NativePlayer) {
      try {
        final sid = (await p.getProperty('sid')).trim();
        if (sid.isEmpty || sid == 'no' || sid == 'auto') return MediaTrack.off;
        final codec = (await p.getProperty('current-tracks/sub/codec')).trim();
        final lang = (await p.getProperty('current-tracks/sub/lang')).trim();
        return MediaTrack(
          id: sid,
          codec: codec.isEmpty ? null : codec,
          language: lang.isEmpty ? null : lang,
        );
      } catch (e) {
        debugPrint('pa_player: subtitle track not read: $e');
      }
    }
    final s = _state.value;
    final selected = s.subtitleTrack;
    if (selected == null || selected.isOff) return selected;
    return s.subtitleTracks.where((t) => t.id == selected.id).firstOrNull ??
        selected;
  }

  Future<void> _applySubtitleRenderingNow() async {
    if (_disposed) return;
    final track = await _selectedSubtitle();
    final renderer = subtitleRendererFor(track, _renderOptions);
    final props = <String, String>{};
    final fonts = _fonts;
    if (fonts != null && renderer == SubtitleRenderer.engine) {
      // libass reads the fallback family only when the track's renderer is
      // set up (`sub-font` itself just redraws): put it first, and when it
      // changes under an engine-drawn track, set the renderer up again
      // (`sub-ass` is a "hard" subtitle option in mpv).
      final family = fonts.fallbackFamily(track?.language);
      if (_subProps['sub-font'] != family) {
        props['sub-font'] = family;
        if (_subProps['sub-ass'] == 'yes') props['sub-ass'] = 'no';
      }
    }
    final wanted = mpvSubtitleProperties(renderer, _renderOptions);
    if (props['sub-ass'] == 'no' && wanted['sub-ass'] == 'yes') {
      await _setSubProps(props);
      props.clear();
    }
    props.addAll(wanted);
    final now = _state.value;
    // media_kit does not see a track mpv selected itself (`subs-fallback`,
    // `sub-add … select`): take mpv's selection, so the track panel and
    // the overlay agree with what is on screen.
    final listed = track == null || track.isOff
        ? null
        : now.subtitleTracks.where((t) => t.id == track.id).firstOrNull;
    final syncTrack = listed != null && (now.subtitleTrack?.id != listed.id);
    if (now.subtitleRenderer != renderer || syncTrack) {
      _set(
        now.copyWith(
          subtitleRenderer: renderer,
          subtitleTrack: syncTrack ? listed : null,
        ),
      );
    }
    await _setSubProps(props);
  }

  /// Sets [props] in order, skipping values mpv already has.
  Future<void> _setSubProps(Map<String, String> props) async {
    final p = _player.platform;
    if (_disposed || p is! mk.NativePlayer) return;
    for (final e in props.entries) {
      // Diagnostics overrides (mpvOptions) win.
      if (mpvOptions.containsKey(e.key) || _subProps[e.key] == e.value) {
        continue;
      }
      try {
        await p.setProperty(e.key, e.value);
        _subProps[e.key] = e.value;
      } catch (err) {
        debugPrint('pa_player: mpv ${e.key}=${e.value} failed: $err');
      }
    }
  }

  void _subscribe() {
    final s = _player.stream;
    _subs
      ..add(s.position.listen((v) => _set(_state.value.copyWith(position: v))))
      ..add(
        s.duration.listen((v) {
          _set(_state.value.copyWith(duration: v));
          if (v > Duration.zero) _readChapters();
        }),
      )
      ..add(s.buffer.listen((v) => _set(_state.value.copyWith(buffer: v))))
      ..add(s.playing.listen((v) => _set(_state.value.copyWith(playing: v))))
      ..add(
        s.buffering.listen((v) => _set(_state.value.copyWith(buffering: v))),
      )
      ..add(
        s.completed.listen((v) => _set(_state.value.copyWith(completed: v))),
      )
      ..add(s.rate.listen((v) => _set(_state.value.copyWith(rate: v))))
      ..add(s.volume.listen((v) => _set(_state.value.copyWith(volume: v))))
      ..add(
        s.playlist.listen(
          (v) => _set(_state.value.copyWith(playlistIndex: v.index)),
        ),
      )
      ..add(s.width.listen((v) => _set(_state.value.copyWith(videoWidth: v))))
      ..add(s.height.listen((v) => _set(_state.value.copyWith(videoHeight: v))))
      ..add(
        s.subtitle.listen(
          (v) => _set(
            _state.value.copyWith(
              subtitleLines: [
                for (final line in v)
                  if (line.trim().isNotEmpty) line.trim(),
              ],
            ),
          ),
        ),
      )
      ..add(s.tracks.listen(_onTracks))
      ..add(s.track.listen(_onTrack))
      ..add(s.videoParams.listen((_) => _refreshInfo()))
      ..add(s.audioParams.listen((_) => _refreshInfo()))
      ..add(
        s.error.listen((e) {
          debugPrint('pa_player: mpv error: $e');
          _set(_state.value.copyWith(error: e));
        }),
      );
  }

  static MediaTrack _audio(mk.AudioTrack t) => MediaTrack(
    id: t.id,
    title: t.title,
    language: t.language,
    codec: t.codec,
    external: t.uri,
  );

  MediaTrack _sub(mk.SubtitleTrack t) {
    final ext = t.title == null ? null : _external[t.title];
    if (ext != null) {
      return MediaTrack(
        id: t.id,
        title: t.title,
        language: ext.language ?? t.language,
        codec: t.codec,
        external: true,
        forced: ext.forced,
        hearingImpaired: ext.hearingImpaired,
        isDefault: ext.isDefault,
      );
    }
    final flags = flagsFromTitle(t.title);
    return MediaTrack(
      id: t.id,
      title: t.title,
      language: t.language,
      codec: t.codec,
      external: t.uri || t.data,
      forced: flags.forced,
      hearingImpaired: flags.hearingImpaired,
      isDefault: t.isDefault ?? false,
    );
  }

  static bool _real(String id) => id != 'auto' && id != 'no';

  void _onTracks(mk.Tracks t) {
    _set(
      _state.value.copyWith(
        audioTracks: t.audio.where((a) => _real(a.id)).map(_audio).toList(),
        subtitleTracks: t.subtitle.where((a) => _real(a.id)).map(_sub).toList(),
      ),
    );
    _refreshInfo();
  }

  void _onTrack(mk.Track t) {
    final subtitle = _real(t.subtitle.id) ? _sub(t.subtitle) : MediaTrack.off;
    _set(
      _state.value.copyWith(
        audioTrack: _audio(t.audio),
        subtitleTrack: subtitle,
      ),
    );
    _refreshInfo();
  }

  void _refreshInfo() {
    final st = _player.state;
    final v = st.videoParams;
    final a = st.audioParams;
    final vt = st.track.video;
    final at = st.track.audio;
    final videoTrack = st.tracks.video.where((t) => t.id == vt.id).firstOrNull;
    final audioTrack = st.tracks.audio.where((t) => t.id == at.id).firstOrNull;
    final info = <String, String>{
      if (st.playlist.medias.isNotEmpty &&
          st.playlist.index < st.playlist.medias.length)
        'File': mediaDisplayName(st.playlist.medias[st.playlist.index].uri),
      if (v.w != null && v.h != null) 'Resolution': '${v.w}×${v.h}',
      if (videoTrack?.codec != null) 'Video codec': videoTrack!.codec!,
      if (videoTrack?.fps != null)
        'Frame rate': '${videoTrack!.fps!.toStringAsFixed(3)} fps',
      if (v.pixelformat != null) 'Pixel format': v.pixelformat!,
      if (v.hwPixelformat != null) 'HW format': v.hwPixelformat!,
      if (v.primaries != null) 'Primaries': v.primaries!,
      if (v.gamma != null) 'Transfer': v.gamma!,
      if (audioTrack?.codec != null) 'Audio codec': audioTrack!.codec!,
      if (a.sampleRate != null) 'Sample rate': '${a.sampleRate} Hz',
      if (a.hrChannels != null || a.channels != null)
        'Channels': (a.hrChannels ?? a.channels)!,
      'Engine': 'media_kit / libmpv',
    };
    _set(_state.value.copyWith(info: info));
  }

  /// Reads the chapters of the open file from mpv (`chapter-list`): the
  /// container's chapters, or one per part of an EDL timeline (LIB-020).
  Future<void> _readChapters() async {
    final p = _player.platform;
    if (_disposed || p is! mk.NativePlayer) return;
    final generation = ++_chapterGeneration;
    try {
      final count = int.tryParse(await p.getProperty('chapter-list/count'));
      final out = <PlaybackChapter>[];
      for (var i = 0; i < (count ?? 0); i++) {
        final time = double.tryParse(
          await p.getProperty('chapter-list/$i/time'),
        );
        if (time == null || time < 0) continue;
        final title = await p.getProperty('chapter-list/$i/title');
        out.add(
          PlaybackChapter(
            start: Duration(milliseconds: (time * 1000).round()),
            title: title.trim().isEmpty ? null : title.trim(),
          ),
        );
      }
      out.sort((a, b) => a.start.compareTo(b.start));
      if (_disposed || generation != _chapterGeneration) return;
      _set(_state.value.copyWith(chapters: out));
    } catch (e) {
      debugPrint('pa_player: chapters not read: $e');
    }
  }

  /// Rows set by [setExtraStats] (live latency, TWI-006).
  Map<String, String> extraStats = const {};

  @override
  void setExtraStats(Map<String, String> rows) => extraStats = rows;

  @override
  Future<void> setStatsActive(bool active) async {
    _statsTimer?.cancel();
    _statsTimer = null;
    if (!active || _disposed) return;
    await _readStats();
    _statsTimer = Timer.periodic(
      const Duration(seconds: 1),
      (_) => _readStats(),
    );
  }

  /// Live numbers for the stats overlay (UI-013), from mpv properties.
  Future<void> _readStats() async {
    final p = _player.platform;
    if (_disposed || p is! mk.NativePlayer) return;
    Future<String?> prop(String name) async {
      try {
        final v = await p.getProperty(name);
        return v.trim().isEmpty ? null : v.trim();
      } catch (_) {
        return null;
      }
    }

    String? kbps(String? bits) {
      final v = double.tryParse(bits ?? '');
      if (v == null || v <= 0) return null;
      return v >= 1e6
          ? '${(v / 1e6).toStringAsFixed(1)} Mbps'
          : '${(v / 1e3).round()} kbps';
    }

    String? fps(String? v) {
      final d = double.tryParse(v ?? '');
      return d == null || d <= 0 ? null : '${d.toStringAsFixed(3)} fps';
    }

    final dropped = await prop('frame-drop-count');
    final decoderDropped = await prop('decoder-frame-drop-count');
    final vbr = kbps(await prop('video-bitrate'));
    final abr = kbps(await prop('audio-bitrate'));
    final cache = double.tryParse(await prop('demuxer-cache-duration') ?? '');
    final hwdec = await prop('hwdec-current');
    final estFps = fps(await prop('estimated-vf-fps'));
    final displayFps = fps(await prop('display-fps'));
    final avsync = double.tryParse(await prop('avsync') ?? '');
    final speed = await prop('cache-speed');
    if (_disposed) return;
    final stats = <String, String>{
      'Dropped frames': '${dropped ?? 0} / ${decoderDropped ?? 0}',
      'Video bitrate': ?vbr,
      'Audio bitrate': ?abr,
      if (cache != null) 'Cache': '${cache.toStringAsFixed(1)} s',
      if (speed != null && (int.tryParse(speed) ?? 0) > 0)
        'Network speed':
            '${((int.parse(speed) * 8) / 1e6).toStringAsFixed(1)} Mbps',
      'Hardware decoding': hwdec == null || hwdec == 'no' ? 'no' : hwdec,
      'Rendered fps': ?estFps,
      'Display fps': ?displayFps,
      if (avsync != null)
        'A/V sync': '${(avsync * 1000).toStringAsFixed(0)} ms',
    };
    _set(_state.value.copyWith(stats: {...stats, ...extraStats}));
  }

  @override
  Future<void> open(
    List<String> uris, {
    int index = 0,
    Duration start = Duration.zero,
    bool play = true,
    bool autoLoadSubtitles = true,
  }) async {
    _external.clear();
    _opened = index < uris.length ? uris[index] : null;
    // Until the new file reports its own, nothing of the old one counts:
    // listeners wait for a duration to know the file is loaded.
    _set(
      _state.value.copyWith(
        completed: false,
        duration: Duration.zero,
        subtitleTracks: const [],
        subtitleLines: const [],
        chapters: const [],
      ),
    );
    _chapterGeneration++;
    await _ready;
    final p = _player.platform;
    if (!_disposed &&
        p is mk.NativePlayer &&
        !mpvOptions.containsKey('sub-auto')) {
      try {
        await p.setProperty('sub-auto', autoLoadSubtitles ? 'fuzzy' : 'no');
      } catch (e) {
        debugPrint('pa_player: mpv sub-auto failed: $e');
      }
    }
    final medias = <mk.Media>[
      for (var i = 0; i < uris.length; i++)
        mk.Media(
          _normalise(uris[i]),
          start: i == index && start > Duration.zero ? start : null,
        ),
    ];
    await _player.open(mk.Playlist(medias, index: index), play: play);
  }

  /// Accepts plain paths as well as URIs.
  static String _normalise(String uri) {
    if (uri.startsWith('/')) return Uri.file(uri).toString();
    return uri;
  }

  @override
  Future<void> play() => _player.play();

  @override
  Future<void> pause() => _player.pause();

  @override
  Future<void> seek(Duration position) => _player.seek(position);

  @override
  Future<void> setRate(double rate) => _player.setRate(rate);

  @override
  Future<void> setVolume(double volume) => _player.setVolume(volume);

  @override
  Future<void> jump(int index) => _player.jump(index);

  @override
  Future<void> insertAt(int index, String uri) async {
    final last = _player.state.playlist.medias.length;
    await _player.add(mk.Media(_normalise(uri)));
    if (index < last) await _player.move(last, index);
  }

  @override
  Future<void> removeAt(int index) => _player.remove(index);

  @override
  Future<void> move(int from, int to) {
    if (from == to) return Future.value();
    // mpv `playlist-move` puts the entry before the one now at the target
    // index: moving down needs the index after the destination.
    return _player.move(from, from < to ? to + 1 : to);
  }

  @override
  Future<void> setRepeat(QueueRepeat mode) =>
      _player.setPlaylistMode(switch (mode) {
        QueueRepeat.off => mk.PlaylistMode.none,
        QueueRepeat.all => mk.PlaylistMode.loop,
        QueueRepeat.one => mk.PlaylistMode.single,
      });

  @override
  Future<void> setAudioProfile(AudioProfile profile) async {
    await _ready;
    final p = _player.platform;
    if (_disposed || p is! mk.NativePlayer) return;
    for (final e in profile.mpvProperties.entries) {
      // Diagnostics overrides (mpvOptions) win over the profile.
      if (mpvOptions.containsKey(e.key)) continue;
      try {
        await p.setProperty(e.key, e.value);
      } catch (err) {
        debugPrint('pa_player: mpv ${e.key}=${e.value} failed: $err');
      }
    }
  }

  @override
  Future<void> setAudioTrack(MediaTrack track) async {
    final t = _player.state.tracks.audio.where((a) => a.id == track.id);
    if (t.isNotEmpty) await _player.setAudioTrack(t.first);
  }

  @override
  Future<void> setSubtitleTrack(MediaTrack track) async {
    if (track.isOff) {
      await _player.setSubtitleTrack(mk.SubtitleTrack.no());
      return;
    }
    final t = _player.state.tracks.subtitle.where((a) => a.id == track.id);
    if (t.isNotEmpty) await _player.setSubtitleTrack(t.first);
  }

  @override
  Future<void> addExternalSubtitle(String uri, {String? title}) =>
      _player.setSubtitleTrack(
        mk.SubtitleTrack.uri(
          _normalise(uri),
          title: title ?? uri.split('/').last,
        ),
      );

  @override
  Future<void> addSubtitle(
    ExternalSubtitle subtitle, {
    bool select = false,
  }) async {
    await _ready;
    final p = _player.platform;
    if (_disposed || p is! mk.NativePlayer) return;
    _external[subtitle.title] = subtitle;
    String path(String uri) =>
        uri.startsWith('file://') ? Uri.parse(uri).toFilePath() : uri;
    var uri = path(subtitle.uri);
    if (subtitle.parts case final parts? when parts.length > 1) {
      // LIB-021: the subtitles of a later part (`Movie.cd2.srt`) start
      // where that part starts on the EDL timeline.
      final joined = await _partSubtitles([
        for (final p in parts) p == null ? null : path(p),
      ]);
      if (joined == null) {
        if (parts.first == null) {
          debugPrint('pa_player: ${subtitle.title}: no part timeline');
          return;
        }
        uri = path(parts.first!);
      } else {
        uri = joined;
      }
    }
    try {
      await p.command([
        'sub-add',
        uri,
        select ? 'select' : 'auto',
        subtitle.title,
        if (subtitle.language != null) subtitle.language!,
      ]);
    } catch (e) {
      debugPrint('pa_player: subtitle ${subtitle.title} not loaded: $e');
    }
  }

  /// One subtitle track of [files] (one per part, null where a part has
  /// none) on the EDL timeline of the open file ([partSubtitlesUri]); the
  /// part lengths come from mpv's chapters of the parts. Null if the open
  /// file is not a timeline of as many parts or mpv does not tell.
  Future<String?> _partSubtitles(List<String?> files) async {
    final p = _player.platform;
    final parts = _opened == null ? null : partsOf(_opened!);
    if (p is! mk.NativePlayer || parts == null) return null;
    if (parts.length != files.length) return null;
    try {
      final count = int.tryParse(await p.getProperty('chapter-list/count'));
      final chapters = <(String, Duration)>[];
      for (var i = 0; i < (count ?? 0); i++) {
        final time = double.tryParse(
          await p.getProperty('chapter-list/$i/time'),
        );
        if (time == null) continue;
        chapters.add((
          await p.getProperty('chapter-list/$i/title'),
          Duration(microseconds: (time * 1e6).round()),
        ));
      }
      final total = double.tryParse(await p.getProperty('duration'));
      if (total == null) return null;
      final lengths = partLengths(
        parts,
        chapters,
        Duration(microseconds: (total * 1e6).round()),
      );
      return lengths == null ? null : partSubtitlesUri(files, lengths);
    } catch (e) {
      debugPrint('pa_player: part timeline not read: $e');
      return null;
    }
  }

  @override
  Future<void> setSubtitleDelay(Duration delay) async {
    await _ready;
    final p = _player.platform;
    if (_disposed || p is! mk.NativePlayer) return;
    try {
      await p.setProperty(
        'sub-delay',
        (delay.inMilliseconds / 1000).toStringAsFixed(3),
      );
      _set(_state.value.copyWith(subtitleDelay: delay));
    } catch (e) {
      debugPrint('pa_player: sub-delay failed: $e');
    }
  }

  @override
  Widget buildVideo(BuildContext context, {BoxFit fit = BoxFit.contain}) {
    final c = _controller;
    if (c == null) return const SizedBox.expand();
    return mkv.Video(
      controller: c,
      fit: fit,
      fill: const Color(0xFF000000),
      controls: mkv.NoVideoControls,
      // Keyboard/remote handling and wakelock live in PlayerScreen.
      wakelock: true,
      // Text subtitles are drawn by PlayerScreen's SubtitleOverlay in the
      // user's style (PLAY-007) from EngineState.subtitleLines; styled ASS
      // and bitmaps by libmpv into the video (PLAY-006).
      subtitleViewConfiguration: const mkv.SubtitleViewConfiguration(
        visible: false,
      ),
    );
  }

  @override
  Future<void> dispose() async {
    if (_disposed) return;
    _disposed = true;
    _statsTimer?.cancel();
    for (final s in _subs) {
      await s.cancel();
    }
    await _player.dispose();
    // _state is left to GC: widgets may still detach listeners after this.
  }
}
