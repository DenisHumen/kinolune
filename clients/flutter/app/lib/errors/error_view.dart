// SPDX-License-Identifier: MPL-2.0
//
// The error block of UI-044: one sentence, one «Исправить» button and the
// technical part (code, the core's message) folded under «Подробнее».
// [runErrorFix] does what the button promises.

import 'package:material_ui/material_ui.dart';

import '../diagnostics/status_screen.dart' show openStatusScreen;
import '../l10n/l10n.dart';
import '../screens/about_screen.dart';
import '../screens/add_source_dialog.dart' show showEditSourceDialog;
import '../screens/devices_screen.dart';
import '../screens/downloads_screen.dart';
import '../screens/sources_screen.dart';
import '../services/app_services.dart';
import '../shell/app_shell.dart';
import '../sources/source_manager.dart';
import '../widgets/focusable.dart';
import 'friendly_errors.dart';

/// Does what the «Исправить» button of [fix] promises. [source] is the
/// source the error is about (its form opens for [ErrorFix.editSource]);
/// [retry] repeats the failed step.
Future<void> runErrorFix(
  BuildContext context,
  ErrorFix fix, {
  ManagedSource? source,
  Future<void> Function()? retry,
}) async {
  Future<void> push(Widget page) =>
      Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => page));
  switch (fix) {
    case ErrorFix.retry:
      await retry?.call();
    case ErrorFix.editSource:
      if (source != null) {
        await showEditSourceDialog(context, source);
      } else {
        await push(const SourcesScreen());
      }
    case ErrorFix.openSources:
      await push(const SourcesScreen());
    case ErrorFix.freeSpace:
      if (Services.read(context).offline != null) {
        await push(const DownloadsScreen());
      } else {
        _openSettings(context);
      }
    case ErrorFix.openSettings:
      _openSettings(context);
    case ErrorFix.openSync:
      await push(const DevicesScreen());
    case ErrorFix.openUpdates:
      await push(const AboutScreen());
    case ErrorFix.report:
      await openStatusScreen(context);
  }
}

void _openSettings(BuildContext context) {
  final shell = context.findAncestorStateOfType<AppShellState>();
  if (shell == null) return;
  shell.select(Section.settings);
  shell.navigatorOf(Section.settings)?.popUntil((r) => r.isFirst);
}

/// An error in plain words with its one action.
class FriendlyErrorView extends StatelessWidget {
  const FriendlyErrorView({
    super.key,
    required this.text,
    required this.fix,
    this.onFix,
    this.details,
    this.fixKey,
    this.compact = false,
  });

  /// The error of the core by its code: the common text and action.
  factory FriendlyErrorView.code(
    BuildContext context, {
    Key? key,
    required String code,
    String? message,
    VoidCallback? onFix,
    Key? fixKey,
  }) => FriendlyErrorView(
    key: key,
    text: errorText(context.l10n, code),
    fix: errorFixFor(code),
    onFix: onFix,
    fixKey: fixKey,
    details: message == null || message.isEmpty ? code : '$code: $message',
  );

  /// The sentence: what happened and what to do.
  final String text;
  final ErrorFix fix;

  /// Null hides the button (the place has its own action already).
  final VoidCallback? onFix;

  /// The code and the technical message: only under «Подробнее».
  final String? details;
  final Key? fixKey;

  /// Small text (inside a card).
  final bool compact;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final style =
        (compact ? theme.textTheme.bodySmall : theme.textTheme.bodyMedium)
            ?.copyWith(color: theme.colorScheme.error);
    final d = details;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(text, style: style),
        if (onFix != null)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Align(
              alignment: Alignment.centerLeft,
              child: Tooltip(
                message: errorFixHint(l, fix),
                child: KButton(
                  key: fixKey,
                  label: errorFixLabel(l, fix),
                  icon: fix == ErrorFix.retry
                      ? Icons.refresh_rounded
                      : Icons.build_outlined,
                  primary: true,
                  onPressed: onFix,
                ),
              ),
            ),
          ),
        if (d != null && d.isNotEmpty) ErrorDetails(details: d),
      ],
    );
  }
}

/// «Подробнее»: the code and the technical message, folded.
class ErrorDetails extends StatelessWidget {
  const ErrorDetails({super.key, required this.details});

  final String details;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    // Own Material: the view also sits on decorated cards.
    return Theme(
      data: theme.copyWith(dividerColor: Colors.transparent),
      child: Material(
        type: MaterialType.transparency,
        child: ExpansionTile(
          key: const ValueKey('error-details'),
          tilePadding: EdgeInsets.zero,
          dense: true,
          title: Text(
            context.l10n.sourcesErrorDetails,
            style: theme.textTheme.labelSmall,
          ),
          children: [
            Align(
              alignment: Alignment.centerLeft,
              child: SelectableText(details, style: theme.textTheme.labelSmall),
            ),
          ],
        ),
      ),
    );
  }
}
