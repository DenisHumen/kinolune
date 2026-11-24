// SPDX-License-Identifier: MPL-2.0
//
// LIB-027 (ADR-0139): «Закинуть файл прямо через приложение». Files and
// folders dragged onto the window (desktop), shared to Kinolune or picked
// (phone) go into the library on the server or NAS: the app picks the
// writable source (asks once when there are several and remembers), puts
// every item into the folder of its kind — movies, shows, music; a Blu-ray
// or DVD folder (BDMV, VIDEO_TS) or an ISO with the movies — uploads it
// through the writes of NET-023 (resumed after a broken connection) and
// the refresh after each file recognizes only what is new. It never asks
// where a file came from. Photos and camera videos keep going to
// «Личное» (UI-035).
//
// The kind folder of a source is the one already there («Movies»,
// «Фильмы», «Кино», «TV Shows», «Сериалы», «Music», «Музыка» …), else a
// new «Movies» / «TV Shows» / «Music», the names Jellyfin, Plex and Kodi
// users know. A taken folder name keeps both (`name (2)`); a taken file
// name too (the core's keep-both).

import 'dart:async';
import 'dart:io';

import 'package:desktop_drop/desktop_drop.dart'
    show DropDirectoryItem, DropFileItem, DropItemKind, DropTarget;
import 'package:file_picker/file_picker.dart';
import 'package:flutter/foundation.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_library/pa_library.dart'
    show
        FilesError,
        FilesException,
        NameConflict,
        PersonalMedia,
        SourceContent,
        SourceFiles;

import '../l10n/l10n.dart';
import '../screens/downloads_screen.dart';
import '../services/app_services.dart';
import '../sources/source_manager.dart';
import '../theme/kinolune_theme.dart';
import 'file_size.dart';
import 'files_controller.dart';
import 'files_dialogs.dart';
import 'personal_files.dart';

// --- what an item is ---------------------------------------------------------

/// What a dropped, shared or picked item is, and so where it goes.
enum DropKind {
  movie,
  series,
  music,

  /// A Blu-ray or DVD folder (BDMV, VIDEO_TS) or a disc image: with the
  /// movies, in a folder of its own.
  disc,

  /// Photos and camera videos: «Личное», never recognized.
  personal,

  /// Nothing the library plays (documents, archives …).
  unsupported,
}

const _videoExts = {
  'mkv', 'mp4', 'm4v', 'avi', 'mov', 'wmv', 'ts', 'm2ts', 'mts', //
  'webm', 'mpg', 'mpeg', 'vob', 'flv', 'ogv', '3gp', 'divx', 'rmvb',
};
const _audioExts = {
  'mp3', 'flac', 'm4a', 'm4b', 'ogg', 'oga', 'opus', 'wav', 'aac', //
  'alac', 'ape', 'wv', 'aiff', 'aif', 'dsf', 'dff', 'wma', 'mka',
};
const _imageExts = {
  'jpg', 'jpeg', 'png', 'heic', 'heif', 'gif', 'webp', 'dng', 'raw',
};
const _subtitleExts = {'srt', 'ass', 'ssa', 'sub', 'idx', 'vtt', 'sup'};
const _discImageExts = {'iso'};

/// Junk the system leaves in folders: not uploaded.
bool _junk(String name) {
  final n = name.toLowerCase();
  return n.startsWith('.') ||
      n == 'thumbs.db' ||
      n == 'desktop.ini' ||
      n == '__macosx';
}

/// `S01E02`, `1x02`, `Season 1`, `Сезон 1`, ` - 05 ` (anime).
final _episode = RegExp(
  r'(?:^|[^a-z0-9])s\d{1,2}[ ._-]?e\d{1,3}'
  r'|(?<![0-9])\d{1,2}x\d{2,3}(?![0-9])'
  r'|season[ ._-]?\d'
  r'|сезон[ ._-]?\d'
  r'|\s-\s\d{2,3}(?:v\d)?(?:\s|\[|\(|\.|$)',
  caseSensitive: false,
);

