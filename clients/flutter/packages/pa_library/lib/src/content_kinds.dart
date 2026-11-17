// SPDX-License-Identifier: MPL-2.0

// LIB-025 (ADR-0099): the core decides what every file holds; the user
// never picks it. This is what the app sees of it: «Мои видео» (footage
// from phones and cameras, never recognized, neither hidden nor locked)
// and «Это не то» on a folder of the file browser. Audiobooks come with
// the music tracks ([MusicTrack.audiobook]).

import 'dart:typed_data' show Uint8List;

/// What a file or a folder holds.
enum ContentKind {
  /// Movies, shows, anime: recognized.
  video,

  /// Footage from a phone or a camera: «Мои видео».
  ownVideo,
  music,
  audiobook;

  /// From the core's `video`, `own_video`, `music`, `audiobook`.
  static ContentKind? parse(String? s) => switch (s) {
    'video' => video,
    'own_video' => ownVideo,
    'music' => music,
    'audiobook' => audiobook,
    _ => null,
  };

  /// What «Это не то» turns it into.
  ContentKind get flipped => switch (this) {
    video => ownVideo,
    ownVideo => video,
    music => audiobook,
    audiobook => music,
  };
}

/// A video of «Мои видео».
class OwnVideo {
  const OwnVideo({
    required this.fileId,
    required this.sourceId,
    required this.relPath,
    required this.title,
    required this.sizeBytes,
    required this.takenAt,
    this.localPath,
    this.durationMs,
  });

  final int fileId;
  final int sourceId;

  /// Relative to the source root.
  final String relPath;

  /// Absolute path for a local source.
  final String? localPath;

  /// The file name without its extension.
  final String title;
  final int sizeBytes;
  final int? durationMs;

  /// When it was shot (the file's time), else added.
  final DateTime takenAt;

  @override
  String toString() => 'OwnVideo($sourceId:$relPath)';
}

/// What a subfolder of a source holds.
class FolderContent {
  const FolderContent({
    required this.path,
    required this.kind,
    this.overridden = false,
  });

  /// Relative to the source root, no leading slash.
  final String path;

  /// What most of its media files are.
  final ContentKind kind;

  /// The user said «Это не то» on it or a folder above it.
  final bool overridden;
}

/// The content kinds of a library. [RustLibrary] has them; the v0 library
/// does not (its app shows no «Мои видео» and no «Это не то»).
abstract interface class LibraryContentKinds {
  /// «Мои видео» changed (a scan, «Это не то»).
  Stream<void> get contentKindChanges;

  /// «Мои видео», newest first.
  Future<List<OwnVideo>> ownVideos();

  /// What the player opens for [video]: local paths or stream URLs; empty
  /// if it cannot be played here.
  Future<List<String>> ownVideoPlayUris(OwnVideo video);

  /// UI-032: the preview frame (JPEG) of [video], taken from the file at
  /// about 10 % of its length; null when it has none worth showing.
  Future<Uint8List?> ownVideoFrame(OwnVideo video);

  /// What the subfolders of [dir] (relative, `''` for the root) of source
  /// [sourceId] hold, by path.
  Future<Map<String, FolderContent>> folderKinds(int sourceId, String dir);

  /// «Это не то» on folder [path] (`''` for the whole source): its files
  /// become the other kind of their medium, and the folder remembers it.
  /// Again, and the detected kind is back. Returns the new kind, null for
  /// a folder without media.
  Future<ContentKind?> notThis(int sourceId, String path);
}
