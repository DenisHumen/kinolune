// SPDX-License-Identifier: MPL-2.0
//
// Updates in Settings → About and the banner across the app (PKG-018,
// ADR-NNNN). The full flavor checks GitHub Releases and installs; store
// builds only point to their store.

import 'dart:async';
import 'dart:io' show Platform;

import 'package:material_ui/material_ui.dart';

import '../../branding.dart';
import '../../l10n/l10n.dart';
import '../../services/external_links.dart';
import '../../theme/kinolune_theme.dart';
import '../../widgets/focusable.dart';
import '../../widgets/settings_tiles.dart';
import 'releases.dart';
import 'update_controller.dart';
import 'update_platform.dart';

/// Store pages of the store flavor, by platform. Google Play is the only
/// store with a known address yet (PKG-011).
const kStorePages = <String, String>{
  'android': 'https://play.google.com/store/apps/details?id=app.kinolune',
};

/// The store that updates a store build on [os] (`Platform.operatingSystem`):
/// Google Play, the App Store (iOS, iPadOS, macOS) or the Microsoft Store;
/// null where store builds come from no store (Linux).
String? storeNameFor(String os) => switch (os) {
  'android' => 'Google Play', // l10n-ignore
  'ios' || 'macos' => 'App Store', // l10n-ignore
  'windows' => 'Microsoft Store', // l10n-ignore
  _ => null,
};

/// The releases page of the repository.
const kReleasesUrl = '$kRepoUrl/releases';

/// Opens [url] in the browser, or copies it where there is none (TVs).
Future<void> _open(BuildContext context, Uri url) async {
  if (await openExternalLink(url)) return;
  if (context.mounted) await copyLink(context, url.toString());
}

/// The Updates section of About in full builds (callers check
/// `AppFlavor.selfUpdate`, so store builds compile none of this).
class UpdatesSection extends StatelessWidget {
  const UpdatesSection({super.key, required this.controller});

  /// Null: this install is updated by the system (Flatpak, Snap) or runs
  /// without the core.
  final UpdateController? controller;

  @override
  Widget build(BuildContext context) {
    final c = controller;
    if (c == null) return const NoSelfUpdateSection(storeBuild: false);
    return ListenableBuilder(
      listenable: c,
      builder: (context, _) => _UpdatesBody(controller: c),
    );
  }
}

/// Store builds (a link to the store) and installs updated by the system.
///
/// [storeBuild]: a store flavor build (`!AppFlavor.selfUpdate`), updated by
/// the store of its platform: Google Play, the App Store or the Microsoft
/// Store (QA-015: these said "Flatpak, Snap" before). Otherwise a full
/// build that the system updates (Flatpak, Snap, a package manager).
class NoSelfUpdateSection extends StatelessWidget {
  const NoSelfUpdateSection({super.key, this.storeBuild = true, this.os});

  final bool storeBuild;

  /// `Platform.operatingSystem` by default; tests pass another.
  final String? os;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final system = os ?? Platform.operatingSystem;
    final storeName = storeBuild ? storeNameFor(system) : null;
    final store = storeBuild ? kStorePages[system] : null;
    return SettingsSection(
      key: const ValueKey('about-updates'),
      icon: Icons.system_update_alt,
      title: l.updatesTitle,
      subtitle: storeName != null
          ? l.updatesStoreNamedBody(kAppName, storeName)
          : l.updatesManagedBody(kAppName),
      children: [
        if (store != null)
          SettingsLinkTile(
            key: const ValueKey('updates-store'),
            icon: Icons.storefront_outlined,
            title: l.updatesStoreLink,
            url: store,
          ),
      ],
    );
  }
}

class _UpdatesBody extends StatelessWidget {
  const _UpdatesBody({required this.controller});

