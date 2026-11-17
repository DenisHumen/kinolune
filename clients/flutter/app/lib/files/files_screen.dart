// SPDX-License-Identifier: MPL-2.0
//
// UI-035 (ADR-0099, ADR-0104): the file view «как в Nextcloud» of a
// writable source or of a personal library's folder: upload from this
// device (gallery or files, several at once, in the background with
// progress), new folder, rename, «Переместить в…» and «Копировать в…»
// (drag and drop onto a folder or a crumb on desktop), delete to the
// trash with «Отменить», and the trash with restore and «Очистить».
// A taken name asks «Оставить оба» (the default) / «Заменить» /
// «Пропустить».

import 'dart:async';
import 'dart:io' show Platform;

import 'package:file_picker/file_picker.dart';
import 'package:intl/intl.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_library/pa_library.dart'
    show FileEntry, FileOutcome, FilesError, FilesException, NameConflict;

import '../l10n/l10n.dart';
import '../platform/form_factor.dart';
import '../screens/detail_sections.dart' show formatSize;
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/cards.dart';
import 'files_controller.dart';
import 'files_dialogs.dart';
import 'files_trash_screen.dart';
import 'uploads_panel.dart';

/// Picks local files to upload: `media` (gallery: photos and videos) or
/// any file. Tests replace it.
typedef PickUploads = Future<List<UploadRequest>> Function({required bool media});

/// The system pickers (several files at once). On Android and iOS the
/// picker hands copies in the app's cache, so they are deleted after the
/// upload.
Future<List<UploadRequest>> systemPickUploads({required bool media}) async {
  final picked = await FilePicker.pickFiles(
    type: media ? FileType.media : FileType.any,
  );
  final copies = Platform.isAndroid || Platform.isIOS;
  return [
    for (final f in picked)
      if (f.path case final p?) UploadRequest(p, fileName: f.name, ownedCopy: copies),
  ];
}

/// The picker the file view uses ([systemPickUploads] unless a test sets
/// another one).
PickUploads pickUploads = systemPickUploads;

/// Opens the file view of [sourceId] at [dir]. [root] is the top the view
/// may go up to (a personal library's folder); [title] names it.
Future<void> openFiles(
  BuildContext context, {
  required int sourceId,
  required String sourceName,
  String dir = '',
  String root = '',
  String? title,
}) => Navigator.of(context).push(
  MaterialPageRoute<void>(
    builder: (_) => FilesScreen(
      sourceId: sourceId,
      sourceName: sourceName,
      dir: dir,
      root: root,
      title: title,
    ),
  ),
);

/// The file view.
class FilesScreen extends StatefulWidget {
  const FilesScreen({
    super.key,
    required this.sourceId,
    required this.sourceName,
    this.dir = '',
    this.root = '',
    this.title,
  });

  final int sourceId;
  final String sourceName;
  final String dir;
  final String root;
  final String? title;

  @override
  State<FilesScreen> createState() => _FilesScreenState();
}

class _FilesScreenState extends State<FilesScreen> {
  late String _dir = widget.dir;
  List<FileEntry>? _entries;
  Object? _error;
  FilesController? _files;
  StreamSubscription<void>? _sub;
  var _busy = false;

  FilesController get files => _files!;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final f = Services.of(context).filesControl;
    if (identical(f, _files)) return;
    _sub?.cancel();
    _files = f;
    _sub = f?.changes.listen((_) => _load());
    _load();
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  Future<void> _load() async {
    final f = _files;
    if (f == null) return;
    final dir = _dir;
    try {
      final entries = await f.files.list(widget.sourceId, dir);
      entries.sort((a, b) {
        if (a.isFolder != b.isFolder) return a.isFolder ? -1 : 1;
        return a.name.toLowerCase().compareTo(b.name.toLowerCase());
      });
      if (mounted && dir == _dir) {
        setState(() {
          _entries = entries;
          _error = null;
        });
      }
    } catch (e) {
      if (mounted && dir == _dir) setState(() => _error = e);
    }
  }

  void _go(String dir) {
    setState(() {
      _dir = dir;
      _entries = null;
      _error = null;
    });
    _load();
  }

  bool get _atRoot => _dir == widget.root;