/// Names phones and cameras give: `IMG_`, `VID_`, `PXL_`, `DSC`, `GOPR`,
/// `20241005_181204`, `VID-20240101-WA0001` …
final _camera = RegExp(
  r'^(?:img|vid|pxl|dsc[nf]?|gopr|g[hx]\d\d|mvi|dji|mov|screen[_ -]?record'
  r'|screenrecorder|whatsapp video|signal)[-_ ]?\d'
  r'|^\d{8}[_-]\d{6}'
  r'|^vid-\d{8}-wa',
  caseSensitive: false,
);

String _ext(String name) {
  final i = name.lastIndexOf('.');
  return i <= 0 ? '' : name.substring(i + 1).toLowerCase();
}

String _stem(String name) {
  final i = name.lastIndexOf('.');
  return i <= 0 ? name : name.substring(0, i);
}

/// The last component of a local path.
String localName(String path) {
  final parts = path.split(RegExp(r'[/\\]')).where((p) => p.isNotEmpty);
  return parts.isEmpty ? path : parts.last;
}

String _localParentName(String path) {
  final parts = path
      .split(RegExp(r'[/\\]'))
      .where((p) => p.isNotEmpty)
      .toList();
  return parts.length < 2 ? '' : parts[parts.length - 2];
}

/// The kind of one file by its name.
DropKind kindOfFile(String name) {
  final ext = _ext(name);
  if (_discImageExts.contains(ext)) return DropKind.disc;
  if (_imageExts.contains(ext)) return DropKind.personal;
  if (_audioExts.contains(ext)) return DropKind.music;
  if (_videoExts.contains(ext)) {
    if (_camera.hasMatch(name)) return DropKind.personal;
    if (_episode.hasMatch(_stem(name))) return DropKind.series;
    return DropKind.movie;
  }
  return DropKind.unsupported;
}

/// The show of an episode file: what comes before `S01E02` (or the
/// episode number), dots and underscores read as spaces. Empty when the
/// name starts with the episode.
String showNameOf(String fileName) {
  final stem = _stem(fileName);
  final m = _episode.firstMatch(stem);
  if (m == null) return '';
  var name = stem.substring(0, m.start);
  // A release group in brackets first: `[Group] Show - 05`.
  name = name.replaceAll(RegExp(r'^\s*\[[^\]]*\]\s*'), '');
  name = name.replaceAll(RegExp(r'[._]+'), ' ');
  name = name.replaceAll(RegExp(r'[\s\-–—(\[]+$'), '').trim();
  return safeFolderName(name);
}

/// One folder name: no separators or characters Windows shares refuse.
String safeFolderName(String name) => name
    .replaceAll(RegExp(r'[/\\:*?"<>|\x00-\x1f]'), ' ')
    .replaceAll(RegExp(r'\s+'), ' ')
    .trim()
    .replaceAll(RegExp(r'^\.+'), '');

// --- the local file system ---------------------------------------------------

/// The local files the app reads to sort a drop (tests use a fake).
abstract interface class LocalFs {
  bool isDirectory(String path);

  /// The files below folder [dir], recursively, relative to it with `/`.
  List<String> filesUnder(String dir);

  /// [rel] (with `/`) inside [dir].
  String join(String dir, String rel);
}

/// [LocalFs] on `dart:io`. Synchronous on purpose: a few stats and one
/// listing, and no I/O future a widget test's fake clock would never end.
class IoLocalFs implements LocalFs {
  const IoLocalFs();

  @override
  bool isDirectory(String path) =>
      FileSystemEntity.typeSync(path) == FileSystemEntityType.directory;

  @override
  List<String> filesUnder(String dir) {
    final root = Directory(dir);
    final base = root.path.endsWith(Platform.pathSeparator)
        ? root.path
        : '${root.path}${Platform.pathSeparator}';
    try {
      return [
        for (final e in root.listSync(recursive: true, followLinks: false))
          if (e is File && e.path.startsWith(base))
            e.path.substring(base.length).replaceAll(r'\', '/'),
      ];
    } on FileSystemException {
      return const [];
    }
  }

  @override
  String join(String dir, String rel) {
    final sep = Platform.pathSeparator;
    final base = dir.endsWith(sep) ? dir : '$dir$sep';
    return '$base${rel.replaceAll('/', sep)}';
  }
}

/// The file system [addToLibrary] reads (tests replace it).
LocalFs libraryLocalFs = const IoLocalFs();

// --- sorting a drop ----------------------------------------------------------

/// A dropped, shared or picked item, sorted.
class SortedDrop {
  const SortedDrop({
    required this.request,
    required this.kind,
    this.isFolder = false,
    this.files = const [],
    this.folderName = '',
  });

