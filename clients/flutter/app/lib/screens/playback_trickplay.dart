// SPDX-License-Identifier: MPL-2.0

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../integration/player.dart';

/// Seek previews of [item] for the player (PLAY-009), or null if the
/// library offers none. With [eager] the previews are prepared at once
/// (the file about to play); otherwise on the first scrub (an Up Next item
/// that may never play).
TrickplaySource? trickplayOf(
  Library library,
  LibraryItem item, {
  bool eager = true,
}) {
  if (library is! LibraryTrickplay) return null;
  final source = LibraryTrickplaySource(
    () => (library as LibraryTrickplay).trickplay(item),
  );
  if (eager) source.start();
  return source;
}

/// [TrickplaySource] over the library's [TrickplayPreviews], opened on
/// [start] or the first [frameAt].
class LibraryTrickplaySource implements TrickplaySource {
  LibraryTrickplaySource(this._open);

  final Future<TrickplayPreviews?> Function() _open;
  Future<TrickplayPreviews?>? _previews;
  bool _disposed = false;

  /// Prepares the previews (finds or starts generating them).
  void start() {
    _previews ??= _open().then<TrickplayPreviews?>(
      (previews) {
        if (_disposed) {
          previews?.dispose();
          return null;
        }
        return previews;
      },
      onError: (Object e) {
        debugPrint('Seek previews unavailable: $e');
        return null;
      },
    );
  }

  @override
  Future<TrickplayFrame?> frameAt(Duration position) async {
    if (_disposed) return null;
    start();
    final previews = await _previews;
    if (previews == null || _disposed) return null;
    final f = await previews.frameAt(position.inMilliseconds);
    if (f == null) return null;
    return TrickplayFrame(
      image: f.jpeg,
      imageId: f.imageId,
      position: Duration(milliseconds: f.positionMs),
      crop: f.isTile
          ? Rect.fromLTWH(
              f.cropX.toDouble(),
              f.cropY.toDouble(),
              f.cropWidth.toDouble(),
              f.cropHeight.toDouble(),
            )
          : null,
    );
  }

  @override
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    unawaited(_previews?.then((p) => p?.dispose()));
  }
}