  void _up() {
    if (_atRoot) {
      Navigator.of(context).maybePop();
      return;
    }
    _go(_dir.contains('/') ? _dir.substring(0, _dir.lastIndexOf('/')) : '');
  }

  void _snack(String text, {SnackBarAction? action}) {
    ScaffoldMessenger.maybeOf(context)
      ?..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(text), action: action));
  }

  Future<void> _run(Future<void> Function() op) async {
    if (_busy) return;
    setState(() => _busy = true);
    try {
      await op();
    } on FilesException catch (e) {
      if (mounted && e.error != FilesError.cancelled) {
        _snack(filesErrorText(context.l10n, e));
      }
    } catch (e) {
      if (mounted) _snack(filesErrorText(context.l10n, e));
    } finally {
      if (mounted) setState(() => _busy = false);
      await _load();
    }
  }

  // --- actions -----------------------------------------------------------

  Future<void> _upload({required bool media}) async {
    final List<UploadRequest> picked;
    try {
      picked = await pickUploads(media: media);
    } catch (e) {
      if (mounted) _snack(filesErrorText(context.l10n, e));
      return;
    }
    if (picked.isEmpty || !mounted) return;
    await uploadInto(context, files, picked, widget.sourceId, _dir);
  }

  Future<void> _chooseUpload() async {
    if (FormFactorScope.of(context) == FormFactor.desktop) {
      return _upload(media: false);
    }
    final l = context.l10n;
    final media = await showModalBottomSheet<bool>(
      context: context,
      builder: (c) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              key: const ValueKey('files-upload-gallery'),
              leading: const Icon(Icons.photo_library_outlined),
              title: Text(l.filesFromGallery),
              onTap: () => Navigator.pop(c, true),
            ),
            ListTile(
              key: const ValueKey('files-upload-files'),
              leading: const Icon(Icons.folder_open_outlined),
              title: Text(l.filesFromFiles),
              onTap: () => Navigator.pop(c, false),
            ),
          ],
        ),
      ),
    );
    if (media != null) await _upload(media: media);
  }

  Future<void> _newFolder() async {
    final l = context.l10n;
    final name = await askFileName(
      context,
      title: l.filesNewFolder,
      label: l.filesFolderName,
    );
    if (name == null || !mounted) return;
    final taken = _entries?.any((e) => e.name == name) ?? false;
    var conflict = NameConflict.keepBoth;
    if (taken) {
      final c = await askNameConflict(context, [name], allowReplace: false);
      if (c == null || c == NameConflict.skip) return;
      conflict = c;
    }
    await _run(() async {
      final out = await files.files.createFolder(
        widget.sourceId,
        _dir,
        name,
        conflict: conflict,
      );
      await files.afterWrite(widget.sourceId, out);
    });
  }

  Future<void> _rename(FileEntry e) async {
    final l = context.l10n;
    final name = await askFileName(
      context,
      title: l.filesRename,
      label: l.filesNewName,
      initial: e.name,
    );
    if (name == null || name == e.name || !mounted) return;
    var conflict = NameConflict.keepBoth;
    if (_entries?.any((x) => x.name == name) ?? false) {
      final c = await askNameConflict(context, [name]);
      if (c == null || c == NameConflict.skip) return;
      conflict = c;
    }
    await _run(() async {
      final out = await files.files.rename(
        widget.sourceId,
        e.relPath,
        name,
        conflict: conflict,
      );
      await files.afterWrite(widget.sourceId, out);
    });
  }

  Future<void> _transfer(FileEntry e, {required bool move}) async {
    final pick = await pickFolder(
      context,
      move: move,
      sourceId: widget.sourceId,
      dir: _dir,
      moving: e.isFolder ? e.relPath : null,
    );
    if (pick == null || !mounted) return;
    await _transferTo(e, pick.sourceId, pick.dir, move: move);
  }

  Future<void> _transferTo(
    FileEntry e,
    int toSource,
    String toDir, {
    required bool move,
  }) async {
    final parent = e.relPath.contains('/')
        ? e.relPath.substring(0, e.relPath.lastIndexOf('/'))
        : '';
    if (move && toSource == widget.sourceId && toDir == parent) return;
    final conflict = await conflictFor(
      context,
      files,
      toSource,
      toDir,
      [e.name],
    );
    if (conflict == null || conflict == NameConflict.skip || !mounted) return;
    final progress = ValueNotifier<double?>(null);
    final task = 'tr-${DateTime.now().microsecondsSinceEpoch}';
    final l = context.l10n;
    final dialog = showDialog<void>(
      context: context,
      barrierDismissible: false,
      builder: (c) => AlertDialog(
        key: const ValueKey('files-transfer'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(move ? l.filesMoving(e.name) : l.filesCopying(e.name)),
            const SizedBox(height: 16),
            ValueListenableBuilder<double?>(
              valueListenable: progress,
              builder: (_, v, _) => LinearProgressIndicator(value: v),
            ),
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => files.files.cancel(task),
            child: Text(l.commonCancel),
          ),
        ],
      ),
    );
    var closed = false;
    unawaited(dialog.whenComplete(() => closed = true));
    await _run(() async {
      try {
        final out = await files.files.transfer(
          taskId: task,
          move: move,
          fromSource: widget.sourceId,
          relPath: e.relPath,
          toSource: toSource,
          toDir: toDir,
          conflict: conflict,
          onProgress: (d, t) => progress.value = t > 0 ? d / t : null,
        );
        await files.afterWrite(
          toSource,
          out,
          fromSource: toSource == widget.sourceId ? null : widget.sourceId,
        );
      } finally {
        if (!closed && mounted) {
          Navigator.of(context, rootNavigator: true).pop();
        }
        progress.dispose();
      }
    });
  }

  Future<void> _trash(FileEntry e) async {
    final l = context.l10n;
    FileOutcome? out;
    await _run(() async {
      out = await files.files.trash(widget.sourceId, e.relPath);
      await files.afterWrite(widget.sourceId, out!);
    });
    if (out == null || !mounted) return;
    _snack(
      l.filesMovedToTrash(e.name),
      action: SnackBarAction(
        key: const ValueKey('files-undo'),
        label: l.filesUndo,
        onPressed: () => _undoTrash(e),
      ),
    );
  }

  Future<void> _undoTrash(FileEntry e) async {
    await _run(() async {
      final items = await files.files.trashList(widget.sourceId);
      final item = items.where((t) => t.original == e.relPath).firstOrNull;
      if (item == null) return;
      final out = await files.files.restore(widget.sourceId, item.id);
      await files.afterWrite(widget.sourceId, out);
    });
  }

  Future<void> _menu(FileEntry e, String action) => switch (action) {
    'rename' => _rename(e),
    'move' => _transfer(e, move: true),
    'copy' => _transfer(e, move: false),
    'trash' => _trash(e),
    _ => Future.value(),
  };

  // --- build -------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final d = KDensity.of(context);
    final desktop = FormFactorScope.of(context) == FormFactor.desktop;
    final entries = _entries;
    final Widget body;
    if (_files == null) {
      body = EmptyState(icon: Icons.folder_off_outlined, title: l.filesTitle);
    } else if (_error != null) {
      body = EmptyState(
        icon: Icons.error_outline,
        title: l.filesTitle,
        message: filesErrorText(l, _error!),
        action: OutlinedButton(onPressed: _load, child: Text(l.filesRetry)),
      );
    } else if (entries == null) {
      body = const Center(child: CircularProgressIndicator());
    } else if (entries.isEmpty) {
      body = EmptyState(
        key: const ValueKey('files-empty'),
        icon: Icons.drive_folder_upload_outlined,
        title: l.filesEmptyFolder,
        message: l.filesEmptyFolderHint,
        action: FilledButton.icon(
          key: const ValueKey('files-upload-empty'),
          onPressed: _chooseUpload,
          icon: const Icon(Icons.upload),
          label: Text(l.filesUpload),
        ),
      );
    } else {
      body = ListView.builder(
        key: const ValueKey('files-list'),
        padding: EdgeInsets.fromLTRB(
          d.pageInsets.left - 8,
          0,
          d.pageInsets.right - 8,
          96,
        ),
        itemCount: entries.length,
        itemBuilder: (_, i) => _row(entries[i], desktop),
      );
    }
    return Scaffold(
      key: const ValueKey('files-page'),
      backgroundColor: KColors.bgBase,
      appBar: AppBar(
        backgroundColor: KColors.bgBase,
        leading: IconButton(
          key: const ValueKey('files-back'),
          tooltip: l.commonBack,
          icon: const Icon(Icons.arrow_back),
          onPressed: _up,
        ),
        title: Text(
          _atRoot
              ? (widget.title ?? widget.sourceName)
              : _dir.split('/').last,
          style: Theme.of(context).textTheme.titleLarge,
        ),
        actions: [
          IconButton(
            key: const ValueKey('files-new-folder'),
            tooltip: l.filesNewFolder,
            icon: const Icon(Icons.create_new_folder_outlined),
            onPressed: _files == null ? null : _newFolder,
          ),
          IconButton(
            key: const ValueKey('files-open-trash'),
            tooltip: l.filesTrash,
            icon: const Icon(Icons.delete_outline),
            onPressed: _files == null
                ? null
                : () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) => FilesTrashScreen(
                        sourceId: widget.sourceId,
                        sourceName: widget.sourceName,
                      ),
                    ),
                  ),
          ),
        ],
      ),
      floatingActionButton: _files == null
          ? null
          : FloatingActionButton.extended(
              key: const ValueKey('files-upload'),
              onPressed: _chooseUpload,
              icon: const Icon(Icons.upload),
              label: Text(l.filesUpload),
            ),
      body: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _Crumbs(
            title: widget.title ?? widget.sourceName,
            root: widget.root,
            dir: _dir,
            onGo: _go,
            onDrop: desktop ? (e, dir) => _dropInto(e, dir) : null,
          ),
          if (_busy) const LinearProgressIndicator(minHeight: 2),
          Expanded(child: body),
          if (_files case final f?)
            UploadsPanel(controller: f, sourceId: widget.sourceId),
        ],
      ),
    );
  }

  Future<void> _dropInto(FileEntry e, String dir) async {
    if (dir == e.relPath || dir.startsWith('${e.relPath}/')) return;
    await _transferTo(e, widget.sourceId, dir, move: true);
  }

  Widget _row(FileEntry e, bool desktop) {
    final l = context.l10n;
    final locale = Localizations.localeOf(context);
    final sub = [
      if (!e.isFolder) formatSize(l, locale, e.sizeBytes),
      if (e.modified case final m?)
        DateFormat.yMMMd(locale.toLanguageTag()).format(m),
    ].join(' · ');
    Widget tile(bool dragging) => ListTile(
      key: ValueKey('files-entry-${e.relPath}'),
      leading: Icon(_iconOf(e), color: e.isFolder ? KColors.accent : null),
      title: Text(e.name, maxLines: 2, overflow: TextOverflow.ellipsis),
      subtitle: sub.isEmpty ? null : Text(sub),
      enabled: !dragging,
      onTap: e.isFolder ? () => _go(e.relPath) : null,
      trailing: PopupMenuButton<String>(
        key: ValueKey('files-menu-${e.relPath}'),
        onSelected: (a) => _menu(e, a),
        itemBuilder: (_) => [
          PopupMenuItem(
            key: const ValueKey('files-action-rename'),
            value: 'rename',
            child: Text(l.filesRename),
          ),
          PopupMenuItem(
            key: const ValueKey('files-action-move'),
            value: 'move',
            child: Text(l.filesMoveTo),
          ),
          PopupMenuItem(
            key: const ValueKey('files-action-copy'),
            value: 'copy',
            child: Text(l.filesCopyTo),
          ),
          PopupMenuItem(
            key: const ValueKey('files-action-trash'),
            value: 'trash',
            child: Text(l.filesDelete),
          ),
        ],
      ),
    );
    if (!desktop) return tile(false);
    Widget row = Draggable<FileEntry>(
      data: e,
      feedback: Material(
        color: KColors.bgElevated,
        borderRadius: BorderRadius.circular(8),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(_iconOf(e)),
              const SizedBox(width: 8),
              Text(e.name),
            ],
          ),
        ),
      ),
      childWhenDragging: tile(true),
      child: tile(false),
    );
    if (e.isFolder) {
      row = DragTarget<FileEntry>(
        onWillAcceptWithDetails: (d) =>
            d.data.relPath != e.relPath &&
            !e.relPath.startsWith('${d.data.relPath}/'),
        onAcceptWithDetails: (d) => _dropInto(d.data, e.relPath),
        builder: (_, candidates, _) => DecoratedBox(
          decoration: BoxDecoration(
            color: candidates.isEmpty
                ? null
                : KColors.accent.withValues(alpha: 0.18),
            borderRadius: BorderRadius.circular(8),
          ),
          child: row,
        ),
      );
    }
    return row;
  }
}

