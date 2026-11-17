// SPDX-License-Identifier: MPL-2.0
//
// In-memory files of writable sources (UI-035) for the demo mode, widget
// tests and screenshots: the same rules as `pa_vfs::files` — a taken name
// keeps both as `name (2).ext`, replace sends the old entry to the trash,
// the trash keeps where an entry was.

import 'dart:async';

import 'package:pa_library/pa_library.dart'
    show
        FileEntry,
        FileOutcome,
        FilesError,
        FilesException,
        NameConflict,
        SourceFiles,
        TouchedDir,
        TransferProgress,
        TrashEntry;

class _Node {
  _Node.folder() : isFolder = true, size = 0;
  _Node.file(this.size) : isFolder = false;

  final bool isFolder;
  final int size;
  DateTime modified = DateTime(2026, 11, 17, 12);
}

class _Trashed {
  _Trashed(this.id, this.original, this.nodes, this.at);
  final String id;
  final String original;

  /// The entry and everything below it, by path relative to it (`''` = it).
  final Map<String, _Node> nodes;
  final DateTime at;
}

/// [SourceFiles] in memory.
class FakeSourceFiles implements SourceFiles {
  FakeSourceFiles({this.readOnly = const {}, this.sizeOf, this.now});

  /// The demo: a writable home server (source `1`) with a few folders.
  factory FakeSourceFiles.demo() {
    final f = FakeSourceFiles();
    f.addFolder(1, 'Фильмы');
    f.addFile(1, 'Фильмы/Дюна (2021).mkv', 8 * 1024 * 1024 * 1024);
    f.addFolder(1, 'Сериалы');
    f.addFolder(1, 'Kinolune/Домашнее видео');
    f.addFile(1, 'Kinolune/Домашнее видео/Камин, 1 час.mp4', 2400000000);
    f.addFile(1, 'Kinolune/Домашнее видео/Аквариум.mp4', 1800000000);
    f.addFile(1, 'Заметки.txt', 2048);
    return f;
  }

  /// Sources that cannot be written.
  final Set<int> readOnly;

  /// Size of a local file to upload; 4 MiB by default.
  final int Function(String localPath)? sizeOf;
  final DateTime Function()? now;

  final _tree = <int, Map<String, _Node>>{};
  final _trash = <int, List<_Trashed>>{};
  var _nextTrash = 1;
  final _cancelled = <String>{};

  /// Every refresh, for tests: (source, touched).
  final refreshes = <(int, List<TouchedDir>)>[];

  /// Called on [refresh] (the demo links it to personal media).
  void Function(int sourceId, List<TouchedDir> touched)? onRefresh;

  /// Uploads wait for this before finishing, if set (tests of progress).
  Completer<void>? holdUploads;

  /// The next upload fails with this error.
  FilesError? failNextUpload;

  Map<String, _Node> _src(int id) => _tree.putIfAbsent(id, () => {});

  void addFolder(int source, String path) {
    final parts = path.split('/');
    for (var i = 1; i <= parts.length; i++) {
      _src(source)[parts.take(i).join('/')] ??= _Node.folder();
    }
  }

  void addFile(int source, String path, int size) {
    final parent = _parent(path);
    if (parent.isNotEmpty) addFolder(source, parent);
    _src(source)[path] = _Node.file(size);
  }

  /// Whether [path] exists, for tests.
  bool exists(int source, String path) => _src(source).containsKey(path);

  /// Size of the file at [path], for tests.
  int? sizeAt(int source, String path) => _src(source)[path]?.size;

  static String _parent(String path) {
    final i = path.lastIndexOf('/');
    return i < 0 ? '' : path.substring(0, i);
  }

  static String _name(String path) => path.substring(path.lastIndexOf('/') + 1);

  static String _join(String dir, String name) =>
      dir.isEmpty ? name : '$dir/$name';

  void _writable(int source) {
    if (readOnly.contains(source)) {
      throw const FilesException(FilesError.readOnly, 'read-only');
    }
  }

  void _checkName(String name) {
    if (name.trim().isEmpty ||
        name.contains('/') ||
        name.contains('\\') ||
        name == '.' ||
        name == '..') {
      throw FilesException(FilesError.invalid, name);
    }
  }

  String _free(int source, String dir, String name) {
    final dot = name.lastIndexOf('.');
    final (stem, ext) = dot > 0
        ? (name.substring(0, dot), name.substring(dot))
        : (name, '');
    for (var n = 2; ; n++) {
      final c = '$stem ($n)$ext';
      if (!_src(source).containsKey(_join(dir, c))) return c;
    }
  }

