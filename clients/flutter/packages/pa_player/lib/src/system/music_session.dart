// SPDX-License-Identifier: MPL-2.0
import '../music/music_player_controller.dart';
import 'media_session.dart';

/// Shows the music player in the [SystemMediaSession] (PLAY-AND-008): from
/// the first time something plays until the queue is stopped. A queue
/// restored at start-up stays out of the notification until it plays.
class MusicSessionBinding {
  MusicSessionBinding(this.music, this.session) {
    music.addListener(_sync);
  }

  final MusicPlayerController music;
  final SystemMediaSession session;
  MediaSessionClaim? _claim;

  /// The claim while the music is in the session (tests).
  MediaSessionClaim? get claim => _claim;

  void _sync() {
    if (!music.hasQueue) {
      _drop();
      return;
    }
    if (_claim == null) {
      if (!music.playing) return;
      _claim = session.claim(_onCommand, debugLabel: 'music');
    }
    final t = music.current;
    if (t == null) return;
    final queue = music.queue;
    _claim!.update(
      MediaSessionInfo(
        kind: MediaSessionKind.music,
        id: t.itemId ?? t.uri,
        title: t.title,
        subtitle: t.artist,
        album: t.album,
        artworkUri: t.artworkUri,
        duration: music.duration,
        position: music.position,
        playing: music.playing,
        buffering: music.buffering,
        hasNext: queue.nextPosition() != null,
        hasPrevious: queue.previousPosition() != null,
      ),
    );
  }

  void _onCommand(MediaSessionCommand command, Duration? position) {
    switch (command) {
      case MediaSessionCommand.play:
        music.resume();
      case MediaSessionCommand.pause:
      case MediaSessionCommand.stop:
        music.pause();
      case MediaSessionCommand.next:
        music.next();
      case MediaSessionCommand.previous:
        music.previous();
      case MediaSessionCommand.seek:
        if (position != null) music.seek(position);
      case MediaSessionCommand.duck:
        music.setDucked(true);
      case MediaSessionCommand.unduck:
        music.setDucked(false);
    }
  }

  void _drop() {
    _claim?.release();
    _claim = null;
  }

  void dispose() {
    music.removeListener(_sync);
    _drop();
  }
}
