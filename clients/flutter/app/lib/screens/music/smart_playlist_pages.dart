// SPDX-License-Identifier: MPL-2.0
//
// Smart playlists (MUS-003 [P1]): the rule editor and the playlist page,
// whose tracks are the rules evaluated over the library right now.

import 'package:material_ui/material_ui.dart';

import '../../features/music/smart_playlists.dart';
import '../../integration/library.dart';
import '../../l10n/l10n.dart';
import '../../services/app_services.dart';
import '../../widgets/focusable.dart';
import '../../widgets/undo_snack.dart';
import 'music_pages.dart';
import 'playlist_io.dart';

String smartFieldLabel(AppLocalizations l, SmartField f) => switch (f) {
  SmartField.genre => l.musicSmartFieldGenre,
  SmartField.year => l.musicSmartFieldYear,
  SmartField.plays => l.musicSmartFieldPlays,
  SmartField.format => l.musicSmartFieldFormat,
};

String smartOpLabel(AppLocalizations l, SmartOp o) => switch (o) {
  SmartOp.contains => l.musicSmartOpContains,
  SmartOp.notContains => l.musicSmartOpNotContains,
  SmartOp.equals => l.musicSmartOpIs,
  SmartOp.notEquals => l.musicSmartOpIsNot,
  SmartOp.atLeast => l.musicSmartOpAtLeast,
  SmartOp.atMost => l.musicSmartOpAtMost,
};

String smartSortLabel(AppLocalizations l, SmartSort s) => switch (s) {
  SmartSort.title => l.musicSmartSortTitle,
  SmartSort.mostPlayed => l.musicSmartSortMostPlayed,
  SmartSort.recentlyPlayed => l.musicSmartSortRecentlyPlayed,
  SmartSort.year => l.musicSmartSortYear,
};

String smartFormatLabel(AppLocalizations l, String v) => switch (v) {
  smartFormatLossless => l.musicChipLossless,
  smartFormatHiRes => l.musicChipHiRes,
  _ => v.toUpperCase(),
};

/// Opens the editor for a new ([initial] null) or an existing smart
/// playlist and saves the result in the profile's store.
Future<SmartPlaylist?> editSmartPlaylist(
  BuildContext context, {
  SmartPlaylist? initial,
}) async {
  final store = Services.read(context).smartPlaylists;
  final result = await showDialog<SmartPlaylist>(
    context: context,
    builder: (_) => _SmartEditor(
      initial:
          initial ??
          SmartPlaylist(
            id: store.newId(),
            name: '',
            rules: const [SmartRule(SmartField.genre, SmartOp.contains, '')],
          ),
    ),
  );
  if (result != null) await store.save(result);
  return result;
}

class _RuleDraft {
  _RuleDraft(SmartRule r)
    : field = r.field,
      op = r.op,
      value = TextEditingController(text: r.value);

  SmartField field;
  SmartOp op;
  final TextEditingController value;

  SmartRule get rule => SmartRule(field, op, value.text.trim());
}

class _SmartEditor extends StatefulWidget {
  const _SmartEditor({required this.initial});
  final SmartPlaylist initial;

  @override
  State<_SmartEditor> createState() => _SmartEditorState();
}

class _SmartEditorState extends State<_SmartEditor> {
  late final _name = TextEditingController(text: widget.initial.name);
  late final _limit = TextEditingController(
    text: widget.initial.limit?.toString() ?? '',
  );
  late final List<_RuleDraft> _rules = [
    for (final r in widget.initial.rules) _RuleDraft(r),
  ];
  late bool _matchAll = widget.initial.matchAll;
  late SmartSort _sort = widget.initial.sort;

  @override
  void dispose() {
    _name.dispose();
    _limit.dispose();
    for (final r in _rules) {
      r.value.dispose();
    }
    super.dispose();
  }

  void _save() {
    final name = _name.text.trim();
    if (name.isEmpty) return;
    final limit = int.tryParse(_limit.text.trim());
    Navigator.pop(
      context,
      widget.initial.copyWith(
        name: name,
        rules: [
          for (final r in _rules)
            if (r.field == SmartField.format || r.rule.value.isNotEmpty) r.rule,
        ],
        matchAll: _matchAll,
        sort: _sort,
        limit: () => limit != null && limit > 0 ? limit : null,
      ),
    );
  }

