// SPDX-License-Identifier: MPL-2.0
//
// LIB-028 (ADR-NNNN): «Закинуть файл — и он сразу виден на всех
// устройствах». A file dropped into the library (LIB-027) is recognized
// before it is copied and shows up at once in the library of every device
// of the family as a grey card with a live bar: «Копируется с <устройства>
// · 42 % · 1,2 ГБ из 2,9 ГБ · 85 МБ/с · ещё 20 с». It cannot be opened
// until it is copied; the device that copies (or an administrator) can
// cancel it; when the copy is checked and renamed the card turns into the
// item.
//
// The durable steps (started, recognized, done, cancelled) go through the
// library and sync as `upload.event` ops; the bar moves on ephemeral `up`
// frames of the core (10 Hz while someone watches, else 1 Hz). Each card
// has its own notifier, so a tick repaints one card, never the list.

import 'dart:async';
import 'dart:convert';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:pa_core/pa_core.dart' as core;
import 'package:pa_library/pa_library.dart' show RustLibrary;

import '../files/files_controller.dart';
import '../integration/library.dart' show ItemKind, Library, LibraryItem;
import '../sync/device_sync.dart';

/// What a card's file is, and so on which library page it shows.
enum LiveUploadKind { movie, series, music, disc }

/// The state of a copy as a card shows it.
enum LiveUploadState {
  waiting,
  copying,

  /// Copied; checked and renamed from `.part`.
  verifying,

  /// The device that copies cannot reach the source; resumes by itself.
  paused,
  done,
  cancelled,
  failed,

  /// The device that copies went quiet: «Пауза: <устройство> не в сети».
  offline,
}

/// The durable steps of an upload.
enum LiveUploadPhase { started, recognized, done, cancelled }

/// Progress of one card: what its bar and status line show.
@immutable
class LiveUploadProgress {
  const LiveUploadProgress({
    this.state = LiveUploadState.waiting,
    this.done = 0,
    this.size = 0,
    this.rate = 0,
  });

  final LiveUploadState state;
  final int done;
  final int size;

  /// Bytes per second (averaged by the device that copies).
  final int rate;

  double? get fraction => size <= 0 ? null : (done / size).clamp(0.0, 1.0);

  /// Whole percent, never 100 before the copy is done.
  int get percent {
    final f = fraction;
    if (f == null) return 0;
    final p = (f * 100).floor();
    return state == LiveUploadState.done ? 100 : p.clamp(0, 99);
  }

  /// Time left at the current speed.
  Duration? get eta => rate > 0 && state == LiveUploadState.copying
      ? Duration(seconds: ((size - done) / rate).ceil())
      : null;

  bool get active =>
      state != LiveUploadState.done &&
      state != LiveUploadState.cancelled &&
      state != LiveUploadState.failed;

  @override
  bool operator ==(Object other) =>
      other is LiveUploadProgress &&
      other.state == state &&
      other.done == done &&
      other.size == size &&
      other.rate == rate;

  @override
  int get hashCode => Object.hash(state, done, size, rate);
}

/// What the library will make of the file (the guess before copying), or
/// what the user picked with «Не то».
@immutable
class LiveUploadGuess {
  const LiveUploadGuess({
    this.title = '',
    this.year,
    this.season,
    this.episode,
    this.posterUrl,
    this.tmdbId,
    this.tmdbKind,
    this.picked = false,
  });

  final String title;
  final int? year;
  final int? season;
  final int? episode;
  final String? posterUrl;
  final String? tmdbId;

  /// TMDB `movie` / `tv`.
  final String? tmdbKind;

  /// The user chose it with «Не то».
  final bool picked;

  bool get isEmpty => title.isEmpty;

  LiveUploadGuess copyWith({
    String? posterUrl,
    String? tmdbId,
    String? tmdbKind,
    String? title,
    int? year,
    bool? picked,
  }) => LiveUploadGuess(
    title: title ?? this.title,
    year: year ?? this.year,
    season: season,
    episode: episode,
    posterUrl: posterUrl ?? this.posterUrl,
    tmdbId: tmdbId ?? this.tmdbId,
    tmdbKind: tmdbKind ?? this.tmdbKind,
    picked: picked ?? this.picked,
  );