  /// Target path for [name] in [dir], or null to skip.
  String? _target(int source, String dir, String name, NameConflict c) {
    final path = _join(dir, name);
    if (!_src(source).containsKey(path)) return path;
    switch (c) {
      case NameConflict.keepBoth:
        return _join(dir, _free(source, dir, name));
      case NameConflict.replace:
        _toTrash(source, path);
        return path;
      case NameConflict.skip:
        return null;
    }
  }

  Map<String, _Node> _take(int source, String path) {
    final s = _src(source);
    final out = <String, _Node>{};
    for (final k in s.keys.toList()) {
      if (k == path || k.startsWith('$path/')) {
        out[k.substring(path.length)] = s.remove(k)!;
      }
    }
    return out;
  }

  void _put(int source, String path, Map<String, _Node> nodes) {
    final parent = _parent(path);
    if (parent.isNotEmpty) addFolder(source, parent);
    for (final e in nodes.entries) {
      _src(source)['$path${e.key}'] = e.value;
    }
  }

  void _toTrash(int source, String path) {
    final nodes = _take(source, path);
    if (nodes.isEmpty) throw FilesException(FilesError.notFound, path);
    _trash
        .putIfAbsent(source, () => [])
        .add(
          _Trashed(
            't${_nextTrash++}',
            path,
            nodes,
            (now ?? DateTime.now).call(),
          ),
        );
  }

  FileOutcome _done(String path, List<TouchedDir> touched) =>
      FileOutcome(relPath: path, touched: touched);

  @override
  Future<bool> writable(int sourceId) async => !readOnly.contains(sourceId);

  @override
  Future<List<FileEntry>> list(int sourceId, String dir) async {
    final s = _src(sourceId);
    if (dir.isNotEmpty && s[dir]?.isFolder != true) {
      throw FilesException(FilesError.notFound, dir);
    }
    return [
      for (final e in s.entries)
        if (_parent(e.key) == dir && e.key.isNotEmpty)
          FileEntry(
            name: _name(e.key),
            relPath: e.key,
            isFolder: e.value.isFolder,
            sizeBytes: e.value.size,
            modified: e.value.modified,
          ),
    ];
  }

  @override
  Future<FileEntry?> stat(int sourceId, String relPath) async {
    final n = _src(sourceId)[relPath];
    if (n == null) return null;
    return FileEntry(
      name: _name(relPath),
      relPath: relPath,
      isFolder: n.isFolder,
      sizeBytes: n.size,
    );
  }

  @override
  Future<FileOutcome> createFolder(
    int sourceId,
    String parent,
    String name, {
    NameConflict conflict = NameConflict.keepBoth,
  }) async {
    _writable(sourceId);
    _checkName(name);
    final path = _join(parent, name);
    if (_src(sourceId).containsKey(path) && conflict != NameConflict.keepBoth) {
      return FileOutcome(relPath: path, skipped: true);
    }
    final target = _src(sourceId).containsKey(path)
        ? _join(parent, _free(sourceId, parent, name))
        : path;
    addFolder(sourceId, target);
    return _done(target, [TouchedDir(parent)]);
  }

  @override
  Future<bool> ensureFolder(int sourceId, String relPath) async {
    _writable(sourceId);
    if (_src(sourceId)[relPath]?.isFolder == true) return false;
    addFolder(sourceId, relPath);
    return true;
  }

  @override
  Future<FileOutcome> rename(
    int sourceId,
    String relPath,
    String newName, {
    NameConflict conflict = NameConflict.keepBoth,
  }) async {
    _writable(sourceId);
    _checkName(newName);
    final dir = _parent(relPath);
    if (newName == _name(relPath)) return _done(relPath, const []);
    final target = _target(sourceId, dir, newName, conflict);
    if (target == null) return FileOutcome(relPath: relPath, skipped: true);
    _put(sourceId, target, _take(sourceId, relPath));
    return _done(target, [TouchedDir(dir)]);
  }

  @override
  Future<FileOutcome> trash(int sourceId, String relPath) async {
    _writable(sourceId);
    _toTrash(sourceId, relPath);
    return _done(relPath, [TouchedDir(_parent(relPath))]);
  }

  @override
  Future<FileOutcome> deletePermanently(int sourceId, String relPath) async {
    _writable(sourceId);
    _take(sourceId, relPath);
    return _done(relPath, [TouchedDir(_parent(relPath))]);
  }

  @override
  Future<List<TrashEntry>> trashList(int sourceId) async => [
    for (final t in (_trash[sourceId] ?? const <_Trashed>[]).reversed)
      TrashEntry(
        id: t.id,
        name: _name(t.original),
        original: t.original,
        isFolder: t.nodes['']!.isFolder,
        sizeBytes: t.nodes['']!.size,
        deletedAt: t.at,
      ),
  ];