  final UpdateController controller;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final c = controller;
    final theme = Theme.of(context);
    final release = c.release;
    final checking = c.phase == UpdatePhase.checking;
    return SettingsSection(
      key: const ValueKey('about-updates'),
      icon: Icons.system_update_alt,
      title: l.updatesTitle,
      subtitle: l.updatesCurrent(c.current.text),
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
          child: Text(
            key: const ValueKey('updates-status'),
            updateStatusText(context, c),
            // A problem is a paragraph, a state one line.
            style: c.phase == UpdatePhase.failed
                ? theme.textTheme.bodyLarge
                : theme.textTheme.titleMedium,
          ),
        ),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12),
          child: Text(
            c.lastCheck == null
                ? l.updatesNeverChecked
                : l.updatesLastChecked(_when(context, c.lastCheck!)),
            style: theme.textTheme.bodySmall,
          ),
        ),
        if (c.phase == UpdatePhase.downloading)
          Padding(
            padding: const EdgeInsets.fromLTRB(12, 10, 12, 0),
            child: LinearProgressIndicator(value: c.progress),
          ),
        const SizedBox(height: 10),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12),
          child: Wrap(
            spacing: 10,
            runSpacing: 10,
            children: [
              KButton(
                key: const ValueKey('updates-check'),
                label: l.updatesCheck,
                icon: Icons.refresh,
                primary: release == null,
                onPressed: checking || _running(c)
                    ? null
                    : () => unawaited(c.check()),
              ),
              ..._actions(context, c),
            ],
          ),
        ),
        if (release != null) ...[
          const SizedBox(height: 14),
          _Changelog(controller: c, release: release),
        ],
        const SizedBox(height: 8),
        SettingsSwitchTile(
          key: const ValueKey('updates-auto-check'),
          icon: Icons.schedule,
          title: l.updatesAutoCheckEvery,
          subtitle: l.updatesAutoCheckEveryHint,
          value: c.autoCheck,
          onChanged: (v) => unawaited(c.setAutoCheck(v)),
        ),
        // PKG-022: only where the app can install in the background.
        if (c.target.downloadsInBackground && c.canInstall)
          SettingsChoiceTile<UpdateMode>(
            key: const ValueKey('updates-mode'),
            icon: Icons.download_for_offline_outlined,
            title: l.updatesMode,
            hint: l.updatesModeHint,
            value: c.mode,
            options: UpdateMode.values,
            label: (m) => switch (m) {
              UpdateMode.auto => l.updatesModeAuto,
              UpdateMode.notify => l.updatesModeNotify,
            },
            onChanged: (m) => unawaited(c.setMode(m)),
          ),
        SettingsChoiceTile<UpdateChannel>(
          key: const ValueKey('updates-channel'),
          icon: Icons.alt_route,
          title: l.updatesChannel,
          value: c.channel,
          options: UpdateChannel.values,
          label: (ch) => switch (ch) {
            UpdateChannel.stable => l.updatesChannelStable,
            UpdateChannel.beta => l.updatesChannelBeta,
          },
          onChanged: (ch) => unawaited(c.setChannel(ch)),
        ),
      ],
    );
  }

  static String _when(BuildContext context, DateTime t) {
    final m = MaterialLocalizations.of(context);
    final local = t.toLocal();
    return '${m.formatMediumDate(local)}, '
        '${m.formatTimeOfDay(TimeOfDay.fromDateTime(local), alwaysUse24HourFormat: true)}';
  }
}

bool _running(UpdateController c) => switch (c.phase) {
  UpdatePhase.downloading ||
  UpdatePhase.verifying ||
  UpdatePhase.installing => true,
  _ => false,
};

/// One line about where the update flow is.
String updateStatusText(BuildContext context, UpdateController c) {
  final l = context.l10n;
  final version = c.release?.version.text ?? '';
  return switch (c.phase) {
    UpdatePhase.idle => l.updatesCurrent(c.current.text),
    UpdatePhase.checking => l.updatesChecking,
    UpdatePhase.upToDate => l.updatesUpToDate(c.current.text),
    UpdatePhase.available => l.updatesAvailable(version),
    UpdatePhase.downloading =>
      c.progress == null
          ? l.updatesDownloadingUnknown
          : l.updatesDownloading('${(c.progress! * 100).floor()}'),
    UpdatePhase.verifying => l.updatesVerifying,
    UpdatePhase.installing => l.updatesInstalling,
    UpdatePhase.restartNeeded when c.restartsQuietly => l.updatesQuietReady(
      version,
    ),
    UpdatePhase.restartNeeded =>
      c.target.kind == UpdateKind.windowsInstaller
          ? l.updatesReadyToInstall(version)
          : l.updatesRestartNeeded(version),
    UpdatePhase.readyToInstall => l.updatesApkReady(version),
    UpdatePhase.installed => l.updatesInstalled(kAppName),
    UpdatePhase.handedOver => l.updatesDmgOpened(kAppName),
    UpdatePhase.failed => updateProblemText(context, c),
  };
}

