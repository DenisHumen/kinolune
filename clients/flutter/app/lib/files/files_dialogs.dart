// SPDX-License-Identifier: MPL-2.0
//
// UI-035: dialogs of the file view — a name, a taken name («Оставить оба»
// first), «Переместить в…» / «Копировать в…» and error texts.

import 'package:material_ui/material_ui.dart';
import 'package:pa_library/pa_library.dart'
    show FileEntry, FilesError, FilesException, NameConflict;

import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../sources/source_manager.dart' show ManagedSource;
import '../sources/source_text.dart' show sourceTypeIcon;
import 'files_controller.dart';

/// The text of a failed file operation.
String filesErrorText(AppLocalizations l, Object e) {
  final kind = e is FilesException ? e.error : FilesError.other;
  return switch (kind) {
    FilesError.notFound => l.filesErrNotFound,
    FilesError.denied => l.filesErrDenied,
    FilesError.readOnly => l.filesErrReadOnly,
    FilesError.exists => l.filesErrExists,
    FilesError.full => l.filesErrFull,
    FilesError.cancelled => l.filesErrCancelled,
    FilesError.unavailable => l.filesErrUnavailable,
    FilesError.invalid => l.filesErrInvalid,
    FilesError.other => l.filesErrOther,
  };
}

/// Asks for a name (a new folder, a rename); null if cancelled or empty.
Future<String?> askFileName(
  BuildContext context, {
  required String title,
  required String label,
  String initial = '',
}) async {
  final controller = TextEditingController(text: initial);
  final dot = initial.lastIndexOf('.');
  controller.selection = TextSelection(
    baseOffset: 0,
    extentOffset: dot > 0 ? dot : initial.length,
  );
  final name = await showDialog<String>(
    context: context,
    builder: (c) => AlertDialog(
      titleTextStyle: Theme.of(context).textTheme.titleLarge,
      title: Text(title),
      content: TextField(
        key: const ValueKey('files-name'),
        controller: controller,
        autofocus: true,
        decoration: InputDecoration(labelText: label),
        onSubmitted: (t) => Navigator.pop(c, t),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(c),
          child: Text(context.l10n.commonCancel),
        ),
        FilledButton(
          key: const ValueKey('files-name-ok'),
          onPressed: () => Navigator.pop(c, controller.text),
          child: Text(context.l10n.commonSave),
        ),
      ],
    ),
  );
  controller.dispose();
  final t = name?.trim();
  return t == null || t.isEmpty ? null : t;
}

/// A name is taken: «Оставить оба» (the default, first and focused),
/// «Заменить» (the old one goes to the trash), «Пропустить». [names] are
/// the taken names; null if the dialog was dismissed.
Future<NameConflict?> askNameConflict(
  BuildContext context,
  List<String> names, {
  bool allowReplace = true,
}) {
  final l = context.l10n;
  return showDialog<NameConflict>(
    context: context,
    builder: (c) => AlertDialog(
      key: const ValueKey('files-conflict'),
      titleTextStyle: Theme.of(context).textTheme.titleLarge,
      title: Text(l.filesConflictTitle),
      content: Text(
        names.length == 1
            ? l.filesConflictOne(names.single)
            : l.filesConflictMany(names.length),
      ),
      actions: [
        TextButton(
          key: const ValueKey('files-conflict-skip'),
          onPressed: () => Navigator.pop(c, NameConflict.skip),
          child: Text(l.filesSkip),
        ),
        if (allowReplace)
          TextButton(
            key: const ValueKey('files-conflict-replace'),
            onPressed: () => Navigator.pop(c, NameConflict.replace),
            child: Text(l.filesReplace),
          ),
        FilledButton(
          key: const ValueKey('files-conflict-keep'),
          autofocus: true,
          onPressed: () => Navigator.pop(c, NameConflict.keepBoth),
          child: Text(l.filesKeepBoth),
        ),
      ],
    ),
  );
}

/// Asks the conflict for [names] about to land in [dir] of [sourceId] if
/// any is taken there; [NameConflict.keepBoth] when none is. Null:
/// cancelled.
Future<NameConflict?> conflictFor(
  BuildContext context,
  FilesController files,
  int sourceId,
  String dir,
  Iterable<String> names, {
  bool allowReplace = true,
}) async {
  List<FileEntry> there;
  try {
    there = await files.files.list(sourceId, dir);
  } catch (_) {
    // The operation reports it; a missing folder has no conflict.
    return NameConflict.keepBoth;
  }
  final taken = {for (final e in there) e.name};
  final clash = [
    for (final n in names)
      if (taken.contains(n)) n,
  ];
  if (clash.isEmpty) return NameConflict.keepBoth;
  if (!context.mounted) return null;
  return askNameConflict(context, clash, allowReplace: allowReplace);
}

