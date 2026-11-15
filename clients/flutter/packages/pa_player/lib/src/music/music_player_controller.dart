// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/foundation.dart';

import '../engine/audio_profile.dart';
import '../engine/media_kit_engine.dart';
import '../engine/player_engine.dart';
import '../play_request.dart';
import '../system/media_session.dart';
import '../system/music_session.dart';
import 'music_queue.dart';

/// App-wide music player. The whole queue is handed to libmpv as a playlist
/// so track changes are gapless (`gapless-audio=yes`, `prefetch-playlist`);
/// loudness follows [audioProfile] (ReplayGain, MUS-002).
///
/// MUS-003: Play Next / Add to Queue, reordering and removing edit the
/// engine's playlist in place ([PlaylistEditingEngine]) so the playing
/// track goes on; [snapshot] / [restore] keep the queue across restarts;
/// [pauseForVideo] makes music global state that a video interrupts.
///
/// PLAY-AND-008: once something plays, the player is in the system media
/// session ([mediaSession], default [SystemMediaSession.instance]): the
/// notification, the lock screen and headset buttons control it, and it
/// keeps playing in the background.
class MusicPlayerController extends ChangeNotifier {
  MusicPlayerController({
    PlayerEngineFactory? engineFactory,
    MusicQueue? queue,
    AudioProfile audioProfile = const AudioProfile(),
    SystemMediaSession? mediaSession,
  }) : _factory = engineFactory ?? MediaKitEngine.create,
       queue = queue ?? MusicQueue(),
       _profile = audioProfile {
    _session = MusicSessionBinding(
      this,
      mediaSession ?? SystemMediaSession.instance,
    );
  }

  late final MusicSessionBinding _session;

  final PlayerEngineFactory _factory;
  final MusicQueue queue;
  PlayerEngine? _engine;
  bool _disposed = false;
  AudioProfile _profile;
  AudioProfile? _applied;

  /// The engine has the current queue: false after [restore] and [stop]
  /// until something plays.
  bool _loaded = false;

  /// Where a restored (not yet loaded) track resumes.
  Duration _pendingStart = Duration.zero;

  /// Engine edits in flight: its playlist index lags behind the queue.
  int _editing = 0;
  PlayRequest? _lastRecorded;
  bool _interrupted = false;

  /// The user's audio settings ([ReplayGainMode.auto] unresolved).
  AudioProfile get audioProfile => _profile;

  /// What the engine gets: [audioProfile] with `auto` resolved against the
  /// current play order (album in order → album gain, else track gain).
  AudioProfile get effectiveAudioProfile =>
      _profile.resolveFor(queue.ordered, shuffled: queue.shuffle);

  /// Changes the audio settings; applies to the playing track.
  Future<void> setAudioProfile(AudioProfile profile) async {
    _profile = profile;
    await _applyProfile();
    notifyListeners();
  }

  Future<void> _applyProfile() async {
    final e = _engine;
    if (e == null) return;
    final p = effectiveAudioProfile;
    if (p == _applied) return;
    _applied = p;
    await e.setAudioProfile(p);
  }

  /// Restart the track instead of going back when past this point.
  static const restartThreshold = Duration(seconds: 3);

  PlayerEngine get _e {
    final e = _engine;
    if (e != null) return e;
    final created = _factory(video: false);
    created.state.addListener(_onEngine);
    return _engine = created;
  }

  EngineState get _s => _engine?.state.value ?? const EngineState();

  PlayRequest? get current => queue.current;
  bool get playing => _loaded && _s.playing;
  bool get buffering => _loaded && _s.buffering;
  Duration get position => _loaded ? _s.position : _pendingStart;
  Duration get duration {
    final d = _loaded ? _s.duration : Duration.zero;
    if (d > Duration.zero) return d;
    final known = current?.durationMs;
    return known == null ? Duration.zero : Duration(milliseconds: known);
  }

  bool get shuffle => queue.shuffle;
  QueueRepeat get repeat => queue.repeat;
  bool get hasQueue => !queue.isEmpty;

  /// What the queue was started from ("Up next from: …").
  QueueSource? get source => queue.source;

  /// Recently played tracks, most recent first.
  List<PlayRequest> get recent => queue.recent;

  /// Music was playing when a video started ([pauseForVideo]): the app
  /// offers to resume it when the video closes.
  bool get interruptedByVideo => _interrupted;

  void _onEngine() {
    final s = _s;
    if (_loaded &&
        _editing == 0 &&
        s.playlistIndex != queue.position &&
        s.playlistIndex >= 0 &&
        s.playlistIndex < queue.length) {
      queue.moveTo(s.playlistIndex);
    }
    _noteCurrent();
    if (!_disposed) notifyListeners();
  }

