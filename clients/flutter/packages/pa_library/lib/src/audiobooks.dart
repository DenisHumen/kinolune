// SPDX-License-Identifier: MPL-2.0

// MUS-018: the audiobook library. The core makes books of the tracks the
// content kind classifier marked (LIB-025): a folder of parts, or an m4b
// with chapters; author, narrator and series from the tags and the path.
// Where the listener stopped is one position per book, saved and synced
// between devices under the book key (SYNC-001).

import 'models.dart';

/// A chapter of a book.
class BookChapter {
  const BookChapter({
    required this.part,
    required this.startMs,
    required this.bookStartMs,
    this.durationMs = 0,
    this.title,
  });

  /// Null: the file has no title for it (the app numbers it).
  final String? title;

  /// Index into [Audiobook.parts].
  final int part;

  /// Start within the part.
  final int startMs;

  /// Start within the whole book.
  final int bookStartMs;

  /// 0 if unknown.
  final int durationMs;

  @override
  String toString() => 'BookChapter(${title ?? '#'} @$bookStartMs)';
}

/// Where the listener stopped in a book.
class BookProgress {
  const BookProgress({
    required this.positionMs,
    required this.updatedAt,
    this.durationMs = 0,
    this.finished = false,
  });

  /// From the start of the book.
  final int positionMs;

  /// 0 if unknown.
  final int durationMs;
  final DateTime updatedAt;

  /// Listened to the end.
  final bool finished;

  /// 0..1, null when the length is unknown.
  double? get fraction => durationMs <= 0
      ? null
      : (positionMs / durationMs).clamp(0.0, 1.0).toDouble();
}

/// An audiobook.
class Audiobook {
  const Audiobook({
    required this.title,
    required this.sourceId,
    required this.relPath,
    required this.parts,
    this.key,
    this.author,
    this.narrator,
    this.series,
    this.seriesNumber,
    this.year,
    this.coverUri,
    this.durationMs = 0,
    this.addedAt,
    this.chapters = const [],
    this.progress,
  });

  /// The book key the place is saved and synced under; null until the
  /// first part is fingerprinted (the place is then not saved).
  final String? key;
  final String title;
  final String? author;
  final String? narrator;
  final String? series;

  /// Number in the series (`2`, `2.5`).
  final double? seriesNumber;
  final int? year;
  final int sourceId;

  /// The book's folder (or file), relative to the source root.
  final String relPath;

  /// Cover: a file path or an http(s) URL.
  final String? coverUri;

  /// 0 if unknown.
  final int durationMs;
  final DateTime? addedAt;

  /// The parts (tracks) in play order.
  final List<LibraryItem> parts;
  final List<BookChapter> chapters;
  final BookProgress? progress;

  /// A stable id for lists: the key, else the folder.
  String get id => key ?? '$sourceId:$relPath';

  /// Started and not finished.
  bool get inProgress {
    final p = progress;
    return p != null && !p.finished && p.positionMs > 0;
  }

  /// Where playing starts: the saved place, or the start once finished.
  int get resumeMs {
    final p = progress;
    if (p == null || p.finished) return 0;
    return p.positionMs;
  }

  /// `2`, `2.5` for a number in the series.
  String? get seriesNumberLabel {
    final n = seriesNumber;
    if (n == null) return null;
    return n == n.roundToDouble() ? '${n.round()}' : '$n';
  }

  Audiobook withProgress(BookProgress? progress) => Audiobook(
    key: key,
    title: title,
    author: author,
    narrator: narrator,
    series: series,
    seriesNumber: seriesNumber,
    year: year,
    sourceId: sourceId,
    relPath: relPath,
    coverUri: coverUri,
    durationMs: durationMs,
    addedAt: addedAt,
    parts: parts,
    chapters: chapters,
    progress: progress,
  );

  @override
  String toString() => 'Audiobook($title, ${parts.length} parts)';
}

/// A library that keeps audiobooks (MUS-018: the Rust core).
abstract interface class LibraryAudiobooks {
  /// The books the active profile sees, now and after every change, by
  /// author, series and title.
  Stream<List<Audiobook>> audiobooks();

  /// Where the listener stopped in the book with [key] (the newest place
  /// of any device).
  Future<BookProgress?> bookProgress(String key);

  /// Saves the place in a book and sends it to paired devices.
  Future<void> saveBookProgress(
    String key, {
    required int positionMs,
    required int durationMs,
    bool finished = false,
  });
}

/// The audiobooks of [library], if it keeps them.
LibraryAudiobooks? libraryAudiobooks(Object library) =>
    library is LibraryAudiobooks ? library : null;
