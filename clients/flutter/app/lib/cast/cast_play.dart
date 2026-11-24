// SPDX-License-Identifier: MPL-2.0
//
// PLAY-012 (ADR-0153): casting a library item — from a detail page or from
// the player's OSD — with the same progress path as the app's player.

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../integration/player.dart';
import '../l10n/l10n.dart';
import '../screens/library_lists_ui.dart' show chooseVersion;
import '../screens/playback.dart';
import '../services/app_services.dart';
import '../sources/jellyfin_link.dart';
import '../widgets/format.dart' show episodeCode;
import 'cast_controller.dart';
import 'cast_platform.dart';
import 'cast_ui.dart';

/// Casts [original] (asking for the version when it has several, unless
/// [pickVersion] is false) to [device] from [startPositionMs]. Tells the
/// user what happens in snack bars and asks before a file the receiver is
/// not expected to play. True once the receiver plays it.
Future<bool> castItem(
  BuildContext context,
  LibraryItem original,
  CastDevice device, {
  int startPositionMs = 0,
  Rect? anchor,
  bool pickVersion = true,
  CastController? controller,
}) async {
  final item = pickVersion ? await chooseVersion(context, original) : original;
  if (item == null || !context.mounted) return false;
  if (item.unplayable case final reason?) {
    await showUnplayable(context, reason);
    return false;
  }
  final c = controller ?? CastController.instance;
  final services = Services.read(context);
  final l = context.l10n;
  final messenger = ScaffoldMessenger.maybeOf(context);
  final library = services.library;

  TechInfo? tech;
  if (library case final LibraryDetails d) {
    try {
      final t = (await d.details(item.id)).tech;
      // Another version than the probed one: judge by its name instead.
      if (t != null && (t.filePath == null || t.filePath == item.filePath)) {
        tech = t;
      }
    } catch (e) {
      debugPrint('Details of ${item.id} not read for casting: $e');
    }
  }
  var serverItem = false;
  if (services.jellyfin case final link?) {
    try {
      serverItem = await link.isServerItem(item.id);
    } catch (e) {
      debugPrint('Server of ${item.id} not known: $e');
    }
  }
  final playbackUri = await playbackUriOf(library, item);
  if (!context.mounted) return false;

  final reporter = JellyfinReporter(services.jellyfin);
  final request = CastRequest(
    itemId: item.id,
    title: item.kind == ItemKind.episode
        ? '${episodeCode(l, item)}  ${item.title}'
        : item.title,
    artworkUrl: item.backdropUrl ?? item.posterUrl,
    playbackUri: playbackUri,
    fileName: item.filePath,
    tech: tech,
    durationMs: item.durationMs ?? tech?.durationMs ?? 0,
    startMs: startPositionMs,
    serverItem: serverItem,
    serverStream: serverItem
        ? (ms) => serverStreamUrl(services, item, ms)
        : null,
    onProgress: castProgressSaver(services, item, reporter),
    onEnded: () {
      reporter.close();
      if (services.handoff.playingItemId == item.id) services.handoff.end();
    },
  );

  messenger
    ?..removeCurrentSnackBar()
    ..showSnackBar(
      SnackBar(content: Text(l.castConnecting(castDeviceName(l, device)))),
    );
  var result = await c.cast(request, device, anchor: anchor);
  if (result.outcome == CastOutcome.unsupported && context.mounted) {
    messenger?.removeCurrentSnackBar();
    if (await showCastUnsupported(context, device, result.unsupported!)) {
      result = await c.cast(request, device, anchor: anchor, force: true);
    }
  }
  messenger?.removeCurrentSnackBar();
  final text = switch (result.outcome) {
    CastOutcome.started => l.castStarted(castDeviceName(l, device)),
    CastOutcome.failed => l.castFailed(castDeviceName(l, device)),
    CastOutcome.noNetwork => l.castNoNetwork,
    CastOutcome.cancelled || CastOutcome.unsupported => null,
  };
  if (text != null) messenger?.showSnackBar(SnackBar(content: Text(text)));
  return result.outcome == CastOutcome.started;
}

/// The cast button of the player's OSD: pauses the player, casts the item
/// from where it is, and closes the player once the receiver plays it (the
/// mini-controller takes over). If casting does not start, the player
/// goes on.
Future<void> castFromPlayer(
  BuildContext context,
  LibraryItem item,
  CastDevice device, {
  Rect? anchor,
  PlayerEngine? engine,
}) async {
  final state = engine?.state.value;
  final wasPlaying = state?.playing ?? false;
  final position = state?.position.inMilliseconds ?? 0;
  await engine?.pause();
  if (!context.mounted) return;
  final started = await castItem(
    context,
    item,
    device,
    startPositionMs: position,
    anchor: anchor,
    pickVersion: false,
  );
  if (!context.mounted) return;
  if (started) {
    await Navigator.of(context).maybePop();
  } else if (wasPlaying) {
    await engine?.play();
  }
}
