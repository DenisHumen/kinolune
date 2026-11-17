// SPDX-License-Identifier: MPL-2.0
//
// META-035: «Тип и нумерация» on the «Исправить распознавание» page. The
// type of the work (фильм / сериал / аниме), the numbering of a series
// («Серия в имени + N = серия сезона S», or a running count) and the
// folder rule its fixes taught (META-017, ADR-0112) with «Забыть правило
// папки». Everything applies to the whole folder, new files too. Kept as
// simple as possible: the numbering is folded into one line and has
// nothing to touch unless the episodes land in the wrong place.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import '../widgets/settings_tiles.dart';

/// `+ 12`, `− 28`, `+ 0`.
String signedOffset(int n) => n < 0 ? '− ${-n}' : '+ $n';

/// The current numbering in words.
String numberingSummary(AppLocalizations l, FolderRule? r) {
  if (r == null || (!r.absolute && r.season == null && r.offset == 0)) {
    return l.reviewNumberingAsNamed;
  }
  final o = signedOffset(r.offset);
  if (r.absolute) return l.reviewNumberingRuleAbsolute(o);
  final s = r.season;
  return s == null
      ? l.reviewNumberingRuleSameSeason(o)
      : l.reviewNumberingRule(o, s);
}

/// The section; nothing for music, unknown items or a library without
/// folder rules. [onKindChanged] runs after the type changed (the work is
/// looked up anew, the page should close).
class FolderRulesSection extends StatefulWidget {
  const FolderRulesSection({
    super.key,
    required this.inbox,
    required this.itemId,
    this.onKindChanged,
  });

  final ReviewInbox inbox;

  /// The work's item id (`m<work>`, `s<work>`).
  final String itemId;
  final VoidCallback? onKindChanged;

  @override
  State<FolderRulesSection> createState() => _FolderRulesSectionState();
}

class _FolderRulesSectionState extends State<FolderRulesSection> {
  FolderRules? _rules;
  bool _editing = false;
  bool _busy = false;
  String? _error;

  // The numbering being edited.
  final _offset = TextEditingController(text: '0');
  final _season = TextEditingController();
  bool _absolute = false;

  @override
  void initState() {
    super.initState();
    _offset.addListener(_changed);
    _season.addListener(_changed);
    _load();
  }

