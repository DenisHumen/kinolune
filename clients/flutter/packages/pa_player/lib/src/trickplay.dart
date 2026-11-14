// SPDX-License-Identifier: MPL-2.0

import 'dart:async';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/widgets.dart';

/// One seek preview (PLAY-009): a JPEG, or a sheet of tiles and the tile to
/// show.
@immutable
class TrickplayFrame {
  const TrickplayFrame({
    required this.image,
    required this.imageId,
    required this.position,
    this.crop,
  });

  /// Encoded image bytes (JPEG).
  final Uint8List image;

  /// Equal ids within one [TrickplaySource] mean equal [image] bytes, so a
  /// decoded image can be reused (a sheet holds many tiles).
  final int imageId;

  /// The tile inside [image] in pixels; null for the whole image.
  final Rect? crop;

  /// Time the preview shows (may differ from the asked one: the nearest
  /// keyframe or the nearest thumbnail generated so far).
  final Duration position;
}

/// Seek previews of the file being played (PLAY-009). The app implements it
/// over the core (`pa-trickplay`: sidecar BIF or Jellyfin folder, the disk
/// cache, or generation through FFmpeg); the player asks it while the user
/// scrubs. The OSD (UI-013) uses the same interface.
abstract interface class TrickplaySource {
  /// The preview for [position], or null if there is none (yet). Answers
  /// in well under 100 ms; callers ask again for later positions while the
  /// user moves, the newest request wins.
  Future<TrickplayFrame?> frameAt(Duration position);

  /// Releases the source; an unfinished generation stops unless another
  /// player uses the same file. [PlayerScreen] calls it when the file it
  /// belongs to stops playing.
  void dispose();
}

/// Fetches previews for a moving position without queueing: one request at
/// a time, then the latest wanted position. Decoded images are kept by
/// [TrickplayFrame.imageId] (a few, for sheets and quick back-and-forth).
class TrickplayLoader extends ChangeNotifier {
  TrickplayLoader(this.source);

  final TrickplaySource source;

  /// The frame for the latest position, decoded; null until one arrives.
  TrickplayFrame? get frame => _frame;
  ui.Image? get image => _image;

  TrickplayFrame? _frame;
  ui.Image? _image;
  Duration? _wanted;
  bool _busy = false;
  bool _disposed = false;
  final Map<int, ui.Image> _decoded = {};
  static const _keep = 8;

  /// Asks for the preview at [position].
  void request(Duration position) {
    _wanted = position;
    if (!_busy) unawaited(_run());
  }

  Future<void> _run() async {
    _busy = true;
    try {
      while (!_disposed) {
        final want = _wanted;
        if (want == null) break;
        _wanted = null;
        final frame = await source.frameAt(want);
        if (_disposed || frame == null) continue;
        final image = await _decode(frame);
        if (_disposed || image == null) continue;
        _frame = frame;
        _image = image;
        notifyListeners();
      }
    } catch (e) {
      debugPrint('Trickplay preview failed: $e');
    } finally {
      _busy = false;
    }
  }

  Future<ui.Image?> _decode(TrickplayFrame frame) async {
    final cached = _decoded.remove(frame.imageId);
    if (cached != null) {
      _decoded[frame.imageId] = cached;
      return cached;
    }
    try {
      final codec = await ui.instantiateImageCodec(frame.image);
      final image = (await codec.getNextFrame()).image;
      codec.dispose();
      if (_disposed) {
        image.dispose();
        return null;
      }
      _decoded[frame.imageId] = image;
      while (_decoded.length > _keep) {
        final oldest = _decoded.keys.first;
        final old = _decoded.remove(oldest)!;
        if (!identical(old, _image)) old.dispose();
      }
      return image;
    } catch (e) {
      debugPrint('Trickplay image not decoded: $e');
      return null;
    }
  }

  @override
  void dispose() {
    _disposed = true;
    for (final image in _decoded.values) {
      image.dispose();
    }
    _decoded.clear();
    super.dispose();
  }
}

/// Paints [frame]'s tile of [image] into the given size (cover).
class TrickplayImage extends StatelessWidget {
  const TrickplayImage({super.key, required this.image, required this.frame});

  final ui.Image image;
  final TrickplayFrame frame;

  @override
  Widget build(BuildContext context) =>
      CustomPaint(painter: _TilePainter(image, frame.crop));
}

class _TilePainter extends CustomPainter {
  _TilePainter(this.image, this.crop);

  final ui.Image image;
  final Rect? crop;

  @override
  void paint(Canvas canvas, Size size) {
    final src =
        crop ??
        Rect.fromLTWH(0, 0, image.width.toDouble(), image.height.toDouble());
    canvas.drawImageRect(
      image,
      src,
      Offset.zero & size,
      Paint()..filterQuality = FilterQuality.medium,
    );
  }

  @override
  bool shouldRepaint(_TilePainter old) =>
      !identical(old.image, image) || old.crop != crop;
}
