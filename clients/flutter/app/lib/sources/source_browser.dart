// SPDX-License-Identifier: MPL-2.0
//
// What the file browser (UI-006) lists: the folders and media files of a
// source, each file with its library item when the library recognized it.
//
// * [CatalogSourceBrowser] (Rust core): the folder read live from the
//   source (`pa-sources` `list_folder`, LIB-018 backends) merged with the
//   catalog's view of it (LIB-009 `browse`: item ids, durations, file
//   counts). Offline, it shows the catalog's view alone.
// * [MemorySourceBrowser] (v0 fallback, demo): local folders through
//   `dart:io`, else the tree of the library's own file paths; files are
//   matched to items by path.
//
// Sorting and "folders first" are applied by [sortBrowseEntries], so both
// look the same.

import 'dart:async';

import '../integration/library.dart';
import '../library/library_queries.dart';
import 'source_manager.dart';

/// A folder or media file in the file browser.
class BrowseEntry {
  const BrowseEntry({
    required this.name,
    required this.relPath,
    required this.isFolder,
    this.itemId,
    this.sizeBytes,
    this.fileCount,
    this.modified,
    this.durationMs,
  });

  final String name;

  /// Relative to the source root, `/`-separated, no leading slash.
  final String relPath;
  final bool isFolder;

  /// Files: the movie, episode or track of the file, if recognized.
  final String? itemId;

  /// Files: size; folders: media inside, when the library knows it.
  final int? sizeBytes;

  /// Folders: media files inside at any depth, when the library knows it.
  final int? fileCount;

  /// Files: modified or added; folders: newest file inside.
  final DateTime? modified;
  final int? durationMs;

  /// Video or audio by extension; null for folders and disc images.
  MediaType? get mediaType => isFolder ? null : mediaTypeOf(name);

  /// A disc image (LIB-020): the player cannot open it.
  bool get isDiscImage => !isFolder && _discImage.hasMatch(name);

  @override
  String toString() => isFolder ? '$relPath/' : relPath;
}

final _discImage = RegExp(r'\.(iso|img)$', caseSensitive: false);

/// One folder of a source.
class BrowseListing {
  const BrowseListing(this.entries, {this.liveError});

  final List<BrowseEntry> entries;

  /// Why the folder could not be read from the source now; the entries are
  /// then what the library knows (possibly none).
  final SourceException? liveError;
}

/// Lists folders of sources for the file browser.
abstract class SourceBrowser {
  /// The browser for [library]: catalog + live listing on the Rust core,
  /// in memory otherwise (v0 fallback, demo).
  factory SourceBrowser.of(Library library, SourceManager manager) =>
      library is RustLibrary
      ? CatalogSourceBrowser(library, manager)
      : MemorySourceBrowser(library, manager);

  /// Entries of folder [dir] (relative, `''` = root) of [source], unsorted.
  Future<BrowseListing> list(ManagedSource source, String dir);

  /// What the player opens for file [relPath] of [source] when the file
  /// has no library item; null if it cannot be played from here.
  String? fileUri(ManagedSource source, String relPath);
}

/// Names a file browser does not show: hidden files and NAS system folders.
bool isHiddenName(String name) =>
    name.startsWith('.') ||
    name.startsWith('@') || // Synology @eaDir
    name.startsWith(r'$') || // $RECYCLE.BIN
    name == '#recycle' || // l10n-ignore
    name == 'System Volume Information' || // l10n-ignore
    name == 'lost+found'; // l10n-ignore

/// Whether the browser lists [name] as a file: media or a disc image.
bool isBrowsableFile(String name) =>
    mediaTypeOf(name) != null || _discImage.hasMatch(name);

/// The folder of a local source: a plain path (v0) or a `file://` URI
/// (the Rust source manager); null for network sources.
String? localRootOf(ManagedSource source) {
  if (source.type != SourceType.local) return null;
  final a = source.address;
  if (!a.startsWith('file:')) return a;
  try {
    return Uri.parse(a).toFilePath();
  } on UnsupportedError {
    return null;
  }
}

