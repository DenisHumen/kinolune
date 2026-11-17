// SPDX-License-Identifier: MPL-2.0
//
// MUS-018: the audiobooks of the library and their player, app-wide.
// * [shown] says whether the «Аудиокниги» section is in the navigation:
//   only while the library has books (the user never says what is a book).
// * [play] opens a book where the listener stopped — on this device or
//   another (the place is read fresh from the library, SYNC-001).
// * The book and the music never play at once: starting one pauses the
//   other.
// * The speed is the listener's, kept across books and restarts.

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../integration/library.dart';
import '../../integration/player.dart';
import '../../services/preferences.dart';

/// Key of the book speed in [Preferences].
const bookSpeedPreference = 'audiobook_speed';

class AudiobookService extends ChangeNotifier {
  AudiobookService({
    required Object library,
    required this.music,
    required this.preferences,
    BookPlayerController? player,
    PlayerEngineFactory? engineFactory,
  }) : _books = libraryAudiobooks(library),
       player = player ?? BookPlayerController(engineFactory: engineFactory) {
    this.player.onSave = _save;
    this.player.addListener(_onBook);
    music.addListener(_onMusic);
    _sub = _books?.audiobooks().listen(
      _onBooks,
      onError: (Object e) {
        debugPrint('audiobooks: $e');
      },
    );
    _loadSpeed();
  }

  final LibraryAudiobooks? _books;
  final MusicPlayerController music;
  final Preferences preferences;
  final BookPlayerController player;
  StreamSubscription<List<Audiobook>>? _sub;

  /// The library has books: the section is in the navigation.
  final ValueNotifier<bool> shown = ValueNotifier(false);

  List<Audiobook> _list = const [];

  /// The books, by author, series and title.
  List<Audiobook> get books => _list;

  /// The book that is open in the player, as the library lists it.
  Audiobook? get current {
    final key = player.book?.key;
    if (player.book == null) return null;
    return _list.where((b) => b.key == key).firstOrNull ?? _opened;
  }

  Audiobook? _opened;
  bool _bookWasPlaying = false;
  bool _musicWasPlaying = false;

  /// Which of the two players was started last: the mini player shows it.
  bool get bookIsLatest => _bookLatest;
  bool _bookLatest = false;

  void _onBooks(List<Audiobook> books) {
    _list = books;
    shown.value = books.isNotEmpty;
    notifyListeners();
  }

  Future<void> _loadSpeed() async {
    try {
      final v = double.tryParse(
        await preferences.read(bookSpeedPreference) ?? '',
      );
      if (v != null) await player.setSpeed(v);
    } catch (e) {
      debugPrint('audiobooks: speed not read: $e');
    }
  }

  /// Plays at [speed] and remembers it.
  Future<void> setSpeed(double speed) async {
    await player.setSpeed(speed);
    try {
      await preferences.write(bookSpeedPreference, '${player.speed}');
    } catch (e) {
      debugPrint('audiobooks: speed not saved: $e');
    }
  }

  Future<void> _save(
    String key,
    int positionMs,
    int durationMs, {
    required bool finished,
  }) async {
    await _books?.saveBookProgress(
      key,
      positionMs: positionMs,
      durationMs: durationMs,
      finished: finished,
    );
  }

  /// What the player gets for [book].
  static AudiobookRequest requestOf(Audiobook book) => AudiobookRequest(
    key: book.key,
    title: book.title,
    author: book.author,
    artworkUri: book.coverUri,
    parts: [
      for (final p in book.parts)
        BookPart(uri: p.playbackUri, durationMs: p.durationMs),
    ],
    chapters: [
      for (final c in book.chapters)
        BookPlayerChapter(startMs: c.bookStartMs, title: c.title),
    ],
  );

  /// Opens [book] where the listener stopped (or at [startMs]) and plays.
  Future<void> play(Audiobook book, {int? startMs}) async {
    var start = startMs;
    if (start == null) {
      var progress = book.progress;
      final key = book.key;
      if (key != null && _books != null) {
        try {
          // Another device may have gone further since the list loaded.
          final fresh = await _books.bookProgress(key);
          if (fresh != null &&
              (progress == null ||
                  !fresh.updatedAt.isBefore(progress.updatedAt))) {
            progress = fresh;
          }
        } catch (e) {
          debugPrint('audiobooks: place not read: $e');
        }
      }
      start = book.withProgress(progress).resumeMs;
    }
    _opened = book;
    if (player.book?.key == book.key &&
        player.book != null &&
        startMs == null) {
      // Already open: go on.
      player.play();
      return;
    }
    await player.open(requestOf(book), start: Duration(milliseconds: start));
  }

  /// Plays chapter [index] of [book] from its start.
  Future<void> playChapter(Audiobook book, int index) async {
    if (index < 0 || index >= book.chapters.length) return;
    final ms = book.chapters[index].bookStartMs;
    if (player.book != null && player.book!.key == book.key) {
      await player.seek(Duration(milliseconds: ms));
      player.play();
      return;
    }
    await play(book, startMs: ms);
  }

  AudiobookRequest? _lastBook;

  /// Engine ticks are the player's to report; this notifies only when the
  /// open book or the latest player changes.
  void _onBook() {
    final playing = player.playing;
    var changed = !identical(player.book, _lastBook);
    _lastBook = player.book;
    if (playing && !_bookWasPlaying) {
      changed |= !_bookLatest;
      _bookLatest = true;
      if (music.playing) music.pause();
    }
    _bookWasPlaying = playing;
    if (changed) notifyListeners();
  }

  void _onMusic() {
    final playing = music.playing;
    if (playing && !_musicWasPlaying) {
      _bookLatest = false;
      if (player.playing) player.pause();
      notifyListeners();
    }
    _musicWasPlaying = playing;
  }

  /// Saves the place and closes the book (another profile, sign out).
  Future<void> close() async {
    await player.close();
    _opened = null;
    notifyListeners();
  }

  @override
  void dispose() {
    _sub?.cancel();
    music.removeListener(_onMusic);
    player
      ..removeListener(_onBook)
      ..dispose();
    shown.dispose();
    super.dispose();
  }
}
