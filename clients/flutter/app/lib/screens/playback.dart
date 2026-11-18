// SPDX-License-Identifier: MPL-2.0

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../integration/player.dart';
import '../l10n/l10n.dart';
import '../platform/form_factor.dart';
import '../services/app_services.dart';
import '../sources/jellyfin_link.dart';
import '../subtitles/online_subtitles.dart';
import '../sync/handoff_ui.dart';
import '../widgets/format.dart';
import 'library_lists_ui.dart';
import 'music/playlist_io.dart' show saveQueueAsPlaylist;
import 'playback_skip.dart';
import 'playback_trickplay.dart';

/// Opens the fullscreen player (pa_player) on the root navigator and saves
/// progress to the library (PLAY-005: every 5 s, on pause, seek, exit and in
/// the background). Pass the saved position to resume, 0 to start over.
/// Episodes get Up Next: the next episode of the show plays after a
/// countdown, resuming where it was left.
///
/// Subtitles (PLAY-007): the sidecar files the library knows for the file
/// are passed to the player (network ones through the stream proxy), and
/// the subtitle track and delay are remembered per show.
///
/// Tracks and sound (PLAY-004): the player picks the audio and subtitle
/// tracks by the active profile's policy (Settings → Playback → Languages
/// and sound) with the title's original language from the library; the
/// audio track and delay are remembered per show with the subtitles, and
/// the profile's sound processing applies (quick toggles in the OSD audio
/// panel change it). A movie of
/// several parts gets the subtitles of each part joined into tracks on the
/// common timeline (LIB-021).
///
/// Seek previews (PLAY-009): the player shows them while scrubbing; they
/// come from the library (sidecar, cache, or generated in the background).
///
/// Skip intro and credits (PLAY-011): the segments the library knows are
/// passed to the player, which offers a button or skips by itself as set
/// in Settings → Playback; playing an episode queues its season for the
/// background analysis.
///
/// Jellyfin (NET-014): the server says how an item of a Jellyfin source
/// plays here; DirectPlay is the item's own stream through the proxy,
/// DirectStream and Transcode open the server's HLS URL. Progress saves
/// are also the server's playback reports, and closing the player stops
/// the playback there.
///
/// Handoff (SYNC-003): before the player opens, a short pull asks the other
/// devices where the title is; a newer position of another device wins
/// over [startPositionMs] unless [fromStart]. The progress saves are also
/// the heartbeats of the session ([HandoffController.report]).
Future<void> playVideo(
  BuildContext context,
  LibraryItem original, {
  int startPositionMs = 0,
  bool fromStart = false,
}) async {
  // LIB-010: a movie with several versions asks which one to play.
  final item = await chooseVersion(context, original);
  if (item == null || !context.mounted) return;
  // LIB-020: a disc image and the like say why instead of opening the player.
  if (item.unplayable case final reason?) {
    return showUnplayable(context, reason);
  }
  final services = Services.read(context);
  final l = context.l10n;
  // MUS-003: music is global state; a video pauses it and offers it back.
  final messenger = ScaffoldMessenger.maybeOf(context);
  final musicInterrupted = services.music.pauseForVideo();
  final handoff = services.handoff;
  final start = await handoff.startPosition(
    item,
    startPositionMs,
    fromStart: fromStart,
  );
  if (!context.mounted) return;
  final request = await _request(services, l, item, start);
  if (!context.mounted) return;
  // UI-014: track panels across the top with tabs for the remote.
  final tv = FormFactorScope.of(context) == FormFactor.tv;
  final reporter = JellyfinReporter(services.jellyfin);
  handoff.begin(item.id, start, durationMs: item.durationMs ?? 0);
  await Navigator.of(context, rootNavigator: true).push(
    MaterialPageRoute<void>(
      fullscreenDialog: true,
      builder: (_) => HandoffScope(
        controller: handoff,
        child: PlayerScreen(
          request: request,
          engineFactory: handoff.engineFactory(services.playerEngineFactory),
          onProgress: _saver(services, item, reporter),
          loadNext: _nextLoader(services, l, item, reporter),
          // QA-015: off only stops the countdown; «Далее» stays.
          continuousPlayback: services.continuousPlayback.value,
          subtitleSettings: services.subtitleSettings,
          onSubtitleSettings: services.setSubtitleSettings,
          subtitleLanguages: services.subtitleLanguages,
          onSubtitleMemory: services.saveSubtitleMemory,
          trackPolicy: services.trackPolicy.value,
          audioProcessing: services.audioProcessing,
          onAudioProcessing: services.setAudioProcessing,
          seekStep: Duration(seconds: services.skipSeconds.value),
          skipMode: services.skipMode.value,
          tvLayout: tv,
        ),
      ),
    ),
  );
  reporter.close();
  if (musicInterrupted) offerMusicResume(services, messenger, l);
}

