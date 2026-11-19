// SPDX-License-Identifier: MPL-2.0

/// Test helpers: a scriptable [FakePlayerEngine] that needs no libmpv.
library;

export 'src/engine/stats_contract.dart';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

import 'src/engine/audio_processing.dart';
import 'src/engine/audio_profile.dart';
import 'src/engine/playback_stats.dart';
import 'src/engine/player_engine.dart';
import 'src/play_request.dart';
import 'src/subtitles/subtitle_rendering.dart';
import 'src/video/osd_model.dart';

/// In-memory engine. Tests drive it with [emit] / [advance] / [finish] and
/// inspect [calls].
class FakePlayerEngine
    implements
        PlayerEngine,
        StatsEngine,
        SubtitleRenderingEngine,
        PlaylistEditingEngine,
        AudioProcessingEngine,
        VideoTrackEngine {
  FakePlayerEngine({this.loadedDuration = const Duration(minutes: 10)});

  /// Duration reported right after [open].
  Duration loadedDuration;
  final ValueNotifier<EngineState> _state = ValueNotifier(const EngineState());
  final List<String> calls = [];
  List<String> uris = const [];

  /// Last profile passed to [setAudioProfile].
  AudioProfile? audioProfile;

  /// `autoLoadSubtitles` of the last [open].
  bool? autoLoadSubtitles;

  /// Subtitles passed to [addSubtitle] since the last [open].
  final List<ExternalSubtitle> addedSubtitles = [];

  /// Embedded subtitle tracks every [open] reports (before sidecars).
  List<MediaTrack> embeddedSubtitles = const [];

  /// Audio tracks every [open] reports; the first is selected.
  List<MediaTrack> openAudioTracks = const [];

  /// Video tracks every [open] reports; the first is selected.
  List<MediaTrack> openVideoTracks = const [];
  bool disposed = false;

  /// Chapters every [open] reports.
  List<PlaybackChapter> openChapters = const [];

  /// Whether the stats overlay asked for statistics.
  bool statsActive = false;

  /// Last options passed to [setSubtitleRendering] (PLAY-006).
  SubtitleRenderOptions? renderOptions;

  /// Last sound processing passed to [setAudioProcessing] (PLAY-004).
  AudioProcessing? audioProcessing;

  /// Extra rows [EngineState.stats] holds while [statsActive].
  Map<String, String> fakeStats = const {'A/V sync': '0 ms'};

  /// The model [EngineState.playbackStats] holds while [statsActive]
  /// (PLAY-010); it follows the contract (`checkStatsContract`).
  PlaybackStats fakePlaybackStats = const PlaybackStats(
    engine: 'fake',
    videoCodec: 'h264',
    audioCodec: 'aac',
    hwDecoding: false,
    audioPassthrough: false,
    bitrateBps: 8000000,
    videoWidth: 1920,
    videoHeight: 1080,
    videoFps: 24,
    dynamicRange: DynamicRange.sdr,
    audioPath: AudioPath.decoded,
    audioChannels: 2,
    source: PlaybackSource.local,
    decoderDroppedFrames: 0,
    stalls: 0,
  );

  @override
  ValueListenable<EngineState> get state => _state;

  /// Replaces parts of the state.
  void emit(EngineState Function(EngineState s) f) =>
      _state.value = f(_state.value);

  /// Moves the playhead forward by [d].
  void advance(Duration d) => emit((s) => s.copyWith(position: s.position + d));

  /// Simulates end of media.
  void finish() => emit(
    (s) => s.copyWith(position: s.duration, playing: false, completed: true),
  );

  @override
  Future<void> open(
    List<String> uris, {
    int index = 0,
    Duration start = Duration.zero,
    bool play = true,
    bool autoLoadSubtitles = true,
  }) async {
    this.uris = uris;
    this.autoLoadSubtitles = autoLoadSubtitles;
    addedSubtitles.clear();
    calls.add('open:${uris.length}:$index:${start.inMilliseconds}:$play');
    emit(
      (s) => s.copyWith(
        position: start,
        duration: loadedDuration,
        // A new open clears the last failure (BUG-003).
        clearError: true,
        playing: play,
        completed: false,
        playlistIndex: index,
        subtitleTracks: embeddedSubtitles,
        subtitleTrack: MediaTrack.off,
        subtitleLines: const [],
        audioTracks: openAudioTracks.isEmpty ? null : openAudioTracks,
        audioTrack: openAudioTracks.firstOrNull,
        videoTracks: openVideoTracks.isEmpty ? null : openVideoTracks,
        videoTrack: openVideoTracks.firstOrNull,
        chapters: openChapters,
      ),
    );
  }

  @override
  Future<void> setStatsActive(bool active) async {
    calls.add('stats:$active');
    statsActive = active;
    emit(
      (s) => s.copyWith(
        stats: active ? {...fakeStats, ...extraStats} : const {},
        playbackStats: active ? fakePlaybackStats : null,
      ),
    );
  }

  /// Rows from [setExtraStats].
  Map<String, String> extraStats = const {};

  @override
  void setExtraStats(Map<String, String> rows) {
    extraStats = rows;
    if (statsActive) emit((s) => s.copyWith(stats: {...fakeStats, ...rows}));
  }

  /// Shows a subtitle cue (as the engine does while subtitles are on).
  void showSubtitle(List<String> lines) =>
      emit((s) => s.copyWith(subtitleLines: lines));

  @override
  Future<void> play() async {
    calls.add('play');
    emit((s) => s.copyWith(playing: true, completed: false));
  }

  @override
  Future<void> pause() async {
    calls.add('pause');
    emit((s) => s.copyWith(playing: false));
  }

  @override
  Future<void> seek(Duration position) async {
    calls.add('seek:${position.inMilliseconds}');
    emit((s) => s.copyWith(position: position));
  }

  @override
  Future<void> setRate(double rate) async {
    calls.add('rate:$rate');
    emit((s) => s.copyWith(rate: rate));
  }

  @override
  Future<void> setVolume(double volume) async {
    calls.add('volume:$volume');
    emit((s) => s.copyWith(volume: volume));
  }

  @override
  Future<void> jump(int index) async {
    calls.add('jump:$index');
    emit((s) => s.copyWith(playlistIndex: index, position: Duration.zero));
  }

  @override
  Future<void> insertAt(int index, String uri) async {
    calls.add('insert:$index');
    uris = [...uris]..insert(index, uri);
    final cur = state.value.playlistIndex;
    if (index <= cur) emit((s) => s.copyWith(playlistIndex: cur + 1));
  }

  @override
  Future<void> removeAt(int index) async {
    calls.add('remove:$index');
    uris = [...uris]..removeAt(index);
    final cur = state.value.playlistIndex;
    if (index < cur) emit((s) => s.copyWith(playlistIndex: cur - 1));
  }

  @override
  Future<void> move(int from, int to) async {
    calls.add('move:$from:$to');
    final l = [...uris];
    l.insert(to, l.removeAt(from));
    uris = l;
  }

  @override
  Future<void> setRepeat(QueueRepeat mode) async =>
      calls.add('repeat:${mode.name}');

  @override
  Future<void> setAudioProfile(AudioProfile profile) async {
    calls.add('profile:${profile.replayGain.name}');
    audioProfile = profile;
  }

  @override
  Future<void> setVideoTrack(MediaTrack track) async {
    calls.add('video:${track.id}');
    emit((s) => s.copyWith(videoTrack: track));
  }

  @override
  Future<void> setAudioTrack(MediaTrack track) async {
    calls.add('audio:${track.id}');
    emit((s) => s.copyWith(audioTrack: track));
  }

  @override
  Future<void> setSubtitleTrack(MediaTrack track) async {
    calls.add('sub:${track.id}');
    emit((s) => s.copyWith(subtitleTrack: track));
    _route();
  }

  @override
  Future<void> setSubtitleRendering(SubtitleRenderOptions options) async {
    // Not in [calls]: the screen sets it on start, before opening.
    renderOptions = options;
    _route();
  }

  /// Decides the renderer like the real engine does.
  void _route() => emit(
    (s) => s.copyWith(
      subtitleRenderer: subtitleRendererFor(
        s.subtitleTrack,
        renderOptions ?? const SubtitleRenderOptions(),
      ),
    ),
  );

  @override
  Future<void> addExternalSubtitle(String uri, {String? title}) async {
    calls.add('subfile:$uri');
    final t = MediaTrack(
      id: 'ext${uris.length}',
      title: title ?? uri,
      external: true,
    );
    emit(
      (s) => s.copyWith(
        subtitleTracks: [...s.subtitleTracks, t],
        subtitleTrack: t,
      ),
    );
  }

  @override
  Future<void> addSubtitle(
    ExternalSubtitle subtitle, {
    bool select = false,
  }) async {
    calls.add('addsub:${subtitle.title}${select ? ':select' : ''}');
    addedSubtitles.add(subtitle);
    final t = MediaTrack(
      id: 'ext-${subtitle.title}',
      title: subtitle.title,
      language: subtitle.language,
      external: true,
      forced: subtitle.forced,
      hearingImpaired: subtitle.hearingImpaired,
      isDefault: subtitle.isDefault,
    );
    emit(
      (s) => s.copyWith(
        subtitleTracks: [...s.subtitleTracks, t],
        subtitleTrack: select ? t : null,
      ),
    );
  }

  @override
  Future<void> setAudioProcessing(AudioProcessing processing) async {
    // Not in [calls]: the screen sets it on start, like the rendering.
    audioProcessing = processing;
  }

  @override
  Future<void> setAudioDelay(Duration delay) async {
    calls.add('audiodelay:${delay.inMilliseconds}');
    emit((s) => s.copyWith(audioDelay: delay));
  }

  @override
  Future<void> setSubtitleDelay(Duration delay) async {
    calls.add('subdelay:${delay.inMilliseconds}');
    emit((s) => s.copyWith(subtitleDelay: delay));
  }

  @override
  Widget buildVideo(BuildContext context, {BoxFit fit = BoxFit.contain}) =>
      const ColoredBox(color: Color(0xFF000000), child: SizedBox.expand());

  @override
  Future<void> dispose() async {
    calls.add('dispose');
    disposed = true;
  }
}