  /// Adds the current track to [recent] once it plays.
  void _noteCurrent() {
    final t = queue.current;
    if (t == null || !_loaded || identical(t, _lastRecorded)) return;
    _lastRecorded = t;
    queue.recordPlayed(t);
  }

  Future<void> _load({Duration start = Duration.zero, bool play = true}) async {
    _loaded = true;
    _pendingStart = Duration.zero;
    await _e.open(
      [for (final t in queue.ordered) t.uri],
      index: queue.position,
      start: start,
      play: play,
    );
    _noteCurrent();
  }

  /// Opens the restored queue in the engine (first play after [restore]).
  Future<void> _ensureLoaded({bool play = true}) async {
    if (_loaded || queue.isEmpty) return;
    await _e.setRepeat(queue.repeat);
    await _applyProfile();
    await _load(start: _pendingStart, play: play);
  }

  /// Replaces the queue with [queue] and starts playing at [index].
  /// [source] names where the tracks come from ("Up next from: …").
  Future<void> play(
    List<PlayRequest> queue, {
    int index = 0,
    QueueSource? source,
  }) async {
    _interrupted = false;
    this.queue.replace(queue, index: index, source: source);
    if (this.queue.isEmpty) return;
    final first = this.queue.current!;
    await _e.setRepeat(this.queue.repeat);
    await _applyProfile();
    await _load(start: Duration(milliseconds: first.startPositionMs));
    notifyListeners();
  }

  void pause() => _engine?.pause();

  /// Volume while another app's short sound plays over the music (audio
  /// focus "may duck", PLAY-AND-008), 0..100.
  static const duckedVolume = 20.0;
  bool _ducked = false;

  /// Plays quieter while [ducked] (audio focus).
  bool get ducked => _ducked;

  Future<void> setDucked(bool ducked) async {
    if (ducked == _ducked) return;
    _ducked = ducked;
    await _engine?.setVolume(ducked ? duckedVolume : 100);
  }

  void resume() {
    _interrupted = false;
    if (!_loaded) {
      _ensureLoaded().then((_) => notifyListeners());
      return;
    }
    _engine?.play();
  }

  void togglePlay() => playing ? pause() : resume();

  /// A video starts: pauses the music and remembers whether it was
  /// playing ([interruptedByVideo]). Returns that.
  bool pauseForVideo() {
    _interrupted = playing;
    if (_interrupted) pause();
    notifyListeners();
    return _interrupted;
  }

  /// The video closed and the user chose to go on with the music.
  void resumeAfterVideo() {
    if (!_interrupted) return;
    resume();
    notifyListeners();
  }

  /// The user did not take the offer to resume.
  void dismissInterruption() {
    if (!_interrupted) return;
    _interrupted = false;
    notifyListeners();
  }

  Future<void> _jumpTo(int p) async {
    queue.moveTo(p);
    if (_loaded) {
      await _e.jump(p);
    } else {
      _pendingStart = Duration.zero;
      await _ensureLoaded();
    }
    notifyListeners();
  }

  void next() {
    final p = queue.nextPosition();
    if (p == null) return;
    _jumpTo(p);
  }

  void previous() {
    if (position > restartThreshold) {
      seek(Duration.zero);
      return;
    }
    final p = queue.previousPosition();
    if (p == null) {
      seek(Duration.zero);
      return;
    }
    _jumpTo(p);
  }

  void seek(Duration d) {
    if (!_loaded) {
      _pendingStart = d;
      notifyListeners();
      return;
    }
    _engine?.seek(d);
  }

  /// Plays the track at [position] of the play order.
  void jumpTo(int position) {
    if (position < 0 || position >= queue.length) return;
    _jumpTo(position);
  }

  // -- queue editing (MUS-003) ------------------------------------------------

  /// Runs an edit of the engine playlist: in place when the engine can,
  /// else by re-sending the playlist and continuing the current track.
  Future<void> _edit(
    Future<void> Function(PlaylistEditingEngine e) inPlace,
  ) async {
    if (!_loaded) return;
    final e = _e;
    if (e case final PlaylistEditingEngine p) {
      _editing++;
      try {
        await inPlace(p);
      } finally {
        _editing--;
      }
      return;
    }
    await _load(start: position, play: playing);
  }

  /// Play Next: [tracks] play right after the current one. With nothing
  /// queued they start playing at once.
  Future<void> playNext(List<PlayRequest> tracks) async {
    if (tracks.isEmpty) return;
    if (queue.isEmpty) return play(tracks);
    final at = queue.playNext(tracks);
    await _edit((e) async {
      for (var i = 0; i < tracks.length; i++) {
        await e.insertAt(at + i, tracks[i].uri);
      }
    });
    await _applyProfile();
    notifyListeners();
  }