/// After a video that paused the music: "Music paused · Resume".
void offerMusicResume(
  AppServices services,
  ScaffoldMessengerState? messenger,
  AppLocalizations l,
) {
  final music = services.music;
  if (!music.interruptedByVideo) return;
  if (messenger == null) {
    music.dismissInterruption();
    return;
  }
  messenger
    ..removeCurrentSnackBar()
    ..showSnackBar(
      SnackBar(
        key: const ValueKey('music-resume-offer'),
        content: Text(l.musicPausedByVideo),
        duration: const Duration(seconds: 8),
        action: SnackBarAction(
          key: const ValueKey('music-resume'),
          label: l.musicResume,
          onPressed: music.resumeAfterVideo,
        ),
      ),
    ).closed.then((reason) {
      if (reason != SnackBarClosedReason.action) music.dismissInterruption();
    });
}

Future<PlayRequest> _request(
  AppServices services,
  AppLocalizations l,
  LibraryItem item,
  int startMs, {
  bool eager = true,
}) async {
  final series = item.kind == ItemKind.episode ? item.showId : null;
  final base = await playbackUriOf(services.library, item);
  final uri = await _serverStream(services, item, startMs) ?? base;
  return PlayRequest(
    uri: uri,
    title: item.kind == ItemKind.episode
        ? '${episodeCode(l, item)}  ${item.title}'
        : item.title,
    startPositionMs: startMs,
    artworkUri: item.backdropUrl ?? item.posterUrl,
    durationMs: item.durationMs,
    subtitles: await sidecarSubtitles(services.library, item),
    subtitleSearch: subtitleSearchFor(
      services.library,
      services.secureStore,
      item,
    ),
    seriesKey: series,
    subtitleMemory: series == null
        ? null
        : await services.subtitleMemory(series),
    originalLanguage: await originalLanguageOf(services.library, item),
    // A timeline of several parts (LIB-020) has no previews of its own; a
    // server stream has the file's timeline.
    trickplay: base == item.playbackUri
        ? trickplayOf(services.library, item, eager: eager)
        : null,
    // Segments are times in the item's own file.
    skipSegments:
        base == item.playbackUri && services.skipMode.value != SkipMode.off
        ? await skipSegmentsOf(services.library, item, analyze: eager)
        : const [],
  );
}

/// The HLS URL of [item] when its Jellyfin server streams it for this
/// device (DirectStream or Transcode); null for DirectPlay, items of other
/// sources, or when the server cannot be asked (the file then plays as
/// is).
Future<String?> _serverStream(
  AppServices services,
  LibraryItem item,
  int startMs,
) async {
  final link = services.jellyfin;
  if (link == null) return null;
  try {
    return (await link.playback(item.id, startMs: startMs))?.url;
  } catch (e) {
    debugPrint('Jellyfin playback of ${item.id} not planned: $e');
    return null;
  }
}

/// Original language of [item]'s title for the "Original" audio and
/// subtitle languages (PLAY-004); null if the library cannot tell.
Future<String?> originalLanguageOf(Library library, LibraryItem item) async {
  if (library is! LibraryOriginalLanguage) return null;
  try {
    return await (library as LibraryOriginalLanguage).originalLanguage(item.id);
  } catch (e) {
    debugPrint('Original language of ${item.id} not read: $e');
    return null;
  }
}

/// What the player opens for [item] (LIB-020): all parts of a split movie
/// or a DVD title set as one mpv EDL timeline, else the item's own URI.
/// Progress stays with the item.
Future<String> playbackUriOf(Library library, LibraryItem item) async {
  if (library is! LibraryParts) return item.playbackUri;
  try {
    final parts = await (library as LibraryParts).playParts(item);
    if (parts.length > 1) return partsUri(parts);
  } catch (e) {
    debugPrint('Parts of ${item.id} not read: $e');
  }
  return item.playbackUri;
}

