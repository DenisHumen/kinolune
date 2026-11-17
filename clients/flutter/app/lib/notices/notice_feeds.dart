// SPDX-License-Identifier: MPL-2.0
//
// Where in-app notices come from (UI-026, ADR-NNNN). Only events the app
// already has, no requests of its own: the library's "recently added"
// queries (they re-run when the library changes, so a scan by hand or by
// the watcher shows up), the takeovers of pa-sync (another device of the
// profile started a title), the offline download queue, the Twitch VOD and
// recording queues of the core (read every 30 s, local calls), the "X is
// live" alerts of TWI-009. New app releases have their own strip (PKG-022).
//
// Nothing is announced for what was already there when the app started.
// A profile switch drops the queue and starts over. Library notices are
// not shown to a restricted (kid) profile; protected and personal
// libraries (UI-025) plug their rule into [InAppNotices.filter].

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../offline/offline_downloads.dart';
import '../screens/playback.dart';
import '../screens/routes.dart';
import '../screens/twitch/twitch_archive_screens.dart';
import '../screens/twitch/twitch_downloads_screen.dart';
import '../screens/twitch/twitch_play.dart';
import '../services/app_services.dart';
import '../sync/device_sync.dart';
import '../twitch/twitch_downloads.dart';
import '../twitch/twitch_service.dart';
import '../widgets/format.dart';
import 'in_app_notices.dart';

/// How often the Twitch download and recording queues are read.
const twitchQueuePoll = Duration(seconds: 30);

/// How many of the newest movies and episodes are compared.
const _recentWindow = 40;

/// Turns the events of [services] into notices of [AppServices.notices].
class NoticeFeeds {
  NoticeFeeds(this.services);

  final AppServices services;

  InAppNotices get notices => services.notices;

  bool _started = false;
  bool _disposed = false;
  final List<StreamSubscription<Object?>> _subs = [];
  final List<VoidCallback> _unlisten = [];
  Timer? _poll;
  int? _profileId;

  /// Follows the events once [services] has loaded.
  void start() {
    if (_started || _disposed) return;
    if (!services.loaded) {
      services.addListener(_waitLoaded);
      return;
    }
    _started = true;
    _profileId = services.profileControl.active?.id;
    _listen(services.profileControl, _onProfile);
    _followLibrary();
    _subs.add(services.sync.events.listen(_onSync, onError: (_) {}));
    if (services.offline case final off?) _listen(off, _onOffline, seed: true);
    _listen(services.liveNotifications, _onLive, seed: true);
    if (services.twitchDownloads != null || services.twitchArchive != null) {
      unawaited(_pollTwitch());
      _poll = Timer.periodic(twitchQueuePoll, (_) => unawaited(_pollTwitch()));
    }
  }

  void _waitLoaded() {
    if (!services.loaded) return;
    services.removeListener(_waitLoaded);
    start();
  }

  void dispose() {
    _disposed = true;
    services.removeListener(_waitLoaded);
    _poll?.cancel();
    for (final s in _subs) {
      unawaited(s.cancel());
    }
    for (final u in _unlisten) {
      u();
    }
  }

  void _listen(Listenable l, VoidCallback f, {bool seed = false}) {
    l.addListener(f);
    _unlisten.add(() => l.removeListener(f));
    if (seed) f();
  }

  bool get _restricted => services.profileControl.restricted;

  void _post(InAppNotice n) {
    if (_disposed || services.onboardingPending) return;
    notices.post(n);
  }

  // -------------------------------------------------------------- profile

  void _onProfile() {
    final id = services.profileControl.active?.id;
    if (id == _profileId) return;
    _profileId = id;
    notices.clear();
    // What the new profile sees is its starting point.
    _seen.clear();
    _since = null;
  }

  // -------------------------------------------------------------- library

  final Map<ItemKind, Set<String>> _seen = {};
  DateTime? _since;
  String? _batchId;
  final List<LibraryItem> _batch = [];
  int _batchSeq = 0;
  final Map<String, LibraryItem?> _shows = {};

  void _followLibrary() {
    for (final kind in const [ItemKind.movie, ItemKind.episode]) {
      _subs.add(
        services.library
            .recentlyAdded(kind: kind, limit: _recentWindow)
            .listen((items) => _onRecent(kind, items), onError: (_) {}),
      );
    }
  }

