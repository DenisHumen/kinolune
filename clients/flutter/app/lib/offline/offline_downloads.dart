// SPDX-License-Identifier: MPL-2.0
//
// Offline copies (NET-019): "Download for offline" of a movie, an episode,
// a season, an album or a playlist from a network source to the device.
// The Rust core (`CoreOffline`: pa-offline on pa-jobs + pa-storage) copies
// the files with resume; the library then plays the local copy through
// its stream proxy, so items and progress stay the same.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:pa_core/pa_core.dart' as core;

/// State of a download.
enum OfflineState {
  queued,
  waiting,
  running,
  paused,
  retrying,
  failed,
  cancelled,
  done;

  static OfflineState parse(String s) => switch (s) {
    'queued' => queued,
    'waiting' => waiting,
    'running' => running,
    'paused' => paused,
    'retrying' => retrying,
    'failed' => failed,
    'done' => done,
    _ => cancelled,
  };

  /// In the queue: not finished.
  bool get isActive =>
      this == queued ||
      this == waiting ||
      this == running ||
      this == paused ||
      this == retrying;
}

/// What the user can do with a download.
enum OfflineAction { pause, resume, cancel, retry, remove, delete }

/// One download or copy.
@immutable
class OfflineDownload {
  const OfflineDownload({
    required this.itemId,
    required this.title,
    required this.state,
    this.jobId,
    this.copyId,
    this.version,
    this.bytesDone = 0,
    this.bytesTotal,
    this.waitingFor,
    this.error,
    this.watched = false,
    this.createdAt,
  });

  final int? jobId;
  final int? copyId;

  /// Library item id (`m12`, `e40`, `t7`).
  final String itemId;
  final String title;

  /// Version label when the item has several.
  final String? version;
  final OfflineState state;
  final int bytesDone;
  final int? bytesTotal;

  /// `disk_reserve`, `quota`, `network`, `user`…
  final String? waitingFor;
  final String? error;
  final bool watched;
  final DateTime? createdAt;

  /// 0–1 when the size is known.
  double? get progress {
    final total = bytesTotal;
    if (total == null || total <= 0) return null;
    return (bytesDone / total).clamp(0.0, 1.0);
  }

  OfflineDownload withBytes(int done, int? total) => OfflineDownload(
    itemId: itemId,
    title: title,
    state: state,
    jobId: jobId,
    copyId: copyId,
    version: version,
    bytesDone: done,
    bytesTotal: total ?? bytesTotal,
    waitingFor: waitingFor,
    error: error,
    watched: watched,
    createdAt: createdAt,
  );

  OfflineDownload withState(OfflineState s) => OfflineDownload(
    itemId: itemId,
    title: title,
    state: s,
    jobId: jobId,
    copyId: copyId,
    version: version,
    bytesDone: bytesDone,
    bytesTotal: bytesTotal,
    waitingFor: s == state ? waitingFor : null,
    error: error,
    watched: watched,
    createdAt: createdAt,
  );

  List<OfflineAction> get actions => switch (state) {
    OfflineState.queued ||
    OfflineState.waiting ||
    OfflineState.running ||
    OfflineState.retrying => const [OfflineAction.pause, OfflineAction.cancel],
    OfflineState.paused => const [OfflineAction.resume, OfflineAction.cancel],
    OfflineState.failed => const [OfflineAction.retry, OfflineAction.remove],
    OfflineState.cancelled => const [OfflineAction.remove],
    OfflineState.done => const [OfflineAction.delete],
  };
}

/// Default auto-cleanup (UI-018, ADR-0123): watched copies are deleted
/// this many days after watching; favourites stay.
const offlineDefaultCleanupDays = 7;

/// Settings and space of the offline folder.
@immutable
class OfflineSettings {
  const OfflineSettings({
    this.folder,
    this.wifiOnly = true,
    this.deleteWatchedAfterDays,
    this.usedBytes = 0,
    this.availableBytes,
    this.totalBytes,
  });

