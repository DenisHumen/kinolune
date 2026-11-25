// SPDX-License-Identifier: MPL-2.0
//
// «Состояние» (UI-017, ADR-0125): everything a bug report needs on one
// screen — the diagnostic report in one click (the same zip as Settings →
// Diagnostics and the player's «Сохранить отчёт», CORE-008 / PLAY-010),
// versions of the app, the core and the engines (libmpv, FFmpeg, SQLite),
// the speed test of each source (the core's `speed_test`, NET-011), the
// tail of the log and the crash reports. Nothing leaves the device by
// itself: the user saves the report or copies a crash report and attaches
// it to an issue. One column on a phone; two on a desktop, tablet and TV
// (remote focus on every action, the log scrolls with the arrows).

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:intl/intl.dart';
import 'package:material_ui/material_ui.dart';

import '../errors/friendly_errors.dart';
import '../l10n/l10n.dart';
import '../screens/sources_screen.dart' show SpeedTestDialog;
import '../services/app_services.dart';
import '../sources/source_manager.dart';
import '../sources/source_text.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import '../widgets/settings_tiles.dart';
import 'diagnostics.dart';
import 'diagnostics_section.dart' show showDiagnosticsSaved;

/// From this width the screen has two columns.
const statusWideFrom = 900.0;

/// Log lines the screen shows (the report carries all of them).
const statusLogLines = 300;

/// Opens the screen over the current page.
Future<void> openStatusScreen(BuildContext context) =>
    Navigator.of(context)
        .push(MaterialPageRoute<void>(builder: (_) => const StatusScreen()));

class StatusScreen extends StatefulWidget {
  const StatusScreen({super.key, this.diagnostics});

  /// Default: [Diagnostics.instance].
  final Diagnostics? diagnostics;

  @override
  State<StatusScreen> createState() => _StatusScreenState();
}

class _StatusScreenState extends State<StatusScreen> {
  Diagnostics get _d => widget.diagnostics ?? Diagnostics.instance;

  List<String> _log = const [];
  List<CrashInfo> _crashes = const [];
  Map<String, String> _engines = const {};
  bool _errorsOnly = false;
  bool _enabled = false;

  @override
  void initState() {
    super.initState();
    _enabled = _d.state.enabled;
    _d.addListener(_changed);
    unawaited(_reload());
    unawaited(
      _d.engineVersions().then((v) {
        if (mounted) setState(() => _engines = v);
      }),
    );
  }

  @override
  void dispose() {
    _d.removeListener(_changed);
    super.dispose();
  }

  /// Opt-in, clear or a new crash: the lists follow.
  void _changed() {
    if (!mounted) return;
    final enabled = _d.state.enabled;
    setState(() => _enabled = enabled);
    unawaited(_reload());
  }

  Future<void> _reload() async {
    final log = await _d.recentLog(maxLines: statusLogLines);
    final crashes = _d.crashes();
    if (!mounted) return;
    setState(() {
      _log = log;
      _crashes = crashes;
    });
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return Scaffold(
      key: const ValueKey('status-page'),
      backgroundColor: KColors.bgBase,
      appBar: AppBar(
        backgroundColor: KColors.bgBase,
        title: Text(
          l.statusTitle,
          style: Theme.of(context).textTheme.titleLarge,
        ),
      ),
      body: ListenableBuilder(
        listenable: Services.of(context),
        builder: (context, _) => LayoutBuilder(
          builder: (context, c) =>
              c.maxWidth >= statusWideFrom ? _wide(context) : _narrow(context),
        ),
      ),
    );
  }

  List<Widget> _side(BuildContext context) => [
    _ExportCard(diagnostics: _d),
    _VersionsSection(engines: _engines),
    const _SpeedSection(),
  ];

  List<Widget> _main(BuildContext context) => [
    _CrashSection(diagnostics: _d, enabled: _enabled, crashes: _crashes),
    _LogSection(
      diagnostics: _d,
      enabled: _enabled,
      lines: _log,
      errorsOnly: _errorsOnly,
      onErrorsOnly: (v) => setState(() => _errorsOnly = v),
      onRefresh: _reload,
    ),
  ];

  /// Phone: the report first, then versions, speed, crashes and the log.
  Widget _narrow(BuildContext context) {
    final d = KDensity.of(context);
    return FocusTraversalGroup(
      child: ListView(
        key: const ValueKey('status-narrow'),
        padding: EdgeInsets.fromLTRB(
          d.pageInsets.left,
          4,
          d.pageInsets.right,
          d.pageInsets.bottom + 24,
        ),
        children: [..._side(context), ..._main(context)],
      ),
    );
  }