  final UploadRequest request;
  final DropKind kind;
  final bool isFolder;

  /// A folder: its files, relative to the folder the upload creates
  /// ([folderName]); a disc folder named `BDMV` itself keeps that name in
  /// front (`BDMV/index.bdmv`).
  final List<String> files;

  /// The folder made in the kind folder for a folder item.
  final String folderName;

  String get name => request.name;
}

/// Sorts [request] (a file or a folder).
SortedDrop sortDropped(LocalFs fs, UploadRequest request) {
  final path = request.localPath;
  if (!fs.isDirectory(path)) {
    return SortedDrop(request: request, kind: kindOfFile(request.name));
  }
  final name = request.name;
  final all = [
    for (final f in fs.filesUnder(path))
      if (!f.split('/').any(_junk)) f,
  ];
  final upper = name.toUpperCase();
  // The disc structure itself: it goes into a folder named after the one
  // holding it.
  if (upper == 'BDMV' || upper == 'VIDEO_TS') {
    final holder = safeFolderName(_localParentName(path));
    return SortedDrop(
      request: request,
      kind: DropKind.disc,
      isFolder: true,
      files: [for (final f in all) '$upper/$f'],
      folderName: holder.isEmpty ? upper : holder,
    );
  }
  final folder = safeFolderName(name);
  SortedDrop item(DropKind kind) => SortedDrop(
    request: request,
    kind: kind,
    isFolder: true,
    files: all,
    folderName: folder.isEmpty ? 'Kinolune' : folder, // l10n-ignore: a folder
  );
  final top = {for (final f in all) f.split('/').first.toUpperCase()};
  final hasDisc = all.any((f) {
    final parts = f.toUpperCase().split('/');
    return parts.length > 1 &&
        parts.length <= 4 &&
        parts.any((p) => p == 'BDMV' || p == 'VIDEO_TS');
  });
  if (top.contains('BDMV') || top.contains('VIDEO_TS') || hasDisc) {
    return item(DropKind.disc);
  }
  final kinds = [for (final f in all) kindOfFile(localName(f))];
  final videos = kinds.where(
    (k) => k == DropKind.movie || k == DropKind.series,
  );
  if (videos.isNotEmpty) {
    final series =
        _episode.hasMatch(name) || kinds.contains(DropKind.series);
    return item(series ? DropKind.series : DropKind.movie);
  }
  if (kinds.contains(DropKind.music)) return item(DropKind.music);
  if (kinds.contains(DropKind.disc)) return item(DropKind.disc);
  if (kinds.isNotEmpty && kinds.every((k) => k == DropKind.personal)) {
    return item(DropKind.personal);
  }
  return item(DropKind.unsupported);
}

// --- where it goes -----------------------------------------------------------

/// Names of the kind folders that are recognized in the root of a source,
/// lower case, without spaces, dashes and underscores.
const _kindFolders = {
  DropKind.movie: {
    'movies', 'movie', 'films', 'film', 'filme', 'фильмы', 'кино', //
    'фільми', 'кіно',
  },
  DropKind.series: {
    'tvshows', 'tvshow', 'tv', 'shows', 'series', 'tvseries', 'сериалы', //
    'серіали',
  },
  DropKind.music: {'music', 'musik', 'музыка', 'музика'},
};

/// The folder made for a kind when the source has none.
const _defaultKindFolder = {
  DropKind.movie: 'Movies', // l10n-ignore: a folder on the source
  DropKind.series: 'TV Shows', // l10n-ignore: a folder on the source
  DropKind.music: 'Music', // l10n-ignore: a folder on the source
};

String _norm(String s) => s.toLowerCase().replaceAll(RegExp(r'[\s_\-.]'), '');

/// One file to upload: into [dir] of the source as [name].
class PlannedUpload {
  const PlannedUpload(this.request, this.dir);
  final UploadRequest request;
  final String dir;