/// [relPath] under the local folder [root].
String joinLocal(String root, String relPath) {
  if (relPath.isEmpty) return root;
  final sep = root.contains(r'\') && !root.contains('/') ? r'\' : '/';
  final base = root.endsWith(sep) ? root.substring(0, root.length - 1) : root;
  return '$base$sep${relPath.replaceAll('/', sep)}';
}

/// [entries] sorted by [sort] (title = [labelOf], case-insensitive and
/// natural: "2" before "10"), folders before files if [foldersFirst].
/// Missing durations and dates go last in both directions.
List<BrowseEntry> sortBrowseEntries(
  List<BrowseEntry> entries, {
  required LibrarySort sort,
  required bool foldersFirst,
  String Function(BrowseEntry e)? labelOf,
}) {
  final label = labelOf ?? (e) => e.name;
  final keys = {for (final e in entries) e: naturalKey(label(e))};
  int byName(BrowseEntry a, BrowseEntry b) => keys[a]!.compareTo(keys[b]!);
  int nullsLast(int? x, int? y) {
    if (x == null && y == null) return 0;
    if (x == null) return 1;
    if (y == null) return -1;
    final c = x.compareTo(y);
    return sort.descending ? -c : c;
  }

  int compare(BrowseEntry a, BrowseEntry b) {
    if (foldersFirst && a.isFolder != b.isFolder) return a.isFolder ? -1 : 1;
    final c = switch (sort.field) {
      SortField.added || SortField.lastWatched || SortField.year => nullsLast(
        a.modified?.millisecondsSinceEpoch,
        b.modified?.millisecondsSinceEpoch,
      ),
      SortField.duration => nullsLast(a.durationMs, b.durationMs),
      SortField.title => sort.descending ? byName(b, a) : byName(a, b),
    };
    return c != 0 ? c : byName(a, b);
  }

  return [...entries]..sort(compare);
}

/// Sort key of a name: lower case, digit runs padded so "Episode 2" comes
/// before "Episode 10", leading articles kept (file names, not titles).
String naturalKey(String name) => name.toLowerCase().replaceAllMapped(
  RegExp(r'\d+'),
  (m) => m[0]!.padLeft(12, '0'),
);

/// Entries of a live listing that the browser shows, with what [known]
/// (the library's view of the same folder, by relative path) adds.
List<BrowseEntry> mergeListing(
  List<SourceFolderEntry> live,
  Map<String, BrowseEntry> known,
) => [
  for (final e in live)
    if (!isHiddenName(e.name) && (e.isFolder || isBrowsableFile(e.name)))
      BrowseEntry(
        name: e.name,
        relPath: e.relPath,
        isFolder: e.isFolder,
        itemId: known[e.relPath]?.itemId,
        sizeBytes: e.isFolder ? known[e.relPath]?.sizeBytes : e.sizeBytes,
        fileCount: known[e.relPath]?.fileCount,
        modified: known[e.relPath]?.modified ?? e.modified,
        durationMs: known[e.relPath]?.durationMs,
      ),
];

/// Catalog (LIB-009 `browse`) and live listing on the Rust core.
class CatalogSourceBrowser implements SourceBrowser {
  CatalogSourceBrowser(this.library, this.manager);

  final RustLibrary library;
  final SourceManager manager;

  Future<List<BrowseEntry>> _catalog(String sourceId, String dir) async {
    try {
      return [
        for (final e in await library.browse(
          sourceId,
          dir: dir,
          foldersFirst: false,
        ))
          BrowseEntry(
            name: e.name,
            relPath: e.relPath,
            isFolder: e.isFolder,
            itemId: e.itemId,
            sizeBytes: e.sizeBytes,
            fileCount: e.isFolder ? e.fileCount : null,
            modified: e.addedAt.millisecondsSinceEpoch == 0 ? null : e.addedAt,
            durationMs: e.durationMs,
          ),
      ];
    } catch (_) {
      return const [];
    }
  }

  @override
  Future<BrowseListing> list(ManagedSource source, String dir) async {
    final catalog = _catalog(source.id, dir);
    try {
      final live = await manager.listFolder(source.id, dir);
      final known = {for (final e in await catalog) e.relPath: e};
      return BrowseListing(mergeListing(live, known));
    } on SourceException catch (e) {
      return BrowseListing(
        await catalog,
        liveError: e.code == SourceErrorCode.unsupported ? null : e,
      );
    }
  }

  @override
  String? fileUri(ManagedSource source, String relPath) {
    final root = localRootOf(source);
    if (root != null) return joinLocal(root, relPath);
    return library.fileStreamUri(source.id, relPath);
  }
}

/// A library that knows files it has no item for (the demo's home videos),
/// so the in-memory browser can list them.
abstract interface class LibraryLooseFiles {
  /// Absolute paths of media files without an item.
  List<String> get looseFiles;
}

/// The v0 fallback and the demo: `dart:io` for local folders (through
/// [SourceManager.listFolder]), else the tree of the library's file paths.
class MemorySourceBrowser implements SourceBrowser {
  MemorySourceBrowser(this.library, this.manager);

  final Library library;
  final SourceManager manager;
  Future<Map<String, LibraryItem>>? _byPath;

  /// Movies, episodes and tracks by file path, read once.
  Future<Map<String, LibraryItem>> _items() => _byPath ??= () async {
    final out = <String, LibraryItem>{};
    void add(Iterable<LibraryItem> items) {
      for (final i in items) {
        out[i.filePath] = i;
      }
    }

    add(await library.movies().first);
    add(await library.tracks().first);
    for (final show in await library.shows().first) {
      add(await library.episodes(show.id).first);
    }
    return out;
  }();

  static String _trim(String p) =>
      p.length > 1 && (p.endsWith('/') || p.endsWith(r'\'))
      ? p.substring(0, p.length - 1)
      : p;

  @override
  Future<BrowseListing> list(ManagedSource source, String dir) async {
    final items = await _items();
    final root = _trim(localRootOf(source) ?? source.address);
    BrowseEntry withItem(BrowseEntry e) {
      final item = e.isFolder ? null : items[joinLocal(root, e.relPath)];
      if (item == null) return e;
      return BrowseEntry(
        name: e.name,
        relPath: e.relPath,
        isFolder: false,
        itemId: item.id,
        sizeBytes: e.sizeBytes,
        modified: e.modified ?? item.addedAt,
        durationMs: item.durationMs,
      );
    }

    try {
      final live = await manager.listFolder(source.id, dir);
      return BrowseListing([
        for (final e in mergeListing(live, const {})) withItem(e),
      ]);
    } on SourceException catch (e) {
      final paths = [
        ...items.keys,
        if (library case LibraryLooseFiles(:final looseFiles)) ...looseFiles,
      ];
      return BrowseListing(
        [for (final e in treeListing(root, dir, paths)) withItem(e)],
        liveError: e.code == SourceErrorCode.unsupported ? null : e,
      );
    }
  }

  @override
  String? fileUri(ManagedSource source, String relPath) {
    final root = localRootOf(source);
    return root == null ? null : joinLocal(root, relPath);
  }
}

/// Folder [dir] of the tree of file [paths] under [root]: its subfolders
/// (with the media files inside counted) and its files.
List<BrowseEntry> treeListing(String root, String dir, Iterable<String> paths) {
  final prefix = dir.isEmpty ? '$root/' : '$root/$dir/';
  final folders = <String, int>{};
  final files = <BrowseEntry>[];
  for (final path in paths) {
    if (!path.startsWith(prefix)) continue;
    final rest = path.substring(prefix.length);
    final slash = rest.indexOf('/');
    if (slash > 0) {
      final name = rest.substring(0, slash);
      folders[name] = (folders[name] ?? 0) + 1;
    } else if (rest.isNotEmpty && isBrowsableFile(rest)) {
      files.add(
        BrowseEntry(
          name: rest,
          relPath: dir.isEmpty ? rest : '$dir/$rest',
          isFolder: false,
        ),
      );
    }
  }
  return [
    for (final MapEntry(key: name, value: count) in folders.entries)
      if (!isHiddenName(name))
        BrowseEntry(
          name: name,
          relPath: dir.isEmpty ? name : '$dir/$name',
          isFolder: true,
          fileCount: count,
        ),
    ...files,
  ];
}