  @override
  void dispose() {
    _offset.dispose();
    _season.dispose();
    super.dispose();
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  Future<void> _load() async {
    FolderRules? r;
    try {
      r = await widget.inbox.folderRules(widget.itemId);
    } catch (_) {
      r = null;
    }
    if (!mounted) return;
    setState(() => _rules = r);
    _reset();
  }

  /// The editor starts on the numbering the work follows now.
  void _reset() {
    final n = _rules?.numbering;
    _offset.text = '${n?.offset ?? 0}';
    _season.text = n?.season?.toString() ?? '';
    setState(() => _absolute = n?.absolute ?? false);
  }

  int get _offsetValue => int.tryParse(_offset.text.trim()) ?? 0;
  int? get _seasonValue => int.tryParse(_season.text.trim());

  bool get _dirty {
    final n = _rules?.numbering;
    return _offsetValue != (n?.offset ?? 0) ||
        _absolute != (n?.absolute ?? false) ||
        (!_absolute && _seasonValue != n?.season);
  }

  Future<void> _run(Future<void> Function() action, String done) async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    final messenger = ScaffoldMessenger.maybeOf(context);
    final failed = context.l10n.reviewFolderFailed;
    try {
      await action();
      messenger?.showSnackBar(SnackBar(content: Text(done)));
    } catch (e) {
      if (mounted) setState(() => _error = failed('$e'));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _setKind(String kind, String label) async {
    var ok = false;
    await _run(() async {
      await widget.inbox.setKind(widget.itemId, kind);
      ok = true;
    }, context.l10n.reviewKindDone(label));
    if (!mounted || !ok) return;
    final changed = widget.onKindChanged;
    if (changed != null) {
      changed();
    } else {
      await _load();
    }
  }

  Future<void> _apply() async {
    await _run(
      () => widget.inbox.setNumbering(
        widget.itemId,
        season: _absolute ? null : _seasonValue,
        offset: _offsetValue,
        absolute: _absolute,
      ),
      context.l10n.reviewNumberingDone,
    );
    if (!mounted || _error != null) return;
    setState(() => _editing = false);
    await _load();
  }

  Future<void> _forget() async {
    await _run(
      () => widget.inbox.forgetFolderRules(widget.itemId),
      context.l10n.reviewFolderForgotten,
    );
    if (mounted) await _load();
  }

  void _stepOffset(int by) {
    _offset.text = '${(_offsetValue + by).clamp(-10000, 10000)}';
  }

  void _stepSeason(int by) {
    final s = _seasonValue;
    final next = switch ((s, by > 0)) {
      (null, true) => 1,
      (null, false) => null,
      (0, false) => null,
      (final v?, _) => (v + by).clamp(0, 1000),
    };
    _season.text = next?.toString() ?? '';
  }

  @override
  Widget build(BuildContext context) {
    final r = _rules;
    if (r == null) return const SizedBox.shrink();
    final l = context.l10n;
    final theme = Theme.of(context);
    final kinds = [
      ('movie', l.reviewKindMovie),
      ('show', l.reviewKindShow),
      ('anime', l.reviewKindAnime),
    ];
    final series = r.kind != 'movie';
    return FocusTraversalGroup(
      child: Column(
        key: const ValueKey('folder-rules'),
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(l.reviewFolderSection, style: theme.textTheme.titleSmall),
          const SizedBox(height: 8),
          Text(l.reviewKindLabel, style: theme.textTheme.labelMedium),
          const SizedBox(height: 6),
          Wrap(
            spacing: 8,
            runSpacing: 8,
            children: [
              for (final (kind, label) in kinds)
                _Choice(
                  key: ValueKey('folder-kind-$kind'),
                  label: label,
                  selected: r.kind == kind,
                  onPressed: _busy || r.kind == kind
                      ? null
                      : () => _setKind(kind, label),
                ),
            ],
          ),
          if (series) ...[
            const SizedBox(height: 14),
            Text(l.reviewNumberingLabel, style: theme.textTheme.labelMedium),
            const SizedBox(height: 4),
            if (!_editing)
              Wrap(
                spacing: 12,
                runSpacing: 8,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: [
                  Text(
                    numberingSummary(l, r.numbering),
                    key: const ValueKey('folder-numbering-summary'),
                    style: theme.textTheme.bodyMedium,
                  ),
                  KButton(
                    key: const ValueKey('folder-numbering-edit'),
                    label: l.reviewNumberingEdit,
                    icon: Icons.tune,
                    onPressed: _busy
                        ? null
                        : () {
                            _reset();
                            setState(() => _editing = true);
                          },
                  ),
                ],
              )
            else
              _editor(l, theme),
          ],
          if (r.rules.isNotEmpty) ...[
            const SizedBox(height: 14),
            for (final rule in r.rules)
              Padding(
                padding: const EdgeInsets.only(bottom: 6),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Icon(
                      Icons.folder_outlined,
                      size: 18,
                      color: KColors.textTertiary,
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        rule.folder.isEmpty
                            ? l.reviewFolderRuleSource
                            : l.reviewFolderRule(rule.folder),
                        key: ValueKey('folder-rule-${rule.id}'),
                        style: theme.textTheme.bodySmall,
                      ),
                    ),
                  ],
                ),
              ),
            const SizedBox(height: 4),
            KButton(
              key: const ValueKey('folder-forget'),
              label: l.reviewFolderForget,
              icon: Icons.link_off,
              onPressed: _busy ? null : _forget,
            ),
          ],
          if (_error != null)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(
                _error!,
                key: const ValueKey('folder-error'),
                style: theme.textTheme.bodySmall?.copyWith(
                  color: theme.colorScheme.error,
                ),
              ),
            ),
        ],
      ),
    );
  }

  Widget _editor(AppLocalizations l, ThemeData theme) {
    final offset = _offsetValue;
    final season = _seasonValue;
    final inName = offset < 0 ? 1 - offset : 1;
    final example = _absolute
        ? l.reviewNumberingExampleAbsolute(inName, inName + offset)
        : season == null
        ? l.reviewNumberingExampleSameSeason(inName, inName + offset)
        : l.reviewNumberingExample(inName, season, inName + offset);
    return Column(
      key: const ValueKey('folder-numbering-editor'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const SizedBox(height: 4),
        Wrap(
          spacing: 8,
          runSpacing: 8,
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            Text(l.reviewNumberingInName, style: theme.textTheme.bodyMedium),
            _NumberField(
              id: 'folder-offset',
              controller: _offset,
              label: l.reviewNumberingOffsetField,
              signed: true,
              onLess: () => _stepOffset(-1),
              onMore: () => _stepOffset(1),
            ),
            Text(
              _absolute
                  ? l.reviewNumberingIsAbsolute
                  : l.reviewNumberingIsSeason,
              style: theme.textTheme.bodyMedium,
            ),
            if (!_absolute)
              _NumberField(
                id: 'folder-season',
                controller: _season,
                label: l.reviewNumberingSeasonField,
                hint: l.reviewNumberingSeasonHint,
                onLess: () => _stepSeason(-1),
                onMore: () => _stepSeason(1),
              ),
          ],
        ),
        const SizedBox(height: 6),
        Text(
          example,
          key: const ValueKey('folder-numbering-example'),
          style: theme.textTheme.bodySmall,
        ),
        const SizedBox(height: 6),
        SettingsSwitchTile(
          key: const ValueKey('folder-absolute'),
          icon: Icons.format_list_numbered,
          title: l.reviewNumberingAbsolute,
          subtitle: l.reviewNumberingAbsoluteHint,
          value: _absolute,
          onChanged: (v) => setState(() => _absolute = v),
        ),
        const SizedBox(height: 6),
        Text(l.reviewNumberingHint, style: theme.textTheme.bodySmall),
        const SizedBox(height: 8),
        Wrap(
          spacing: 10,
          runSpacing: 10,
          children: [
            KButton(
              key: const ValueKey('folder-numbering-apply'),
              label: l.reviewNumberingApply,
              icon: Icons.check,
              primary: true,
              onPressed: _busy || !_dirty ? null : _apply,
            ),
            KButton(
              key: const ValueKey('folder-numbering-cancel'),
              label: l.commonCancel,
              onPressed: _busy
                  ? null
                  : () {
                      _reset();
                      setState(() => _editing = false);
                    },
            ),
          ],
        ),
      ],
    );
  }
}

