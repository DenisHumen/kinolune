// SPDX-License-Identifier: MPL-2.0
//
// Making audio playlists from the queue and from M3U files (MUS-003); the
// screens add the dialogs and file pickers around these.

import '../../integration/library.dart';
import 'm3u.dart';

/// What an M3U import made.
class M3uImport {
  const M3uImport(this.playlist, this.added, this.missing);

  final MusicPlaylist playlist;

  /// Tracks put into [playlist] (a track listed twice counts once).
  final int added;

  /// Lines of the file no library track was found for.
  final List<String> missing;
}

/// Creates the playlist [name] with [trackIds] in order (ids the library
/// does not know are skipped by it); returns the playlist and how many
/// tracks it got.
Future<(MusicPlaylist, int)> createPlaylistWith(
  LibraryMusic music,
  String name,
  Iterable<String> trackIds,
) async {
  final p = await music.createMusicPlaylist(name);
  var added = 0;
  for (final id in trackIds) {
    try {
      if (await music.addToMusicPlaylist(p.id, id)) added++;
    } catch (_) {
      // A track gone from the library meanwhile: leave it out.
    }
  }
  return (p, added);
}

/// Imports the M3U/M3U8 [text] as the playlist [name]: lines are resolved
/// against [path] (where the file is) and the library tracks. Null when
/// no line matched a track (nothing is created).
Future<M3uImport?> importM3uPlaylist(
  LibraryMusic music, {
  required String name,
  required String text,
  String? path,
}) async {
  final tracks = await music.musicTracks().first;
  final m = resolveM3u(parseM3u(text), tracks, playlistPath: path);
  if (m.tracks.isEmpty) return null;
  final (p, added) = await createPlaylistWith(music, name, [
    for (final t in m.tracks) t.id,
  ]);
  return M3uImport(p, added, m.missing);
}