  /// Add to Queue: [tracks] play after what the user already queued.
  Future<void> addToQueue(List<PlayRequest> tracks) async {
    if (tracks.isEmpty) return;
    if (queue.isEmpty) return play(tracks);
    final at = queue.addToQueue(tracks);
    await _edit((e) async {
      for (var i = 0; i < tracks.length; i++) {
        await e.insertAt(at + i, tracks[i].uri);
      }
    });
    await _applyProfile();
    notifyListeners();
  }

  /// Plays [track] now, keeping the rest of the queue after it.
  Future<void> playNow(PlayRequest track) async {
    if (queue.isEmpty) return play([track]);
    final at = queue.playNext([track]);
    await _edit((e) => e.insertAt(at, track.uri));
    await _jumpTo(at);
  }

  /// Drag to reorder: moves up-next track [from] to [to] (indices into
  /// [MusicQueue.upNextEntries]).
  Future<void> moveUpNext(int from, int to) async {
    final n = queue.length - queue.position - 1;
    if (from < 0 || from >= n || to < 0 || to >= n || from == to) return;
    final base = queue.position + 1;
    queue.moveUpNext(from, to);
    await _edit((e) => e.move(base + from, base + to));
    notifyListeners();
  }

  /// Removes up-next track [index] (index into
  /// [MusicQueue.upNextEntries]).
  Future<void> removeUpNext(int index) async {
    final at = queue.position + 1 + index;
    if (index < 0 || at >= queue.length) return;
    queue.removeUpNext(index);
    await _edit((e) => e.removeAt(at));
    await _applyProfile();
    notifyListeners();
  }

  /// "Clear queue": removes every track after the current one.
  Future<void> clearUpNext() async {
    final from = queue.position + 1;
    final n = queue.length - from;
    if (n <= 0) return;
    queue.clearUpNext();
    await _edit((e) async {
      for (var i = n - 1; i >= 0; i--) {
        await e.removeAt(from + i);
      }
    });
    await _applyProfile();
    notifyListeners();
  }

  /// Stops and empties the queue (e.g. another profile became active).
  Future<void> stop() async {
    _interrupted = false;
    if (_loaded) await _engine?.pause();
    _loaded = false;
    _pendingStart = Duration.zero;
    _lastRecorded = null;
    queue.clear();
    queue.clearRecent();
    notifyListeners();
  }

  Future<void> setShuffle(bool on) async {
    if (on == queue.shuffle) return;
    queue.setShuffle(on);
    await _applyProfile();
    if (_loaded && _engine != null && !queue.isEmpty) {
      // Re-send the playlist in the new order, continuing the current track.
      await _load(start: position, play: playing);
    }
    notifyListeners();
  }

  Future<void> toggleShuffle() => setShuffle(!queue.shuffle);

  Future<void> cycleRepeat() async {
    final m = queue.cycleRepeat();
    await _engine?.setRepeat(m);
    notifyListeners();
  }

  Future<void> setRepeat(QueueRepeat m) async {
    queue.setRepeat(m);
    await _engine?.setRepeat(m);
    notifyListeners();
  }

  // -- persistence (MUS-003) --------------------------------------------------

  /// The queue and the playhead as JSON, for [restore] after a restart.
  Map<String, Object?> snapshot() => {
    ...queue.toJson(),
    'positionMs': position.inMilliseconds,
  };

  /// Puts back a [snapshot] without playing: the current track waits,
  /// paused at its saved position, until the user presses play. [resolve]
  /// may update or drop tracks (see [MusicQueue.restore]). Returns
  /// `false` for unreadable data.
  Future<bool> restore(
    Map<String, Object?> snapshot, {
    PlayRequest? Function(PlayRequest)? resolve,
  }) async {
    if (!queue.restore(snapshot, resolve: resolve)) return false;
    if (_loaded) await _engine?.pause();
    _loaded = false;
    _interrupted = false;
    final ms = snapshot['positionMs'];
    // The saved playhead belongs to the saved current track, which
    // [resolve] may have dropped.
    final tracks = snapshot['tracks'] as List;
    final at = snapshot['position'];
    final saved = at is int && at >= 0 && at < tracks.length
        ? musicRequestFromJson(tracks[at])
        : null;
    final cur = queue.current;
    final same =
        saved != null &&
        cur != null &&
        (saved.itemId ?? saved.uri) == (cur.itemId ?? cur.uri);
    _pendingStart = ms is int && ms > 0 && same
        ? Duration(milliseconds: ms)
        : Duration.zero;
    // The restored current track is already the top of [recent].
    _lastRecorded = queue.current;
    await _engine?.setRepeat(queue.repeat);
    notifyListeners();
    return true;
  }

  @override
  void dispose() {
    _disposed = true;
    _session.dispose();
    _engine?.state.removeListener(_onEngine);
    _engine?.dispose();
    super.dispose();
  }
}