/// A choice chip of the type switch.
class _Choice extends StatelessWidget {
  const _Choice({
    super.key,
    required this.label,
    required this.selected,
    required this.onPressed,
  });

  final String label;
  final bool selected;
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return KFocusable(
      onActivate: onPressed,
      radius: 999,
      light: false,
      focusScale: 1.0,
      selected: selected,
      semanticLabel: label,
      builder: (context, focused) {
        final Color bg, fg;
        if (focused) {
          bg = KColors.focusFill;
          fg = KColors.focusLabel;
        } else if (selected) {
          bg = KColors.accent;
          fg = Colors.white;
        } else {
          bg = Colors.white.withValues(alpha: .12);
          fg = Colors.white;
        }
        return Container(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
          decoration: BoxDecoration(
            color: bg,
            borderRadius: BorderRadius.circular(999),
          ),
          child: ExcludeSemantics(
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (selected) ...[
                  Icon(Icons.check, size: 16, color: fg),
                  const SizedBox(width: 6),
                ],
                Text(
                  label,
                  style: theme.textTheme.labelLarge?.copyWith(color: fg),
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}

/// A short number field between «−» and «+». On a TV remote the buttons
/// do the work; up and down leave the field (no focus trap), left and
/// right still move the cursor.
class _NumberField extends StatelessWidget {
  const _NumberField({
    required this.id,
    required this.controller,
    required this.label,
    required this.onLess,
    required this.onMore,
    this.hint,
    this.signed = false,
  });

  /// Key of the field; the buttons get `<id>-less` and `<id>-more`.
  final String id;
  final TextEditingController controller;
  final String label;
  final String? hint;
  final bool signed;
  final VoidCallback onLess;
  final VoidCallback onMore;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        _StepButton(
          key: ValueKey('$id-less'),
          icon: Icons.remove,
          label: '${l.reviewNumberingLess}: $label',
          onPressed: onLess,
        ),
        const SizedBox(width: 4),
        SizedBox(
          width: 96,
          child: Shortcuts(
            shortcuts: const {
              SingleActivator(
                LogicalKeyboardKey.arrowUp,
              ): DirectionalFocusIntent(
                TraversalDirection.up,
                ignoreTextFields: false,
              ),
              SingleActivator(
                LogicalKeyboardKey.arrowDown,
              ): DirectionalFocusIntent(
                TraversalDirection.down,
                ignoreTextFields: false,
              ),
            },
            child: TextField(
              key: ValueKey(id),
              controller: controller,
              textAlign: TextAlign.center,
              keyboardType: TextInputType.numberWithOptions(signed: signed),
              inputFormatters: [
                FilteringTextInputFormatter.allow(
                  RegExp(signed ? r'^-?\d{0,5}' : r'^\d{0,4}'),
                ),
              ],
              decoration: InputDecoration(
                labelText: label,
                hintText: hint,
                isDense: true,
                floatingLabelBehavior: FloatingLabelBehavior.always,
              ),
            ),
          ),
        ),
        const SizedBox(width: 4),
        _StepButton(
          key: ValueKey('$id-more'),
          icon: Icons.add,
          label: '${l.reviewNumberingMore}: $label',
          onPressed: onMore,
        ),
      ],
    );
  }
}

class _StepButton extends StatelessWidget {
  const _StepButton({
    super.key,
    required this.icon,
    required this.label,
    required this.onPressed,
  });

  final IconData icon;
  final String label;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => KFocusable(
    onActivate: onPressed,
    radius: 999,
    light: false,
    focusScale: 1.0,
    semanticLabel: label,
    builder: (context, focused) => Container(
      width: 40,
      height: 40,
      decoration: BoxDecoration(
        color: focused
            ? KColors.focusFill
            : Colors.white.withValues(alpha: .12),
        shape: BoxShape.circle,
      ),
      child: Icon(
        icon,
        size: 20,
        color: focused ? KColors.focusLabel : Colors.white,
      ),
    ),
  );
}
