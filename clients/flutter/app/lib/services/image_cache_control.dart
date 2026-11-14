// SPDX-License-Identifier: MPL-2.0

import '../integration/library.dart' show RustArtworkCache;

/// Usage of the image cache: bytes and files on disk and the size limit.
typedef ImageCacheUsage = ({int bytes, int files, int maxBytes});

/// Size limits offered in Settings → Storage (UI-016). The default is the
/// core's (`defaultArtworkCacheBytes`, 256 MiB).
const imageCacheLimits = <int>[
  128 << 20,
  256 << 20,
  512 << 20,
  1024 << 20,
  2048 << 20,
];

/// Settings → Storage view of the poster and backdrop cache (LIB-013): its
/// usage and limit, and clearing it. The app's [RustArtworkCache] on the
/// core; [MemoryImageCacheControl] in the demo and tests.
abstract interface class ImageCacheControl {
  ImageCacheUsage usage();

  /// Changes the size limit; the cache shrinks at once.
  Future<void> setMaxBytes(int maxBytes);

  /// Deletes every cached image.
  Future<void> clear();
}

/// [ImageCacheControl] over the core's artwork cache.
class RustImageCacheControl implements ImageCacheControl {
  RustImageCacheControl(this.cache);

  final RustArtworkCache cache;

  @override
  ImageCacheUsage usage() => cache.usage;

  @override
  Future<void> setMaxBytes(int maxBytes) => cache.setMaxBytes(maxBytes);

  @override
  Future<void> clear() => cache.clear();
}

/// In-memory [ImageCacheControl] for the demo mode and tests.
class MemoryImageCacheControl implements ImageCacheControl {
  MemoryImageCacheControl({
    this.bytes = 0,
    this.files = 0,
    this.maxBytes = 256 << 20,
  });

  int bytes;
  int files;
  int maxBytes;

  @override
  ImageCacheUsage usage() => (bytes: bytes, files: files, maxBytes: maxBytes);

  @override
  Future<void> setMaxBytes(int maxBytes) async {
    this.maxBytes = maxBytes;
    if (bytes > maxBytes) {
      files = bytes == 0 ? 0 : (files * maxBytes / bytes).floor();
      bytes = maxBytes;
    }
  }

  @override
  Future<void> clear() async {
    bytes = 0;
    files = 0;
  }
}

/// The demo mode's cache: some posters already on "disk".
MemoryImageCacheControl demoImageCache() =>
    MemoryImageCacheControl(bytes: 87 << 20, files: 1240);