  /// Desktop, tablet, TV: the report, versions and speed on the left;
  /// crashes and the log on the right.
  Widget _wide(BuildContext context) {
    final d = KDensity.of(context);
    return Row(
      key: const ValueKey('status-wide'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(
          width: d.isTv ? 420 : 400,
          child: FocusTraversalGroup(
            child: ListView(
              key: const ValueKey('status-side'),
              padding: EdgeInsets.fromLTRB(
                d.pageInsets.left,
                4,
                12,
                d.pageInsets.bottom + 24,
              ),
              children: _side(context),
            ),
          ),
        ),
        Expanded(
          child: FocusTraversalGroup(
            child: ListView(
              key: const ValueKey('status-main'),
              padding: EdgeInsets.fromLTRB(
                12,
                4,
                d.pageInsets.right,
                d.pageInsets.bottom + 24,
              ),
              children: _main(context),
            ),
          ),
        ),
      ],
    );
  }
}

// ----------------------------------------------------------------- export

/// Types and the last measured speed of the sources, without names,
/// addresses or user names: what the report carries about them.
String statusSourcesSummary(List<ManagedSource> sources) => [
  for (final s in sources)
    [
      s.type.name,
      if (!s.enabled) 'disabled',
      switch (s.status.online) {
        true => 'online',
        false => 'offline',
        null => 'unchecked',
      },
      if (s.status.speedBytesPerSec != null) '${s.status.speedBytesPerSec} B/s',
      if (s.status.latencyMs != null) '${s.status.latencyMs} ms',
      if (s.status.speedMeasuredAt != null)
        s.status.speedMeasuredAt!.toUtc().toIso8601String(),
    ].join(' · '),
].join('\n');

/// Writes the diagnostic report (with the sources' summary) and shows the
/// "Report saved" dialog with the prefilled issue.
Future<void> exportStatusReport(BuildContext context, Diagnostics d) async {
  final services = Services.read(context);
  final l = context.l10n;
  final sources = statusSourcesSummary(services.managedSources);
  DiagnosticsExport? report;
  try {
    report = await d.exportReport(
      build: services.buildInfo,
      preferences: services.preferences,
      attachments: {if (sources.isNotEmpty) 'sources.txt': sources},
    );
  } catch (e) {
    if (context.mounted) {
      _say(context, l.settingsDiagnosticsFailed(errorTextOf(l, e)));
    }
    return;
  }
  if (report == null || !context.mounted) return;
  await showDiagnosticsSaved(context, report);
}

class _ExportCard extends StatelessWidget {
  const _ExportCard({required this.diagnostics});

  final Diagnostics diagnostics;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    final busy = diagnostics.busy;
    return SettingsSection(
      key: const ValueKey('status-export'),
      icon: Icons.archive_outlined,
      title: l.statusExportTitle,
      subtitle: diagnostics.available
          ? l.statusExportBody
          : l.statusUnavailable,
      children: [
        Align(
          alignment: Alignment.centerLeft,
          child: KButton(
            key: const ValueKey('status-export-button'),
            label: busy ? l.settingsDiagnosticsExporting : l.statusExportButton,
            icon: Icons.download_rounded,
            primary: true,
            autofocus: d.isTv,
            onPressed: !diagnostics.available || busy
                ? null
                : () => exportStatusReport(context, diagnostics),
          ),
        ),
        if (diagnostics.available && !diagnostics.state.enabled) ...[
          const SizedBox(height: 8),
          Text(
            l.settingsDiagnosticsExportOff,
            style: theme.textTheme.bodySmall,
          ),
        ],
      ],
    );
  }
}

// --------------------------------------------------------------- versions

/// Rows of the versions section: label → value.
List<(String, String, String)> statusVersionRows(
  AppLocalizations l,
  AppServices services,
  Map<String, String> engines,
) {
  final b = services.buildInfo;
  return [
    ('app', l.statusVersionApp, '${b.appVersion} (${b.flavor})'),
    (
      'core',
      l.statusVersionCore,
      b.coreVersion == null
          ? l.aboutCoreMissing
          : '${b.coreVersion} (${b.coreFlavor ?? b.flavor})',
    ),
    ('mpv', l.statusVersionMpv, engines['mpv'] ?? l.statusVersionUnknown),
    (
      'ffmpeg',
      l.statusVersionFfmpeg,
      engines['ffmpeg'] ?? l.statusVersionUnknown,
    ),
    (
      'sqlite',
      l.statusVersionSqlite,
      engines['sqlite'] ?? l.statusVersionUnknown,
    ),
    ('platform', l.statusVersionPlatform, currentOsVersion()),
  ];
}

