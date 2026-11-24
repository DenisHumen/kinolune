// SPDX-License-Identifier: MPL-2.0

/// Lyrics stored with a music file (MUS-006), as the core reads them from
/// any source (local, SMB, WebDAV…) through `pa-vfs`.
class TrackLyricsFiles {
  const TrackLyricsFiles({this.synced = const [], this.plain, this.lrc});

  /// ID3v2 `SYLT` lines by time.
  final List<(Duration, String)> synced;

  /// The plain lyrics tag (`USLT`, `LYRICS`, `©lyr`); may hold LRC text.
  final String? plain;

  /// The `.lrc` file next to the track (same base name).
  final String? lrc;

  bool get isEmpty => synced.isEmpty && plain == null && lrc == null;
}

/// A library that can read the lyrics stored with its music files.
abstract interface class LibraryLyrics {
  /// Lyrics of the file the player plays at [playbackUri]
  /// ([LibraryItem.playbackUri] of a track); null when the file is not in
  /// the library. Throws when its source cannot be reached.
  Future<TrackLyricsFiles?> trackLyrics(String playbackUri);
}
