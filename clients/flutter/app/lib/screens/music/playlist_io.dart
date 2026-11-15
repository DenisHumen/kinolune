// SPDX-License-Identifier: MPL-2.0
//
// Playlist dialogs of MUS-003: "Save as playlist" of the queue, M3U/M3U8
// import and export with the system file pickers.

import 'dart:convert';
import 'dart:io';

import 'package:file_picker/file_picker.dart';
import 'package:material_ui/material_ui.dart';

import '../../features/music/m3u.dart';
import '../../features/music/playlist_actions.dart';
import '../../integration/library.dart';
import '../../integration/player.dart';
import '../../l10n/l10n.dart';
import '../../services/app_services.dart';

void _snack(ScaffoldMessengerState? m, String text) => m
  ?..removeCurrentSnackBar()
  ..showSnackBar(SnackBar(content: Text(text)));

/// Asks for a playlist name; null when cancelled or empty.
Future<String?> askPlaylistName(
  BuildContext context, {
  required String title,
  String initial = '',
}) async {
  final chosen = await showDialog<String>(
    context: context,
    builder: (_) => _NameDialog(title: title, initial: initial),
  );
  return chosen == null || chosen.isEmpty ? null : chosen;
}

/// Owns its text controller, so it lives as long as the closing animation.
class _NameDialog extends StatefulWidget {
  const _NameDialog({required this.title, required this.initial});
  final String title;
  final String initial;

  @override
  State<_NameDialog> createState() => _NameDialogState();
}

class _NameDialogState extends State<_NameDialog> {
  late final _name = TextEditingController(text: widget.initial);

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return AlertDialog(
      title: Text(widget.title),
      content: TextField(
        key: const ValueKey('playlist-name'),
        controller: _name,
        autofocus: true,
        decoration: InputDecoration(labelText: l.libraryPlaylistName),
        onSubmitted: (v) => Navigator.pop(context, v.trim()),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: Text(l.commonCancel),
        ),
        FilledButton(
          key: const ValueKey('playlist-name-ok'),
          onPressed: () => Navigator.pop(context, _name.text.trim()),
          child: Text(l.libraryPlaylistCreate),
        ),
      ],
    );
  }
}

/// "Save as playlist" of Now Playing: the queue's library tracks become a
/// new audio playlist.
Future<void> saveQueueAsPlaylist(
  BuildContext context,
  List<PlayRequest> tracks,
) async {
  final services = Services.read(context);
  final music = libraryMusic(services.library);
  final l = context.l10n;
  final messenger = ScaffoldMessenger.maybeOf(context);
  if (!music.hasMusicPlaylists) {
    _snack(messenger, l.musicPlaylistsUnavailable);
    return;
  }
  final name = await askPlaylistName(
    context,
    title: l.musicSaveQueueTitle,
    initial: services.music.source?.label ?? '',
  );
  if (name == null) return;
  final (playlist, added) = await createPlaylistWith(music, name, [
    for (final t in tracks) ?t.itemId,
  ]);
  _snack(messenger, l.musicQueueSaved(playlist.name, added));
}

/// Exports [tracks] as `<name>.m3u8`. Where the system says where the file
/// went (desktop), it is rewritten with paths relative to that folder.
Future<void> exportM3u(
  BuildContext context,
  String name,
  List<MusicTrack> tracks,
) async {
  final l = context.l10n;
  final messenger = ScaffoldMessenger.maybeOf(context);
  try {
    final uri = await FilePicker.saveFile(
      fileName: '$name.m3u8',
      bytes: utf8.encode(writeM3u(tracks)),
      mimeType: 'audio/x-mpegurl',
      dialogTitle: l.musicM3uExport,
    );
    if (uri == null) return;
    if (uri.scheme == 'file') {
      final path = uri.toFilePath();
      await File(path).writeAsString(writeM3u(tracks, playlistPath: path));
    }
    _snack(messenger, l.musicM3uExported(tracks.length));
  } catch (e) {
    debugPrint('M3U export failed: $e');
    _snack(messenger, l.musicM3uFailed);
  }
}

/// Picks an M3U/M3U8 file and makes an audio playlist of the library
/// tracks it lists.
Future<void> importM3u(BuildContext context, LibraryMusic music) async {
  final l = context.l10n;
  final messenger = ScaffoldMessenger.maybeOf(context);
  try {
    final file = await FilePicker.pickFile(
      dialogTitle: l.musicM3uImport,
      type: FileType.custom,
      allowedExtensions: const ['m3u', 'm3u8'],
    );
    if (file == null) return;
    final result = await importM3uPlaylist(
      music,
      name: m3uPlaylistName(file.name),
      text: decodeM3u(await file.readAsBytes()),
      path: file.path,
    );
    if (result == null) {
      _snack(messenger, l.musicM3uNoMatches);
      return;
    }
    _snack(
      messenger,
      result.missing.isEmpty
          ? l.musicM3uImported(result.playlist.name, result.added)
          : l.musicM3uImportedMissing(
              result.playlist.name,
              result.added,
              result.missing.length,
            ),
    );
  } catch (e) {
    debugPrint('M3U import failed: $e');
    _snack(messenger, l.musicM3uFailed);
  }
}
