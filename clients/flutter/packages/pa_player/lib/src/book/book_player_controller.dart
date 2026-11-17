// SPDX-License-Identifier: MPL-2.0
import 'dart:async';

import 'package:flutter/foundation.dart';

import '../engine/media_kit_engine.dart';
import '../engine/player_engine.dart';
import '../system/media_session.dart';

/// One file of an audiobook.
@immutable
class BookPart {
  const BookPart({required this.uri, this.durationMs});

  /// File path, `file://` or http(s) URI.
  final String uri;

  /// Length when known (the library probed it); null or 0: unknown.
  final int? durationMs;
}

/// A chapter, on the book's own time line.
@immutable
class BookPlayerChapter {
  const BookPlayerChapter({required this.startMs, this.title});

  /// Milliseconds from the start of the book.
  final int startMs;

  /// Null: the app shows "Глава N".
  final String? title;
}

/// What the book player plays (MUS-018): parts in order, chapters and
/// the key the place is saved under.
@immutable
class AudiobookRequest {
  const AudiobookRequest({
    required this.key,
    required this.title,
    required this.parts,
    this.chapters = const [],
    this.author,
    this.artworkUri,
  });

  /// The book key (`file:<fingerprint>`); null: the place is not saved.
  final String? key;
  final String title;
  final String? author;

  /// Cover: a file path or an http(s) URL.
  final String? artworkUri;
  final List<BookPart> parts;

  /// By start; empty: one chapter per part.
  final List<BookPlayerChapter> chapters;
}

/// The sleep timer (MUS-018): after a time, or at the end of the chapter.
@immutable
class SleepTimer {
  const SleepTimer.after(Duration this.duration) : endOfChapter = false;
  const SleepTimer.endOfChapter() : duration = null, endOfChapter = true;

  final Duration? duration;
  final bool endOfChapter;

  @override
  bool operator ==(Object other) =>
      other is SleepTimer &&
      other.duration == duration &&
      other.endOfChapter == endOfChapter;

  @override
  int get hashCode => Object.hash(duration, endOfChapter);
}

/// Saves where the listener is in a book: [positionMs] from the start of
/// the book; [finished] at the end.
typedef BookPositionSaver =
    Future<void> Function(
      String key,
      int positionMs,
      int durationMs, {
      required bool finished,
    });

/// The audiobook player (MUS-018): one book at a time, parts as one engine
/// playlist, a position on the book's time line, chapters, speed 0.5–3×,
/// ±30 s jumps, a sleep timer (minutes or «до конца главы») and the place
/// saved per book through [onSave] (the library syncs it, SYNC-001).
///
/// Like the music player it holds the system media session while it has a
/// book (PLAY-AND-008): the notification, the lock screen and headset
/// buttons control it and it plays on in the background. Next and previous
/// there are the next and previous chapter.
class BookPlayerController extends ChangeNotifier {
  BookPlayerController({
    PlayerEngineFactory? engineFactory,
    SystemMediaSession? mediaSession,
    this.onSave,
    this.saveEvery = const Duration(seconds: 15),
    Timer Function(Duration, void Function())? timer,
  }) : _factory = engineFactory ?? MediaKitEngine.create,
       _session = mediaSession ?? SystemMediaSession.instance,
       _timer = timer ?? Timer.new;

  /// Slowest and fastest speed.
  static const minSpeed = 0.5;
  static const maxSpeed = 3.0;