  Map<String, Object?> toJson() => {
    'title': title,
    'year': ?year,
    'season': ?season,
    'episode': ?episode,
    'poster': ?posterUrl,
    'tmdb': ?tmdbId,
    'tmdb_kind': ?tmdbKind,
    if (picked) 'picked': true,
  };

  static LiveUploadGuess fromJson(Object? j) {
    if (j is! Map) return const LiveUploadGuess();
    int? n(Object? v) => v is int ? v : null;
    String? s(Object? v) => v is String && v.isNotEmpty ? v : null;
    return LiveUploadGuess(
      title: s(j['title']) ?? '',
      year: n(j['year']),
      season: n(j['season']),
      episode: n(j['episode']),
      posterUrl: s(j['poster']),
      tmdbId: s(j['tmdb']),
      tmdbKind: s(j['tmdb_kind']),
      picked: j['picked'] == true,
    );
  }
}

/// The durable part of a card (the `info` of its `upload.event` ops).
@immutable
class LiveUploadInfo {
  const LiveUploadInfo({
    required this.name,
    required this.kind,
    this.size = 0,
    this.dir = '',
    this.deviceName = '',
    this.guess = const LiveUploadGuess(),
  });

  /// The file (or folder) name.
  final String name;
  final LiveUploadKind kind;
  final int size;

  /// Where it goes in the source.
  final String dir;

  /// The device that copies, as the family knows it.
  final String deviceName;
  final LiveUploadGuess guess;

  LiveUploadInfo withGuess(LiveUploadGuess g) => LiveUploadInfo(
    name: name,
    kind: kind,
    size: size,
    dir: dir,
    deviceName: deviceName,
    guess: g,
  );

  String toJson() => jsonEncode({
    'v': 1,
    'name': name,
    'kind': kind.name,
    'size': size,
    'dir': dir,
    'device_name': deviceName,
    'guess': guess.toJson(),
  });

  static LiveUploadInfo? fromJson(String text) {
    try {
      final j = jsonDecode(text);
      if (j is! Map || j['name'] is! String) return null;
      return LiveUploadInfo(
        name: j['name'] as String,
        kind: LiveUploadKind.values.firstWhere(
          (k) => k.name == j['kind'],
          orElse: () => LiveUploadKind.movie,
        ),
        size: j['size'] is int ? j['size'] as int : 0,
        dir: j['dir'] is String ? j['dir'] as String : '',
        deviceName: j['device_name'] is String
            ? j['device_name'] as String
            : '',
        guess: LiveUploadGuess.fromJson(j['guess']),
      );
    } catch (_) {
      return null;
    }
  }
}

/// One card: a file on its way into the library on this or another device.
class LiveUpload {
  LiveUpload({
    required this.id,
    required this.deviceId,
    required this.local,
    required LiveUploadInfo info,
    this.phase = LiveUploadPhase.started,
    LiveUploadProgress progress = const LiveUploadProgress(),
  }) : info = ValueNotifier(info),
       progress = ValueNotifier(progress);

  final String id;

  /// The device that copies.
  final String deviceId;

  /// This device copies it.
  final bool local;

  /// Name, kind, guess; changes with «Не то» and recognition.
  final ValueNotifier<LiveUploadInfo> info;

  /// The bar: changes up to 10 times a second, repaints this card only.
  final ValueNotifier<LiveUploadProgress> progress;
  LiveUploadPhase phase;

  /// The item it became, once the library has it.
  String? itemId;

  /// When the copy was done.
  DateTime? doneAt;

  /// Jobs of this device's queue behind this card (a folder: many files).
  final List<String> jobIds = [];

  void dispose() {
    info.dispose();
    progress.dispose();
  }
}

/// A step stored in the library.
@immutable
class LiveUploadRecord {
  const LiveUploadRecord({
    required this.id,
    required this.phase,
    required this.info,
    required this.deviceId,
  });

  final String id;
  final LiveUploadPhase phase;
  final String info;
  final String deviceId;
}

