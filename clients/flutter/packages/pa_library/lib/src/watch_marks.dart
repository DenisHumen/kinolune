// SPDX-License-Identifier: MPL-2.0

// "Mark unwatched" (QA-015). A position report never clears "watched"
// (the core keeps it monotonic, LIB-011); only this explicit call does:
// the watched mark and the position go back to zero under every key of the
// item, and paired devices get the change through sync.

import 'library.dart';
import 'rust_library.dart';

/// A library that can clear the watched mark of an item.
abstract interface class LibraryWatchMarks {
  /// Clears "watched" and the position of a movie or an episode (a show:
  /// call it for each episode). False when there was nothing to clear.
  Future<bool> markUnwatched(String itemId);
}

/// The watch marks of [library]: its own, the core's for [RustLibrary],
/// else null (the menu entry is then not shown).
LibraryWatchMarks? watchMarksOf(Library library) => switch (library) {
  final LibraryWatchMarks m => m,
  final RustLibrary r => _RustWatchMarks(r),
  _ => null,
};

class _RustWatchMarks implements LibraryWatchMarks {
  const _RustWatchMarks(this._library);

  final RustLibrary _library;

  @override
  Future<bool> markUnwatched(String itemId) async {
    final cleared = await _library.core.markUnwatched(
      itemId: itemId,
      updatedAtMs: DateTime.now().millisecondsSinceEpoch,
    );
    if (cleared) _library.notifyExternalChange();
    return cleared;
  }
}