IconData _iconOf(FileEntry e) {
  if (e.isFolder) return Icons.folder;
  final ext = e.name.contains('.')
      ? e.name.substring(e.name.lastIndexOf('.') + 1).toLowerCase()
      : '';
  return switch (ext) {
    'mp4' || 'mkv' || 'mov' || 'avi' || 'webm' || 'm4v' || 'ts' || '3gp' =>
      Icons.movie_outlined,
    'jpg' || 'jpeg' || 'png' || 'heic' || 'webp' || 'gif' => Icons.image_outlined,
    'mp3' || 'flac' || 'm4a' || 'ogg' || 'opus' || 'wav' || 'm4b' =>
      Icons.audiotrack_outlined,
    _ => Icons.insert_drive_file_outlined,
  };
}

/// Breadcrumbs from [root]; on desktop each crumb takes a dropped entry.
class _Crumbs extends StatelessWidget {
  const _Crumbs({
    required this.title,
    required this.root,
    required this.dir,
    required this.onGo,
    this.onDrop,
  });

  final String title;
  final String root;
  final String dir;
  final ValueChanged<String> onGo;
  final void Function(FileEntry e, String dir)? onDrop;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final rest = dir.length > root.length
        ? dir.substring(root.isEmpty ? 0 : root.length + 1).split('/')
        : const <String>[];
    final crumbs = <(String, String)>[(title, root)];
    var at = root;
    for (final p in rest) {
      at = at.isEmpty ? p : '$at/$p';
      crumbs.add((p, at));
    }
    return Padding(
      padding: EdgeInsets.fromLTRB(d.pageInsets.left, 0, d.pageInsets.right, 8),
      child: Wrap(
        key: const ValueKey('files-crumbs'),
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          for (final (i, (name, path)) in crumbs.indexed) ...[
            if (i > 0)
              const Icon(Icons.chevron_right, size: 18, color: Colors.white54),
            _crumb(context, name, path, i == crumbs.length - 1),
          ],
        ],
      ),
    );
  }

  Widget _crumb(BuildContext context, String name, String path, bool current) {
    Widget chip(bool hot) => TextButton(
      key: ValueKey('files-crumb-$path'),
      style: TextButton.styleFrom(
        backgroundColor: hot ? KColors.accent.withValues(alpha: 0.18) : null,
        foregroundColor: current ? Colors.white : KColors.textSecondary,
      ),
      onPressed: current ? null : () => onGo(path),
      child: Text(name, maxLines: 1, overflow: TextOverflow.ellipsis),
    );
    final drop = onDrop;
    if (drop == null || current) return chip(false);
    return DragTarget<FileEntry>(
      onAcceptWithDetails: (d) => drop(d.data, path),
      builder: (_, c, _) => chip(c.isNotEmpty),
    );
  }
}

/// Uploads [picked] into [dir] of [sourceId]: asks once if names are
/// taken there, then queues them; tells how many go.
Future<void> uploadInto(
  BuildContext context,
  FilesController files,
  List<UploadRequest> picked,
  int sourceId,
  String dir, {
  String? placeName,
}) async {
  final conflict = await conflictFor(
    context,
    files,
    sourceId,
    dir,
    picked.map((p) => p.name),
  );
  if (conflict == null || !context.mounted) return;
  await files.enqueue(
    picked,
    sourceId: sourceId,
    dir: dir,
    conflict: conflict,
  );
  if (!context.mounted) return;
  final l = context.l10n;
  ScaffoldMessenger.maybeOf(context)
    ?..hideCurrentSnackBar()
    ..showSnackBar(
      SnackBar(
        content: Text(
          placeName == null
              ? l.filesUploadingCount(picked.length)
              : l.filesUploadingTo(picked.length, placeName),
        ),
      ),
    );
}