  void _onRecent(ItemKind kind, List<LibraryItem> items) {
    final seen = _seen[kind];
    final fresh = seen == null
        ? const <LibraryItem>[]
        : [
            for (final i in items)
              if (!seen.contains(i.id) && _isNew(i)) i,
          ];
    (_seen[kind] ??= {}).addAll(items.map((i) => i.id));
    for (final i in items) {
      final at = i.addedAt;
      if (at != null && (_since == null || at.isAfter(_since!))) _since = at;
    }
    if (fresh.isEmpty || _restricted) return;
    if (_batchId == null || !notices.contains(_batchId!)) {
      _batchId = 'library-${++_batchSeq}';
      _batch.clear();
    }
    _batch.addAll(fresh);
    unawaited(_postBatch(_batchId!, List.of(_batch)));
  }

  /// Not in the window before, and added after the newest item then (an
  /// older item can slide into the window when a newer one is removed).
  bool _isNew(LibraryItem i) {
    final at = i.addedAt;
    final since = _since;
    return at == null || since == null || at.isAfter(since);
  }

  Future<LibraryItem?> _show(String? id) async {
    if (id == null) return null;
    if (_shows.containsKey(id)) return _shows[id];
    LibraryItem? show;
    try {
      show = await services.library.item(id);
    } catch (_) {}
    return _shows[id] = show;
  }

  int _postSeq = 0;

  Future<void> _postBatch(String id, List<LibraryItem> batch) async {
    final seq = ++_postSeq;
    final movies = [
      for (final i in batch)
        if (i.kind == ItemKind.movie) i,
    ];
    final episodes = [
      for (final i in batch)
        if (i.kind == ItemKind.episode) i,
    ];
    final showIds = {for (final e in episodes) e.showId};
    final shows = <String?, LibraryItem?>{
      for (final s in showIds) s: await _show(s),
    };
    if (_disposed || id != _batchId || seq != _postSeq) return;
    String showName(LibraryItem e) => shows[e.showId]?.title ?? e.title;
    final InAppNotice n;
    if (episodes.isEmpty && movies.length == 1) {
      final m = movies.single;
      n = InAppNotice(
        id: id,
        kind: InAppNoticeKind.library,
        itemId: m.id,
        title: (_) => m.year == null ? m.title : '${m.title} (${m.year})',
        body: (l) => l.noticeNewMovie,
        onAction: (ctx) => openItem(ctx, m),
      );
    } else if (movies.isEmpty && showIds.length == 1) {
      final show = shows[episodes.first.showId];
      final e = episodes.first;
      n = InAppNotice(
        id: id,
        kind: InAppNoticeKind.library,
        itemId: episodes.length == 1 ? e.id : show?.id,
        title: (_) => showName(e),
        body: (l) => episodes.length == 1
            ? l.noticeNewEpisode('${episodeCode(l, e)} ${e.title}')
            : l.noticeNewEpisodes(episodes.length),
        onAction: show == null
            ? null
            : (ctx) => episodes.length == 1
                  ? openItem(ctx, e, show: show)
                  : openItem(ctx, show),
      );
    } else {
      final names = <String>{
        for (final i in batch)
          i.kind == ItemKind.episode ? showName(i) : i.title,
      }.toList();
      n = InAppNotice(
        id: id,
        kind: InAppNoticeKind.library,
        title: (l) => l.noticeNewInLibrary,
        body: (l) => names.length > 3
            ? l.noticeNamesMore(names.take(3).join(', '), names.length - 3)
            : names.join(', '),
      );
    }
    _post(n);
  }

  // -------------------------------------------------------------- handoff

  /// Another device of the profile started a title (`pb.takeover`); the
  /// title playing here has its own toast in the player (SYNC-003).
  void _onSync(SyncEvent e) {
    final itemId = e.itemId;
    final device = e.deviceId;
    if (e.kind != SyncEventKind.takeover || itemId == null || device == null) {
      return;
    }
    if (itemId == services.handoff.playingItemId) return;
    unawaited(_postHandoff(device, e.name, itemId));
  }

  Future<void> _postHandoff(
    String deviceId,
    String? name,
    String itemId,
  ) async {
    LibraryItem? item;
    try {
      item = await services.library.item(itemId);
    } catch (_) {}
    if (item == null || _disposed) return;
    final it = item;
    String? device = name;
    if (device == null) {
      for (final d in services.sync.devices()) {
        if (d.id == deviceId) device = d.name;
      }
    }
    _post(
      InAppNotice(
        id: 'handoff-$deviceId-$itemId',
        kind: InAppNoticeKind.handoff,
        itemId: it.id,
        title: (l) => it.kind == ItemKind.episode
            ? '${episodeCode(l, it)} ${it.title}'
            : it.title,
        body: (l) =>
            l.noticeHandoffBody(device ?? l.devicesHandoffAnotherDevice),
        actionLabel: (l) => l.noticeResumeHere,
        onAction: (ctx) => playVideo(ctx, it),
      ),
    );
  }

