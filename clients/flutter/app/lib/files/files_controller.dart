// SPDX-License-Identifier: MPL-2.0
//
// UI-035 (ADR-0099, ADR-0104): files «как в Nextcloud» — the state behind
// the file view.
//
// * The upload queue: files from the phone (gallery, files, «Поделиться →
//   Kinolune») go up one after another with progress. The queue is saved,
//   so after the app restarts it goes on; the core resumes a broken file
//   from its `.part`.
// * After every write only the touched folders are re-listed
//   ([SourceFiles.refresh]); uploads into «Личное» land in a folder of the
//   personal library, so they show up there unrecognized.
// * «Личное хранилище» ([PersonalStorage]): the first writable source, else
//   a folder of the app on this device; Settings can pin another one.
// * The trash of every writable source is purged of items older than
//   30 days on start, at most once a day per source.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:pa_library/pa_library.dart'
    show
        FileOutcome,
        FilesError,
        FilesException,
        NameConflict,
        PersonalLibrary,
        PersonalMedia,
        SourceFiles,
        TouchedDir;

import '../services/preferences.dart';
import '../sources/source_manager.dart';

/// Preference keys of the file view (kept here, not in the shared list).
abstract final class FilesPrefs {
  /// «Личное хранилище»: `auto`, `device` or `source:<id>`.
  static const storage = 'files_personal_storage';

  /// The id of the app's own folder on this device, once created.
  static const deviceSource = 'files_device_source';

  /// Unfinished uploads (JSON list).
  static const queue = 'files_upload_queue';

  /// Last trash purge per source (JSON map id → Unix ms).
  static const purged = 'files_trash_purged';
}

/// Source kinds whose protocol can write (NET-023).
const writableTypes = {
  SourceType.local,
  SourceType.smb,
  SourceType.webdav,
  SourceType.sftp,
  SourceType.ftp,
};

/// Where uploads to «Личное» go: a folder of a source.
class StoragePlace {
  const StoragePlace(this.sourceId, this.base, {this.onDevice = false});

  final int sourceId;

  /// Folder under which each personal library gets its own, `''` = root.
  final String base;

  /// The app's own folder on this device (no writable source found).
  final bool onDevice;

  @override
  bool operator ==(Object other) =>
      other is StoragePlace &&
      other.sourceId == sourceId &&
      other.base == base &&
      other.onDevice == onDevice;

  @override
  int get hashCode => Object.hash(sourceId, base, onDevice);
}

/// The «Личное хранилище» setting.
sealed class StorageChoice {
  const StorageChoice();

  static StorageChoice parse(String? s) => switch (s) {
    'device' => const DeviceStorage(),
    final v? when v.startsWith('source:') => switch (int.tryParse(
      v.substring(7),
    )) {
      final id? => SourceStorage(id),
      null => const AutoStorage(),
    },
    _ => const AutoStorage(),
  };

  String get value;
}

/// Chosen by itself (the default).
class AutoStorage extends StorageChoice {
  const AutoStorage();
  @override
  String get value => 'auto';
  @override
  bool operator ==(Object other) => other is AutoStorage;
  @override
  int get hashCode => 0;
}

/// The app's folder on this device.
class DeviceStorage extends StorageChoice {
  const DeviceStorage();
  @override
  String get value => 'device';
  @override
  bool operator ==(Object other) => other is DeviceStorage;
  @override
  int get hashCode => 1;
}

/// A source the user picked.
class SourceStorage extends StorageChoice {
  const SourceStorage(this.sourceId);
  final int sourceId;
  @override
  String get value => 'source:$sourceId';
  @override
  bool operator ==(Object other) =>
      other is SourceStorage && other.sourceId == sourceId;
  @override
  int get hashCode => sourceId;
}

/// Folder of the uploads in a writable source.
const storageBase = 'Kinolune';

/// State of an upload.
enum UploadState { waiting, running, failed, done }