/// Live progress of another device's upload.
@immutable
class LiveUploadTick {
  const LiveUploadTick({
    required this.id,
    required this.deviceId,
    required this.progress,
  });

  final String id;
  final String deviceId;
  final LiveUploadProgress progress;
}

/// A provider result for «Не то».
@immutable
class LiveUploadCandidate {
  const LiveUploadCandidate({
    required this.title,
    this.year,
    this.posterUrl,
    this.tmdbId,
    this.tmdbKind,
    this.overview,
  });

  final String title;
  final int? year;
  final String? posterUrl;
  final String? tmdbId;
  final String? tmdbKind;
  final String? overview;
}

/// What a [LiveUploadLink] tells.
enum LiveUploadSignalKind {
  /// The stored steps changed (another device's op arrived).
  records,

  /// Live progress of another device changed.
  ticks,

  /// Another device asks this one to cancel [LiveUploadSignal.ids].
  cancel,
}

@immutable
class LiveUploadSignal {
  const LiveUploadSignal(this.kind, {this.ids = const [], this.deviceId});
  final LiveUploadSignalKind kind;
  final List<String> ids;
  final String? deviceId;
}

/// Where cards come from and go to: the library (durable steps), the sync
/// service (live progress) and the recognizer.
abstract interface class LiveUploadLink {
  /// This device's id in the family (`local` without sync).
  String get deviceId;

  Future<List<LiveUploadRecord>> records();
  Future<void> record(String id, LiveUploadPhase phase, String info);

  /// Other devices' live progress.
  List<LiveUploadTick> ticks();

  /// This device's progress (paced by the core).
  void report(String id, LiveUploadState state, int done, int size);

  /// This app shows cards (the copying devices go to 10 Hz).
  void watch(bool on);
  Future<void> cancel(String deviceId, List<String> ids);
  Stream<LiveUploadSignal> get signals;

  /// The guess before copying for [dest] (a folder ends with `/`).
  LiveUploadGuess guess(String dest);

  /// Provider results by title for «Не то» and the poster of a guess.
  Future<List<LiveUploadCandidate>> search(
    LiveUploadKind kind,
    String title,
    int? year,
  );
}

/// [LiveUploadLink] on the Rust core.
class CoreLiveUploadLink implements LiveUploadLink {
  CoreLiveUploadLink(this.library, this.sync) {
    _sub = sync?.events.listen((e) {
      switch (e.kind) {
        case SyncEventKind.libraryChanged:
          _signals.add(const LiveUploadSignal(LiveUploadSignalKind.records));
        case SyncEventKind.uploads:
          _signals.add(const LiveUploadSignal(LiveUploadSignalKind.ticks));
        case SyncEventKind.uploadCancel:
          final ids = <String>[];
          try {
            final list = jsonDecode(e.payload ?? '[]');
            if (list is List) ids.addAll(list.whereType<String>());
          } catch (_) {}
          _signals.add(
            LiveUploadSignal(
              LiveUploadSignalKind.cancel,
              ids: ids,
              deviceId: e.deviceId,
            ),
          );
        default:
          break;
      }
    });
  }

  final RustLibrary library;
  final CoreDeviceSync? sync;
  final _signals = StreamController<LiveUploadSignal>.broadcast();
  StreamSubscription<SyncEvent>? _sub;

  core.CoreSync? get _core => sync?.core;

  @override
  String get deviceId {
    try {
      return _core?.deviceId() ?? 'local';
    } catch (_) {
      return 'local';
    }
  }

  static LiveUploadPhase _phase(core.CoreUploadPhase p) => switch (p) {
    core.CoreUploadPhase.started => LiveUploadPhase.started,
    core.CoreUploadPhase.recognized => LiveUploadPhase.recognized,
    core.CoreUploadPhase.done => LiveUploadPhase.done,
    core.CoreUploadPhase.cancelled => LiveUploadPhase.cancelled,
  };

  static LiveUploadState _state(core.CoreUploadState s) => switch (s) {
    core.CoreUploadState.waiting => LiveUploadState.waiting,
    core.CoreUploadState.copying => LiveUploadState.copying,
    core.CoreUploadState.verifying => LiveUploadState.verifying,
    core.CoreUploadState.paused => LiveUploadState.paused,
    core.CoreUploadState.done => LiveUploadState.done,
    core.CoreUploadState.cancelled => LiveUploadState.cancelled,
    core.CoreUploadState.failed => LiveUploadState.failed,
    core.CoreUploadState.offline => LiveUploadState.offline,
  };

