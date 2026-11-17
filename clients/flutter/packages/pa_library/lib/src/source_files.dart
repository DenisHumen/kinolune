// SPDX-License-Identifier: MPL-2.0

// Files of a writable source (UI-035, «Файлы», ADR-0104): list, create a
// folder, upload with resume, rename, move and copy within and between
// sources, the trash with restore and «очистить». Paths are relative to
// the source root without a leading slash; `''` is the root.
//
// An operation does not update the library: it returns the folders it
// touched, and [SourceFiles.refresh] re-lists only those.
//
// [RustSourceFiles] implements it on the Rust core (`pa_vfs::files`).

import 'dart:async';

import 'package:pa_core/pa_core.dart';

import 'rust_library.dart';

/// What to do when the target name is taken.
enum NameConflict {
  /// The new entry gets a free name, `name (2).ext` (the default).
  keepBoth,

  /// The existing entry goes to the trash first.
  replace,

  /// Nothing is written.
  skip,
}

/// Why a file operation failed.
enum FilesError {
  notFound,

  /// The share or account does not let us.
  denied,

  /// The source cannot be written at all.
  readOnly,

  /// The name is taken.
  exists,

  /// No space left.
  full,
  cancelled,

  /// The source cannot be reached now.
  unavailable,
  invalid,
  other,
}

/// A failed file operation.
class FilesException implements Exception {
  const FilesException(this.error, [this.message = '']);

  final FilesError error;
  final String message;

  /// Reads the coded message of the core (`code: text`).
  factory FilesException.parse(String coded) {
    final i = coded.indexOf(':');
    final code = i < 0 ? coded : coded.substring(0, i);
    final text = i < 0 ? '' : coded.substring(i + 1).trim();
    final error = switch (code) {
      'not_found' => FilesError.notFound,
      'denied' => FilesError.denied,
      'read_only' => FilesError.readOnly,
      'exists' => FilesError.exists,
      'full' => FilesError.full,
      'cancelled' => FilesError.cancelled,
      'unavailable' => FilesError.unavailable,
      'invalid' => FilesError.invalid,
      _ => FilesError.other,
    };
    return FilesException(error, text);
  }

  @override
  String toString() => 'FilesException(${error.name}: $message)';
}

/// A file or folder in a source.
class FileEntry {
  const FileEntry({
    required this.name,
    required this.relPath,
    this.isFolder = false,
    this.sizeBytes = 0,
    this.modified,
  });

  final String name;

  /// Relative to the source root, no leading slash.
  final String relPath;
  final bool isFolder;
  final int sizeBytes;
  final DateTime? modified;

  @override
  String toString() => 'FileEntry($relPath${isFolder ? '/' : ''})';
}

/// An entry in the trash of a source.
class TrashEntry {
  const TrashEntry({
    required this.id,
    required this.name,
    required this.original,
    this.isFolder = false,
    this.sizeBytes = 0,
    required this.deletedAt,
  });

  final String id;
  final String name;

  /// Where it was, relative to the source root.
  final String original;
  final bool isFolder;
  final int sizeBytes;
  final DateTime deletedAt;
}

/// A folder whose listing changed.
class TouchedDir {
  const TouchedDir(this.dir, {this.recursive = false});

  /// Relative to the source root, `''` for the root.
  final String dir;

  /// Everything below it changed too.
  final bool recursive;

  @override
  bool operator ==(Object other) =>
      other is TouchedDir && other.dir == dir && other.recursive == recursive;

  @override
  int get hashCode => Object.hash(dir, recursive);
}

/// Result of a write.
class FileOutcome {
  const FileOutcome({
    required this.relPath,
    this.skipped = false,
    this.touched = const [],
    this.touchedOrigin = const [],
  });

  /// Where the entry is now (skipped: the entry that blocked it).
  final String relPath;

  /// Nothing was written (a taken name with [NameConflict.skip]).
  final bool skipped;

  /// Folders of the (destination) source to refresh.
  final List<TouchedDir> touched;

  /// Folders of the origin source to refresh (a move between sources).
  final List<TouchedDir> touchedOrigin;
}

/// `(bytes done, bytes total)` of the current file.
typedef TransferProgress = void Function(int done, int total);

/// The file operations of sources.
abstract interface class SourceFiles {
  /// Whether [sourceId] can be written now (protocol and permissions).
  Future<bool> writable(int sourceId);

  /// The visible entries of folder [dir], in no particular order.
  Future<List<FileEntry>> list(int sourceId, String dir);