  @override
  String toString() => 'PlannedUpload(${request.localPath} → $dir/${request.name})';
}

/// What [LibraryPlanner.plan] decided.
class LibraryPlan {
  const LibraryPlan({
    this.uploads = const [],
    this.personal = const [],
    this.skipped = const [],
    this.kinds = const {},
  });

  final List<PlannedUpload> uploads;

  /// Photos and camera videos, for «Личное».
  final List<UploadRequest> personal;

  /// Names of items the library cannot hold.
  final List<String> skipped;

  /// The kinds that go to the library.
  final Set<DropKind> kinds;

  /// The folders of the source to create first.
  Set<String> get folders => {for (final u in uploads) u.dir};
}

/// Decides where the items of a drop go in source [sourceId].
class LibraryPlanner {
  LibraryPlanner(this.files, this.sourceId, {LocalFs? fs})
    : fs = fs ?? libraryLocalFs;

  final SourceFiles files;
  final int sourceId;
  final LocalFs fs;

  List<String>? _rootFolders;
  final _taken = <String>{};

  Future<List<String>> _roots() async => _rootFolders ??= [
    for (final e in await files.list(sourceId, ''))
      if (e.isFolder) e.name,
  ];

  /// The kind folder of [kind] in the source: the one there, else a new
  /// one by its usual name.
  Future<String> kindFolder(DropKind kind) async {
    final k = kind == DropKind.disc ? DropKind.movie : kind;
    final names = _kindFolders[k]!;
    for (final r in await _roots()) {
      if (names.contains(_norm(r))) return r;
    }
    return _defaultKindFolder[k]!;
  }

  /// A free folder name for [name] in [dir]: `name`, `name (2)` …
  Future<String> _freeFolder(String dir, String name) async {
    for (var n = 1; ; n++) {
      final candidate = n == 1 ? name : '$name ($n)';
      final path = dir.isEmpty ? candidate : '$dir/$candidate';
      if (_taken.contains(path)) continue;
      if (await files.stat(sourceId, path) == null) {
        _taken.add(path);
        return candidate;
      }
    }
  }

  /// Sorts [requests] and decides where each file goes.
  Future<LibraryPlan> plan(List<UploadRequest> requests) async =>
      planItems([for (final r in requests) sortDropped(fs, r)]);

