// SPDX-License-Identifier: MPL-2.0
//
// UI-035: the trash of a source (ADR-0104: `/.kinolune-trash` in the source
// itself, kept 30 days). Restore puts an entry back where it was; delete
// forever and «Очистить» ask first, since they cannot be undone.

import 'dart:async';

import 'package:intl/intl.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_library/pa_library.dart' show NameConflict, TrashEntry;

import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/cards.dart';
import 'files_controller.dart';
import 'file_size.dart';
import 'files_dialogs.dart';

class FilesTrashScreen extends StatefulWidget {
  const FilesTrashScreen({
    super.key,
    required this.sourceId,
    required this.sourceName,
  });

  final int sourceId;
  final String sourceName;

  @override
  State<FilesTrashScreen> createState() => _FilesTrashScreenState();
}

class _FilesTrashScreenState extends State<FilesTrashScreen> {
  List<TrashEntry>? _items;
  Object? _error;
  FilesController? _files;
  var _busy = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final f = Services.of(context).filesControl;
    if (identical(f, _files)) return;
    _files = f;
    unawaited(_open());
  }

  /// Old items go first (the 30-day purge), then the list.
  Future<void> _open() async {
    try {
      await _files?.files.purgeTrash(widget.sourceId);
    } catch (_) {}
    await _load();
  }

  Future<void> _load() async {
    final f = _files;
    if (f == null) return;
    try {
      final items = await f.files.trashList(widget.sourceId);
      if (mounted) {
        setState(() {
          _items = items;
          _error = null;
        });
      }
    } catch (e) {
      if (mounted) setState(() => _error = e);
    }
  }

  Future<void> _run(Future<void> Function() op) async {
    if (_busy) return;
    setState(() => _busy = true);
    try {
      await op();
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.maybeOf(context)?.showSnackBar(
          SnackBar(content: Text(filesErrorText(context.l10n, e))),
        );
      }
    } finally {
      if (mounted) setState(() => _busy = false);
      await _load();
    }
  }

  Future<bool> _confirm(String text, String action) async =>
      await showDialog<bool>(
        context: context,
        builder: (c) => AlertDialog(
          key: const ValueKey('files-confirm'),
          content: Text(text),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(c, false),
              child: Text(context.l10n.commonCancel),
            ),
            FilledButton(
              key: const ValueKey('files-confirm-ok'),
              style: FilledButton.styleFrom(
                backgroundColor: Theme.of(context).colorScheme.error,
              ),
              onPressed: () => Navigator.pop(c, true),
              child: Text(action),
            ),
          ],
        ),
      ) ??
      false;

  Future<void> _restore(TrashEntry t) async {
    final f = _files!;
    final parent = t.original.contains('/')
        ? t.original.substring(0, t.original.lastIndexOf('/'))
        : '';
    final conflict = await conflictFor(context, f, widget.sourceId, parent, [
      t.name,
    ]);
    if (conflict == null || conflict == NameConflict.skip) return;
    await _run(() async {
      final out = await f.files.restore(
        widget.sourceId,
        t.id,
        conflict: conflict,
      );
      await f.afterWrite(widget.sourceId, out);
    });
  }

  Future<void> _deleteForever(TrashEntry t) async {
    final l = context.l10n;
    if (!await _confirm(
      l.filesDeleteForeverConfirm(t.name),
      l.filesDeleteForever,
    )) {
      return;
    }
    await _run(() => _files!.files.deleteFromTrash(widget.sourceId, t.id));
  }

  Future<void> _empty() async {
    final l = context.l10n;
    final n = _items?.length ?? 0;
    if (n == 0) return;
    if (!await _confirm(l.filesEmptyTrashConfirm(n), l.filesEmptyTrash)) {
      return;
    }
    await _run(() async {
      await _files!.files.emptyTrash(widget.sourceId);
      _files!.changed();
    });
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final d = KDensity.of(context);
    final locale = Localizations.localeOf(context);
    final items = _items;
    final Widget body;
    if (_error != null) {
      body = EmptyState(
        icon: Icons.error_outline,
        title: l.filesTrash,
        message: filesErrorText(l, _error!),
      );
    } else if (items == null) {
      body = const Center(child: CircularProgressIndicator());
    } else if (items.isEmpty) {
      body = EmptyState(
        key: const ValueKey('files-trash-empty'),
        icon: Icons.delete_outline,
        title: l.filesTrashEmpty,
        message: l.filesTrashHint,
      );
    } else {
      body = ListView(
        key: const ValueKey('files-trash-list'),
        padding: EdgeInsets.fromLTRB(
          d.pageInsets.left - 8,
          0,
          d.pageInsets.right - 8,
          32,
        ),
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
            child: Text(
              l.filesTrashHint,
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ),
          for (final t in items)
            ListTile(
              key: ValueKey('files-trash-${t.id}'),
              leading: Icon(
                t.isFolder
                    ? Icons.folder_outlined
                    : Icons.insert_drive_file_outlined,
              ),
              title: Text(t.name, maxLines: 2, overflow: TextOverflow.ellipsis),
              subtitle: Text(
                [
                  if (t.original.contains('/'))
                    t.original.substring(0, t.original.lastIndexOf('/')),
                  if (!t.isFolder) fileSize(l, locale, t.sizeBytes),
                  l.filesDeletedOn(
                    DateFormat.yMMMd(locale.toLanguageTag())
                        .format(t.deletedAt),
                  ),
                ].join(' · '),
              ),
              trailing: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  IconButton(
                    key: ValueKey('files-restore-${t.id}'),
                    tooltip: l.filesRestore,
                    icon: const Icon(Icons.restore),
                    onPressed: () => _restore(t),
                  ),
                  IconButton(
                    key: ValueKey('files-forever-${t.id}'),
                    tooltip: l.filesDeleteForever,
                    icon: const Icon(Icons.delete_forever_outlined),
                    onPressed: () => _deleteForever(t),
                  ),
                ],
              ),
            ),
        ],
      );
    }
    return Scaffold(
      key: const ValueKey('files-trash-page'),
      backgroundColor: KColors.bgBase,
      appBar: AppBar(
        backgroundColor: KColors.bgBase,
        title: Text(
          l.filesTrashOf(widget.sourceName),
          style: Theme.of(context).textTheme.titleLarge,
        ),
        actions: [
          if (items?.isNotEmpty ?? false)
            TextButton.icon(
              key: const ValueKey('files-empty-trash'),
              onPressed: _empty,
              icon: const Icon(Icons.delete_sweep_outlined),
              label: Text(l.filesEmptyTrash),
            ),
        ],
      ),
      body: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (_busy) const LinearProgressIndicator(minHeight: 2),
          Expanded(child: body),
        ],
      ),
    );
  }
}