  /// The entry at [relPath], null if there is none.
  Future<FileEntry?> stat(int sourceId, String relPath);

  Future<FileOutcome> createFolder(
    int sourceId,
    String parent,
    String name, {
    NameConflict conflict = NameConflict.keepBoth,
  });

  /// [relPath] is there or is created with its parents; true if created.
  Future<bool> ensureFolder(int sourceId, String relPath);

  Future<FileOutcome> rename(
    int sourceId,
    String relPath,
    String newName, {
    NameConflict conflict = NameConflict.keepBoth,
  });

  /// Into the trash of the source (kept 30 days).
  Future<FileOutcome> trash(int sourceId, String relPath);

  Future<FileOutcome> deletePermanently(int sourceId, String relPath);

  /// The trash, newest first.
  Future<List<TrashEntry>> trashList(int sourceId);

  Future<FileOutcome> restore(
    int sourceId,
    String id, {
    NameConflict conflict = NameConflict.keepBoth,
  });

  Future<void> deleteFromTrash(int sourceId, String id);

  /// «Очистить»: everything in the trash; how many items.
  Future<int> emptyTrash(int sourceId);

  /// Deletes trash items older than 30 days; how many.
  Future<int> purgeTrash(int sourceId);

  /// Uploads the local file [localPath] as [name] into [dir], resuming an
  /// earlier attempt of the same file. [cancel] with [taskId] stops it.
  Future<FileOutcome> upload({
    required String taskId,
    required String localPath,
    required int sourceId,
    required String dir,
    required String name,
    NameConflict conflict = NameConflict.keepBoth,
    TransferProgress? onProgress,
  });

  /// Moves or copies [relPath] of [fromSource] into [toDir] of [toSource]
  /// (the same source or another one).
  Future<FileOutcome> transfer({
    required String taskId,
    required bool move,
    required int fromSource,
    required String relPath,
    required int toSource,
    required String toDir,
    NameConflict conflict = NameConflict.keepBoth,
    TransferProgress? onProgress,
  });

  /// Stops the upload or transfer [taskId]; false if it is not running.
  bool cancel(String taskId);

  /// Updates the library after a write: re-lists [touched] folders only.
  /// True if its media files changed.
  Future<bool> refresh(int sourceId, List<TouchedDir> touched);
}

/// [SourceFiles] on the Rust core.
class RustSourceFiles implements SourceFiles {
  RustSourceFiles(this.library);

  final RustLibrary library;

  CoreLibrary get _core => library.core;

  static String _conflict(NameConflict c) => switch (c) {
    NameConflict.keepBoth => 'keep_both',
    NameConflict.replace => 'replace',
    NameConflict.skip => 'skip',
  };

  static Future<T> _call<T>(Future<T> Function() f) async {
    try {
      return await f();
    } on LibraryError catch (e) {
      throw FilesException.parse(e.message);
    }
  }

  static FileEntry _entry(CoreFileEntry e) => FileEntry(
    name: e.name,
    relPath: e.relPath,
    isFolder: e.isFolder,
    sizeBytes: e.sizeBytes.toInt(),
    modified: switch (e.modifiedMs) {
      final ms? => DateTime.fromMillisecondsSinceEpoch(ms),
      null => null,
    },
  );

  static List<TouchedDir> _touched(List<CoreTouched> t) => [
    for (final d in t) TouchedDir(d.dir, recursive: d.recursive),
  ];

  static FileOutcome _outcome(CoreFileOutcome o) => FileOutcome(
    relPath: o.relPath,
    skipped: o.skipped,
    touched: _touched(o.touched),
    touchedOrigin: _touched(o.touchedOrigin),
  );

  Future<FileOutcome> _stream(
    Stream<CoreTransferEvent> events,
    TransferProgress? onProgress,
  ) async {
    await for (final e in events) {
      if (!e.finished) {
        onProgress?.call(e.done.toInt(), e.total.toInt());
        continue;
      }
      if (e.error case final err?) throw FilesException.parse(err);
      if (e.outcome case final o?) return _outcome(o);
    }
    throw const FilesException(FilesError.other, 'the transfer ended early');
  }

  @override
  Future<bool> writable(int sourceId) =>
      _call(() => _core.filesWritable(sourceId: sourceId));

  @override
  Future<List<FileEntry>> list(int sourceId, String dir) => _call(
    () async => [
      for (final e in await _core.filesList(sourceId: sourceId, dir: dir))
        _entry(e),
    ],
  );

