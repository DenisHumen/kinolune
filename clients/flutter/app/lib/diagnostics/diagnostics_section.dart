// SPDX-License-Identifier: MPL-2.0
//
// Settings → Diagnostics (CORE-008): the opt-in, "Export diagnostic
// report", the «Состояние» screen (UI-017) and deleting what is kept.
// Hidden without the native core.

import 'package:intl/intl.dart';
import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../services/external_links.dart';
import '../widgets/settings_tiles.dart';
import 'diagnostics.dart';
import 'status_screen.dart';

class DiagnosticsSettingsSection extends StatelessWidget {
  const DiagnosticsSettingsSection({super.key, this.diagnostics});

  /// Default: [Diagnostics.instance].
  final Diagnostics? diagnostics;

  @override
  Widget build(BuildContext context) {
    final d = diagnostics ?? Diagnostics.instance;
    if (!d.available) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: d,
      builder: (context, _) => _build(context, d),
    );
  }

  Widget _build(BuildContext context, Diagnostics d) {
    final l = context.l10n;
    final s = d.state;
    final locale = Localizations.localeOf(context);
    return SettingsSection(
      key: const ValueKey('settings-section-diagnostics'),
      icon: Icons.bug_report_outlined,
      title: l.settingsDiagnosticsTitle,
      subtitle: l.settingsDiagnosticsSubtitle,
      children: [
        SettingsSwitchTile(
          key: const ValueKey('settings-diagnostics-keep'),
          icon: Icons.receipt_long_outlined,
          title: l.settingsDiagnosticsKeep,
          subtitle: l.settingsDiagnosticsKeepSubtitle,
          value: s.enabled,
          onChanged: (on) => _run(context, () => d.setEnabled(on)),
        ),
        // UI-017: the log, crash reports, versions and source speed.
        SettingsTile(
          key: const ValueKey('settings-diagnostics-status'),
          icon: Icons.monitor_heart_outlined,
          title: l.statusSettingsTile,
          subtitle: l.statusSettingsTileSubtitle,
          onTap: () => openStatusScreen(context),
        ),
        SettingsTile(
          key: const ValueKey('settings-diagnostics-export'),
          icon: Icons.archive_outlined,
          title: l.settingsDiagnosticsExport,
          subtitle: d.busy
              ? l.settingsDiagnosticsExporting
              : s.enabled
              ? l.settingsDiagnosticsExportKept(
                  l.detailSizeMb(
                    NumberFormat(
                      '0.#',
                      locale.toLanguageTag(),
                    ).format(s.logBytes / (1 << 20)),
                  ),
                  s.crashReports,
                )
              : l.settingsDiagnosticsExportOff,
          subtitleLines: 2,
          onTap: d.busy ? null : () => _export(context, d),
          trailing: const SizedBox.shrink(),
        ),
        if (s.enabled && s.hasData)
          SettingsTile(
            key: const ValueKey('settings-diagnostics-clear'),
            icon: Icons.delete_sweep_outlined,
            title: l.settingsDiagnosticsClear,
            onTap: () => _run(context, () async {
              await d.clear();
              if (context.mounted) _say(context, l.settingsDiagnosticsCleared);
            }),
            trailing: const SizedBox.shrink(),
          ),
      ],
    );
  }

  Future<void> _run(BuildContext context, Future<void> Function() f) async {
    try {
      await f();
    } catch (e) {
      if (context.mounted) {
        _say(context, context.l10n.settingsDiagnosticsFailed('$e'));
      }
    }
  }

  Future<void> _export(BuildContext context, Diagnostics d) async {
    final services = Services.of(context);
    final l = context.l10n;
    DiagnosticsExport? report;
    try {
      report = await d.exportReport(
        build: services.buildInfo,
        preferences: services.preferences,
      );
    } catch (e) {
      if (context.mounted) _say(context, l.settingsDiagnosticsFailed('$e'));
      return;
    }
    if (report == null || !context.mounted) return;
    await showDiagnosticsSaved(context, report);
  }
}

/// «Сохранить отчёт о воспроизведении» in the player (PLAY-010): the
/// diagnostic report with the player's stats in `extra/`, then the same
/// "Report saved" dialog as in Settings.
Future<void> exportPlaybackReport(
  BuildContext context,
  Map<String, String> files,
) async {
  final d = Diagnostics.instance;
  if (!d.available) return;
  final services = Services.of(context);
  final l = context.l10n;
  DiagnosticsExport? report;
  try {
    report = await d.exportReport(
      build: services.buildInfo,
      preferences: services.preferences,
      attachments: files,
    );
  } catch (e) {
    if (context.mounted) _say(context, l.settingsDiagnosticsFailed('$e'));
    return;
  }
  if (report == null || !context.mounted) return;
  await showDiagnosticsSaved(context, report);
}

/// "Report saved": where it is and the prefilled GitHub issue.
Future<void> showDiagnosticsSaved(
  BuildContext context,
  DiagnosticsExport report,
) => showDialog<void>(
  context: context,
  builder: (c) {
    final l = c.l10n;
    return AlertDialog(
      key: const ValueKey('diagnostics-saved'),
      // The theme's text style carries the app font (the dialog theme's
      // title style alone falls back to the platform font).
      title: Text(
        l.settingsDiagnosticsSavedTitle,
        style: Theme.of(c).textTheme.headlineSmall,
      ),
      content: SelectableText(l.settingsDiagnosticsSavedBody(report.shownPath)),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(c).pop(),
          child: Text(l.settingsDiagnosticsClose),
        ),
        FilledButton(
          key: const ValueKey('diagnostics-open-issue'),
          autofocus: true,
          onPressed: () async {
            final navigator = Navigator.of(c);
            if (await openExternalLink(report.issueUrl)) {
              navigator.pop();
            } else if (c.mounted) {
              _say(c, l.settingsDiagnosticsLinkFailed);
            }
          },
          child: Text(l.settingsDiagnosticsOpenIssue),
        ),
      ],
    );
  },
);

void _say(BuildContext context, String text) =>
    ScaffoldMessenger.maybeOf(context)
        ?.showSnackBar(SnackBar(content: Text(text)));