  Widget _ruleRow(AppLocalizations l, int i) {
    final r = _rules[i];
    final ops = smartOpsFor(r.field);
    return Padding(
      key: ValueKey('smart-rule-$i'),
      padding: const EdgeInsets.only(bottom: 8),
      child: Wrap(
        spacing: 8,
        runSpacing: 4,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          DropdownButton<SmartField>(
            key: ValueKey('smart-rule-$i-field'),
            value: r.field,
            items: [
              for (final f in SmartField.values)
                DropdownMenuItem(value: f, child: Text(smartFieldLabel(l, f))),
            ],
            onChanged: (f) => setState(() {
              if (f == null) return;
              r.field = f;
              if (!smartOpsFor(f).contains(r.op)) r.op = smartOpsFor(f).first;
              if (f == SmartField.format) {
                r.value.text = smartFormatLossless;
              } else if (r.value.text == smartFormatLossless ||
                  smartFormatValues.contains(r.value.text)) {
                r.value.clear();
              }
            }),
          ),
          DropdownButton<SmartOp>(
            key: ValueKey('smart-rule-$i-op'),
            value: r.op,
            items: [
              for (final o in ops)
                DropdownMenuItem(value: o, child: Text(smartOpLabel(l, o))),
            ],
            onChanged: (o) => setState(() => r.op = o ?? r.op),
          ),
          if (r.field == SmartField.format)
            DropdownButton<String>(
              key: ValueKey('smart-rule-$i-format'),
              value: smartFormatValues.contains(r.value.text)
                  ? r.value.text
                  : smartFormatLossless,
              items: [
                for (final v in smartFormatValues)
                  DropdownMenuItem(
                    value: v,
                    child: Text(smartFormatLabel(l, v)),
                  ),
              ],
              onChanged: (v) => setState(() => r.value.text = v ?? ''),
            )
          else
            SizedBox(
              width: 160,
              child: TextField(
                key: ValueKey('smart-rule-$i-value'),
                controller: r.value,
                keyboardType: r.field == SmartField.genre
                    ? TextInputType.text
                    : TextInputType.number,
                decoration: InputDecoration(labelText: l.musicSmartValue),
              ),
            ),
          IconButton(
            tooltip: l.musicSmartRemoveRule,
            icon: const Icon(Icons.remove_circle_outline),
            onPressed: () => setState(() => _rules.removeAt(i).value.dispose()),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return AlertDialog(
      title: Text(
        widget.initial.name.isEmpty ? l.musicSmartNew : l.musicSmartEdit,
      ),
      content: SizedBox(
        width: 560,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              TextField(
                key: const ValueKey('smart-name'),
                controller: _name,
                autofocus: true,
                decoration: InputDecoration(labelText: l.libraryPlaylistName),
              ),
              const SizedBox(height: 12),
              SegmentedButton<bool>(
                segments: [
                  ButtonSegment(value: true, label: Text(l.musicSmartMatchAll)),
                  ButtonSegment(
                    value: false,
                    label: Text(l.musicSmartMatchAny),
                  ),
                ],
                selected: {_matchAll},
                onSelectionChanged: (s) => setState(() => _matchAll = s.first),
              ),
              const SizedBox(height: 12),
              for (var i = 0; i < _rules.length; i++) _ruleRow(l, i),
              Align(
                alignment: AlignmentDirectional.centerStart,
                child: TextButton.icon(
                  key: const ValueKey('smart-add-rule'),
                  icon: const Icon(Icons.add),
                  label: Text(l.musicSmartAddRule),
                  onPressed: () => setState(
                    () => _rules.add(
                      _RuleDraft(
                        const SmartRule(SmartField.genre, SmartOp.contains, ''),
                      ),
                    ),
                  ),
                ),
              ),
              const SizedBox(height: 8),
              Wrap(
                spacing: 12,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: [
                  Text(l.musicSmartSortBy),
                  DropdownButton<SmartSort>(
                    key: const ValueKey('smart-sort'),
                    value: _sort,
                    items: [
                      for (final s in SmartSort.values)
                        DropdownMenuItem(
                          value: s,
                          child: Text(smartSortLabel(l, s)),
                        ),
                    ],
                    onChanged: (s) => setState(() => _sort = s ?? _sort),
                  ),
                  SizedBox(
                    width: 140,
                    child: TextField(
                      key: const ValueKey('smart-limit'),
                      controller: _limit,
                      keyboardType: TextInputType.number,
                      decoration: InputDecoration(labelText: l.musicSmartLimit),
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: Text(l.commonCancel),
        ),
        FilledButton(
          key: const ValueKey('smart-save'),
          onPressed: _save,
          child: Text(l.musicSmartSave),
        ),
      ],
    );
  }
}

/// A smart playlist page: its rules over the library tracks of [index].
class SmartPlaylistScreen extends StatelessWidget {
  const SmartPlaylistScreen({
    super.key,
    required this.index,
    required this.playlistId,
  });

  final MusicIndex index;
  final String playlistId;

  @override
  Widget build(BuildContext context) {
    final store = Services.read(context).smartPlaylists;
    final l = context.l10n;
    return ValueListenableBuilder<List<SmartPlaylist>>(
      valueListenable: store.playlists,
      builder: (context, lists, _) {
        final p = lists.where((x) => x.id == playlistId).firstOrNull;
        if (p == null) return const SizedBox.shrink();
        final tracks = p.evaluate(index.tracks);
        return TrackListScreen(
          overline: l.musicSmartLabel,
          title: p.name,
          tracks: tracks,
          artUrl: tracks.isEmpty ? null : tracks.first.item.posterUrl,
          artSeed: p.name,
          actions: [
            KButton(
              key: const ValueKey('smart-edit'),
              label: l.musicSmartEdit,
              icon: Icons.tune,
              onPressed: () => editSmartPlaylist(context, initial: p),
            ),
            KButton(
              key: const ValueKey('smart-export'),
              label: l.musicM3uExport,
              icon: Icons.file_upload_outlined,
              onPressed: tracks.isEmpty
                  ? null
                  : () => exportM3u(context, p.name, tracks),
            ),
            KButton(
              key: const ValueKey('smart-delete'),
              label: l.musicSmartDelete,
              icon: Icons.delete_outline,
              // QA-015: at once, with «Отменить» (the files' pattern).
              onPressed: () async {
                final nav = Navigator.of(context);
                final messenger = ScaffoldMessenger.maybeOf(context);
                await store.delete(p.id);
                nav.maybePop();
                final undone = await showUndoSnack(
                  messenger,
                  message: l.musicSmartDeleted(p.name),
                  undoLabel: l.commonUndo,
                );
                if (undone) await store.save(p);
              },
            ),
          ],
        );
      },
    );
  }
}