  @override
  Future<FileEntry?> stat(int sourceId, String relPath) => _call(() async {
    final e = await _core.filesStat(sourceId: sourceId, relPath: relPath);
    return e == null ? null : _entry(e);
  });

  @override
  Future<FileOutcome> createFolder(
    int sourceId,
    String parent,
    String name, {
    NameConflict conflict = NameConflict.keepBoth,
  }) => _call(
    () async => _outcome(
      await _core.filesCreateFolder(
        sourceId: sourceId,
        parent: parent,
        name: name,
        conflict: _conflict(conflict),
      ),
    ),
  );

  @override
  Future<bool> ensureFolder(int sourceId, String relPath) => _call(
    () => _core.filesEnsureFolder(sourceId: sourceId, relPath: relPath),
  );

  @override
  Future<FileOutcome> rename(
    int sourceId,
    String relPath,
    String newName, {
    NameConflict conflict = NameConflict.keepBoth,
  }) => _call(
    () async => _outcome(
      await _core.filesRename(
        sourceId: sourceId,
        relPath: relPath,
        newName: newName,
        conflict: _conflict(conflict),
      ),
    ),
  );

  @override
  Future<FileOutcome> trash(int sourceId, String relPath) => _call(
    () async =>
        _outcome(await _core.filesTrash(sourceId: sourceId, relPath: relPath)),
  );

  @override
  Future<FileOutcome> deletePermanently(int sourceId, String relPath) => _call(
    () async => _outcome(
      await _core.filesDeletePermanently(sourceId: sourceId, relPath: relPath),
    ),
  );

  @override
  Future<List<TrashEntry>> trashList(int sourceId) => _call(
    () async => [
      for (final t in await _core.filesListTrash(sourceId: sourceId))
        TrashEntry(
          id: t.id,
          name: t.name,
          original: t.original,
          isFolder: t.isFolder,
          sizeBytes: t.sizeBytes.toInt(),
          deletedAt: DateTime.fromMillisecondsSinceEpoch(t.deletedMs),
        ),
    ],
  );

  @override
  Future<FileOutcome> restore(
    int sourceId,
    String id, {
    NameConflict conflict = NameConflict.keepBoth,
  }) => _call(
    () async => _outcome(
      await _core.filesRestore(
        sourceId: sourceId,
        id: id,
        conflict: _conflict(conflict),
      ),
    ),
  );

  @override
  Future<void> deleteFromTrash(int sourceId, String id) =>
      _call(() => _core.filesDeleteFromTrash(sourceId: sourceId, id: id));

  @override
  Future<int> emptyTrash(int sourceId) =>
      _call(() => _core.filesEmptyTrash(sourceId: sourceId));

  @override
  Future<int> purgeTrash(int sourceId) =>
      _call(() => _core.filesPurgeTrash(sourceId: sourceId));

  @override
  Future<FileOutcome> upload({
    required String taskId,
    required String localPath,
    required int sourceId,
    required String dir,
    required String name,
    NameConflict conflict = NameConflict.keepBoth,
    TransferProgress? onProgress,
  }) => _stream(
    _core.filesUpload(
      taskId: taskId,
      localPath: localPath,
      sourceId: sourceId,
      dir: dir,
      name: name,
      conflict: _conflict(conflict),
    ),
    onProgress,
  );

  @override
  Future<FileOutcome> transfer({
    required String taskId,
    required bool move,
    required int fromSource,
    required String relPath,
    required int toSource,
    required String toDir,
    NameConflict conflict = NameConflict.keepBoth,
    TransferProgress? onProgress,
  }) => _stream(
    _core.filesTransfer(
      taskId: taskId,
      mv: move,
      fromSource: fromSource,
      relPath: relPath,
      toSource: toSource,
      toDir: toDir,
      conflict: _conflict(conflict),
    ),
    onProgress,
  );

  @override
  bool cancel(String taskId) => _core.filesCancel(taskId: taskId);

  @override
  Future<bool> refresh(int sourceId, List<TouchedDir> touched) async {
    if (touched.isEmpty) return false;
    final changed = await _call(
      () => _core.filesRefresh(
        sourceId: sourceId,
        touched: [
          for (final t in touched)
            CoreTouched(dir: t.dir, recursive: t.recursive),
        ],
      ),
    );
    if (changed) library.notifyExternalChange();
    return changed;
  }
}