  @override
  Future<List<LiveUploadRecord>> records() async => [
    for (final r in await library.core.liveUploads())
      LiveUploadRecord(
        id: r.uploadId,
        phase: _phase(r.phase),
        info: r.info,
        deviceId: r.deviceId,
      ),
  ];

  @override
  Future<void> record(String id, LiveUploadPhase phase, String info) =>
      library.core.recordUploadEvent(
        uploadId: id,
        phase: switch (phase) {
          LiveUploadPhase.started => core.CoreUploadPhase.started,
          LiveUploadPhase.recognized => core.CoreUploadPhase.recognized,
          LiveUploadPhase.done => core.CoreUploadPhase.done,
          LiveUploadPhase.cancelled => core.CoreUploadPhase.cancelled,
        },
        info: info,
      );

  @override
  List<LiveUploadTick> ticks() {
    final c = _core;
    if (c == null) return const [];
    return [
      for (final t in c.uploads())
        LiveUploadTick(
          id: t.uploadId,
          deviceId: t.deviceId,
          progress: LiveUploadProgress(
            state: _state(t.state),
            done: t.done,
            size: t.size,
            rate: t.rate,
          ),
        ),
    ];
  }

  @override
  void report(String id, LiveUploadState state, int done, int size) =>
      _core?.uploadReport(
        uploadId: id,
        state: switch (state) {
          LiveUploadState.waiting => core.CoreUploadState.waiting,
          LiveUploadState.copying => core.CoreUploadState.copying,
          LiveUploadState.verifying => core.CoreUploadState.verifying,
          LiveUploadState.paused => core.CoreUploadState.paused,
          LiveUploadState.done => core.CoreUploadState.done,
          LiveUploadState.cancelled => core.CoreUploadState.cancelled,
          LiveUploadState.failed => core.CoreUploadState.failed,
          LiveUploadState.offline => core.CoreUploadState.paused,
        },
        done: done,
        size: size,
      );

  @override
  void watch(bool on) => _core?.uploadWatch(on_: on);

  @override
  Future<void> cancel(String deviceId, List<String> ids) async =>
      _core?.uploadCancel(to: deviceId, ids: ids);

  @override
  Stream<LiveUploadSignal> get signals => _signals.stream;

  @override
  LiveUploadGuess guess(String dest) {
    final g = core.guessUpload(dest: dest);
    return LiveUploadGuess(
      title: g.title,
      year: g.year,
      season: g.season,
      episode: g.episode,
    );
  }

  @override
  Future<List<LiveUploadCandidate>> search(
    LiveUploadKind kind,
    String title,
    int? year,
  ) async {
    final found = await library.core.uploadSearch(
      settings: library.metadataSettings(),
      kind: kind == LiveUploadKind.series ? 'show' : 'movie',
      title: title,
      year: year,
    );
    return [
      for (final c in found)
        LiveUploadCandidate(
          title: c.title,
          year: c.year,
          posterUrl: c.posterUrl,
          tmdbId: c.provider == 'tmdb' ? c.extId : null,
          tmdbKind: c.provider == 'tmdb' ? c.extKind : null,
          overview: c.overview,
        ),
    ];
  }

  void dispose() {
    unawaited(_sub?.cancel());
    unawaited(_signals.close());
  }
}

/// The cards of the family's uploads: this device's from its queue
/// ([FilesController]), the others' from the library and the sync service.
class LiveUploads extends ChangeNotifier {
  LiveUploads({
    required this.link,
    this.files,
    this.library,
    String Function()? deviceName,
    DateTime Function()? clock,
  }) : _deviceName = deviceName ?? (() => ''),
       clock = clock ?? DateTime.now;

  final LiveUploadLink link;
  final FilesController? files;

  /// The library, to find the item a finished card became.
  final Library? library;
  final String Function() _deviceName;
  final DateTime Function() clock;