/// What went wrong, in words.
String updateProblemText(BuildContext context, UpdateController c) {
  final l = context.l10n;
  return switch (c.problem) {
    UpdateProblem.network || null => l.updatesErrorNetwork,
    UpdateProblem.rateLimited => l.updatesErrorRateLimited,
    UpdateProblem.noAsset => l.updatesErrorNoAsset,
    UpdateProblem.checksum => l.updatesErrorChecksum,
    UpdateProblem.signature => l.updatesErrorSignature,
    UpdateProblem.signatureMismatch => l.updatesErrorSignatureMismatch(
      kAppName,
    ),
    UpdateProblem.needsPermission => l.updatesErrorNeedsPermission(kAppName),
    UpdateProblem.aborted => l.updatesErrorAborted,
    UpdateProblem.notWritable => l.updatesErrorNotWritable,
    UpdateProblem.killed => l.updatesErrorKilled,
    UpdateProblem.installFailed => l.updatesErrorInstall(c.detail ?? '?'),
  };
}

/// The buttons that fit the state and this installation.
List<Widget> _actions(BuildContext context, UpdateController c) {
  final l = context.l10n;
  final release = c.release;
  if (release == null) return const [];
  Widget page({bool primary = false}) => KButton(
    key: const ValueKey('updates-release-page'),
    label: l.updatesOpenReleasePage,
    icon: Icons.open_in_new,
    primary: primary,
    onPressed: () => unawaited(_open(context, release.page)),
  );
  switch (c.phase) {
    case UpdatePhase.downloading:
      return [
        KButton(
          key: const ValueKey('updates-cancel'),
          label: l.commonCancel,
          icon: Icons.close,
          onPressed: c.cancel,
        ),
      ];
    case UpdatePhase.verifying || UpdatePhase.installing:
    case UpdatePhase.installed:
    case UpdatePhase.handedOver:
      return const [];
    case UpdatePhase.restartNeeded:
      return [
        KButton(
          key: const ValueKey('updates-restart'),
          label: l.updatesRestart,
          icon: Icons.restart_alt,
          primary: true,
          onPressed: () => unawaited(c.restart()),
        ),
      ];
    case UpdatePhase.failed when c.problem == UpdateProblem.needsPermission:
      return [
        KButton(
          key: const ValueKey('updates-allow'),
          label: l.updatesAllowInstalls,
          icon: Icons.security,
          primary: true,
          onPressed: () => unawaited(c.openInstallPermission()),
        ),
        KButton(
          key: const ValueKey('updates-install'),
          label: l.updatesTryAgain,
          icon: Icons.download,
          onPressed: () => unawaited(c.update()),
        ),
      ];
    case UpdatePhase.failed
        when c.problem == UpdateProblem.signatureMismatch ||
            c.problem == UpdateProblem.notWritable ||
            c.problem == UpdateProblem.killed ||
            c.problem == UpdateProblem.noAsset:
      return [page(primary: true)];
    default:
      if (!c.canInstall) {
        // Only notifies (the Windows zip, packages, tar.gz): the file of
        // this platform, and the page with the rest.
        return [
          if (c.target.assetIn(release) != null)
            KButton(
              key: const ValueKey('updates-download-file'),
              label: l.updatesDownload,
              icon: Icons.download,
              primary: true,
              onPressed: () =>
                  unawaited(_open(context, c.target.downloadLink(release))),
            ),
          page(primary: c.target.assetIn(release) == null),
        ];
      }
      return [
        KButton(
          key: const ValueKey('updates-install'),
          label: switch (c.phase) {
            UpdatePhase.failed => l.updatesTryAgain,
            UpdatePhase.readyToInstall => l.updatesInstallNow,
            _ => l.updatesInstall,
          },
          icon: Icons.download,
          primary: true,
          onPressed: () => unawaited(c.update()),
        ),
        page(),
      ];
  }
}

/// "What's new" of the offered release, and how to update by hand where
/// the app does not install.
class _Changelog extends StatelessWidget {
  const _Changelog({required this.controller, required this.release});