/// A folder of a writable source.
typedef FolderPick = ({int sourceId, String dir});

/// «Переместить в…» / «Копировать в…»: browse the folders of the writable
/// sources, starting at [sourceId]/[dir]; [moving] is excluded (a folder
/// cannot go into itself).
Future<FolderPick?> pickFolder(
  BuildContext context, {
  required bool move,
  required int sourceId,
  required String dir,
  String? moving,
}) => showDialog<FolderPick>(
  context: context,
  builder: (_) => _FolderPicker(
    move: move,
    sourceId: sourceId,
    dir: dir,
    moving: moving,
  ),
);

class _FolderPicker extends StatefulWidget {
  const _FolderPicker({
    required this.move,
    required this.sourceId,
    required this.dir,
    this.moving,
  });

  final bool move;
  final int sourceId;
  final String dir;
  final String? moving;

  @override
  State<_FolderPicker> createState() => _FolderPickerState();
}

class _FolderPickerState extends State<_FolderPicker> {
  int? _source;
  String _dir = '';
  List<ManagedSource>? _sources;
  List<FileEntry>? _folders;
  Object? _error;

  FilesController get _files => Services.of(context).filesControl!;

  @override
  void initState() {
    super.initState();
    _source = widget.sourceId;
    _dir = widget.dir;
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_sources == null) {
      _files.writableSources().then((s) {
        if (mounted) setState(() => _sources = s);
      });
      _load();
    }
  }

  Future<void> _load() async {
    final source = _source;
    if (source == null) return;
    setState(() {
      _folders = null;
      _error = null;
    });
    try {
      final all = await _files.files.list(source, _dir);
      final folders = [
        for (final e in all)
          if (e.isFolder &&
              !(source == widget.sourceId && e.relPath == widget.moving))
            e,
      ]..sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));
      if (mounted) setState(() => _folders = folders);
    } catch (e) {
      if (mounted) setState(() => _error = e);
    }
  }

  void _go(int? source, String dir) {
    setState(() {
      _source = source;
      _dir = dir;
    });
    _load();
  }

  String _sourceName(int id) =>
      _sources?.where((s) => s.id == '$id').firstOrNull?.name ?? '';

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final source = _source;
    final Widget body;
    if (source == null) {
      final sources = _sources;
      body = sources == null
          ? const Center(child: CircularProgressIndicator())
          : ListView(
              children: [
                for (final s in sources)
                  ListTile(
                    key: ValueKey('files-pick-source-${s.id}'),
                    leading: Icon(sourceTypeIcon(s.type)),
                    title: Text(s.name),
                    onTap: () => _go(int.parse(s.id), ''),
                  ),
              ],
            );
    } else if (_error != null) {
      body = Center(child: Text(filesErrorText(l, _error!)));
    } else if (_folders == null) {
      body = const Center(child: CircularProgressIndicator());
    } else {
      body = ListView(
        children: [
          for (final f in _folders!)
            ListTile(
              key: ValueKey('files-pick-${f.relPath}'),
              leading: const Icon(Icons.folder_outlined),
              title: Text(f.name),
              onTap: () => _go(source, f.relPath),
            ),
        ],
      );
    }
    final here = source == null
        ? l.filesChooseFolder
        : _dir.isEmpty
        ? _sourceName(source)
        : _dir.split('/').last;
    return AlertDialog(
      key: const ValueKey('files-picker'),
      titleTextStyle: Theme.of(context).textTheme.titleLarge,
      title: Row(
        children: [
          IconButton(
            key: const ValueKey('files-pick-up'),
            tooltip: l.commonBack,
            icon: const Icon(Icons.arrow_upward),
            onPressed: source == null
                ? null
                : () => _dir.isEmpty
                      ? _go(null, '')
                      : _go(
                          source,
                          _dir.contains('/')
                              ? _dir.substring(0, _dir.lastIndexOf('/'))
                              : '',
                        ),
          ),
          const SizedBox(width: 4),
          Expanded(
            child: Text(here, maxLines: 1, overflow: TextOverflow.ellipsis),
          ),
        ],
      ),
      content: SizedBox(width: 420, height: 360, child: body),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: Text(l.commonCancel),
        ),
        FilledButton(
          key: const ValueKey('files-pick-here'),
          onPressed: source == null
              ? null
              : () => Navigator.pop(context, (sourceId: source, dir: _dir)),
          child: Text(widget.move ? l.filesMoveHere : l.filesCopyHere),
        ),
      ],
    );
  }
}
