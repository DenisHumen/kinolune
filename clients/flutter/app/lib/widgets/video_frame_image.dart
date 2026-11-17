// SPDX-License-Identifier: MPL-2.0
//
// UI-032: a preview frame taken from a video file by the core (personal
// libraries, «Мои видео») over the tile it replaces. The core makes and
// caches the frame (a personal library's only while it is open, ADR-0098)
// and hands over JPEG bytes, never a path; until they come, or when the
// file has no frame worth showing, [fallback] stays. The decoded image
// leaves Flutter's memory cache with the tile, so a frame of a library
// that was locked meanwhile is not kept around.

import 'dart:async' show unawaited;
import 'dart:typed_data' show Uint8List;

import 'package:material_ui/material_ui.dart';

class VideoFrameImage extends StatefulWidget {
  const VideoFrameImage({
    super.key,
    required this.load,
    required this.fallback,
    this.imageKey,
  });

  /// Fetches the frame; null or an error keeps [fallback].
  final Future<Uint8List?> Function() load;

  /// Shown until the frame comes, and instead of a missing one.
  final Widget fallback;

  /// Key of the image once it shows (tests).
  final Key? imageKey;

  @override
  State<VideoFrameImage> createState() => _VideoFrameImageState();
}

class _VideoFrameImageState extends State<VideoFrameImage> {
  MemoryImage? _image;

  @override
  void initState() {
    super.initState();
    unawaited(_fetch());
  }

  Future<void> _fetch() async {
    final Uint8List? bytes;
    try {
      bytes = await widget.load();
    } catch (_) {
      // Locked meanwhile, unreadable, no FFmpeg: the tile stays.
      return;
    }
    if (bytes == null || bytes.isEmpty || !mounted) return;
    final image = MemoryImage(bytes);
    setState(() => _image = image);
  }

  @override
  void dispose() {
    _image?.evict();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final image = _image;
    if (image == null) return widget.fallback;
    return Stack(
      fit: StackFit.expand,
      children: [
        widget.fallback,
        Image(
          key: widget.imageKey,
          image: image,
          fit: BoxFit.cover,
          gaplessPlayback: true,
          errorBuilder: (_, _, _) => const SizedBox.shrink(),
        ),
      ],
    );
  }
}