  final UpdateController controller;
  final AppRelease release;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final c = controller;
    final hint = switch (c.target.kind) {
      UpdateKind.linuxArchive => l.updatesArchiveHint(
        c.target.assetIn(release)?.name ?? 'tar.gz', // l10n-ignore
      ),
      UpdateKind.windowsPortable => l.updatesPortableHint,
      UpdateKind.linuxPackage => l.updatesPackageHint(
        c.target.packageType ?? '?',
      ),
      UpdateKind.releasePage => l.updatesReleasePageHint,
      _ => null,
    };
    final notes = release.changelog;
    return Container(
      key: const ValueKey('updates-changelog'),
      margin: const EdgeInsets.symmetric(horizontal: 12),
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: KColors.bgElevated2,
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: KColors.stroke),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (hint != null) ...[
            Text(hint, style: theme.textTheme.bodyMedium),
            const SizedBox(height: 12),
          ],
          Text(l.updatesWhatsNew, style: theme.textTheme.titleSmall),
          const SizedBox(height: 6),
          ConstrainedBox(
            constraints: const BoxConstraints(maxHeight: 260),
            child: SingleChildScrollView(
              child: SelectableText(
                notes.isEmpty ? l.updatesNoNotes : notes,
                style: theme.textTheme.bodySmall,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// The strip across the top of the app (app.dart, PKG-022): «Доступна
/// версия X» with the changelog, «Скачать» and «Позже»; the download's
/// progress; «Перезапустить» once the update is ready; what went wrong.
class UpdateBanner extends StatefulWidget {
  const UpdateBanner({
    super.key,
    required this.controller,
    this.safeTop = true,
    this.showAvailable = true,
  });

  final UpdateController controller;
  final bool safeTop;

  /// Also for a release that is only available (false: the strip appears
  /// once an update runs, needs a restart or failed).
  final bool showAvailable;

  @override
  State<UpdateBanner> createState() => _UpdateBannerState();
}

class _UpdateBannerState extends State<UpdateBanner> {
  bool _notes = false;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.controller,
    builder: (context, _) {
      final c = widget.controller;
      if (!c.bannerVisible ||
          (!widget.showAvailable && c.phase == UpdatePhase.available)) {
        // PKG-024: after an update, a short «Что нового» instead.
        return c.whatsNewVisible
            ? _whatsNewStrip(context, c, c.whatsNew!)
            : const SizedBox.shrink();
      }
      final l = context.l10n;
      final release = c.release!;
      final version = release.version.text;
      // Short: the whole explanation is in About.
      final text = switch (c.phase) {
        UpdatePhase.available => l.updatesAvailable(version),
        UpdatePhase.failed when c.problem == UpdateProblem.signatureMismatch =>
          l.updatesBannerReinstall(kAppName, version),
        UpdatePhase.failed when c.problem == UpdateProblem.needsPermission =>
          l.updatesErrorNeedsPermission(kAppName),
        UpdatePhase.failed => l.updatesBannerFailed(kAppName, version),
        _ => updateStatusText(context, c),
      };
      final notes = release.changelog;
      final showsNotes =
          c.phase == UpdatePhase.available ||
          c.phase == UpdatePhase.restartNeeded ||
          c.phase == UpdatePhase.readyToInstall;
      final actions = <Widget>[
        if (showsNotes && notes.isNotEmpty)
          TextButton(
            key: const ValueKey('update-banner-notes'),
            onPressed: () => setState(() => _notes = !_notes),
            child: Text(_notes ? l.updatesHideNotes : l.updatesWhatsNew),
          ),
        if (c.phase == UpdatePhase.available)
          FilledButton(
            key: const ValueKey('update-banner-update'),
            onPressed: () => unawaited(
              c.canInstall
                  ? c.update()
                  : _open(context, c.target.downloadLink(release)),
            ),
            child: Text(l.updatesDownload),
          ),
        if (c.phase == UpdatePhase.readyToInstall)
          FilledButton(
            key: const ValueKey('update-banner-install'),
            onPressed: () => unawaited(c.update()),
            child: Text(l.updatesInstallNow),
          ),
        if (c.phase == UpdatePhase.restartNeeded)
          FilledButton(
            key: const ValueKey('update-banner-restart'),
            onPressed: () => unawaited(c.restart()),
            child: Text(l.updatesRestart),
          ),
        if (c.phase == UpdatePhase.failed &&
            c.problem == UpdateProblem.needsPermission)
          TextButton(
            onPressed: () => unawaited(c.openInstallPermission()),
            child: Text(l.updatesAllowInstalls),
          ),
        if (c.phase == UpdatePhase.failed &&
            c.problem == UpdateProblem.needsPermission)
          TextButton(
            key: const ValueKey('update-banner-retry'),
            onPressed: () => unawaited(c.update()),
            child: Text(l.updatesTryAgain),
          ),
        if (c.phase == UpdatePhase.failed &&
            c.problem == UpdateProblem.signatureMismatch)
          TextButton(
            onPressed: () => unawaited(_open(context, release.page)),
            child: Text(l.updatesOpenReleasePage),
          ),
        if (!_running(c))
          TextButton(
            key: const ValueKey('update-banner-later'),
            onPressed: () => unawaited(c.dismiss()),
            child: Text(l.updatesLater),
          ),
      ];
      return _strip(
        context,
        key: const ValueKey('update-banner'),
        text: text,
        actions: actions,
        progress: c.phase == UpdatePhase.downloading,
        progressValue: c.progress,
        notes: _notes && showsNotes && notes.isNotEmpty ? notes : null,
        notesKey: const ValueKey('update-banner-changelog'),
      );
    },
  );

  /// The note after an update (PKG-024): «Kinolune обновлён до X», its
  /// changelog under «Что нового», «Закрыть».
  Widget _whatsNewStrip(BuildContext context, UpdateController c, WhatsNew w) {
    final l = context.l10n;
    final notes = w.notes;
    return _strip(
      context,
      key: const ValueKey('update-whats-new'),
      text: l.updatesUpdatedTo(kAppName, w.version.text),
      actions: [
        if (notes.isNotEmpty)
          TextButton(
            key: const ValueKey('update-whats-new-notes'),
            onPressed: () => setState(() => _notes = !_notes),
            child: Text(_notes ? l.updatesHideNotes : l.updatesWhatsNew),
          ),
        TextButton(
          key: const ValueKey('update-whats-new-close'),
          onPressed: () => unawaited(c.dismissWhatsNew()),
          child: Text(l.commonDismiss),
        ),
      ],
      notes: _notes && notes.isNotEmpty ? notes : null,
      notesKey: const ValueKey('update-whats-new-changelog'),
    );
  }

  Widget _strip(
    BuildContext context, {
    required Key key,
    required String text,
    required List<Widget> actions,
    bool progress = false,
    double? progressValue,
    String? notes,
    required Key notesKey,
  }) {
    final theme = Theme.of(context);
    final wide = MediaQuery.sizeOf(context).width >= 600;
    return Material(
      key: key,
      color: KColors.bgElevated2,
      child: SafeArea(
        top: widget.safeTop,
        bottom: false,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 8, 8),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                children: [
                  Icon(Icons.system_update_alt, color: KColors.accentSoft),
                  const SizedBox(width: 12),
                  Expanded(
                    child: Text(
                      key: const ValueKey('update-banner-text'),
                      text,
                      style: theme.textTheme.titleSmall,
                    ),
                  ),
                  if (wide)
                    Wrap(
                      spacing: 6,
                      crossAxisAlignment: WrapCrossAlignment.center,
                      children: actions,
                    ),
                ],
              ),
              if (!wide)
                Align(
                  alignment: Alignment.centerRight,
                  child: Wrap(
                    spacing: 6,
                    crossAxisAlignment: WrapCrossAlignment.center,
                    children: actions,
                  ),
                ),
              if (progress)
                Padding(
                  padding: const EdgeInsets.only(top: 6, right: 8),
                  child: LinearProgressIndicator(value: progressValue),
                ),
              if (notes != null)
                Padding(
                  padding: const EdgeInsets.fromLTRB(36, 6, 8, 4),
                  child: ConstrainedBox(
                    constraints: const BoxConstraints(maxHeight: 200),
                    child: SingleChildScrollView(
                      child: SelectableText(
                        key: notesKey,
                        notes,
                        style: theme.textTheme.bodySmall,
                      ),
                    ),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}