/// One file to upload.
class UploadJob {
  UploadJob({
    required this.id,
    required this.localPath,
    required this.sourceId,
    required this.dir,
    required this.name,
    this.conflict = NameConflict.keepBoth,
    this.ownedCopy = false,
    this.size = 0,
    this.started = false,
  });

  final String id;
  final String localPath;
  final int sourceId;

  /// Folder in the source, `''` = root.
  final String dir;
  final String name;
  final NameConflict conflict;

  /// [localPath] is a copy the app made (a share, a picked file on
  /// Android): deleted after the upload.
  final bool ownedCopy;
  int size;

  /// Bytes reached the source once (resume, not a new copy).
  bool started;

  UploadState state = UploadState.waiting;
  int done = 0;
  FilesError? error;
  String? resultPath;

  double? get fraction => size <= 0 ? null : (done / size).clamp(0.0, 1.0);

  Map<String, Object?> toJson() => {
    'id': id,
    'path': localPath,
    'source': sourceId,
    'dir': dir,
    'name': name,
    'conflict': conflict.name,
    'owned': ownedCopy,
    'size': size,
    'started': started,
  };

  static UploadJob? fromJson(Object? j) {
    if (j is! Map) return null;
    final id = j['id'], path = j['path'], source = j['source'];
    final dir = j['dir'], name = j['name'];
    if (id is! String ||
        path is! String ||
        source is! int ||
        dir is! String ||
        name is! String) {
      return null;
    }
    return UploadJob(
      id: id,
      localPath: path,
      sourceId: source,
      dir: dir,
      name: name,
      conflict: NameConflict.values.firstWhere(
        (c) => c.name == j['conflict'],
        orElse: () => NameConflict.keepBoth,
      ),
      ownedCopy: j['owned'] == true,
      size: j['size'] is int ? j['size'] as int : 0,
      started: j['started'] == true,
    );
  }
}

/// A file to upload, before its conflict is decided.
class UploadRequest {
  const UploadRequest(this.localPath, {String? name, this.ownedCopy = false})
    : _name = name;

  final String localPath;
  final String? _name;
  final bool ownedCopy;

  String get name =>
      _name ?? localPath.split(RegExp(r'[/\\]')).lastWhere((p) => p.isNotEmpty);
}

/// The file view's state: the upload queue, refresh after writes, the
/// personal storage and the trash purge.
class FilesController extends ChangeNotifier {
  FilesController({
    required this.files,
    required this.preferences,
    required this.sources,
    this.personal,
    this.createDeviceSource,
    DateTime Function()? clock,
    Future<int> Function(String path)? fileSize,
    Future<void> Function(String path)? deleteLocal,
  }) : clock = clock ?? DateTime.now,
       _fileSize = fileSize ?? _realSize,
       _deleteLocal = deleteLocal ?? _realDelete;

  final SourceFiles files;
  final Preferences preferences;
  final PersonalMedia? personal;

  /// The sources, freshest first ([AppServices.managedSources]).
  final List<ManagedSource> Function() sources;

  /// Creates the app's own folder on this device as a local source;
  /// returns its id. Null where it cannot (tests).
  final Future<int> Function()? createDeviceSource;
  final DateTime Function() clock;
  final Future<int> Function(String path) _fileSize;
  final Future<void> Function(String path) _deleteLocal;

  static Future<int> _realSize(String path) async {
    try {
      return await File(path).length();
    } catch (_) {
      return 0;
    }
  }

  static Future<void> _realDelete(String path) async {
    try {
      await File(path).delete();
    } catch (_) {}
  }

  final _changes = StreamController<void>.broadcast();

  /// A write landed and the library was refreshed: lists re-read.
  Stream<void> get changes => _changes.stream;

  final List<UploadJob> _jobs = [];

  /// The upload queue, finished ones included until cleared.
  List<UploadJob> get uploads => List.unmodifiable(_jobs);

  /// Uploads still to do.
  int get pendingUploads => _jobs
      .where(
        (j) => j.state == UploadState.waiting || j.state == UploadState.running,
      )
      .length;