  final String? folder;
  final bool wifiOnly;

  /// Delete watched copies after N days (0: right away); null: keep.
  final int? deleteWatchedAfterDays;
  final int usedBytes;
  final int? availableBytes;
  final int? totalBytes;
}

/// What to download.
enum OfflineTargetKind { item, season, album, playlist, musicPlaylist }

@immutable
class OfflineTarget {
  const OfflineTarget.item(this.id, {this.versionFileId})
    : kind = OfflineTargetKind.item,
      season = null;
  const OfflineTarget.season(this.id, int this.season)
    : kind = OfflineTargetKind.season,
      versionFileId = null;
  const OfflineTarget.album(this.id)
    : kind = OfflineTargetKind.album,
      season = null,
      versionFileId = null;
  const OfflineTarget.playlist(this.id, {bool music = false})
    : kind = music
          ? OfflineTargetKind.musicPlaylist
          : OfflineTargetKind.playlist,
      season = null,
      versionFileId = null;

  final OfflineTargetKind kind;

  /// Item id, or the album / playlist id.
  final String id;
  final int? season;
  final int? versionFileId;
}

/// Why a download was refused.
class OfflineException implements Exception {
  const OfflineException(this.code, [this.message = '']);

  /// `no_space`, `busy`, `local`, `unplayable`, `no_folder`…
  final String code;
  final String message;

  @override
  String toString() => 'OfflineException($code: $message)';
}

/// The network the device is on, for "Wi-Fi only".
enum OfflineNetwork { unmetered, metered, offline }

/// The core side of offline copies.
abstract interface class OfflineBackend {
  /// Queues the copies; returns how many downloads were added.
  Future<int> download(OfflineTarget target);
  Future<List<OfflineDownload>> downloads();
  Future<void> control(OfflineDownload d, OfflineAction action);

  /// Deletes the copies and downloads of an item.
  Future<void> deleteItem(String itemId);
  Future<OfflineSettings> settings();
  Future<void> setFolder(String folder);
  Future<void> setWifiOnly(bool on);
  Future<void> setDeleteWatched(int? days);

  /// Marks watched copies and applies "delete watched"; returns how many
  /// copies were deleted.
  Future<int> cleanup();
  Future<void> setNetwork(OfflineNetwork network);

  /// Changes of the downloads, from the core's job events.
  Stream<OfflineChange> changes();
}

/// What a job event tells the downloads list (UI-018).
enum OfflineChangeKind {
  /// Bytes of a job moved on: update its row in place.
  progress,

  /// A job changed its state: update its row, then reload for details.
  state,

  /// A job left the queue: reload when it is one of ours.
  removed,

  /// Something else changed (a job was added): reload the list.
  reload,
}

/// One change of the downloads.
@immutable
class OfflineChange {
  const OfflineChange.progress(
    int this.jobId,
    this.bytesDone, [
    this.bytesTotal,
  ]) : kind = OfflineChangeKind.progress,
       state = null;
  const OfflineChange.state(int this.jobId, OfflineState this.state)
    : kind = OfflineChangeKind.state,
      bytesDone = 0,
      bytesTotal = null;
  const OfflineChange.removed(int this.jobId)
    : kind = OfflineChangeKind.removed,
      state = null,
      bytesDone = 0,
      bytesTotal = null;
  const OfflineChange.reload()
    : kind = OfflineChangeKind.reload,
      jobId = null,
      state = null,
      bytesDone = 0,
      bytesTotal = null;

  final OfflineChangeKind kind;
  final int? jobId;
  final OfflineState? state;
  final int bytesDone;
  final int? bytesTotal;
}

/// Job kind of offline copies in the shared pa-jobs queue
/// (`pa_jobs::kinds::OFFLINE_COPY`).
const offlineJobKind = 'offline_copy';