  /// The speeds the speed menu offers.
  static const speeds = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];

  /// A jump back or forward.
  static const jump = Duration(seconds: 30);

  /// Previous chapter restarts the chapter when past this point.
  static const restartThreshold = Duration(seconds: 3);

  /// Past this share of the book it counts as listened to (as the
  /// library counts "watched").
  static const _finishedShare = 0.95;

  final PlayerEngineFactory _factory;
  final SystemMediaSession _session;
  final Timer Function(Duration, void Function()) _timer;
  BookPositionSaver? onSave;
  final Duration saveEvery;

  PlayerEngine? _engine;
  MediaSessionClaim? _claim;
  bool _disposed = false;

  AudiobookRequest? _book;
  List<int> _starts = const [];
  List<int?> _lengths = const [];
  double _speed = 1.0;
  int _lastSaved = -1;
  DateTime? _savedAt;
  bool _wasPlaying = false;

  /// While the engine moves to another part: the book position it goes to.
  int? _pendingMs;

  SleepTimer? _sleep;
  Timer? _sleepTimer;
  DateTime? _sleepAt;

  /// End of the chapter the «до конца главы» timer waits for.
  int? _sleepAtMs;

  PlayerEngine get _e {
    final e = _engine;
    if (e != null) return e;
    final created = _factory(video: false);
    created.state.addListener(_onEngine);
    return _engine = created;
  }

  EngineState get _s => _engine?.state.value ?? const EngineState();

  /// The book that is open; null before [open] and after [close].
  AudiobookRequest? get book => _book;
  bool get playing => _book != null && _s.playing;
  bool get buffering => _book != null && _s.buffering;
  double get speed => _speed;

  /// Where the listener is, from the start of the book.
  Duration get position {
    if (_book == null) return Duration.zero;
    final pending = _pendingMs;
    if (pending != null) return Duration(milliseconds: pending);
    final i = _s.playlistIndex.clamp(0, _starts.length - 1);
    return Duration(milliseconds: _starts[i]) + _s.position;
  }

  /// The whole book: the parts the library measured, the playing part as
  /// the engine reports it.
  Duration get duration {
    if (_book == null) return Duration.zero;
    var ms = 0;
    for (var i = 0; i < _lengths.length; i++) {
      ms += _lengthOf(i);
    }
    return Duration(milliseconds: ms);
  }

  int _lengthOf(int part) {
    final known = _lengths[part];
    if (known != null && known > 0) return known;
    if (part == _s.playlistIndex && _pendingMs == null) {
      return _s.duration.inMilliseconds;
    }
    return 0;
  }

  /// Chapters: the book's, else one per part.
  List<BookPlayerChapter> get chapters {
    final b = _book;
    if (b == null) return const [];
    if (b.chapters.isNotEmpty) return b.chapters;
    return [
      for (var i = 0; i < b.parts.length; i++)
        BookPlayerChapter(startMs: _starts[i]),
    ];
  }

  /// Index of the chapter [position] is in.
  int get chapterIndex => _chapterAt(position.inMilliseconds);

  int _chapterAt(int ms) {
    final list = chapters;
    var i = 0;
    while (i + 1 < list.length && list[i + 1].startMs <= ms) {
      i++;
    }
    return i;
  }

  /// Start of chapter [i] and its end (the next chapter, else the book).
  (int, int) chapterBounds(int i) {
    final list = chapters;
    final start = list[i].startMs;
    final end = i + 1 < list.length
        ? list[i + 1].startMs
        : duration.inMilliseconds;
    return (start, end < start ? start : end);
  }

  /// The sleep timer that is set, if any.
  SleepTimer? get sleepTimer => _sleep;

  /// Time left on a minutes timer (null for «до конца главы» and off).
  Duration? get sleepLeft {
    final at = _sleepAt;
    if (at == null) return null;
    final left = at.difference(DateTime.now());
    return left.isNegative ? Duration.zero : left;
  }

  // -- opening ----------------------------------------------------------------

  /// Opens [book] at [start] (from the start of the book) and plays it
  /// unless [play] is false. Another open book is saved first.
  Future<void> open(
    AudiobookRequest book, {
    Duration start = Duration.zero,
    bool play = true,
  }) async {
    if (book.parts.isEmpty) return;
    if (_book != null) await _save(force: true);
    _book = book;
    _lengths = [for (final p in book.parts) p.durationMs];
    _starts = [];
    var at = 0;
    for (final l in _lengths) {
      _starts.add(at);
      at += (l ?? 0) > 0 ? l! : 0;
    }
    _lastSaved = -1;
    _savedAt = null;
    final (part, offset) = _locate(start.inMilliseconds);
    _pendingMs = null;
    await _e.open(
      [for (final p in book.parts) p.uri],
      index: part,
      start: Duration(milliseconds: offset),
      play: play,
    );
    await _e.setRate(_speed);
    _syncSession();
    notifyListeners();
  }

  /// The part and the offset in it of [ms] from the start of the book.
  (int, int) _locate(int ms) {
    final n = _starts.length;
    for (var i = 0; i < n; i++) {
      final len = _lengths[i] ?? 0;
      // Past a part of known length: a later one.
      if (i < n - 1 && len > 0 && ms >= _starts[i] + len) continue;
      final offset = ms - _starts[i];
      return (i, offset < 0 ? 0 : offset);
    }
    return (0, 0);
  }

  // -- transport --------------------------------------------------------------

  void play() {
    if (_book == null) return;
    _engine?.play();
  }

  void pause() {
    if (_book == null) return;
    _engine?.pause();
  }

  void togglePlay() => playing ? pause() : play();

  /// Moves to [target] on the book's time line.
  Future<void> seek(Duration target) async {
    final b = _book;
    if (b == null) return;
    var ms = target.inMilliseconds;
    final total = duration.inMilliseconds;
    if (ms < 0) ms = 0;
    if (total > 0 && ms > total) ms = total;
    final (part, offset) = _locate(ms);
    if (part == _s.playlistIndex) {
      await _e.seek(Duration(milliseconds: offset));
    } else {
      // Another file: reopen the playlist there, keeping play / pause.
      _pendingMs = ms;
      notifyListeners();
      await _e.open(
        [for (final p in b.parts) p.uri],
        index: part,
        start: Duration(milliseconds: offset),
        play: _s.playing,
      );
      _pendingMs = null;
      await _e.setRate(_speed);
    }
    if (_sleep?.endOfChapter ?? false) {
      _sleepAtMs = chapterBounds(chapterIndex).$2;
    }
    await _save(force: true);
    notifyListeners();
  }

  /// 30 seconds back.
  Future<void> back() => seek(position - jump);

  /// 30 seconds forward.
  Future<void> forward() => seek(position + jump);

  /// Plays chapter [i] from its start.
  Future<void> jumpToChapter(int i) async {
    final list = chapters;
    if (i < 0 || i >= list.length) return;
    await seek(Duration(milliseconds: list[i].startMs));
  }

  Future<void> nextChapter() async {
    final i = chapterIndex;
    if (i + 1 < chapters.length) await jumpToChapter(i + 1);
  }

  /// The start of this chapter, or the previous one near its start.
  Future<void> previousChapter() async {
    final i = chapterIndex;
    final (start, _) = chapterBounds(i);
    if (position.inMilliseconds - start > restartThreshold.inMilliseconds ||
        i == 0) {
      await jumpToChapter(i);
    } else {
      await jumpToChapter(i - 1);
    }
  }

  /// Plays at [speed] (0.5–3×), the pitch kept.
  Future<void> setSpeed(double speed) async {
    final s = speed.clamp(minSpeed, maxSpeed).toDouble();
    if (s == _speed) return;
    _speed = s;
    if (_book != null) await _engine?.setRate(s);
    _syncSession();
    notifyListeners();
  }

  // -- sleep timer ------------------------------------------------------------

  /// Sets the sleep timer; null turns it off. A minutes timer counts real
  /// time, whatever the speed.
  void setSleepTimer(SleepTimer? timer) {
    _sleepTimer?.cancel();
    _sleepTimer = null;
    _sleepAt = null;
    _sleepAtMs = null;
    _sleep = timer;
    final d = timer?.duration;
    if (d != null) {
      _sleepAt = DateTime.now().add(d);
      _sleepTimer = _timer(d, _sleepNow);
    } else if (timer != null && timer.endOfChapter && _book != null) {
      _sleepAtMs = chapterBounds(chapterIndex).$2;
    }
    notifyListeners();
  }

  void _sleepNow() {
    final at = _sleepAtMs;
    _sleepTimer?.cancel();
    _sleepTimer = null;
    _sleep = null;
    _sleepAt = null;
    _sleepAtMs = null;
    if (_book == null) return;
    _engine?.pause();
    // Stop exactly at the end of the chapter, so the next listen starts
    // the next chapter.
    if (at != null && (position.inMilliseconds - at).abs() > 500) {
      seek(Duration(milliseconds: at));
    } else {
      _save(force: true);
    }
    notifyListeners();
  }

  // -- engine and saving ------------------------------------------------------

  void _onEngine() {
    if (_book == null) {
      if (!_disposed) notifyListeners();
      return;
    }
    final s = _s;
    final ms = position.inMilliseconds;
    final end = _sleepAtMs;
    if (end != null && _pendingMs == null && s.playing && ms >= end - 250) {
      _sleepNow();
    }
    if (_wasPlaying && !s.playing) {
      _save(force: true);
    } else if (s.playing) {
      _save();
    }
    if (s.completed) _save(force: true);
    _wasPlaying = s.playing;
    _syncSession();
    if (!_disposed) notifyListeners();
  }

  /// Saves the place every [saveEvery] while playing, and at once when
  /// [force]d (pause, seek, close, the end).
  Future<void> _save({bool force = false}) async {
    final b = _book;
    final key = b?.key;
    final save = onSave;
    if (b == null || key == null || save == null || _pendingMs != null) return;
    final now = DateTime.now();
    final last = _savedAt;
    if (!force && last != null && now.difference(last) < saveEvery) return;
    final ms = position.inMilliseconds;
    if (force && ms == _lastSaved) return;
    _lastSaved = ms;
    _savedAt = now;
    final total = duration.inMilliseconds;
    final finished =
        (_s.completed && _s.playlistIndex >= b.parts.length - 1) ||
        (total > 0 && ms >= total * _finishedShare);
    try {
      await save(key, ms, total, finished: finished);
    } catch (e) {
      debugPrint('book: saving the place failed: $e');
    }
  }

  void _syncSession() {
    final b = _book;
    if (b == null) {
      _claim?.release();
      _claim = null;
      return;
    }
    if (_claim == null) {
      if (!_s.playing) return;
      _claim = _session.claim(_onCommand, debugLabel: 'book');
    }
    final i = chapterIndex;
    final title = chapters.isEmpty ? null : chapters[i].title;
    _claim!.update(
      MediaSessionInfo(
        kind: MediaSessionKind.music,
        id: b.key ?? b.title,
        title: title ?? b.title,
        subtitle: b.author,
        album: b.title,
        artworkUri: b.artworkUri,
        duration: duration,
        position: position,
        playing: playing,
        buffering: buffering,
        rate: _speed,
        hasNext: i + 1 < chapters.length,
        hasPrevious: true,
      ),
    );
  }

  void _onCommand(MediaSessionCommand command, Duration? at) {
    switch (command) {
      case MediaSessionCommand.play:
        play();
      case MediaSessionCommand.pause:
        pause();
      case MediaSessionCommand.stop:
        pause();
        _claim?.release();
        _claim = null;
      case MediaSessionCommand.next:
        nextChapter();
      case MediaSessionCommand.previous:
        previousChapter();
      case MediaSessionCommand.seek:
        if (at != null) seek(at);
      case MediaSessionCommand.duck:
        _engine?.setVolume(20);
      case MediaSessionCommand.unduck:
        _engine?.setVolume(100);
    }
  }

  /// Saves the place, stops and forgets the book.
  Future<void> close() async {
    if (_book == null) return;
    await _engine?.pause();
    await _save(force: true);
    setSleepTimer(null);
    _book = null;
    _claim?.release();
    _claim = null;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _sleepTimer?.cancel();
    _claim?.release();
    _engine?.state.removeListener(_onEngine);
    _engine?.dispose();
    super.dispose();
  }
}