  bool _running = false;
  var _seq = 0;
  final _writable = <int, bool>{};
  StoragePlace? _place;

  String _newId() =>
      'up-${clock().microsecondsSinceEpoch.toRadixString(36)}-${_seq++}';

  void _emit() {
    if (!_changes.isClosed) _changes.add(null);
  }

  // --- start -------------------------------------------------------------

  /// Resumes the saved queue and purges old trash; called once at start.
  Future<void> start() async {
    await _restore();
    unawaited(purgeTrashes());
  }

  Future<void> _restore() async {
    final raw = await preferences.read(FilesPrefs.queue);
    if (raw == null || raw.isEmpty) return;
    try {
      final list = jsonDecode(raw);
      if (list is! List) return;
      for (final j in list) {
        final job = UploadJob.fromJson(j);
        if (job != null && !_jobs.any((x) => x.id == job.id)) _jobs.add(job);
      }
    } catch (_) {
      return;
    }
    notifyListeners();
    _pump();
  }

  Future<void> _save() async {
    final open = [
      for (final j in _jobs)
        if (j.state != UploadState.done) j.toJson(),
    ];
    await preferences.write(
      FilesPrefs.queue,
      open.isEmpty ? null : jsonEncode(open),
    );
  }

  // --- uploads -----------------------------------------------------------

  /// Puts [requests] into folder [dir] of [sourceId] in the queue.
  Future<List<UploadJob>> enqueue(
    List<UploadRequest> requests, {
    required int sourceId,
    required String dir,
    NameConflict conflict = NameConflict.keepBoth,
  }) async {
    final added = <UploadJob>[];
    for (final r in requests) {
      final job = UploadJob(
        id: _newId(),
        localPath: r.localPath,
        sourceId: sourceId,
        dir: dir,
        name: r.name,
        conflict: conflict,
        ownedCopy: r.ownedCopy,
        size: await _fileSize(r.localPath),
      );
      _jobs.add(job);
      added.add(job);
    }
    await _save();
    notifyListeners();
    _pump();
    return added;
  }

  /// Stops [job] and forgets it (its partial file on the source goes too).
  Future<void> cancelUpload(UploadJob job) async {
    files.cancel(job.id);
    _jobs.remove(job);
    if (job.started) {
      final part = '${_join(job.dir, job.name)}.part';
      try {
        await files.deletePermanently(job.sourceId, part);
      } catch (_) {}
    }
    if (job.ownedCopy) await _deleteLocal(job.localPath);
    await _save();
    notifyListeners();
  }

  /// Tries a failed upload again (it resumes).
  void retry(UploadJob job) {
    if (job.state != UploadState.failed) return;
    job
      ..state = UploadState.waiting
      ..error = null;
    notifyListeners();
    _pump();
  }

  /// Drops finished uploads from the list.
  void clearFinished() {
    _jobs.removeWhere((j) => j.state == UploadState.done);
    notifyListeners();
  }

  void _pump() {
    if (_running) return;
    final next = _jobs
        .where((j) => j.state == UploadState.waiting)
        .firstOrNull;
    if (next == null) return;
    _running = true;
    unawaited(
      _upload(next).whenComplete(() {
        _running = false;
        _pump();
      }),
    );
  }

  Future<void> _upload(UploadJob job) async {
    job
      ..state = UploadState.running
      ..error = null;
    notifyListeners();
    try {
      // A restart right after the file was finished: it is there already.
      if (job.started && await _alreadyThere(job)) {
        job
          ..state = UploadState.done
          ..done = job.size
          ..resultPath = _join(job.dir, job.name);
      } else {
        job.started = true;
        await _save();
        var last = DateTime.fromMillisecondsSinceEpoch(0);
        final out = await files.upload(
          taskId: job.id,
          localPath: job.localPath,
          sourceId: job.sourceId,
          dir: job.dir,
          name: job.name,
          conflict: job.conflict,
          onProgress: (done, total) {
            job
              ..done = done
              ..size = total;
            final now = clock();
            if (now.difference(last).inMilliseconds >= 200 || done == total) {
              last = now;
              notifyListeners();
            }
          },
        );
        job
          ..state = UploadState.done
          ..done = job.size
          ..resultPath = out.relPath;
        await _refresh(job.sourceId, out);
      }
      if (job.ownedCopy) await _deleteLocal(job.localPath);
    } on FilesException catch (e) {
      if (!_jobs.contains(job)) return; // cancelled and removed
      job
        ..state = UploadState.failed
        ..error = e.error;
    } catch (e) {
      if (!_jobs.contains(job)) return;
      job
        ..state = UploadState.failed
        ..error = FilesError.other;
    }
    await _save();
    notifyListeners();
  }