  /// [plan] for items sorted already.
  Future<LibraryPlan> planItems(List<SortedDrop> items) async {
    final uploads = <PlannedUpload>[];
    final personal = <UploadRequest>[];
    final skipped = <String>[];
    final kinds = <DropKind>{};
    final subtitles = <SortedDrop>[];
    for (final item in items) {
      switch (item.kind) {
        case DropKind.personal:
          if (item.isFolder) {
            for (final f in item.files) {
              personal.add(
                UploadRequest(
                  fs.join(item.request.localPath, f),
                  ownedCopy: item.request.ownedCopy,
                ),
              );
            }
          } else {
            personal.add(item.request);
          }
        case DropKind.unsupported:
          if (!item.isFolder && _subtitleExts.contains(_ext(item.name))) {
            subtitles.add(item);
          } else {
            skipped.add(item.name);
          }
        case DropKind.movie ||
            DropKind.series ||
            DropKind.music ||
            DropKind.disc:
          kinds.add(item.kind);
          final root = await kindFolder(item.kind);
          if (item.isFolder) {
            final folder = await _freeFolder(root, item.folderName);
            final base = '$root/$folder';
            // A disc folder named BDMV keeps that name in front of its
            // files, but they are read from the folder itself.
            final upper = item.name.toUpperCase();
            final inner = upper == 'BDMV' || upper == 'VIDEO_TS';
            for (final f in item.files) {
              final i = f.lastIndexOf('/');
              final sub = i < 0 ? '' : f.substring(0, i);
              final local = inner ? f.substring(upper.length + 1) : f;
              uploads.add(
                PlannedUpload(
                  UploadRequest(
                    fs.join(item.request.localPath, local),
                    fileName: localName(f),
                    ownedCopy: item.request.ownedCopy,
                  ),
                  sub.isEmpty ? base : '$base/$sub',
                ),
              );
            }
          } else {
            var dir = root;
            if (item.kind == DropKind.series) {
              final show = showNameOf(item.name);
              if (show.isNotEmpty) dir = '$root/$show';
            }
            uploads.add(PlannedUpload(item.request, dir));
          }
      }
    }
    // Subtitles go next to the video they belong to, else they are skipped.
    for (final s in subtitles) {
      final stem = _stem(s.name).toLowerCase();
      final video = uploads
          .where(
            (u) =>
                _videoExts.contains(_ext(u.request.name)) &&
                stem.startsWith(_stem(u.request.name).toLowerCase()),
          )
          .firstOrNull;
      if (video == null) {
        skipped.add(s.name);
      } else {
        uploads.add(PlannedUpload(s.request, video.dir));
      }
    }
    return LibraryPlan(
      uploads: uploads,
      personal: personal,
      skipped: skipped,
      kinds: kinds,
    );
  }
}

/// Puts [plan] into the upload queue of [files]: its folders are made
/// first, then every file goes with keep-both on a taken name. Returns the
/// jobs.
Future<List<UploadJob>> enqueuePlan(
  FilesController files,
  int sourceId,
  LibraryPlan plan,
) async {
  final byDir = <String, List<UploadRequest>>{};
  for (final u in plan.uploads) {
    byDir.putIfAbsent(u.dir, () => []).add(u.request);
  }
  for (final dir in byDir.keys) {
    await files.files.ensureFolder(sourceId, dir);
  }
  return [
    for (final e in byDir.entries)
      ...await files.enqueue(
        e.value,
        sourceId: sourceId,
        dir: e.key,
        conflict: NameConflict.keepBoth,
        toLibrary: true,
      ),
  ];
}

// --- which source ------------------------------------------------------------

/// The sources the library can be uploaded to: writable, not the app's
/// own folder on this device and not a source of personal media.
Future<List<ManagedSource>> librarySources(
  FilesController files, {
  PersonalMedia? personal,
}) async {
  final device = await files.deviceSourceId();
  final out = <ManagedSource>[];
  for (final s in await files.writableSources()) {
    if (s.id == '$device') continue;
    final id = int.tryParse(s.id);
    if (personal != null && id != null) {
      try {
        if (await personal.sourceContent(id) == SourceContent.personal) {
          continue;
        }
      } catch (_) {}
    }
    out.add(s);
  }
  return out;
}

/// The source the library goes to: the one remembered, the only one, or
/// the one the user picks now (remembered). Null if there is none or the
/// user closed the question.
Future<ManagedSource?> chooseLibrarySource(
  BuildContext context,
  FilesController files, {
  PersonalMedia? personal,
}) async {
  final all = await librarySources(files, personal: personal);
  if (all.isEmpty) return null;
  final saved = await files.preferences.read(FilesPrefs.librarySource);
  final remembered = all.where((s) => s.id == saved).firstOrNull;
  if (remembered != null) return remembered;
  if (all.length == 1) return all.single;
  if (!context.mounted) return null;
  final chosen = await showDialog<ManagedSource>(
    context: context,
    builder: (context) => _SourceDialog(sources: all),
  );
  if (chosen != null) {
    await files.preferences.write(FilesPrefs.librarySource, chosen.id);
  }
  return chosen;
}

class _SourceDialog extends StatelessWidget {
  const _SourceDialog({required this.sources});

  final List<ManagedSource> sources;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return AlertDialog(
      key: const ValueKey('library-upload-source-dialog'),
      title: Text(l.libUploadWhereTitle),
      content: SizedBox(
        width: 420,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              l.libUploadWhereHint,
              style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                color: KColors.textSecondary,
              ),
            ),
            const SizedBox(height: 8),
            for (final (i, s) in sources.indexed)
              ListTile(
                key: ValueKey('library-upload-source-${s.id}'),
                autofocus: i == 0,
                leading: const Icon(Icons.dns_outlined),
                title: Text(s.name),
                subtitle: Text(
                  s.address,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
                onTap: () => Navigator.of(context).pop(s),
              ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.commonCancel),
        ),
      ],
    );
  }
}

// --- adding --------------------------------------------------------------------