/// The change an event of the core's bus (`core_events`, CORE-009) means
/// for offline downloads; null for events of other domains and of other
/// job kinds (Twitch downloads share the queue). Progress and removal
/// events carry no kind: the controller keeps those of its own jobs only.
OfflineChange? offlineChangeOf(core.CoreEvent e) {
  final job = e.jobChanged;
  if (e.domain != core.CoreEventDomain.jobChanged || job == null) return null;
  final id = job.jobId.toInt();
  return switch (job.change) {
    core.CoreJobChange.progress => OfflineChange.progress(
      id,
      job.bytesDone.toInt(),
      job.bytesTotal?.toInt(),
    ),
    core.CoreJobChange.removed => OfflineChange.removed(id),
    core.CoreJobChange.added when job.kind == offlineJobKind =>
      const OfflineChange.reload(),
    core.CoreJobChange.stateChanged when job.kind == offlineJobKind =>
      OfflineChange.state(id, _jobState(job.state)),
    _ => null,
  };
}

/// A pa-jobs state as the state of a download.
OfflineState _jobState(String? s) => s == 'waiting_resources'
    ? OfflineState.waiting
    : OfflineState.parse(s ?? '');

/// [offlineChangeOf] over a stream of core events.
Stream<OfflineChange> offlineChanges(Stream<core.CoreEvent> events) =>
    events.map(offlineChangeOf).where((c) => c != null).cast<OfflineChange>();

/// [OfflineBackend] over the Rust core.
class CoreOfflineBackend implements OfflineBackend {
  CoreOfflineBackend(this._core);

  final core.CoreOffline _core;

  /// Opens offline copies of [library] with copies in [folder]; null if
  /// the core cannot.
  static Future<CoreOfflineBackend?> open({
    required core.CoreLibrary library,
    required String folder,
  }) async {
    try {
      return CoreOfflineBackend(
        await core.CoreOffline.open(library_: library, folder: folder),
      );
    } on core.CoreOfflineError catch (e) {
      debugPrint('Offline copies unavailable: ${e.code}');
      return null;
    }
  }

  Future<T> _call<T>(Future<T> Function() f) async {
    try {
      return await f();
    } on core.CoreOfflineError catch (e) {
      throw OfflineException(e.code, e.message);
    }
  }

  @override
  Future<int> download(OfflineTarget t) => _call(() async {
    final ids = await _core.download(
      target: core.CoreOfflineTarget(
        kind: switch (t.kind) {
          OfflineTargetKind.item => 'item',
          OfflineTargetKind.season => 'season',
          OfflineTargetKind.album => 'album',
          OfflineTargetKind.playlist => 'playlist',
          OfflineTargetKind.musicPlaylist => 'music_playlist',
        },
        id: t.id,
        season: t.season,
        versionFileId: t.versionFileId,
      ),
    );
    return ids.length;
  });

  @override
  Future<List<OfflineDownload>> downloads() => _call(() async {
    return [
      for (final d in await _core.downloads())
        OfflineDownload(
          jobId: d.jobId,
          copyId: d.copyId,
          itemId: d.itemId,
          title: d.title,
          version: d.version,
          state: OfflineState.parse(d.state),
          bytesDone: d.bytesDone,
          bytesTotal: d.bytesTotal,
          waitingFor: d.waitingFor,
          error: d.error,
          watched: d.watched,
          createdAt: DateTime.fromMillisecondsSinceEpoch(d.createdAtMs),
        ),
    ];
  });

  @override
  Future<void> control(OfflineDownload d, OfflineAction action) =>
      _call(() async {
        if (action == OfflineAction.delete) {
          final copy = d.copyId;
          if (copy != null) await _core.deleteCopy(copyId: copy);
          return;
        }
        final job = d.jobId;
        if (job != null) await _core.control(jobId: job, action: action.name);
      });