  Future<bool> _alreadyThere(UploadJob job) async {
    try {
      final path = _join(job.dir, job.name);
      final part = await files.stat(job.sourceId, '$path.part');
      if (part != null) return false;
      final f = await files.stat(job.sourceId, path);
      return f != null && !f.isFolder && f.sizeBytes == job.size;
    } catch (_) {
      return false;
    }
  }

  // --- after a write -----------------------------------------------------

  /// Re-lists what [out] touched (and its origin source [fromSource] for
  /// a move between sources), then tells the views.
  Future<void> afterWrite(
    int sourceId,
    FileOutcome out, {
    int? fromSource,
  }) async {
    await _refresh(sourceId, out, fromSource: fromSource);
  }

  Future<void> _refresh(int sourceId, FileOutcome out, {int? fromSource}) async {
    try {
      await files.refresh(sourceId, out.touched);
      if (fromSource != null && out.touchedOrigin.isNotEmpty) {
        await files.refresh(fromSource, out.touchedOrigin);
      }
    } catch (e) {
      // The next scan picks it up.
      debugPrint('Library refresh after a write failed: $e');
    }
    _emit();
  }

  /// Tells the views that something changed (a write without a refresh).
  void changed() => _emit();

  // --- writable ----------------------------------------------------------

  /// Whether [sourceId] can be written (asked once per session, unless
  /// [fresh]); false when it cannot be reached.
  Future<bool> writable(int sourceId, {bool fresh = false}) async {
    if (!fresh && _writable.containsKey(sourceId)) return _writable[sourceId]!;
    var w = false;
    try {
      w = await files.writable(sourceId).timeout(const Duration(seconds: 15));
    } catch (_) {
      w = false;
    }
    _writable[sourceId] = w;
    return w;
  }

  /// Ids of the sources whose protocol can write, in list order.
  List<int> candidateSources({bool includeDevice = false}) => [
    for (final s in sources())
      if (writableTypes.contains(s.type) && s.enabled)
        if (int.tryParse(s.id) case final id?)
          if (includeDevice || id != _deviceId) id,
  ];

  int? _deviceId;

  /// The writable sources (checked now), for Settings and «Переместить в…».
  Future<List<ManagedSource>> writableSources() async {
    final out = <ManagedSource>[];
    for (final s in sources()) {
      final id = int.tryParse(s.id);
      if (id == null || !writableTypes.contains(s.type) || !s.enabled) {
        continue;
      }
      if (await writable(id)) out.add(s);
    }
    return out;
  }

  // --- personal storage --------------------------------------------------

  Future<StorageChoice> storageChoice() async =>
      StorageChoice.parse(await preferences.read(FilesPrefs.storage));

  Future<void> setStorageChoice(StorageChoice c) async {
    await preferences.write(
      FilesPrefs.storage,
      c is AutoStorage ? null : c.value,
    );
    _place = null;
    notifyListeners();
  }

  Future<int?> _deviceSource({required bool create}) async {
    final saved = int.tryParse(
      await preferences.read(FilesPrefs.deviceSource) ?? '',
    );
    if (saved != null && sources().any((s) => s.id == '$saved')) {
      return _deviceId = saved;
    }
    final make = createDeviceSource;
    if (!create || make == null) return null;
    final id = await make();
    await preferences.write(FilesPrefs.deviceSource, '$id');
    return _deviceId = id;
  }