  // ------------------------------------------------------------ downloads

  final Map<String, OfflineState> _offline = {};
  bool _offlineSeeded = false;

  void _onOffline() {
    final off = services.offline;
    if (off == null) return;
    for (final d in off.downloads) {
      final key = d.jobId != null
          ? 'j${d.jobId}'
          : d.copyId != null
          ? 'c${d.copyId}'
          : 'i${d.itemId}';
      final prev = _offline[key];
      _offline[key] = d.state;
      final finished =
          d.state == OfflineState.done && (prev == null || prev.isActive);
      if (_offlineSeeded && finished && prev != OfflineState.done) {
        final itemId = d.itemId;
        final title = d.title;
        _post(
          InAppNotice(
            id: 'download-$itemId',
            kind: InAppNoticeKind.download,
            itemId: itemId,
            title: (_) => title,
            body: (l) => l.noticeDownloadOffline,
            actionLabel: (l) => l.noticePlay,
            onAction: (ctx) async {
              final item = await services.library.item(itemId);
              if (item != null && ctx.mounted) await playVideo(ctx, item);
            },
          ),
        );
      }
    }
    _offlineSeeded = true;
  }

  final Map<String, TwitchDownloadState> _twitch = {};
  bool _twitchSeeded = false;

  bool _finished(TwitchDownloadState? prev, TwitchDownloadState now) =>
      now == TwitchDownloadState.done &&
      prev != TwitchDownloadState.done &&
      (prev == null || prev.isActive || prev == TwitchDownloadState.paused);

  Future<void> _pollTwitch() async {
    final hidden = services.profileControl.twitchHidden;
    final downloads = services.twitchDownloads;
    if (downloads != null) {
      try {
        for (final d in await downloads.list()) {
          final key = 'v${d.jobId}';
          final prev = _twitch[key];
          _twitch[key] = d.state;
          if (_twitchSeeded && !hidden && _finished(prev, d.state)) {
            final name = d.partLabel ?? d.file;
            _post(
              InAppNotice(
                id: 'twitch-$key',
                kind: InAppNoticeKind.download,
                title: (l) => name ?? l.noticeTwitchVideo,
                body: (l) => l.noticeDownloadTwitch,
                onAction: (ctx) => Navigator.of(ctx).push(
                  MaterialPageRoute<void>(
                    builder: (_) => const TwitchDownloadsScreen(),
                  ),
                ),
              ),
            );
          }
        }
      } catch (_) {}
    }
    final archive = services.twitchArchive;
    if (archive != null) {
      try {
        for (final r in await archive.recordings()) {
          final key = 'r${r.jobId}';
          final prev = _twitch[key];
          _twitch[key] = r.state;
          // Recordings are never shown where the archive is hidden.
          if (_twitchSeeded &&
              services.visibleArchive != null &&
              _finished(prev, r.state)) {
            final login = r.login ?? r.userId;
            _post(
              InAppNotice(
                id: 'twitch-$key',
                kind: InAppNoticeKind.download,
                title: (_) => login,
                body: (l) => l.noticeRecordingSaved,
                onAction: openTwitchArchive,
              ),
            );
          }
        }
      } catch (_) {}
    }
    _twitchSeeded = true;
  }

  // ----------------------------------------------------------------- live

  int _liveCount = 0;
  bool _liveSeeded = false;

  void _onLive() {
    final shown = services.liveNotifications.shown;
    if (_liveSeeded && shown.length > _liveCount) {
      for (final a in shown.skip(_liveCount)) {
        final extra = [?a.title, ?a.gameName].where((s) => s.isNotEmpty);
        _post(
          InAppNotice(
            id: 'live-${a.userId}-${a.streamId}',
            kind: InAppNoticeKind.live,
            title: (l) => l.twitchLiveNotificationTitle(a.displayName),
            body: extra.isEmpty ? null : (_) => extra.join(' · '),
            actionLabel: (l) => l.noticeWatch,
            onAction: (ctx) => playTwitch(ctx, TwitchPlayTarget.live(a.login)),
          ),
        );
      }
    }
    _liveCount = shown.length;
    _liveSeeded = true;
  }
}