void _snack(BuildContext context, String text, {SnackBarAction? action}) =>
    ScaffoldMessenger.maybeOf(context)
      ?..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(text), action: action));

/// Opens «Загрузки» over the current page.
void openDownloadsScreen(BuildContext context) => Navigator.of(context).push(
  MaterialPageRoute<void>(builder: (_) => const DownloadsScreen()),
);

/// Adds [picked] (files and folders) to the library: sorts them, uploads
/// the library's part to the chosen source and photos and camera videos to
/// «Личное». Returns the library jobs. The snack bar's «Загрузки» runs
/// [onOpenDownloads] (else pushes the screen over the current page).
Future<List<UploadJob>> addToLibrary(
  BuildContext context,
  List<UploadRequest> picked, {
  VoidCallback? onOpenDownloads,
}) async {
  if (picked.isEmpty) return const [];
  final s = Services.read(context);
  final files = s.filesControl;
  final l = context.l10n;
  if (files == null) {
    _snack(context, l.libUploadUnavailable);
    return const [];
  }
  final items = [for (final r in picked) sortDropped(libraryLocalFs, r)];
  final toLibrary = items.where(
    (i) => i.kind != DropKind.personal && i.kind != DropKind.unsupported,
  );
  var jobs = <UploadJob>[];
  var plan = const LibraryPlan();
  ManagedSource? source;
  if (toLibrary.isNotEmpty) {
    source = await chooseLibrarySource(context, files, personal: s.personal);
    if (!context.mounted) return const [];
    if (source == null) {
      _snack(context, l.libUploadNowhere);
      return const [];
    }
    final id = int.parse(source.id);
    try {
      plan = await LibraryPlanner(files.files, id).planItems(items);
      jobs = await enqueuePlan(files, id, plan);
    } catch (e) {
      if (context.mounted) _snack(context, filesErrorText(l, e));
      return const [];
    }
  } else {
    // Only photos, camera videos or nothing the library holds: no source
    // is needed.
    plan = await LibraryPlanner(files.files, -1).planItems(items);
  }
  if (!context.mounted) return jobs;
  if (plan.personal.isNotEmpty) {
    await uploadToPersonal(context, plan.personal);
    if (!context.mounted) return jobs;
  }
  if (jobs.isNotEmpty) {
    final skipped = plan.skipped.length;
    final text = [
      l.libUploadStarted(jobs.length, source!.name),
      if (skipped > 0) l.libUploadSkipped(skipped),
    ].join(' ');
    _snack(
      context,
      text,
      action: SnackBarAction(
        label: l.libUploadOpenDownloads,
        onPressed: () {
          if (onOpenDownloads != null) {
            onOpenDownloads();
          } else if (context.mounted) {
            openDownloadsScreen(context);
          }
        },
      ),
    );
  } else if (plan.personal.isEmpty) {
    _snack(context, l.libUploadNothing);
  }
  return jobs;
}

/// Picks files or a folder to add to the library. Tests replace it.
typedef PickLibrary = Future<List<UploadRequest>> Function({
  required bool folder,
});

/// The system pickers. On Android and iOS picked files are copies in the
/// app's cache, deleted after the upload.
Future<List<UploadRequest>> systemPickLibrary({required bool folder}) async {
  if (folder) {
    final dir = await FilePicker.getDirectoryPath();
    return dir == null ? const [] : [UploadRequest(dir)];
  }
  final picked = await FilePicker.pickFiles(type: FileType.any);
  final copies = Platform.isAndroid || Platform.isIOS;
  return [
    for (final f in picked)
      if (f.path case final p?)
        UploadRequest(p, fileName: f.name, ownedCopy: copies),
  ];
}

/// The picker «Добавить в медиатеку» uses.
PickLibrary pickLibrary = systemPickLibrary;

/// «Добавить в медиатеку» → files or a folder → [addToLibrary].
Future<void> pickAndAddToLibrary(
  BuildContext context, {
  required bool folder,
}) async {
  final picked = await pickLibrary(folder: folder);
  if (picked.isEmpty || !context.mounted) return;
  await addToLibrary(context, picked);
}