  @override
  Future<void> deleteItem(String itemId) =>
      _call(() => _core.deleteItem(itemId: itemId));

  @override
  Future<OfflineSettings> settings() => _call(() async {
    final s = await _core.settings();
    return OfflineSettings(
      folder: s.folder,
      wifiOnly: s.wifiOnly,
      deleteWatchedAfterDays: s.deleteWatchedAfterDays,
      usedBytes: s.usedBytes,
      availableBytes: s.availableBytes,
      totalBytes: s.totalBytes,
    );
  });

  @override
  Future<void> setFolder(String folder) =>
      _call(() => _core.setFolder(folder: folder));

  @override
  Future<void> setWifiOnly(bool on) => _call(() => _core.setWifiOnly(on_: on));

  @override
  Future<void> setDeleteWatched(int? days) =>
      _call(() => _core.setDeleteWatched(days: days));

  @override
  Future<int> cleanup() => _call(() => _core.cleanup());

  @override
  Future<void> setNetwork(OfflineNetwork network) =>
      _call(() => _core.setNetwork(network: network.name));

  /// The app-wide event bus of the core (CORE-009, ADR-0121): one stream
  /// for every screen instead of a stream per call.
  @override
  Stream<OfflineChange> changes() => offlineChanges(core.coreEvents());
}

/// Downloads for the UI: the list, the state of each item, settings.
class OfflineController extends ChangeNotifier {
  OfflineController(this.backend);

  final OfflineBackend backend;

  List<OfflineDownload> _downloads = const [];
  Map<String, OfflineDownload> _byItem = const {};
  OfflineSettings _settings = const OfflineSettings();
  StreamSubscription<OfflineChange>? _sub;
  Timer? _reload;
  bool _disposed = false;

  /// Downloads in the queue and copies on the device, newest first.
  List<OfflineDownload> get downloads => _downloads;
  OfflineSettings get settings => _settings;

  /// The copy (else the newest download) of an item.
  OfflineDownload? stateOf(String itemId) => _byItem[itemId];

  /// Downloads that are not finished.
  int get activeCount => _downloads.where((d) => d.state.isActive).length;

  /// The queue: unfinished, failed and cancelled downloads.
  List<OfflineDownload> get queue => [
    for (final d in _downloads)
      if (d.state != OfflineState.done) d,
  ];

  /// Copies on the device.
  List<OfflineDownload> get copies => [
    for (final d in _downloads)
      if (d.state == OfflineState.done) d,
  ];

  /// Watched copies (what auto-cleanup deletes).
  List<OfflineDownload> get watchedCopies => [
    for (final d in copies)
      if (d.watched) d,
  ];

  /// Bytes of [list].
  static int bytesOf(Iterable<OfflineDownload> list) =>
      list.fold(0, (n, d) => n + d.bytesDone);

  /// 0–1 over the unfinished downloads with a known size; null when none.
  double? get activeProgress {
    var done = 0, total = 0;
    for (final d in _downloads) {
      final t = d.bytesTotal;
      if (!d.state.isActive || t == null || t <= 0) continue;
      done += d.bytesDone.clamp(0, t);
      total += t;
    }
    return total == 0 ? null : done / total;
  }

  /// Some download runs or waits (not paused): «Pause all» applies.
  bool get canPauseAll =>
      _downloads.any((d) => d.state.isActive && d.state != OfflineState.paused);

  /// Some download is paused: «Resume all» applies.
  bool get canResumeAll =>
      _downloads.any((d) => d.state == OfflineState.paused);

  /// Loads the list and follows the core's events.
  Future<void> start() async {
    _sub ??= backend.changes().listen(_onChange, onError: (_) {});
    await refresh();
  }