  final _uploads = <String, LiveUpload>{};
  final _byJob = <String, LiveUpload>{};
  StreamSubscription<LiveUploadSignal>? _sub;
  var _watchers = 0;
  var _started = false;

  /// A finished card of another device waits this long at most for its
  /// item to show up in this library (its source is scanned on its own).
  static const doneLinger = Duration(minutes: 10);

  /// Every card shown now, newest first.
  List<LiveUpload> get uploads {
    final now = clock();
    return [
      for (final u in _uploads.values.toList().reversed)
        if (u.phase != LiveUploadPhase.cancelled &&
            (u.doneAt == null || now.difference(u.doneAt!) < doneLinger))
          u,
    ];
  }

  /// The cards of one library page: movies (and discs) or shows.
  List<LiveUpload> forPage({required bool shows}) => [
    for (final u in uploads)
      if (shows
          ? u.info.value.kind == LiveUploadKind.series
          : u.info.value.kind == LiveUploadKind.movie ||
                u.info.value.kind == LiveUploadKind.disc)
        u,
  ];

  LiveUpload? byId(String id) => _uploads[id];

  /// Starts listening: the stored steps first, then signals and the
  /// queue's callbacks.
  Future<void> start() async {
    if (_started) return;
    _started = true;
    _sub = link.signals.listen(_onSignal);
    files?.onJobChanged = _onJob;
    files?.onJobCancelled = _onJobCancelled;
    await _reload();
    _onTicks();
  }

  // --- this device ---------------------------------------------------------

  /// A file or folder of [jobs] goes into the library (LIB-027 planned
  /// it): the card appears here and on every device, recognized before a
  /// byte is copied. [dest] is where it goes in the source (a folder ends
  /// with `/`). Returns the card.
  Future<LiveUpload> began({
    required List<UploadJob> jobs,
    required String name,
    required LiveUploadKind kind,
    required String dest,
  }) async {
    final first = jobs.first;
    final id = _idFor(first.id);
    var guess = const LiveUploadGuess();
    try {
      guess = link.guess(dest);
    } catch (_) {}
    final slash = dest.endsWith('/') ? dest.length - 1 : dest.lastIndexOf('/');
    final info = LiveUploadInfo(
      name: name,
      kind: kind,
      size: jobs.fold(0, (s, j) => s + j.size),
      dir: slash <= 0 ? '' : dest.substring(0, slash),
      deviceName: _deviceName(),
      guess: guess,
    );
    final upload = LiveUpload(
      id: id,
      deviceId: link.deviceId,
      local: true,
      info: info,
      progress: LiveUploadProgress(size: info.size),
    );
    for (final j in jobs) {
      upload.jobIds.add(j.id);
      _byJob[j.id] = upload;
    }
    _uploads[id] = upload;
    notifyListeners();
    await _record(upload, LiveUploadPhase.started);
    _report(upload);
    // The queue may have moved on while the card was made.
    _onJob(first);
    if (!guess.isEmpty) unawaited(_findPoster(upload));
    return upload;
  }

  /// The card's id: the first job's id and this device, unique in the
  /// family.
  String _idFor(String jobId) {
    final dev = link.deviceId.replaceAll(RegExp(r'[^A-Za-z0-9]'), '');
    final tag = dev.length > 8 ? dev.substring(dev.length - 8) : dev;
    final raw = '$jobId-$tag'.replaceAll(RegExp(r'[^A-Za-z0-9_-]'), '');
    return raw.length > 40 ? raw.substring(raw.length - 40) : raw;
  }

  Future<void> _record(LiveUpload u, LiveUploadPhase phase) async {
    u.phase = phase;
    try {
      await link.record(u.id, phase, u.info.value.toJson());
    } catch (e) {
      debugPrint('Live upload step not stored: $e');
    }
  }