/// The «Добавить в медиатеку» button: files or a folder.
class AddToLibraryButton extends StatelessWidget {
  const AddToLibraryButton({super.key});

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return PopupMenuButton<bool>(
      key: const ValueKey('library-upload-add'),
      tooltip: l.libUploadAdd,
      onSelected: (folder) =>
          unawaited(pickAndAddToLibrary(context, folder: folder)),
      itemBuilder: (context) => [
        PopupMenuItem(
          key: const ValueKey('library-upload-files'),
          value: false,
          child: Text(l.libUploadPickFiles),
        ),
        PopupMenuItem(
          key: const ValueKey('library-upload-folder'),
          value: true,
          child: Text(l.libUploadPickFolder),
        ),
      ],
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            const Icon(Icons.add_to_photos_outlined, size: 20),
            const SizedBox(width: 8),
            Text(l.libUploadAdd),
          ],
        ),
      ),
    );
  }
}

// --- drag and drop on the desktop -------------------------------------------

/// Whether the system drag and drop is attached: desktops, outside
/// `flutter test` (the plugin has no test binding; tests call
/// [LibraryDropZoneState.drop]).
bool get systemDropAvailable =>
    !kIsWeb &&
    (Platform.isLinux || Platform.isWindows || Platform.isMacOS) &&
    !Platform.environment.containsKey('FLUTTER_TEST');

/// The whole window takes files and folders dragged from the system: a
/// hint while they hover, then [addToLibrary].
class LibraryDropZone extends StatefulWidget {
  const LibraryDropZone({
    super.key,
    required this.child,
    this.onOpenDownloads,
    bool? enabled,
  }) : _enabled = enabled;

  final Widget child;

  /// Opens «Загрузки» from the snack bar after a drop.
  final VoidCallback? onOpenDownloads;
  final bool? _enabled;

  @override
  State<LibraryDropZone> createState() => LibraryDropZoneState();
}

class LibraryDropZoneState extends State<LibraryDropZone> {
  var _hover = false;

  /// Files are dragged over the window.
  bool get hovering => _hover;

  /// Shows or hides the hint.
  void hover(bool on) {
    if (on != _hover && mounted) setState(() => _hover = on);
  }

  /// Files or folders at [paths] were dropped.
  Future<List<UploadJob>> drop(List<String> paths) async {
    hover(false);
    if (paths.isEmpty || !mounted) return const [];
    return addToLibrary(context, [
      for (final p in paths) UploadRequest(p),
    ], onOpenDownloads: widget.onOpenDownloads);
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final stack = Stack(
      children: [
        widget.child,
        if (_hover)
          Positioned.fill(
            child: IgnorePointer(
              child: Container(
                key: const ValueKey('library-drop-hint'),
                color: KColors.bgBase.withValues(alpha: .82),
                alignment: Alignment.center,
                child: Container(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 32,
                    vertical: 24,
                  ),
                  decoration: BoxDecoration(
                    color: KColors.bgElevated,
                    borderRadius: BorderRadius.circular(16),
                    border: Border.all(color: KColors.accent, width: 2),
                  ),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Icon(
                        Icons.file_download_outlined,
                        size: 48,
                        color: KColors.accentSoft,
                      ),
                      const SizedBox(height: 12),
                      Text(
                        l.libUploadDropHint,
                        style: Theme.of(context).textTheme.titleMedium,
                        textAlign: TextAlign.center,
                      ),
                      const SizedBox(height: 4),
                      Text(
                        l.libUploadDropKinds,
                        style: Theme.of(context).textTheme.bodySmall?.copyWith(
                          color: KColors.textSecondary,
                        ),
                        textAlign: TextAlign.center,
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
      ],
    );
    if (!(widget._enabled ?? systemDropAvailable)) return stack;
    return DropTarget(
      acceptedKinds: const {DropItemKind.file, DropItemKind.directory},
      onEnter: (_) => hover(true),
      onLeave: (_) => hover(false),
      onDrop: (details) async {
        await drop([
          for (final i in details.items)
            if (i case DropFileItem(:final localPath?))
              localPath
            else if (i case DropDirectoryItem(:final localPath?))
              localPath,
        ]);
      },
      child: stack,
    );
  }
}

// --- progress on «Загрузки» ----------------------------------------------------

/// The files going to the library, on «Загрузки» (UI-018): each with its
/// progress, «Отменить», «Повторить»; a lost connection says it goes on by
/// itself; no space says so.
class LibraryUploadsSection extends StatelessWidget {
  const LibraryUploadsSection({super.key, required this.files});

  final FilesController files;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: files,
    builder: (context, _) {
      final jobs = files.libraryUploads;
      if (jobs.isEmpty) return const SizedBox.shrink();
      final l = context.l10n;
      final theme = Theme.of(context);
      final d = KDensity.of(context);
      final left = jobs
          .where(
            (j) =>
                j.state == UploadState.waiting ||
                j.state == UploadState.running,
          )
          .length;
      return Column(
        key: const ValueKey('library-uploads'),
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: EdgeInsets.fromLTRB(
              d.pageInsets.left,
              16,
              d.pageInsets.right,
              6,
            ),
            child: Wrap(
              spacing: 12,
              runSpacing: 8,
              crossAxisAlignment: WrapCrossAlignment.center,
              children: [
                Semantics(
                  header: true,
                  child: Text(
                    l.libUploadSectionTitle,
                    style: theme.textTheme.titleMedium,
                  ),
                ),
                Text(
                  left > 0 ? l.filesUploadsLeft(left) : l.filesUploadsDone,
                  style: theme.textTheme.bodySmall?.copyWith(
                    color: KColors.textSecondary,
                  ),
                ),
                if (left < jobs.length)
                  TextButton(
                    key: const ValueKey('library-uploads-clear'),
                    onPressed: files.clearFinished,
                    child: Text(l.filesClearDone),
                  ),
              ],
            ),
          ),
          for (final j in jobs) _LibraryUploadTile(files: files, job: j),
        ],
      );
    },
  );
}

class _LibraryUploadTile extends StatelessWidget {
  const _LibraryUploadTile({required this.files, required this.job});

