// SPDX-License-Identifier: MPL-2.0
//
// Playing music from the music screens (MUS-004) and counting plays: a
// track counts as played when it becomes the current track of the music
// player, however it got there (tap, next, auto-advance). MUS-003: Play
// Next, Add to Queue and the source of the queue ("Up next from: …").

import 'package:material_ui/material_ui.dart';

import '../../features/music/music_queue_store.dart';
import '../../integration/library.dart';
import '../../integration/player.dart';
import '../../l10n/l10n.dart';
import '../../services/app_services.dart';

/// Counts plays of library tracks played by [player] in [music].
class MusicPlayRecorder {
  MusicPlayRecorder._(this._player, this._music) {
    _player.addListener(_onChange);
  }

  static final _recorders = Expando<MusicPlayRecorder>();

  /// The recorder of [services] (one per music player).
  static MusicPlayRecorder of(AppServices services) {
    final music = libraryMusic(services.library);
    final r = _recorders[services.music];
    if (r != null && identical(r._music, music)) return r;
    r?._player.removeListener(r._onChange);
    return _recorders[services.music] = MusicPlayRecorder._(
      services.music,
      music,
    );
  }

  final MusicPlayerController _player;
  final LibraryMusic _music;

  /// Track id by playback URI.
  final Map<String, String> _ids = {};
  String? _current;

  /// Makes the tracks known by their playback URI.
  void register(Iterable<MusicTrack> tracks) {
    for (final t in tracks) {
      _ids[t.item.playbackUri] = t.id;
    }
  }

  void _onChange() {
    final cur = _player.current;
    final uri = cur?.uri;
    if (uri == _current) return;
    _current = uri;
    final id = uri == null ? null : (cur?.itemId ?? _ids[uri]);
    if (id != null) _music.recordTrackPlay(id).ignore();
  }
}

/// Plays [tracks] from [index]; [shuffle] plays them in a random order.
/// [source] names them in the queue ("Up next from: <source>").
void playMusic(
  BuildContext context,
  List<MusicTrack> tracks, {
  int index = 0,
  bool shuffle = false,
  String? source,
}) {
  if (tracks.isEmpty) return;
  final services = Services.read(context);
  final list = shuffle ? ([...tracks]..shuffle()) : tracks;
  MusicPlayRecorder.of(services).register(list);
  services.music.play(
    [for (final t in list) musicRequest(t)],
    index: shuffle ? 0 : index,
    source: source == null || source.isEmpty ? null : QueueSource(source),
  );
}

/// Play Next ([next]) or Add to Queue: [tracks] go into the queue after
/// the current track; a snack bar confirms.
Future<void> queueMusic(
  BuildContext context,
  List<MusicTrack> tracks, {
  required bool next,
}) async {
  if (tracks.isEmpty) return;
  final services = Services.read(context);
  final l = context.l10n;
  final messenger = ScaffoldMessenger.maybeOf(context);
  MusicPlayRecorder.of(services).register(tracks);
  final requests = [for (final t in tracks) musicRequest(t)];
  if (next) {
    await services.music.playNext(requests);
  } else {
    await services.music.addToQueue(requests);
  }
  messenger
    ?..removeCurrentSnackBar()
    ..showSnackBar(
      SnackBar(
        content: Text(
          next
              ? l.musicQueuedNext(tracks.length)
              : l.musicQueuedLast(tracks.length),
        ),
      ),
    );
}
