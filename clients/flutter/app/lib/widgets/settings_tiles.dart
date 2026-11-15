// SPDX-License-Identifier: MPL-2.0
//
// Building blocks of Settings and About (UI-016): a titled section panel and
// focusable rows that work with touch, mouse and the TV remote.

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../theme/kinolune_theme.dart';
import 'focusable.dart';

/// A titled panel of settings rows.
class SettingsSection extends StatelessWidget {
  const SettingsSection({
    super.key,
    required this.title,
    required this.children,
    this.subtitle,
    this.icon,
  });

  final String title;
  final String? subtitle;
  final IconData? icon;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final d = KDensity.of(context);
    return Padding(
      padding: EdgeInsets.only(bottom: d.isTv ? 18 : 24),
      child: FocusTraversalGroup(
        child: Container(
          padding: const EdgeInsets.all(18),
          decoration: BoxDecoration(
            color: KColors.bgElevated,
            borderRadius: BorderRadius.circular(d.panelRadius * .75),
            border: Border.all(color: KColors.stroke),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Semantics(
                header: true,
                child: Row(
                  children: [
                    if (icon != null) ...[
                      Icon(icon, size: 22, color: KColors.accentSoft),
                      const SizedBox(width: 10),
                    ],
                    Expanded(
                      child: Text(title, style: theme.textTheme.titleLarge),
                    ),
                  ],
                ),
              ),
              if (subtitle != null) ...[
                const SizedBox(height: 2),
                Text(subtitle!, style: theme.textTheme.bodySmall),
              ],
              const SizedBox(height: 12),
              ...children,
            ],
          ),
        ),
      ),
    );
  }
}

/// A focusable row: icon, title, optional subtitle and trailing widget.
class SettingsTile extends StatelessWidget {
  const SettingsTile({
    super.key,
    required this.icon,
    required this.title,
    this.subtitle,
    this.subtitleLines = 1,
    this.onTap,
    this.trailing,
    this.selected,
    this.toggled,
  });

  final IconData icon;
  final String title;
  final String? subtitle;
  final int subtitleLines;
  final VoidCallback? onTap;
  final Widget? trailing;

  /// Radio-style tiles report their state to screen readers.
  final bool? selected;

  /// Switch-style tiles report on/off to screen readers.
  final bool? toggled;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Semantics(
      selected: selected,
      toggled: toggled,
      inMutuallyExclusiveGroup: selected != null,
      child: KFocusable(
        onActivate: onTap,
        // A row without its own action around its own buttons (a source
        // with Rescan and Remove) is no Tab stop itself (UI-021).
        focusable: onTap != null || trailing == null,
        focusScale: 1.0,
        light: false,
        radius: 12,
        semanticLabel: title,
        builder: (context, focused) => AnimatedContainer(
          duration: KMotion.focus,
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
          color: focused
              ? Colors.white.withValues(alpha: .09)
              : Colors.transparent,
          child: Row(
            children: [
              Icon(icon, color: KColors.textSecondary),
              const SizedBox(width: 14),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(title, style: theme.textTheme.titleMedium),
                    if (subtitle != null)
                      Text(
                        subtitle!,
                        style: theme.textTheme.labelSmall,
                        maxLines: subtitleLines,
                        overflow: TextOverflow.ellipsis,
                      ),
                  ],
                ),
              ),
              trailing ??
                  (onTap != null
                      ? Icon(Icons.chevron_right, color: KColors.textTertiary)
                      : const SizedBox.shrink()),
            ],
          ),
        ),
      ),
    );
  }
}

/// An on/off row; the whole row toggles.
class SettingsSwitchTile extends StatelessWidget {
  const SettingsSwitchTile({
    super.key,
    required this.icon,
    required this.title,
    required this.value,
    required this.onChanged,
    this.subtitle,
  });

  final IconData icon;
  final String title;
  final String? subtitle;
  final bool value;
  final ValueChanged<bool> onChanged;

  @override
  Widget build(BuildContext context) => SettingsTile(
    icon: icon,
    title: title,
    subtitle: subtitle,
    subtitleLines: 2,
    onTap: () => onChanged(!value),
    trailing: ExcludeFocus(
      child: ExcludeSemantics(
        child: Switch(value: value, onChanged: onChanged),
      ),
    ),
    toggled: value,
  );
}

/// A row that shows the current choice and opens a dialog to pick another.
/// Options get the keys `<key>-<index>` when the tile has a [ValueKey].
class SettingsChoiceTile<T> extends StatelessWidget {
  const SettingsChoiceTile({
    super.key,
    required this.icon,
    required this.title,
    required this.value,
    required this.options,
    required this.label,
    required this.onChanged,
    this.hint,
  });

  final IconData icon;
  final String title;

  /// Shown under the options in the dialog.
  final String? hint;
  final T value;
  final List<T> options;
  final String Function(T) label;
  final ValueChanged<T> onChanged;

  Future<void> _pick(BuildContext context) async {
    final k = key;
    final prefix = k is ValueKey<String> ? k.value : null;
    final picked = await showDialog<(T,)>(
      context: context,
      builder: (context) => SimpleDialog(
        title: Text(title),
        children: [
          for (final (i, o) in options.indexed)
            SimpleDialogOption(
              key: prefix == null ? null : ValueKey('$prefix-$i'),
              onPressed: () => Navigator.of(context).pop((o,)),
              child: Semantics(
                selected: o == value,
                inMutuallyExclusiveGroup: true,
                child: Row(
                  children: [
                    Icon(
                      o == value
                          ? Icons.radio_button_checked
                          : Icons.radio_button_unchecked,
                      size: 20,
                    ),
                    const SizedBox(width: 12),
                    Expanded(child: Text(label(o))),
                  ],
                ),
              ),
            ),
          if (hint != null)
            Padding(
              padding: const EdgeInsets.fromLTRB(24, 8, 24, 0),
              child: Text(hint!, style: Theme.of(context).textTheme.bodySmall),
            ),
        ],
      ),
    );
    // A record: null is a valid option (T may be nullable).
    if (picked != null && picked.$1 != value) onChanged(picked.$1);
  }

  @override
  Widget build(BuildContext context) => SettingsTile(
    icon: icon,
    title: title,
    subtitle: label(value),
    subtitleLines: 2,
    onTap: () => _pick(context),
  );
}

/// A row for a web address. Activating it copies the address: the app has
/// no browser integration, and on a TV there may be no browser at all.
class SettingsLinkTile extends StatelessWidget {
  const SettingsLinkTile({
    super.key,
    required this.icon,
    required this.title,
    required this.url,
    this.subtitle,
  });

  final IconData icon;
  final String title;
  final String url;

  /// Shown instead of [url] under the title.
  final String? subtitle;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return Tooltip(
      message: l.aboutCopyLink,
      child: SettingsTile(
        icon: icon,
        title: title,
        subtitle: subtitle ?? url,
        subtitleLines: 2,
        onTap: () => copyLink(context, url),
        trailing: Icon(Icons.copy_rounded, color: KColors.textTertiary),
      ),
    );
  }
}

/// Copies [url] to the clipboard and says so.
Future<void> copyLink(BuildContext context, String url) async {
  final messenger = ScaffoldMessenger.maybeOf(context);
  final copied = context.l10n.aboutLinkCopied;
  await Clipboard.setData(ClipboardData(text: url));
  messenger?.showSnackBar(SnackBar(content: Text(copied)));
}