  final FilesController files;
  final UploadJob job;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final locale = Localizations.localeOf(context);
    final d = KDensity.of(context);
    final j = job;
    final status = switch (j.state) {
      UploadState.waiting => l.filesUploadWaiting,
      UploadState.running =>
        j.size > 0
            ? '${fileSize(l, locale, j.done)} / ${fileSize(l, locale, j.size)}'
            : l.filesUploadWaiting,
      UploadState.done => l.libUploadDone(j.dir),
      UploadState.failed => switch (j.error) {
        FilesError.unavailable || FilesError.other when j.retryAt != null =>
          l.libUploadWillResume,
        FilesError.full => l.libUploadNoSpace,
        _ => l.filesUploadFailed(
          filesErrorText(l, FilesException(j.error ?? FilesError.other)),
        ),
      },
    };
    return ListTile(
      key: ValueKey('library-upload-${j.name}'),
      contentPadding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
      leading: Icon(switch (j.state) {
        UploadState.done => Icons.check_circle_outline,
        UploadState.failed when j.retryAt != null => Icons.wifi_off_rounded,
        UploadState.failed => Icons.error_outline,
        _ => Icons.upload_file,
      }),
      title: Text(j.name, maxLines: 1, overflow: TextOverflow.ellipsis),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (j.state == UploadState.running)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 4),
              child: LinearProgressIndicator(value: j.fraction),
            ),
          Text(status, key: ValueKey('library-upload-${j.name}-state')),
        ],
      ),
      trailing: switch (j.state) {
        UploadState.done => null,
        UploadState.failed => Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            IconButton(
              key: ValueKey('library-upload-retry-${j.name}'),
              tooltip: l.filesRetry,
              icon: const Icon(Icons.refresh),
              onPressed: () => files.retry(j),
            ),
            IconButton(
              key: ValueKey('library-upload-cancel-${j.name}'),
              tooltip: l.commonCancel,
              icon: const Icon(Icons.close),
              onPressed: () => unawaited(files.cancelUpload(j)),
            ),
          ],
        ),
        _ => IconButton(
          key: ValueKey('library-upload-cancel-${j.name}'),
          tooltip: l.commonCancel,
          icon: const Icon(Icons.close),
          onPressed: () => unawaited(files.cancelUpload(j)),
        ),
      },
    );
  }
}
