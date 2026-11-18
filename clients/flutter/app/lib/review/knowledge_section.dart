// SPDX-License-Identifier: MPL-2.0
//
// LIB-014: «Сохранить в файлах» on the «Исправить распознавание» page.
// «Записать NFO» puts what the library knows (title, plot, provider ids)
// next to the files in the Kodi/Jellyfin format; «Добавить ID в имя
// папки» appends ` [tmdbid-…]` to the work's own folder. Both are the
// user's command, both show where they write first; an existing NFO is
// replaced only after a question, and the old one goes to the source's
// trash. Read-only sources are left out by the core (ADR-0122).

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';

/// The section; nothing for music or unknown items.
class KnowledgeSection extends StatefulWidget {
  const KnowledgeSection({
    super.key,
    required this.inbox,
    required this.itemId,
  });

  final ReviewInbox inbox;

  /// The work's item id (`m<work>`, `s<work>`).
  final String itemId;

  @override
  State<KnowledgeSection> createState() => _KnowledgeSectionState();
}

class _KnowledgeSectionState extends State<KnowledgeSection> {
  KnowledgePlan? _plan;
  bool _busy = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    KnowledgePlan? p;
    try {
      p = await widget.inbox.knowledgePlan(widget.itemId);
    } catch (_) {
      p = null;
    }
    if (mounted) setState(() => _plan = p);
  }

  Future<void> _run(Future<String> Function() action) async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    final messenger = ScaffoldMessenger.maybeOf(context);
    final failed = context.l10n.reviewFolderFailed;
    try {
      final done = await action();
      messenger?.showSnackBar(SnackBar(content: Text(done)));
    } catch (e) {
      if (mounted) setState(() => _error = failed('$e'));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
    if (mounted) await _load();
  }

  Future<bool> _confirmReplace() async {
    final l = context.l10n;
    return await showDialog<bool>(
          context: context,
          builder: (c) => AlertDialog(
            key: const ValueKey('knowledge-replace-dialog'),
            title: Text(l.reviewNfoReplaceTitle),
            content: Text(l.reviewNfoReplaceBody),
            actions: [
              TextButton(
                key: const ValueKey('knowledge-replace-cancel'),
                onPressed: () => Navigator.pop(c, false),
                child: Text(l.commonCancel),
              ),
              FilledButton(
                key: const ValueKey('knowledge-replace-ok'),
                onPressed: () => Navigator.pop(c, true),
                child: Text(l.reviewNfoReplace),
              ),
            ],
          ),
        ) ??
        false;
  }

  Future<void> _export() async {
    final p = _plan;
    if (p == null) return;
    final replace = p.existingNfo.isNotEmpty;
    if (replace && !await _confirmReplace()) return;
    if (!mounted) return;
    final l = context.l10n;
    await _run(() async {
      final r = await widget.inbox.exportNfo(widget.itemId, replace: replace);
      return r.written > 0 ? l.reviewNfoDone : l.reviewNfoNothing;
    });
  }

  Future<void> _rename() async {
    final l = context.l10n;
    await _run(() async {
      final n = await widget.inbox.renameWithIds(widget.itemId);
      return n > 0 ? l.reviewIdRenameDone : l.reviewIdRenameNothing;
    });
  }

  @override
  Widget build(BuildContext context) {
    final p = _plan;
    if (p == null) return const SizedBox.shrink();
    final l = context.l10n;
    final theme = Theme.of(context);
    final small = theme.textTheme.bodySmall;
    final quiet = small?.copyWith(color: KColors.textTertiary);
    return FocusTraversalGroup(
      child: Column(
        key: const ValueKey('knowledge'),
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(l.reviewKnowledgeSection, style: theme.textTheme.titleSmall),
          const SizedBox(height: 4),
          Text(l.reviewKnowledgeHint, style: small),
          const SizedBox(height: 8),
          if (p.ids.isEmpty)
            Text(
              l.reviewKnowledgeNoIds,
              key: const ValueKey('knowledge-no-ids'),
              style: small,
            )
          else ...[
            if (p.nfoPaths.isNotEmpty) ...[
              KButton(
                key: const ValueKey('knowledge-nfo'),
                label: l.reviewNfoExport,
                icon: Icons.description_outlined,
                onPressed: _busy ? null : _export,
              ),
              const SizedBox(height: 4),
              for (final path in p.nfoPaths)
                Text(
                  p.existingNfo.contains(path) ? l.reviewNfoExists(path) : path,
                  style: quiet,
                ),
            ],
            if (p.renames.isNotEmpty) ...[
              const SizedBox(height: 12),
              KButton(
                key: const ValueKey('knowledge-rename'),
                label: l.reviewIdRename,
                icon: Icons.drive_file_rename_outline,
                onPressed: _busy ? null : _rename,
              ),
              const SizedBox(height: 4),
              for (final (_, name) in p.renames)
                Text(l.reviewIdRenameHint(name), style: quiet),
            ],
            if (p.nfoPaths.isEmpty && p.renames.isEmpty && !p.readOnly)
              Text(
                l.reviewKnowledgeNowhere,
                key: const ValueKey('knowledge-nowhere'),
                style: small,
              ),
          ],
          if (p.readOnly)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(
                l.reviewKnowledgeReadOnly,
                key: const ValueKey('knowledge-read-only'),
                style: small,
              ),
            ),
          if (_error != null)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(
                _error!,
                key: const ValueKey('knowledge-error'),
                style: small?.copyWith(color: theme.colorScheme.error),
              ),
            ),
        ],
      ),
    );
  }
}
