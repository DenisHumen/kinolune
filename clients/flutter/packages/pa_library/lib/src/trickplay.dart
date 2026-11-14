// SPDX-License-Identifier: MPL-2.0

import 'dart:typed_data';

import 'models.dart';

/// One seek preview (PLAY-009): a JPEG, or a sheet of tiles and the tile.
class TrickplayPreview {
  const TrickplayPreview({
    required this.jpeg,
    required this.imageId,
    required this.positionMs,
    this.cropX = 0,
    this.cropY = 0,
    this.cropWidth = 0,
    this.cropHeight = 0,
  });

  final Uint8List jpeg;

  /// Equal ids within one [TrickplayPreviews] mean equal [jpeg] bytes.
  final int imageId;

  /// Time the preview shows.
  final int positionMs;

  /// The tile inside [jpeg] in pixels; all zero for the whole image.
  final int cropX;
  final int cropY;
  final int cropWidth;
  final int cropHeight;

  bool get isTile => cropWidth > 0 && cropHeight > 0;
}

/// Seek previews of one file; [dispose] when it stops playing.
abstract interface class TrickplayPreviews {
  /// The preview for [positionMs], or null if there is none (yet).
  Future<TrickplayPreview?> frameAt(int positionMs);

  /// Stops an unfinished generation unless the file is open elsewhere.
  void dispose();
}

/// A library that offers seek previews of its videos (PLAY-009): a BIF or
/// Jellyfin trickplay folder next to the file, previews generated earlier,
/// or (if [trickplayGenerate]) generated now through FFmpeg.
abstract interface class LibraryTrickplay {
  /// Previews of the file [item] plays ([LibraryItem.playbackUri]), or
  /// null if this library cannot offer any for it.
  Future<TrickplayPreviews?> trickplay(LibraryItem item);

  /// Whether previews are generated for files that have none (a user
  /// option; on by default).
  bool get trickplayGenerate;
  set trickplayGenerate(bool value);
}