  /// The source of the app's own folder, if it was created.
  Future<int?> deviceSourceId() => _deviceSource(create: false);

  /// Where uploads to «Личное» go now ([create]: make the device folder if
  /// it is needed); null if there is nowhere.
  Future<StoragePlace?> storagePlace({bool create = true}) async {
    if (_place case final p?) return p;
    final device = await _deviceSource(create: false);
    final choice = await storageChoice();
    StoragePlace? place;
    if (choice case SourceStorage(:final sourceId)) {
      if (sourceId == device) {
        place = StoragePlace(sourceId, '', onDevice: true);
      } else if (await writable(sourceId)) {
        place = StoragePlace(sourceId, storageBase);
      }
    }
    if (place == null && choice is! DeviceStorage) {
      for (final id in candidateSources()) {
        if (await writable(id)) {
          place = StoragePlace(id, storageBase);
          break;
        }
      }
    }
    if (place == null) {
      final id = await _deviceSource(create: create);
      if (id != null) place = StoragePlace(id, '', onDevice: true);
    }
    return _place = place;
  }

  /// The folder of [library] in the personal storage: the library's
  /// folder in that source if it has one, else `Kinolune/<name>` is made
  /// and joins the library (so what lands there is never recognized).
  /// [library] must be open.
  Future<(int, String)?> uploadFolderOf(PersonalLibrary library) async {
    final p = personal;
    final place = await storagePlace();
    if (p == null || place == null) return null;
    for (final f in library.folders) {
      if (f.sourceId == place.sourceId) {
        return (f.sourceId, f.path.replaceAll(RegExp(r'^/+|/+$'), ''));
      }
    }
    final name = _safeName(library.name);
    final dir = place.base.isEmpty ? name : '${place.base}/$name';
    await files.ensureFolder(place.sourceId, dir);
    await p.addFolder(library.id, place.sourceId, '/$dir');
    return (place.sourceId, dir);
  }

  /// The library uploads to «Личное» go to: the first open one; a new
  /// «Личное» without a lock when there is none. Null when every library
  /// is locked (the caller unlocks one first).
  Future<PersonalLibrary?> defaultLibrary(String newName) async {
    final p = personal;
    if (p == null) return null;
    final libs = await p.libraries();
    if (libs.isEmpty) return p.createLibrary(newName);
    return libs.where((l) => !l.locked).firstOrNull;
  }

  // --- trash -------------------------------------------------------------

  /// Purges trash items older than 30 days in every writable source, at
  /// most once a day per source.
  Future<int> purgeTrashes() async {
    final now = clock().millisecondsSinceEpoch;
    Map<String, dynamic> last;
    try {
      last =
          jsonDecode(await preferences.read(FilesPrefs.purged) ?? '{}')
              as Map<String, dynamic>;
    } catch (_) {
      last = {};
    }
    var n = 0;
    var touched = false;
    for (final id in candidateSources(includeDevice: true)) {
      final at = last['$id'];
      if (at is int && now - at < const Duration(days: 1).inMilliseconds) {
        continue;
      }
      try {
        n += await files.purgeTrash(id).timeout(const Duration(minutes: 2));
        last['$id'] = now;
        touched = true;
      } catch (_) {
        // Offline or read-only: next start.
      }
    }
    if (touched) await preferences.write(FilesPrefs.purged, jsonEncode(last));
    return n;
  }

  @override
  void dispose() {
    unawaited(_changes.close());
    super.dispose();
  }
}

String _join(String dir, String name) => dir.isEmpty ? name : '$dir/$name';

/// A folder name from a library name: one component.
String _safeName(String name) {
  final s = name
      .replaceAll(RegExp(r'[/\\:*?"<>|\x00-\x1f]'), ' ')
      .trim()
      .replaceAll(RegExp(r'^\.+'), '');
  return s.isEmpty ? 'Личное' : s; // l10n-ignore: a folder on disk
}
