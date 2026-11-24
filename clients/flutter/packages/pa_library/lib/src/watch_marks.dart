// SPDX-License-Identifier: MPL-2.0

// "Mark unwatched" (QA-015). A position report never clears "watched"
// (the core keeps it monotonic, LIB-011); only this explicit call does:
// the watched mark and the position go back to zero under every key of the
// item, and paired devices get the change through sync.
//
// Bulk marks (UI-049): an episode with the ones before it, a season or a
// whole show marked watched or not in one transaction of the core, and
// «Отменить» putting back the marks and positions the episodes had.

import 'package:pa_core/pa_core.dart' show CoreWatchMark;

import 'library.dart';
import 'rust_library.dart';

/// The watch state an item had before a bulk mark: what «Отменить» puts
/// back ([LibraryWatchMarks.restoreWatchMarks]).
class WatchMarkState {
  const WatchMarkState({
    required this.itemId,
    required this.positionMs,
    required this.durationMs,
    required this.watched,
  });

  final String itemId;
  final int positionMs;

  /// 0 if unknown.
  final int durationMs;
  final bool watched;

  @override
  bool operator ==(Object other) =>
      other is WatchMarkState &&
      other.itemId == itemId &&
      other.positionMs == positionMs &&
      other.durationMs == durationMs &&
      other.watched == watched;

  @override
  int get hashCode => Object.hash(itemId, positionMs, durationMs, watched);

  @override
  String toString() =>
      'WatchMarkState($itemId, $positionMs/$durationMs, watched: $watched)';
}

/// A library that can clear the watched mark of an item and mark many
/// items at once.
abstract interface class LibraryWatchMarks {
  /// Clears "watched" and the position of a movie or an episode (a show:
  /// call it for each episode). False when there was nothing to clear.
  Future<bool> markUnwatched(String itemId);

  /// Marks movies and episodes [watched] or not, all at once (UI-049).
  /// Items already in that state (and shows) are skipped. Returns the
  /// previous state of each item that changed, for [restoreWatchMarks].
  Future<List<WatchMarkState>> setWatchedMany(
    List<String> itemIds, {
    required bool watched,
  });

  /// «Отменить» of [setWatchedMany]: puts back the watched marks and
  /// positions of [states].
  Future<void> restoreWatchMarks(List<WatchMarkState> states);
}

/// The watch marks of [library]: its own, the core's for [RustLibrary],
/// else null (the menu entries are then not shown).
LibraryWatchMarks? watchMarksOf(Library library) => switch (library) {
  final LibraryWatchMarks m => m,
  final RustLibrary r => _RustWatchMarks(r),
  _ => null,
};

class _RustWatchMarks implements LibraryWatchMarks {
  const _RustWatchMarks(this._library);

  final RustLibrary _library;

  static int _now() => DateTime.now().millisecondsSinceEpoch;

  @override
  Future<bool> markUnwatched(String itemId) async {
    final cleared = await _library.core.markUnwatched(
      itemId: itemId,
      updatedAtMs: _now(),
    );
    if (cleared) _library.notifyExternalChange();
    return cleared;
  }

  @override
  Future<List<WatchMarkState>> setWatchedMany(
    List<String> itemIds, {
    required bool watched,
  }) async {
    if (itemIds.isEmpty) return const [];
    final before = await _library.core.setWatchedMany(
      itemIds: itemIds,
      watched: watched,
      updatedAtMs: _now(),
    );
    if (before.isNotEmpty) _library.notifyExternalChange();
    return [
      for (final m in before)
        WatchMarkState(
          itemId: m.itemId,
          positionMs: m.positionMs,
          durationMs: m.durationMs,
          watched: m.watched,
        ),
    ];
  }

  @override
  Future<void> restoreWatchMarks(List<WatchMarkState> states) async {
    if (states.isEmpty) return;
    await _library.core.restoreWatchMarks(
      marks: [
        for (final s in states)
          CoreWatchMark(
            itemId: s.itemId,
            positionMs: s.positionMs,
            durationMs: s.durationMs,
            watched: s.watched,
          ),
      ],
      updatedAtMs: _now(),
    );
    _library.notifyExternalChange();
  }
}