  /// The poster of the guess (one provider search, network).
  Future<void> _findPoster(LiveUpload u) async {
    final g = u.info.value.guess;
    try {
      final found = await link.search(u.info.value.kind, g.title, g.year);
      final best = found.firstOrNull;
      if (best == null || u.info.value.guess.picked) return;
      u.info.value = u.info.value.withGuess(
        g.copyWith(
          posterUrl: best.posterUrl,
          tmdbId: best.tmdbId,
          tmdbKind: best.tmdbKind,
        ),
      );
      await _record(u, LiveUploadPhase.recognized);
    } catch (_) {
      // No provider (no key, offline): the card keeps the name's guess.
    }
  }

  /// «Не то»: the user picked what it is; the card shows it now and the
  /// item takes it once it is in the library.
  Future<void> correct(LiveUpload u, LiveUploadCandidate c) async {
    final g = u.info.value.guess;
    u.info.value = u.info.value.withGuess(
      LiveUploadGuess(
        title: c.title,
        year: c.year,
        season: g.season,
        episode: g.episode,
        posterUrl: c.posterUrl,
        tmdbId: c.tmdbId,
        tmdbKind: c.tmdbKind,
        picked: true,
      ),
    );
    await _record(u, LiveUploadPhase.recognized);
    if (u.phase == LiveUploadPhase.done) await _applyPick(u);
  }

  LiveUploadProgress _sum(LiveUpload u) {
    final jobs = [
      for (final j in files?.uploads ?? const <UploadJob>[])
        if (u.jobIds.contains(j.id)) j,
    ];
    if (jobs.isEmpty) return u.progress.value;
    final done = jobs.fold(0, (s, j) => s + j.done);
    final size = jobs.fold(0, (s, j) => s + j.size);
    LiveUploadState state;
    if (jobs.every((j) => j.state == UploadState.done)) {
      state = LiveUploadState.done;
    } else if (jobs.any(
      (j) => j.state == UploadState.failed && j.retryAt != null,
    )) {
      state = LiveUploadState.paused;
    } else if (jobs.any((j) => j.state == UploadState.failed)) {
      state = LiveUploadState.failed;
    } else if (jobs.any((j) => j.state == UploadState.running)) {
      state = done >= size && size > 0
          ? LiveUploadState.verifying
          : LiveUploadState.copying;
    } else {
      state = LiveUploadState.waiting;
    }
    return LiveUploadProgress(state: state, done: done, size: size);
  }

  void _report(LiveUpload u) {
    final p = u.progress.value;
    link.report(u.id, p.state, p.done, p.size);
  }

  // Speed of this device's cards: the same moving average as the core's.
  final _speed = <String, (DateTime, int, double)>{};

  int _rate(LiveUpload u, LiveUploadProgress p) {
    final now = clock();
    final last = _speed[u.id];
    if (p.state != LiveUploadState.copying) {
      _speed.remove(u.id);
      return 0;
    }
    if (last == null || p.done < last.$2) {
      _speed[u.id] = (now, p.done, 0);
      return 0;
    }
    final dt = now.difference(last.$1).inMilliseconds;
    if (dt < 100) return last.$3.round();
    final inst = (p.done - last.$2) * 1000 / dt;
    final a = 1 - math.exp(-dt / 2000);
    final rate = last.$3 == 0 ? inst : last.$3 + a * (inst - last.$3);
    _speed[u.id] = (now, p.done, rate);
    return rate.round();
  }

  void _onJob(UploadJob job) {
    final u = _byJob[job.id];
    if (u == null) return;
    final sum = _sum(u);
    final p = LiveUploadProgress(
      state: sum.state,
      done: sum.done,
      size: sum.size,
      rate: _rate(u, sum),
    );
    final was = u.progress.value.state;
    u.progress.value = p;
    _report(u);
    if (p.state == LiveUploadState.done && was != LiveUploadState.done) {
      unawaited(_finished(u));
    }
  }

  /// This device's copy is in the library (LIB-027 refreshed the folder
  /// before the job turned done): the card gives way to the item now.
  Future<void> _finished(LiveUpload u) async {
    u.doneAt = clock();
    await _record(u, LiveUploadPhase.done);
    await _applyPick(u);
    _drop(u.id);
  }

  /// Finished cards of other devices whose item is in this library now
  /// go away.
  Future<void> _settleDone() async {
    for (final u in _uploads.values.toList()) {
      if (u.local || u.doneAt == null) continue;
      await _applyPick(u);
      if (u.itemId != null) _drop(u.id);
    }
  }