class _VersionsSection extends StatelessWidget {
  const _VersionsSection({required this.engines});

  final Map<String, String> engines;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final rows = statusVersionRows(l, Services.of(context), engines);
    return SettingsSection(
      key: const ValueKey('status-versions'),
      icon: Icons.memory_rounded,
      title: l.statusVersionsTitle,
      children: [
        for (final (id, label, value) in rows)
          Padding(
            key: ValueKey('status-version-$id'),
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                SizedBox(
                  width: 150,
                  child: Text(label, style: theme.textTheme.bodyMedium),
                ),
                Expanded(
                  child: Text(
                    value,
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: KColors.textSecondary,
                    ),
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ],
            ),
          ),
        const SizedBox(height: 8),
        Align(
          alignment: Alignment.centerLeft,
          child: KButton(
            key: const ValueKey('status-versions-copy'),
            label: l.statusCopy,
            icon: Icons.copy_rounded,
            onPressed: () async {
              await Clipboard.setData(
                ClipboardData(
                  text: [for (final (_, k, v) in rows) '$k: $v'].join('\n'),
                ),
              );
              if (context.mounted) _say(context, l.statusCopied);
            },
          ),
        ),
      ],
    );
  }
}

// ------------------------------------------------------------------ speed

class _SpeedSection extends StatelessWidget {
  const _SpeedSection();

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final services = Services.of(context);
    final canTest = services.sourceManager.canSpeedTest;
    final sources = services.managedSources;
    return SettingsSection(
      key: const ValueKey('status-speed'),
      icon: Icons.speed_rounded,
      title: l.statusSpeedTitle,
      subtitle: canTest ? l.statusSpeedSubtitle : l.statusSpeedUnavailable,
      children: [
        if (sources.isEmpty)
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
            child: Text(
              l.statusSpeedNoSources,
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ),
        for (final s in sources)
          SettingsTile(
            key: ValueKey('status-speed-${s.id}'),
            icon: Icons.storage_rounded,
            title: s.name,
            subtitle: _last(l, s),
            subtitleLines: 2,
            onTap: canTest && s.enabled
                ? () => showDialog<void>(
                    context: context,
                    builder: (_) => SpeedTestDialog(source: s),
                  )
                : null,
            trailing: canTest && s.enabled
                ? Tooltip(
                    message: l.statusSpeedRun,
                    child: Icon(
                      Icons.play_circle_outline_rounded,
                      color: KColors.accentSoft,
                      semanticLabel: l.statusSpeedRun,
                    ),
                  )
                : const SizedBox.shrink(),
          ),
      ],
    );
  }

  String _last(AppLocalizations l, ManagedSource s) {
    final type = sourceTypeLabel(l, s.type);
    final speed = s.status.speedBytesPerSec;
    final latency = s.status.latencyMs;
    final at = s.status.speedMeasuredAt;
    if (latency == null || at == null) return '$type · ${l.statusSpeedNever}';
    return [
      type,
      if (speed != null) formatSpeed(l, speed),
      l.sourcesSpeedResponse(l.sourcesLatencyMs(latency)),
      formatAgo(l, at),
    ].join(' · ');
  }
}

// ---------------------------------------------------------------- crashes

class _CrashSection extends StatelessWidget {
  const _CrashSection({
    required this.diagnostics,
    required this.enabled,
    required this.crashes,
  });