/// Tells why [item]'s file cannot be played (LIB-020), in place of the
/// player.
Future<void> showUnplayable(BuildContext context, Unplayable reason) {
  final l = context.l10n;
  return showDialog<void>(
    context: context,
    builder: (context) => AlertDialog(
      key: const ValueKey('unplayable-dialog'),
      title: Text(l.libraryUnplayableTitle),
      content: Text(switch (reason) {
        Unplayable.discImage => l.libraryUnplayableDiscImage,
        Unplayable.other => l.libraryUnplayableOther,
      }),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: Text(l.libraryUnplayableOk),
        ),
      ],
    ),
  );
}

/// Subtitle files of [item]'s file from the library, for the player; null
/// if the library cannot tell (the player then looks next to local files
/// itself).
Future<List<ExternalSubtitle>?> sidecarSubtitles(
  Library library,
  LibraryItem item,
) async {
  if (library is! LibrarySubtitles) return null;
  final sidecars = library as LibrarySubtitles;
  try {
    final subs = await sidecars.sidecarSubtitles(item);
    return [
      for (final s in subs)
        ExternalSubtitle(
          uri: s.uri,
          title: s.name,
          language: s.language,
          forced: s.forced,
          hearingImpaired: s.hearingImpaired,
          isDefault: s.isDefault,
          parts: s.parts,
        ),
    ];
  } catch (e) {
    debugPrint('Sidecar subtitles of ${item.id} not read: $e');
    return null;
  }
}

/// Saves a progress report of the player and sends it as the heartbeat of
/// the handoff session (SYNC-003) and to a Jellyfin server (NET-014).
void Function(int, int) _saver(
  AppServices services,
  LibraryItem item,
  JellyfinReporter reporter,
) => (pos, dur) {
  services.library.saveProgress(
    Progress(
      itemId: item.id,
      positionMs: pos,
      durationMs: dur,
      updatedAt: DateTime.now(),
    ),
  );
  services.handoff.report(item.id, pos, dur);
  reporter.report(item.id, pos);
};

/// Up Next for an episode: the next one of its show, from its saved
/// position if it is in progress. With continuous playback off (Settings →
/// Playback, UI-016) the card still comes, without the countdown (QA-015).
Future<UpNextItem?> Function()? _nextLoader(
  AppServices services,
  AppLocalizations l,
  LibraryItem item,
  JellyfinReporter reporter,
) {
  if (item.kind != ItemKind.episode) return null;
  final library = services.library;
  return () async {
    final next = await library.nextEpisode(item.id);
    if (next == null) return null;
    final start = resumePositionMs(await library.progress(next.id));
    return UpNextItem(
      request: await _request(services, l, next, start, eager: false),
      onProgress: _saver(services, next, reporter),
      loadNext: _nextLoader(services, l, next, reporter),
    );
  };
}

/// Starts [tracks] from [index] in the music player.
void playTracks(BuildContext context, List<LibraryItem> tracks, int index) {
  Services.read(context).music.play([
    for (final t in tracks)
      PlayRequest(
        uri: t.playbackUri,
        itemId: t.id,
        title: t.title,
        artist: t.artist,
        album: t.album,
        artworkUri: t.posterUrl,
        durationMs: t.durationMs,
      ),
  ], index: index);
}

/// Plays a file outside the library (the file browser, "Open with
/// Kinolune", UI-022): audio in the music player, video in the fullscreen
/// player without progress.
Future<void> playLooseFile(
  BuildContext context,
  PlayRequest request, {
  required bool audio,
}) async {
  final services = Services.read(context);
  if (audio) {
    services.music.play([request]);
    return;
  }
  services.music.pause();
  await Navigator.of(context, rootNavigator: true).push(
    MaterialPageRoute<void>(
      fullscreenDialog: true,
      builder: (_) => PlayerScreen(
        request: request,
        // A file outside the library has no progress to keep.
        onProgress: (_, _) {},
        engineFactory: services.playerEngineFactory,
        subtitleSettings: services.subtitleSettings,
        subtitleLanguages: services.subtitleLanguages,
        seekStep: Duration(seconds: services.skipSeconds.value),
      ),
    ),
  );
}

void openNowPlaying(BuildContext context) {
  final services = Services.read(context);
  Navigator.of(context, rootNavigator: true).push(
    MaterialPageRoute<void>(
      fullscreenDialog: true,
      builder: (_) => NowPlayingScreen(
        controller: services.music,
        onSaveQueue: saveQueueAsPlaylist,
      ),
    ),
  );
}