  /// A pick of «Не то» goes to the item once the library has it.
  Future<void> _applyPick(LiveUpload u) async {
    final lib = library;
    final g = u.info.value.guess;
    if (lib == null) return;
    try {
      final recent = await lib
          .recentlyAdded(
            kind: u.info.value.kind == LiveUploadKind.series
                ? ItemKind.episode
                : ItemKind.movie,
            limit: 30,
          )
          .first;
      final name = u.info.value.name;
      final item = recent
          .where((i) => i.filePath.contains(name))
          .cast<LibraryItem?>()
          .firstOrNull;
      if (item == null) return;
      u.itemId = item.id;
      final tmdb = int.tryParse(g.tmdbId ?? '');
      if (g.picked && u.local) {
        await lib.fixMatch(item.id, tmdbId: tmdb, title: g.title, year: g.year);
      }
    } catch (_) {}
  }

  void _onJobCancelled(UploadJob job) {
    final u = _byJob.remove(job.id);
    if (u == null) return;
    u.jobIds.remove(job.id);
    if (u.jobIds.any((id) => files?.uploads.any((j) => j.id == id) ?? false)) {
      _onJob(job);
      return;
    }
    u.progress.value = LiveUploadProgress(
      state: LiveUploadState.cancelled,
      done: u.progress.value.done,
      size: u.progress.value.size,
    );
    _report(u);
    unawaited(_record(u, LiveUploadPhase.cancelled));
    _drop(u.id);
  }

  /// Cancels a card: this device's copy at once; another device's by
  /// asking it (the device that copies or an administrator may).
  Future<void> cancel(LiveUpload u) async {
    if (u.local) {
      for (final id in List.of(u.jobIds)) {
        final job = files?.uploads.where((j) => j.id == id).firstOrNull;
        if (job != null) await files!.cancelUpload(job);
      }
      return;
    }
    await link.cancel(u.deviceId, [u.id]);
  }

  void _drop(String id) {
    final u = _uploads.remove(id);
    if (u == null) return;
    for (final j in u.jobIds) {
      _byJob.remove(j);
    }
    notifyListeners();
    // Its notifiers are not disposed: a snack bar or a closing page may
    // still show it; nothing feeds them any more.
  }

  // --- the other devices ---------------------------------------------------

  void _onSignal(LiveUploadSignal s) {
    switch (s.kind) {
      case LiveUploadSignalKind.records:
        unawaited(_reload());
      case LiveUploadSignalKind.ticks:
        _onTicks();
      case LiveUploadSignalKind.cancel:
        for (final id in s.ids) {
          final u = _uploads[id];
          if (u != null && u.local) unawaited(cancel(u));
        }
    }
  }

  Future<void> _reload() async {
    List<LiveUploadRecord> records;
    try {
      records = await link.records();
    } catch (_) {
      return;
    }
    final me = link.deviceId;
    var changed = false;
    for (final r in records.reversed) {
      if (r.deviceId == me || r.deviceId == 'local') continue;
      final info = LiveUploadInfo.fromJson(r.info);
      if (info == null) continue;
      final known = _uploads[r.id];
      if (known == null) {
        if (r.phase == LiveUploadPhase.cancelled ||
            r.phase == LiveUploadPhase.done) {
          continue;
        }
        _uploads[r.id] = LiveUpload(
          id: r.id,
          deviceId: r.deviceId,
          local: false,
          info: info,
          phase: r.phase,
          progress: LiveUploadProgress(size: info.size),
        );
        changed = true;
        continue;
      }
      known.info.value = info;
      if (known.phase != r.phase) {
        known.phase = r.phase;
        changed = true;
        if (r.phase == LiveUploadPhase.cancelled) {
          _drop(r.id);
        } else if (r.phase == LiveUploadPhase.done) {
          known.progress.value = LiveUploadProgress(
            state: LiveUploadState.done,
            done: info.size,
            size: info.size,
          );
          known.doneAt = clock();
        }
      }
    }
    if (changed) notifyListeners();
    _onTicks();
    unawaited(_settleDone());
  }