  final Diagnostics diagnostics;
  final bool enabled;
  final List<CrashInfo> crashes;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final locale = Localizations.localeOf(context).toLanguageTag();
    String when(DateTime? t) =>
        t == null ? '' : DateFormat.yMMMd(locale).add_Hm().format(t.toLocal());
    return SettingsSection(
      key: const ValueKey('status-crashes'),
      icon: Icons.report_problem_outlined,
      title: l.statusCrashTitle,
      subtitle: l.statusCrashSubtitle,
      children: [
        if (!enabled)
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
            child: Text(l.statusCrashOff, style: theme.textTheme.bodySmall),
          )
        else if (crashes.isEmpty)
          Padding(
            key: const ValueKey('status-crashes-none'),
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
            child: Text(l.statusCrashNone, style: theme.textTheme.bodySmall),
          ),
        for (final c in crashes)
          SettingsTile(
            key: ValueKey('status-crash-${c.name}'),
            icon: Icons.bug_report_outlined,
            title: c.headline.isEmpty ? c.kind : c.headline,
            subtitle: [
              when(c.time),
              c.kind,
            ].where((s) => s.isNotEmpty).join(' · '),
            onTap: () => _open(context, c),
          ),
      ],
    );
  }

  Future<void> _open(BuildContext context, CrashInfo c) async {
    final l = context.l10n;
    final text = diagnostics.crashText(c.name);
    if (text == null) {
      _say(context, l.statusCrashGone);
      return;
    }
    await showDialog<void>(
      context: context,
      builder: (dialog) {
        final theme = Theme.of(dialog);
        return AlertDialog(
          key: const ValueKey('status-crash-dialog'),
          title: Text(
            c.headline.isEmpty ? c.kind : c.headline,
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: theme.textTheme.headlineSmall,
          ),
          content: SizedBox(
            width: 640,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text(l.statusCrashSendHint, style: theme.textTheme.bodySmall),
                const SizedBox(height: 10),
                Flexible(
                  child: SingleChildScrollView(
                    child: SelectableText(
                      text,
                      style: const TextStyle(
                        fontFamily: kFontFamily,
                        fontFeatures: [FontFeature.tabularFigures()],
                        fontSize: 12,
                        height: 1.35,
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.of(dialog).pop(),
              child: Text(l.settingsDiagnosticsClose),
            ),
            TextButton(
              key: const ValueKey('status-crash-copy'),
              onPressed: () async {
                await Clipboard.setData(ClipboardData(text: text));
                if (dialog.mounted) _say(dialog, l.statusCopied);
              },
              child: Text(l.statusCopy),
            ),
            FilledButton(
              key: const ValueKey('status-crash-report'),
              autofocus: true,
              onPressed: () {
                Navigator.of(dialog).pop();
                unawaited(exportStatusReport(context, diagnostics));
              },
              child: Text(l.statusExportButton),
            ),
          ],
        );
      },
    );
  }
}

// -------------------------------------------------------------------- log

/// `ERROR` / `WARN` lines of the log (the format `pa-log` writes).
bool statusIsError(String line) =>
    line.contains(' ERROR ') || line.contains(' WARN ');

class _LogSection extends StatelessWidget {
  const _LogSection({
    required this.diagnostics,
    required this.enabled,
    required this.lines,
    required this.errorsOnly,
    required this.onErrorsOnly,
    required this.onRefresh,
  });

  final Diagnostics diagnostics;
  final bool enabled;
  final List<String> lines;
  final bool errorsOnly;
  final ValueChanged<bool> onErrorsOnly;
  final Future<void> Function() onRefresh;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final shown = errorsOnly ? lines.where(statusIsError).toList() : lines;
    return SettingsSection(
      key: const ValueKey('status-log'),
      icon: Icons.receipt_long_outlined,
      title: l.statusLogTitle,
      subtitle: enabled ? l.statusLogLines(lines.length) : null,
      children: [
        if (diagnostics.available)
          SettingsSwitchTile(
            key: const ValueKey('status-log-keep'),
            icon: Icons.save_outlined,
            title: l.settingsDiagnosticsKeep,
            subtitle: l.settingsDiagnosticsKeepSubtitle,
            value: enabled,
            onChanged: (on) async {
              try {
                await diagnostics.setEnabled(on);
              } catch (e) {
                if (context.mounted) {
                  _say(context, l.settingsDiagnosticsFailed(errorTextOf(l, e)));
                }
              }
            },
          ),
        if (enabled) ...[
          const SizedBox(height: 8),
          Wrap(
            spacing: 8,
            runSpacing: 8,
            children: [
              KButton(
                key: const ValueKey('status-log-errors'),
                label: errorsOnly ? l.statusLogAll : l.statusLogErrorsOnly,
                icon: errorsOnly
                    ? Icons.list_rounded
                    : Icons.error_outline_rounded,
                onPressed: () => onErrorsOnly(!errorsOnly),
              ),
              KButton(
                key: const ValueKey('status-log-refresh'),
                label: l.statusLogRefresh,
                icon: Icons.refresh_rounded,
                onPressed: onRefresh,
              ),
              KButton(
                key: const ValueKey('status-log-copy'),
                label: l.statusCopy,
                icon: Icons.copy_rounded,
                onPressed: shown.isEmpty
                    ? null
                    : () async {
                        await Clipboard.setData(
                          ClipboardData(text: shown.join('\n')),
                        );
                        if (context.mounted) _say(context, l.statusCopied);
                      },
              ),
              if (diagnostics.state.hasData)
                KButton(
                  key: const ValueKey('status-log-clear'),
                  label: l.settingsDiagnosticsClear,
                  icon: Icons.delete_sweep_outlined,
                  onPressed: () async {
                    await diagnostics.clear();
                    if (context.mounted) {
                      _say(context, l.settingsDiagnosticsCleared);
                    }
                  },
                ),
            ],
          ),
          const SizedBox(height: 12),
          if (shown.isEmpty)
            Padding(
              key: const ValueKey('status-log-empty'),
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
              child: Text(l.statusLogEmpty, style: theme.textTheme.bodySmall),
            )
          else
            LogView(lines: shown),
        ],
      ],
    );
  }
}

/// The log lines in a box of their own: newest at the bottom, scrolled
/// there on open; with focus (TV remote, keyboard) the arrows scroll it and
/// leave it at its ends.
class LogView extends StatefulWidget {
  const LogView({super.key, required this.lines});

  final List<String> lines;

  @override
  State<LogView> createState() => _LogViewState();
}

class _LogViewState extends State<LogView> {
  final _scroll = ScrollController();
  final _focus = FocusNode(debugLabel: 'status-log-view');
  bool _focused = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _toEnd());
  }

  @override
  void didUpdateWidget(LogView old) {
    super.didUpdateWidget(old);
    if (old.lines.length != widget.lines.length) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _toEnd());
    }
  }

  void _toEnd() {
    if (_scroll.hasClients) _scroll.jumpTo(_scroll.position.maxScrollExtent);
  }

  @override
  void dispose() {
    _scroll.dispose();
    _focus.dispose();
    super.dispose();
  }

  KeyEventResult _key(FocusNode _, KeyEvent e) {
    if (e is KeyUpEvent || !_scroll.hasClients) return KeyEventResult.ignored;
    final p = _scroll.position;
    final step = switch (e.logicalKey) {
      LogicalKeyboardKey.arrowUp => -72.0,
      LogicalKeyboardKey.arrowDown => 72.0,
      LogicalKeyboardKey.pageUp => -p.viewportDimension * .9,
      LogicalKeyboardKey.pageDown => p.viewportDimension * .9,
      _ => 0.0,
    };
    if (step == 0) return KeyEventResult.ignored;
    // At the end the arrow moves focus on (the next section).
    if ((step < 0 && p.pixels <= p.minScrollExtent) ||
        (step > 0 && p.pixels >= p.maxScrollExtent)) {
      return KeyEventResult.ignored;
    }
    _scroll.jumpTo(
      (p.pixels + step).clamp(p.minScrollExtent, p.maxScrollExtent),
    );
    return KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final error = Theme.of(context).colorScheme.error;
    return Focus(
      focusNode: _focus,
      onKeyEvent: _key,
      onFocusChange: (f) => setState(() => _focused = f),
      child: Semantics(
        label: context.l10n.statusLogTitle,
        child: AnimatedContainer(
          key: const ValueKey('status-log-view'),
          duration: KMotion.focus,
          height: d.isTv ? 300 : 360,
          decoration: BoxDecoration(
            color: KColors.bgBase,
            borderRadius: BorderRadius.circular(10),
            border: Border.all(
              color: _focused ? KColors.accentSoft : KColors.stroke,
              width: _focused ? 2 : 1,
            ),
          ),
          child: Scrollbar(
            controller: _scroll,
            child: SingleChildScrollView(
              controller: _scroll,
              padding: const EdgeInsets.all(10),
              child: SelectionArea(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    for (final line in widget.lines)
                      Text(
                        line,
                        style: TextStyle(
                          fontFamily: kFontFamily,
                          fontFeatures: const [FontFeature.tabularFigures()],
                          fontSize: d.isTv ? 13 : 12,
                          height: 1.35,
                          color: line.contains(' ERROR ')
                              ? error
                              : line.contains(' WARN ')
                              ? const Color(0xFFE0B252)
                              : KColors.textSecondary,
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

void _say(BuildContext context, String text) =>
    ScaffoldMessenger.maybeOf(context)
        ?.showSnackBar(SnackBar(content: Text(text)));