  @override
  Future<FileOutcome> restore(
    int sourceId,
    String id, {
    NameConflict conflict = NameConflict.keepBoth,
  }) async {
    _writable(sourceId);
    final list = _trash[sourceId] ?? [];
    final t = list.where((t) => t.id == id).firstOrNull;
    if (t == null) throw FilesException(FilesError.notFound, id);
    final dir = _parent(t.original);
    final target = _target(sourceId, dir, _name(t.original), conflict);
    if (target == null) return FileOutcome(relPath: t.original, skipped: true);
    list.remove(t);
    _put(sourceId, target, t.nodes);
    return _done(target, [TouchedDir(dir, recursive: t.nodes['']!.isFolder)]);
  }

  @override
  Future<void> deleteFromTrash(int sourceId, String id) async {
    _trash[sourceId]?.removeWhere((t) => t.id == id);
  }

  @override
  Future<int> emptyTrash(int sourceId) async {
    _writable(sourceId);
    final n = _trash[sourceId]?.length ?? 0;
    _trash[sourceId]?.clear();
    return n;
  }

  /// Purge calls, for tests.
  final purges = <int>[];

  @override
  Future<int> purgeTrash(int sourceId) async {
    purges.add(sourceId);
    if (readOnly.contains(sourceId)) return 0;
    final cut = (now ?? DateTime.now)().subtract(const Duration(days: 30));
    final list = _trash[sourceId] ?? [];
    final before = list.length;
    list.removeWhere((t) => !t.at.isAfter(cut));
    return before - list.length;
  }

  /// Uploads, for tests: (local path, source, folder, name).
  final uploads = <(String, int, String, String)>[];

  @override
  Future<FileOutcome> upload({
    required String taskId,
    required String localPath,
    required int sourceId,
    required String dir,
    required String name,
    NameConflict conflict = NameConflict.keepBoth,
    TransferProgress? onProgress,
  }) async {
    _writable(sourceId);
    _checkName(name);
    if (failNextUpload case final e?) {
      failNextUpload = null;
      throw FilesException(e, 'test');
    }
    final size = sizeOf?.call(localPath) ?? 4 * 1024 * 1024;
    for (var i = 1; i <= 4; i++) {
      await Future<void>.delayed(Duration.zero);
      if (_cancelled.remove(taskId)) {
        throw const FilesException(FilesError.cancelled, 'cancelled');
      }
      onProgress?.call(size * i ~/ 4, size);
      if (i == 2 && holdUploads != null) await holdUploads!.future;
    }
    if (_cancelled.remove(taskId)) {
      throw const FilesException(FilesError.cancelled, 'cancelled');
    }
    final target = _target(sourceId, dir, name, conflict);
    if (target == null) {
      return FileOutcome(relPath: _join(dir, name), skipped: true);
    }
    if (dir.isNotEmpty) addFolder(sourceId, dir);
    _src(sourceId)[target] = _Node.file(size);
    uploads.add((localPath, sourceId, dir, _name(target)));
    return _done(target, [TouchedDir(dir)]);
  }

  /// Transfers, for tests: (move, from source, path, to source, folder).
  final transfers = <(bool, int, String, int, String)>[];

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
  }) async {
    _writable(toSource);
    if (move) _writable(fromSource);
    if (fromSource == toSource &&
        (toDir == relPath || toDir.startsWith('$relPath/'))) {
      throw FilesException(FilesError.invalid, 'into itself');
    }
    final node = _src(fromSource)[relPath];
    if (node == null) throw FilesException(FilesError.notFound, relPath);
    final target = _target(toSource, toDir, _name(relPath), conflict);
    if (target == null) return FileOutcome(relPath: relPath, skipped: true);
    final nodes = move
        ? _take(fromSource, relPath)
        : {
            for (final e in _src(fromSource).entries)
              if (e.key == relPath || e.key.startsWith('$relPath/'))
                e.key.substring(relPath.length): e.value,
          };
    _put(toSource, target, nodes);
    onProgress?.call(node.size, node.size);
    transfers.add((move, fromSource, relPath, toSource, toDir));
    final landed = TouchedDir(toDir, recursive: node.isFolder);
    if (!move) return _done(target, [landed]);
    final origin = TouchedDir(_parent(relPath));
    return fromSource == toSource
        ? _done(target, [origin, landed])
        : FileOutcome(
            relPath: target,
            touched: [landed],
            touchedOrigin: [origin],
          );
  }

  @override
  bool cancel(String taskId) => _cancelled.add(taskId);

  @override
  Future<bool> refresh(int sourceId, List<TouchedDir> touched) async {
    refreshes.add((sourceId, touched));
    onRefresh?.call(sourceId, touched);
    return touched.isNotEmpty;
  }
}