  void _onTicks() {
    List<LiveUploadTick> ticks;
    try {
      ticks = link.ticks();
    } catch (_) {
      return;
    }
    for (final t in ticks) {
      final u = _uploads[t.id];
      if (u == null || u.local) continue;
      if (u.progress.value.state == LiveUploadState.done) continue;
      u.progress.value = t.progress;
      if (t.progress.state == LiveUploadState.cancelled) _drop(t.id);
    }
  }

  // --- who watches ---------------------------------------------------------

  /// A screen with cards is open: the devices that copy go to 10 Hz.
  void addWatcher() {
    if (_watchers++ == 0) link.watch(true);
  }

  void removeWatcher() {
    if (_watchers > 0 && --_watchers == 0) link.watch(false);
  }

  @override
  void dispose() {
    unawaited(_sub?.cancel());
    if (files?.onJobChanged == _onJob) files?.onJobChanged = null;
    for (final u in _uploads.values) {
      u.dispose();
    }
    _uploads.clear();
    super.dispose();
  }
}

/// [LiveUploadLink] in memory: the demo, tests, and builds without the
/// Rust core (cards of this device only, no other devices).
class MemoryLiveUploadLink implements LiveUploadLink {
  MemoryLiveUploadLink({this.deviceId = 'local', this.candidates = const []});

  @override
  final String deviceId;

  /// What [search] finds.
  List<LiveUploadCandidate> candidates;
  final _records = <String, LiveUploadRecord>{};
  final _ticks = <String, LiveUploadTick>{};
  final _signals = StreamController<LiveUploadSignal>.broadcast();

  /// The last reports of this device: id → progress.
  final reports = <String, LiveUploadProgress>{};

  /// Whether a screen with cards is open.
  bool watching = false;

  /// Cancel requests sent: (device, id).
  final cancelled = <(String, String)>[];

  /// Another device's step arrives.
  void remoteRecord(LiveUploadRecord r) {
    _records[r.id] = r;
    _signals.add(const LiveUploadSignal(LiveUploadSignalKind.records));
  }

  /// Another device's tick arrives.
  void remoteTick(LiveUploadTick t) {
    _ticks[t.id] = t;
    _signals.add(const LiveUploadSignal(LiveUploadSignalKind.ticks));
  }

  /// Another device asks to cancel this device's [ids].
  void remoteCancel(List<String> ids, String from) => _signals.add(
    LiveUploadSignal(LiveUploadSignalKind.cancel, ids: ids, deviceId: from),
  );

  @override
  Future<List<LiveUploadRecord>> records() async => _records.values.toList();

  @override
  Future<void> record(String id, LiveUploadPhase phase, String info) async =>
      _records[id] = LiveUploadRecord(
        id: id,
        phase: phase,
        info: info,
        deviceId: deviceId,
      );

  @override
  List<LiveUploadTick> ticks() => _ticks.values.toList();

  @override
  void report(String id, LiveUploadState state, int done, int size) =>
      reports[id] = LiveUploadProgress(state: state, done: done, size: size);

  @override
  void watch(bool on) => watching = on;

  @override
  Future<void> cancel(String deviceId, List<String> ids) async {
    for (final id in ids) {
      cancelled.add((deviceId, id));
    }
  }

  @override
  Stream<LiveUploadSignal> get signals => _signals.stream;

  @override
  LiveUploadGuess guess(String dest) {
    // The demo has no core parser: the name up to the year.
    final name = dest.split('/').where((p) => p.isNotEmpty).last;
    final m = RegExp(r'^(.+?)[ ._(\[]+((?:19|20)\d\d)\b').firstMatch(name);
    if (m == null) return const LiveUploadGuess();
    return LiveUploadGuess(
      title: m.group(1)!.replaceAll(RegExp(r'[._]'), ' ').trim(),
      year: int.parse(m.group(2)!),
    );
  }

  @override
  Future<List<LiveUploadCandidate>> search(
    LiveUploadKind kind,
    String title,
    int? year,
  ) async => [
    for (final c in candidates)
      if (c.title.toLowerCase().contains(title.toLowerCase()) || title.isEmpty)
        c,
  ];
}