  void _onChange(OfflineChange e) {
    final i = e.jobId == null
        ? -1
        : _downloads.indexWhere((d) => d.jobId == e.jobId);
    switch (e.kind) {
      case OfflineChangeKind.progress:
        // Progress of another kind of job (Twitch) or of a job the list
        // has not loaded yet: its `added` event reloads the list.
        if (i < 0) return;
        _downloads = List.of(_downloads)
          ..[i] = _downloads[i].withBytes(e.bytesDone, e.bytesTotal);
      case OfflineChangeKind.state:
        if (i >= 0) {
          _downloads = List.of(_downloads)
            ..[i] = _downloads[i].withState(e.state!);
        }
        // Details (the copy of a finished job, why it waits) come with
        // the list.
        _scheduleReload();
        if (i < 0) return;
      case OfflineChangeKind.removed:
        if (i >= 0) _scheduleReload();
        return;
      case OfflineChangeKind.reload:
        _scheduleReload();
        return;
    }
    _index();
    _notify();
  }

  void _scheduleReload() {
    _reload?.cancel();
    _reload = Timer(const Duration(milliseconds: 150), () => refresh());
  }

  void _index() {
    final by = <String, OfflineDownload>{};
    for (final d in _downloads) {
      final seen = by[d.itemId];
      if (seen == null || (d.state == OfflineState.done)) by[d.itemId] = d;
    }
    _byItem = by;
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  Future<void> refresh() async {
    try {
      _downloads = await backend.downloads();
      _settings = await backend.settings();
    } catch (e) {
      debugPrint('Downloads not read: $e');
    }
    _index();
    _notify();
  }

  Future<int> download(OfflineTarget target) async {
    final n = await backend.download(target);
    await refresh();
    return n;
  }

  /// Downloads several items one by one (a season in the display order);
  /// returns how many were queued.
  Future<int> downloadItems(Iterable<String> itemIds) async {
    var n = 0;
    for (final id in itemIds) {
      try {
        n += await backend.download(OfflineTarget.item(id));
      } on OfflineException catch (e) {
        if (e.code != 'local' && e.code != 'unplayable') rethrow;
      }
    }
    await refresh();
    return n;
  }

  Future<void> control(OfflineDownload d, OfflineAction action) async {
    await backend.control(d, action);
    await refresh();
  }

  /// Pauses every running or waiting download.
  Future<void> pauseAll() => _each(
    (d) => d.state.isActive && d.state != OfflineState.paused,
    OfflineAction.pause,
  );

  /// Resumes every paused download.
  Future<void> resumeAll() =>
      _each((d) => d.state == OfflineState.paused, OfflineAction.resume);

  /// Deletes every watched copy now; returns how many.
  Future<int> deleteWatched() => _each(
    (d) => d.state == OfflineState.done && d.watched,
    OfflineAction.delete,
  );

  Future<int> _each(
    bool Function(OfflineDownload) test,
    OfflineAction a,
  ) async {
    var n = 0;
    for (final d in _downloads.where(test).toList()) {
      try {
        await backend.control(d, a);
        n++;
      } catch (e) {
        debugPrint('Download ${d.jobId ?? d.copyId}: ${a.name} failed: $e');
      }
    }
    await refresh();
    return n;
  }

  Future<void> deleteItem(String itemId) async {
    await backend.deleteItem(itemId);
    await refresh();
  }

  Future<void> setFolder(String folder) async {
    await backend.setFolder(folder);
    await refresh();
  }

  Future<void> setWifiOnly(bool on) async {
    await backend.setWifiOnly(on);
    await refresh();
  }

  Future<void> setDeleteWatched(int? days) async {
    await backend.setDeleteWatched(days);
    await backend.cleanup();
    await refresh();
  }

  Future<int> cleanup() async {
    final n = await backend.cleanup();
    if (n > 0) await refresh();
    return n;
  }

  Future<void> setNetwork(OfflineNetwork network) =>
      backend.setNetwork(network);

  @override
  void dispose() {
    _disposed = true;
    _reload?.cancel();
    unawaited(_sub?.cancel());
    super.dispose();
  }
}
