// SPDX-License-Identifier: MPL-2.0
//
// Settings (UI-016, UI-041): one scrolling page. A fresh install shows only
// the basic sections — Profiles, Library, Language, Devices, Downloads,
// Updates and about; «Показать расширенные настройки» at their bottom
// (remembered) adds Playback, Shows, Trailers, Music, Metadata, Network,
// Twitch, Personal storage, Storage, Privacy and Diagnostics. The search
// field on top finds basic and advanced sections alike. Every row is
// focusable for the TV remote.

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:intl/intl.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart'
    show ReplayGainMode, SkipMode, kSkipStepsSeconds;

import '../review/review_inbox_screen.dart';
import '../branding.dart';
import '../diagnostics/diagnostics_section.dart';
import '../files/storage_settings.dart';
import '../flavor.dart';
import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../profiles/profile_avatar.dart';
import '../profiles/profile_flows.dart';
import '../profiles/profile_picker.dart';
import '../profiles/profiles_screen.dart';
import '../services/app_services.dart';
import '../services/image_cache_control.dart';
import '../theme/kinolune_theme.dart';
import '../trailers/trailer_settings_section.dart';
import '../widgets/cards.dart';
import '../widgets/focusable.dart';
import '../widgets/secret_storage_text.dart';
import '../widgets/settings_tiles.dart';
import '../widgets/scan_status.dart';
import 'about_screen.dart';
import 'add_source_dialog.dart';
import 'anime_db_settings.dart';
import 'devices_screen.dart';
import 'downloads_screen.dart';
import 'onboarding_screen.dart';
import 'show_detail_screen.dart' show orderLabel;
import 'sources_screen.dart';
import 'subtitle_settings_screen.dart';
import 'track_settings_screen.dart';
import 'twitch/twitch_archive_screens.dart';

/// One section of the page: what it is called and contains for the search,
/// and whether it waits for «Показать расширенные настройки».
class _Entry {
  const _Entry(this.child, {required this.terms, this.advanced = false});

  final Widget child;
  final List<String> terms;
  final bool advanced;

  bool matches(String query) =>
      terms.any((t) => t.toLowerCase().contains(query));
}

class SettingsScreen extends StatefulWidget {
  const SettingsScreen({super.key});

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen> {
  final _query = TextEditingController();
  final _field = FocusNode(debugLabel: 'settings-search');

  @override
  void dispose() {
    _query.dispose();
    _field.dispose();
    super.dispose();
  }

  KeyEventResult _fieldKey(FocusNode _, KeyEvent event) {
    if (event is! KeyDownEvent) return KeyEventResult.ignored;
    final key = event.logicalKey;
    if (key == LogicalKeyboardKey.escape && _query.text.isNotEmpty) {
      _query.clear();
      return KeyEventResult.handled;
    }
    // The single-line field would move the caret; the remote moves on.
    if (key == LogicalKeyboardKey.arrowDown) {
      _field.focusInDirection(TraversalDirection.down);
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.arrowUp) {
      return _field.focusInDirection(TraversalDirection.up)
          ? KeyEventResult.handled
          : KeyEventResult.ignored;
    }
    return KeyEventResult.ignored;
  }

  /// The sections in page order, basic first. Search terms are the visible
  /// titles of each section and its rows.
  List<_Entry> _entries(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    final offline = services.offline;
    final reducedSources = services.sourceManager.reduced;
    return [
      // ---------------------------------------------------------- basic
      if (services.profileControl.available)
        _Entry(
          const _ProfilesSection(),
          terms: [
            l.profilesTitle,
            l.profilesSectionSubtitle,
            l.profilesSwitch,
            l.profilesManage,
            l.profilesPickerOnStart,
            l.profilesAppLock,
            l.profilesSearchLock,
          ],
        ),
      _Entry(
        const _LibrarySection(),
        terms: [
          l.settingsLibrarySourcesTitle,
          l.sourcesManage,
          l.addSourceTitle,
          l.settingsScanNow,
          for (final s in services.sources) s.name,
        ],
      ),
      _Entry(
        const _LanguageSection(),
        terms: [
          l.settingsLanguageTitle,
          l.settingsLanguageBasicSubtitle,
          l.settingsLanguagesSound,
          l.tracksAudioLanguage,
          l.tracksSubtitleLanguage,
        ],
      ),
      _Entry(
        const _DevicesSection(),
        terms: [
          l.settingsDevicesTitle,
          l.settingsDevicesSubtitle,
          l.devicesTitle,
        ],
      ),
      // NET-019: offline downloads.
      if (offline != null)
        _Entry(
          ListenableBuilder(
            listenable: offline,
            builder: (context, _) =>
                OfflineSettingsSection(offline: offline, showOpen: true),
          ),
          terms: [
            l.offlineSettingsTitle,
            l.offlineSettingsSubtitle,
            l.offlineSettingsFolder,
            l.offlineSettingsWifiOnly,
            l.offlineSettingsDeleteWatched,
          ],
        ),
      _Entry(
        const _AboutSection(),
        terms: [
          l.settingsUpdatesAboutTitle,
          l.settingsAboutTitle,
          l.settingsAboutApp(kAppName),
          l.settingsOnboarding,
        ],
      ),
      // ------------------------------------------------------- advanced
      _Entry(
        const _PlaybackSection(),
        advanced: true,
        terms: [
          l.settingsPlaybackTitle,
          l.subtitlesStyleTitle,
          l.subtitlesOnlineTitle,
          'OpenSubtitles',
          l.settingsContinuousPlayback,
          l.settingsSkipInterval,
          l.settingsSkipSegments,
          l.settingsTrickplay,
          l.settingsTakeoverPause,
        ],
      ),
      _Entry(
        const _ShowsSection(),
        advanced: true,
        terms: [
          l.settingsShowsTitle,
          l.settingsHideSpoilers,
          l.settingsEpisodeOrder,
        ],
      ),
      // UI-036: trailer autoplay and internet trailers.
      _Entry(
        const TrailerSettingsSection(),
        advanced: true,
        terms: [
          l.settingsTrailersTitle,
          l.settingsTrailersAutoplay,
          l.settingsTrailersInternet,
        ],
      ),
      _Entry(
        const _MusicSection(),
        advanced: true,
        terms: [
          l.settingsMusicTitle,
          l.settingsMusicGapless,
          l.settingsMusicReplayGain,
        ],
      ),
      _Entry(
        const _MetadataSection(),
        advanced: true,
        terms: [
          l.settingsMetadataTitle,
          'TMDB',
          'TheTVDB',
          'Fanart.tv',
          l.settingsTmdbKeyLabel,
          l.settingsMetadataLanguage,
          l.settingsAnimeDbTitle,
          l.settingsAnimeFrameSearch,
          l.settingsExtraProvidersTitle,
        ],
      ),
      // NET-006: RSA keys of SFTP sources (RUSTSEC-2023-0071).
      if (!reducedSources)
        _Entry(
          const _NetworkSection(),
          advanced: true,
          terms: [
            l.settingsNetworkTitle,
            l.settingsNetworkSubtitle,
            l.settingsSshRsaKeys,
            'SFTP',
          ],
        ),
      _Entry(
        const _TwitchSection(),
        advanced: true,
        terms: [
          l.settingsTwitchTitle,
          'Twitch',
          l.settingsShowTwitch,
          l.settingsTwitchLiveNotifications,
          l.twitchArchiveSettings,
          l.twitchChatPauseAnimations,
        ],
      ),
      // UI-035: where uploads to «Личное» go.
      if (services.filesControl != null)
        _Entry(
          const PersonalStorageSettingsSection(),
          advanced: true,
          terms: [l.filesStorageTitle, l.filesStorageHint],
        ),
      _Entry(
        const _StorageSection(),
        advanced: true,
        terms: [
          l.settingsStorageTitle,
          l.settingsStorageSubtitle,
          l.settingsImageCache,
          l.settingsImageCacheLimit,
          l.settingsImageCacheClear,
        ],
      ),
      _Entry(
        const _PrivacySection(),
        advanced: true,
        terms: [
          l.settingsPrivacyTitle,
          l.settingsPrivacySecrets,
          l.settingsPrivacyClearSearch,
          l.settingsPrivacyPolicy,
        ],
      ),
      // CORE-008: opt-in logs, crash reports, the diagnostic report.
      _Entry(
        const DiagnosticsSettingsSection(),
        advanced: true,
        terms: [
          l.settingsDiagnosticsTitle,
          l.settingsDiagnosticsSubtitle,
          l.settingsDiagnosticsKeep,
          l.statusSettingsTile,
          l.settingsDiagnosticsExport,
          l.settingsDiagnosticsClear,
        ],
      ),
    ];
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final services = Services.of(context);
    final entries = _entries(context);

    Widget column(Widget child) => Padding(
      padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
      child: Align(
        alignment: Alignment.topLeft,
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 760),
          child: child,
        ),
      ),
    );

    final search = Padding(
      padding: const EdgeInsets.only(bottom: 20),
      child: Focus(
        canRequestFocus: false,
        skipTraversal: true,
        onKeyEvent: _fieldKey,
        child: ListenableBuilder(
          listenable: _query,
          builder: (context, _) => TextField(
            key: const ValueKey('settings-search'),
            controller: _query,
            focusNode: _field,
            textInputAction: TextInputAction.search,
            style: Theme.of(context).textTheme.bodyLarge,
            decoration: InputDecoration(
              prefixIcon: const Icon(Icons.search),
              hintText: l.settingsSearchHint,
              suffixIcon: _query.text.isEmpty
                  ? null
                  : IconButton(
                      key: const ValueKey('settings-search-clear'),
                      tooltip: l.commonClear,
                      onPressed: _query.clear,
                      icon: const Icon(Icons.close),
                    ),
            ),
          ),
        ),
      ),
    );

    // Not lazy: a few panels only, and every row stays reachable by focus
    // traversal (TV remote) and by `ensureVisible`.
    return SingleChildScrollView(
      padding: EdgeInsets.only(bottom: d.pageInsets.bottom + 90),
      child: ListenableBuilder(
        listenable: Listenable.merge([_query, services.showAdvancedSettings]),
        builder: (context, _) {
          final query = _query.text.trim().toLowerCase();
          final advanced = services.showAdvancedSettings.value;
          final List<Widget> body;
          if (query.isNotEmpty) {
            final found = [
              for (final (i, e) in entries.indexed)
                if (e.matches(query)) KeyedSubtree(key: ValueKey(i), child: e.child),
            ];
            body = found.isNotEmpty
                ? found
                : [
                    Padding(
                      padding: const EdgeInsets.symmetric(vertical: 24),
                      child: Text(
                        l.settingsSearchEmpty(_query.text.trim()),
                        key: const ValueKey('settings-search-empty'),
                        style: Theme.of(context).textTheme.bodyMedium,
                      ),
                    ),
                  ];
          } else {
            body = [
              for (final (i, e) in entries.indexed)
                if (!e.advanced) KeyedSubtree(key: ValueKey(i), child: e.child),
              Padding(
                padding: EdgeInsets.only(bottom: d.isTv ? 18 : 24),
                child: SettingsSwitchTile(
                  key: const ValueKey('settings-advanced'),
                  icon: Icons.tune,
                  title: l.settingsAdvancedShow,
                  subtitle: l.settingsAdvancedShowSubtitle,
                  value: advanced,
                  onChanged: services.setShowAdvancedSettings,
                ),
              ),
              if (advanced) ...[
                Padding(
                  padding: const EdgeInsets.only(bottom: 12),
                  child: Semantics(
                    header: true,
                    child: Text(
                      l.settingsAdvancedHeader,
                      key: const ValueKey('settings-advanced-header'),
                      style: Theme.of(context).textTheme.headlineSmall,
                    ),
                  ),
                ),
                for (final (i, e) in entries.indexed)
                  if (e.advanced)
                    KeyedSubtree(key: ValueKey(i), child: e.child),
              ],
            ];
          }
          return Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              PageTitle(l.navSettings),
              column(search),
              for (final s in body)
                KeyedSubtree(key: s.key, child: column(s)),
            ],
          );
        },
      ),
    );
  }
}

void _push(BuildContext context, Widget screen) =>
    Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => screen));

void _say(BuildContext context, String text) =>
    ScaffoldMessenger.maybeOf(context)
        ?.showSnackBar(SnackBar(content: Text(text)));

// ---------------------------------------------------------------- profiles

/// Profiles and parental control (UI-015): the active profile, switching,
/// management and the locks. Changes that weaken protection ask for the
/// parent PIN.
class _ProfilesSection extends StatelessWidget {
  const _ProfilesSection();

  Future<void> _toggle(
    BuildContext context,
    Future<void> Function() change, {
    bool needsPin = false,
  }) async {
    if (!await ensureParent(context) || !context.mounted) return;
    if (needsPin && !await ensureParentPinExists(context)) return;
    try {
      await change();
    } catch (e) {
      if (context.mounted) sayProfileError(context, e);
    }
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final ctl = Services.of(context).profileControl;
    return ListenableBuilder(
      listenable: ctl,
      builder: (context, _) {
        final active = ctl.active;
        return SettingsSection(
          key: const ValueKey('settings-section-profiles'),
          icon: Icons.people_outline,
          title: l.profilesTitle,
          subtitle: l.profilesSectionSubtitle,
          children: [
            if (active != null)
              Padding(
                padding: const EdgeInsets.only(bottom: 8),
                child: Row(
                  children: [
                    ProfileAvatar.of(active, size: 44),
                    const SizedBox(width: 14),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(
                            profileName(l, active),
                            style: Theme.of(context).textTheme.titleMedium,
                          ),
                          Text(
                            profileSummary(l, active, active: true),
                            style: Theme.of(context).textTheme.bodySmall,
                          ),
                        ],
                      ),
                    ),
                  ],
                ),
              ),
            if (ctl.all.length > 1)
              SettingsTile(
                key: const ValueKey('settings-profiles-switch'),
                icon: Icons.switch_account_outlined,
                title: l.profilesSwitch,
                onTap: () => _push(context, const ProfilePickerScreen()),
              ),
            SettingsTile(
              key: const ValueKey('settings-profiles-manage'),
              icon: Icons.manage_accounts_outlined,
              title: l.profilesManage,
              subtitle: l.profilesManageSubtitle,
              onTap: () async {
                if (await ensureParent(context) && context.mounted) {
                  _push(context, const ProfilesScreen());
                }
              },
            ),
            SettingsSwitchTile(
              key: const ValueKey('settings-profiles-picker'),
              icon: Icons.groups_outlined,
              title: l.profilesPickerOnStart,
              subtitle: l.profilesPickerOnStartSubtitle,
              value: ctl.pickerOnStart,
              onChanged: (v) => _toggle(context, () => ctl.setPickerOnStart(v)),
            ),
            SettingsSwitchTile(
              key: const ValueKey('settings-profiles-app-lock'),
              icon: Icons.lock_outline,
              title: l.profilesAppLock,
              subtitle: l.profilesAppLockSubtitle,
              value: ctl.appLock,
              onChanged: (v) =>
                  _toggle(context, () => ctl.setAppLock(v), needsPin: v),
            ),
            SettingsSwitchTile(
              key: const ValueKey('settings-profiles-search-lock'),
              icon: Icons.screen_search_desktop_outlined,
              title: l.profilesSearchLock,
              subtitle: l.profilesSearchLockSubtitle,
              value: ctl.searchLock,
              onChanged: (v) =>
                  _toggle(context, () => ctl.setSearchLock(v), needsPin: v),
            ),
            if (ctl.restricted)
              Text(
                l.profilesRestrictedNote,
                style: Theme.of(context).textTheme.bodySmall,
              ),
          ],
        );
      },
    );
  }
}

// ------------------------------------------------------- library & sources

/// Sources, their management and the library scan.
class _LibrarySection extends StatelessWidget {
  const _LibrarySection();

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    return SettingsSection(
      key: const ValueKey('settings-section-library'),
      icon: Icons.video_library_outlined,
      title: l.settingsLibrarySourcesTitle,
      subtitle: l.settingsSourcesSubtitle(kAppName),
      children: [
        if (services.sources.isEmpty)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 8),
            child: Text(
              l.settingsNoSources,
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ),
        for (final s in services.sources) _SourceTile(source: s),
        SettingsTile(
          icon: Icons.tune,
          title: l.sourcesManage,
          subtitle: l.sourcesManageSubtitle,
          onTap: () => _push(context, const SourcesScreen()),
        ),
        const SizedBox(height: 8),
        Align(
          alignment: Alignment.centerLeft,
          child: KButton(
            label: l.addSourceTitle,
            icon: Icons.add,
            primary: services.sources.isEmpty,
            onPressed: () => showAddSourceDialog(context),
          ),
        ),
        const SizedBox(height: 16),
        const _ScanStatus(),
        // META-016: recognitions to check or make.
        const ReviewInboxTile(),
      ],
    );
  }
}

class _SourceTile extends StatelessWidget {
  const _SourceTile({required this.source});

  final MediaSource source;

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final smb = source.kind == SourceKind.smb;
    final l = context.l10n;
    return SettingsTile(
      icon: smb ? Icons.lan_outlined : Icons.folder_outlined,
      title: smb ? '${source.name}  ${l.settingsSourceSmbSuffix}' : source.name,
      subtitle: [
        source.path,
        if (source.username != null) l.settingsSourceUser(source.username!),
      ].join('  ·  '),
      trailing: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          IconButton(
            tooltip: l.settingsScanSource,
            onPressed: services.scanning
                ? null
                : () => services.scan(sourceId: source.id),
            icon: const Icon(Icons.refresh),
          ),
          IconButton(
            tooltip: l.settingsRemoveSource,
            onPressed: () async {
              final ok = await showDialog<bool>(
                context: context,
                builder: (c) => AlertDialog(
                  title: Text(l.settingsRemoveSourceTitle),
                  content: Text(l.settingsRemoveSourceBody(source.name)),
                  actions: [
                    TextButton(
                      onPressed: () => Navigator.pop(c, false),
                      child: Text(l.commonCancel),
                    ),
                    FilledButton(
                      onPressed: () => Navigator.pop(c, true),
                      child: Text(l.settingsRemove),
                    ),
                  ],
                ),
              );
              if (ok == true) await services.deleteSource(source.id);
            },
            icon: const Icon(Icons.delete_outline),
          ),
        ],
      ),
    );
  }
}

/// Automatic recognition (LIB-024): when the sources were last checked by
/// themselves and whether recognition still runs or how much waits.
class _AutoRecognitionStatus extends StatelessWidget {
  const _AutoRecognitionStatus({required this.status});

  final ValueListenable<RecognitionStatus?> status;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    return ValueListenableBuilder<RecognitionStatus?>(
      valueListenable: status,
      builder: (context, s, _) {
        if (s == null) return const SizedBox.shrink();
        final String check;
        final at = s.lastCheck;
        if (at == null) {
          check = l.settingsAutoCheckNotYet;
        } else if (s.lastCheckError != null) {
          check = l.settingsAutoCheckFailed(s.lastCheckError!);
        } else {
          final ago = DateTime.now().difference(at);
          final String when;
          if (ago < const Duration(minutes: 1)) {
            when = l.settingsAutoCheckJustNow;
          } else if (ago < const Duration(hours: 1)) {
            when = l.settingsAutoCheckMinutesAgo(ago.inMinutes);
          } else {
            when = MaterialLocalizations.of(context)
                .formatTimeOfDay(TimeOfDay.fromDateTime(at));
          }
          check = l.settingsAutoCheckLast(when);
        }
        final progress = s.progress;
        final String recognition;
        if (s.running && progress != null) {
          recognition = scanStatusText(l, progress);
        } else if (s.running) {
          recognition = l.settingsRecognitionRunning;
        } else if (s.waiting > 0) {
          recognition = l.settingsRecognitionWaiting(s.waiting);
        } else {
          recognition = l.settingsRecognitionDone;
        }
        final style = theme.textTheme.bodySmall;
        return Padding(
          padding: const EdgeInsets.only(top: 8),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(l.settingsLibraryAutoHint, style: style),
              const SizedBox(height: 6),
              Text(
                check,
                key: const ValueKey('settings-auto-check'),
                style: style,
              ),
              Text(
                recognition,
                key: const ValueKey('settings-recognition-status'),
                style: style,
              ),
              if (s.running && s.waiting > 0 && progress == null)
                Text(l.settingsRecognitionWaiting(s.waiting), style: style),
              if (!s.running && s.worksFailed > 0)
                Text(l.settingsRecognitionRetry(s.worksFailed), style: style),
            ],
          ),
        );
      },
    );
  }
}

/// "Scan now" with the progress of the running scan.
class _ScanStatus extends StatelessWidget {
  const _ScanStatus();

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final p = services.scanProgress;
    final theme = Theme.of(context);
    final l = context.l10n;
    final String status;
    if (p == null) {
      status = l.settingsScanNever;
    } else if (services.scanning) {
      status = scanStatusText(l, p);
    } else if (p.error != null) {
      status = l.settingsScanFailed(p.error!);
    } else {
      status = l.settingsScanUpToDate(p.filesFound);
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(l.settingsLibrarySubtitle, style: theme.textTheme.bodySmall),
        // LIB-024: recognition runs on its own; the button stays.
        _AutoRecognitionStatus(status: services.recognitionStatus),
        const SizedBox(height: 10),
        Row(
          children: [
            KButton(
              label: services.scanning ? l.settingsScanning : l.settingsScanNow,
              icon: Icons.sync,
              onPressed: services.scanning || services.sources.isEmpty
                  ? null
                  : () => services.scan(),
            ),
            const SizedBox(width: 16),
            Expanded(child: Text(status, style: theme.textTheme.bodySmall)),
          ],
        ),
        if (p != null && services.scanning) ...[
          const SizedBox(height: 12),
          ClipRRect(
            borderRadius: BorderRadius.circular(4),
            child: LinearProgressIndicator(
              value: (p.total ?? 0) == 0 ? null : p.processed / p.total!,
              minHeight: 6,
            ),
          ),
          if (p.currentPath != null) ...[
            const SizedBox(height: 6),
            Text(
              p.currentPath!,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: theme.textTheme.labelSmall,
            ),
          ],
        ],
      ],
    );
  }
}

// ---------------------------------------------------------------- playback

/// Subtitle style (PLAY-007), languages and sound (PLAY-004), continuous
/// playback and the takeover pause (SYNC-003).
class _PlaybackSection extends StatelessWidget {
  const _PlaybackSection();

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    return SettingsSection(
      key: const ValueKey('settings-section-playback'),
      icon: Icons.play_circle_outline,
      title: l.settingsPlaybackTitle,
      subtitle: l.settingsPlaybackSubtitle,
      children: [
        ValueListenableBuilder(
          valueListenable: services.subtitleSettings,
          builder: (context, settings, _) => SettingsTile(
            icon: Icons.subtitles_outlined,
            title: l.subtitlesStyleTitle,
            subtitle: settings.useSystemStyle
                ? l.subtitlesSummarySystem
                : l.subtitlesSummaryCustom,
            onTap: () => _push(context, const SubtitleSettingsScreen()),
          ),
        ),
        ValueListenableBuilder<bool>(
          valueListenable: services.continuousPlayback,
          builder: (context, on, _) => SettingsSwitchTile(
            key: const ValueKey('settings-continuous-playback'),
            icon: Icons.playlist_play,
            title: l.settingsContinuousPlayback,
            subtitle: l.settingsContinuousPlaybackSubtitle,
            value: on,
            onChanged: services.setContinuousPlayback,
          ),
        ),
        ValueListenableBuilder<int>(
          valueListenable: services.skipSeconds,
          builder: (context, seconds, _) => SettingsChoiceTile<int>(
            key: const ValueKey('settings-skip-interval'),
            icon: Icons.forward_10,
            title: l.settingsSkipInterval,
            hint: l.settingsSkipIntervalSubtitle,
            value: seconds,
            options: kSkipStepsSeconds,
            label: l.settingsSkipSeconds,
            onChanged: services.setSkipSeconds,
          ),
        ),
        ValueListenableBuilder<SkipMode>(
          valueListenable: services.skipMode,
          builder: (context, mode, _) => SettingsChoiceTile<SkipMode>(
            key: const ValueKey('settings-skip-mode'),
            icon: Icons.fast_forward_outlined,
            title: l.settingsSkipSegments,
            hint: l.settingsSkipSegmentsHint,
            value: mode,
            options: const [
              SkipMode.button,
              SkipMode.auto,
              SkipMode.instant,
              SkipMode.off,
            ],
            label: (m) => switch (m) {
              SkipMode.button => l.settingsSkipModeButton,
              SkipMode.auto => l.settingsSkipModeAuto,
              SkipMode.instant => l.settingsSkipModeInstant,
              SkipMode.off => l.settingsSkipModeOff,
            },
            onChanged: services.setSkipMode,
          ),
        ),
        if (services.library is LibraryTrickplay)
          ValueListenableBuilder<bool>(
            valueListenable: services.trickplayGenerate,
            builder: (context, on, _) => SettingsSwitchTile(
              key: const ValueKey('settings-trickplay'),
              icon: Icons.burst_mode_outlined,
              title: l.settingsTrickplay,
              subtitle: l.settingsTrickplaySubtitle,
              value: on,
              onChanged: services.setTrickplayGenerate,
            ),
          ),
        if (services.sync.available)
          ValueListenableBuilder<bool>(
            valueListenable: services.handoff.pauseOnTakeover,
            builder: (context, on, _) => SettingsSwitchTile(
              key: const ValueKey('settings-takeover-pause'),
              icon: Icons.pause_circle_outline,
              title: l.settingsTakeoverPause,
              subtitle: l.settingsTakeoverPauseSubtitle,
              value: on,
              onChanged: services.setPauseOnTakeover,
            ),
          ),
      ],
    );
  }
}

// ------------------------------------------------------------------- shows

/// Hide Spoilers and the default episode order (UI-010).
class _ShowsSection extends StatelessWidget {
  const _ShowsSection();

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    return SettingsSection(
      key: const ValueKey('settings-section-shows'),
      icon: Icons.tv_outlined,
      title: l.settingsShowsTitle,
      subtitle: l.settingsShowsSubtitle,
      children: [
        ValueListenableBuilder<bool>(
          valueListenable: services.hideSpoilers,
          builder: (context, on, _) => SettingsSwitchTile(
            key: const ValueKey('settings-hide-spoilers'),
            icon: Icons.visibility_off_outlined,
            title: l.settingsHideSpoilers,
            subtitle: l.settingsHideSpoilersSubtitle,
            value: on,
            onChanged: services.setHideSpoilers,
          ),
        ),
        ValueListenableBuilder<EpisodeOrder>(
          valueListenable: services.defaultEpisodeOrder,
          builder: (context, order, _) => SettingsChoiceTile<EpisodeOrder>(
            key: const ValueKey('settings-episode-order'),
            icon: Icons.format_list_numbered,
            title: l.settingsEpisodeOrder,
            hint: l.settingsEpisodeOrderSubtitle,
            value: order,
            options: EpisodeOrder.values,
            label: (o) => orderLabel(l, o),
            onChanged: services.setDefaultEpisodeOrder,
          ),
        ),
      ],
    );
  }
}

// ------------------------------------------------------------------- music

/// Gapless playback and ReplayGain (MUS-002).
class _MusicSection extends StatelessWidget {
  const _MusicSection();

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    final profile = services.music.audioProfile;
    return SettingsSection(
      key: const ValueKey('settings-section-music'),
      icon: Icons.music_note_outlined,
      title: l.settingsMusicTitle,
      subtitle: l.settingsMusicSubtitle,
      children: [
        SettingsSwitchTile(
          key: const ValueKey('settings-music-gapless'),
          icon: Icons.album_outlined,
          title: l.settingsMusicGapless,
          subtitle: l.settingsMusicGaplessSubtitle,
          value: profile.gapless,
          onChanged: (v) => services.setMusicAudio(gapless: v),
        ),
        SettingsChoiceTile<ReplayGainMode>(
          key: const ValueKey('settings-replay-gain'),
          icon: Icons.equalizer,
          title: l.settingsMusicReplayGain,
          value: profile.replayGain,
          options: const [
            ReplayGainMode.auto,
            ReplayGainMode.track,
            ReplayGainMode.album,
            ReplayGainMode.off,
          ],
          label: (m) => switch (m) {
            ReplayGainMode.auto => l.settingsReplayGainAuto,
            ReplayGainMode.track => l.settingsReplayGainTrack,
            ReplayGainMode.album => l.settingsReplayGainAlbum,
            ReplayGainMode.off => l.settingsReplayGainOff,
          },
          onChanged: (m) => services.setMusicAudio(replayGain: m),
        ),
      ],
    );
  }
}

// ----------------------------------------------------------------- devices

/// Pairing and LAN sync (UI-019).
class _DevicesSection extends StatelessWidget {
  const _DevicesSection();

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    return SettingsSection(
      key: const ValueKey('settings-section-devices'),
      icon: Icons.devices_other,
      title: l.settingsDevicesTitle,
      subtitle: l.settingsDevicesSubtitle,
      children: [
        SettingsTile(
          icon: Icons.devices,
          title: l.devicesTitle,
          subtitle: l.settingsDevicesCount(
            services.sync.devices().where((d) => !d.thisDevice).length,
          ),
          onTap: () => _push(context, const DevicesScreen()),
        ),
      ],
    );
  }
}

// ---------------------------------------------------------------- metadata

/// The TMDB key and the metadata language.
class _MetadataSection extends StatefulWidget {
  const _MetadataSection();

  @override
  State<_MetadataSection> createState() => _MetadataSectionState();
}

class _MetadataSectionState extends State<_MetadataSection> {
  final _key = TextEditingController();
  bool _loaded = false;
  bool _obscure = true;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_loaded) return;
    _loaded = true;
    Services.read(context).tmdbKey().then((k) {
      if (mounted && k != null) setState(() => _key.text = k);
    });
  }

  @override
  void dispose() {
    _key.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final services = Services.of(context);
    final l = context.l10n;
    String languageName(String? tag) => switch (tag) {
      null => l.settingsMetadataLanguageUi,
      'ru-RU' => l.settingsLanguageRussian,
      _ => l.settingsLanguageEnglish,
    };
    return SettingsSection(
      key: const ValueKey('settings-section-metadata'),
      icon: Icons.auto_awesome_outlined,
      title: l.settingsMetadataTitle,
      subtitle: l.settingsTmdbSubtitle(kAppName),
      children: [
        Row(
          children: [
            Expanded(
              child: TextField(
                controller: _key,
                obscureText: _obscure,
                decoration: InputDecoration(
                  labelText: l.settingsTmdbKeyLabel,
                  suffixIcon: IconButton(
                    tooltip: _obscure ? l.commonShow : l.commonHide,
                    onPressed: () => setState(() => _obscure = !_obscure),
                    icon: Icon(
                      _obscure
                          ? Icons.visibility_outlined
                          : Icons.visibility_off_outlined,
                    ),
                  ),
                ),
              ),
            ),
            const SizedBox(width: 12),
            KButton(
              label: l.commonSave,
              onPressed: () async {
                await Services.read(context).setTmdbKey(_key.text);
                if (!context.mounted) return;
                _say(
                  context,
                  _key.text.trim().isEmpty
                      ? l.settingsTmdbKeyRemoved
                      : l.settingsTmdbKeySaved,
                );
              },
            ),
          ],
        ),
        const SizedBox(height: 10),
        Text(
          '${l.settingsTmdbHint} ${l.commonTmdbNotice}',
          style: theme.textTheme.labelSmall,
        ),
        const SizedBox(height: 8),
        ValueListenableBuilder<String?>(
          valueListenable: services.metadataLanguage,
          builder: (context, tag, _) => SettingsChoiceTile<String?>(
            key: const ValueKey('settings-metadata-language'),
            icon: Icons.translate,
            title: l.settingsMetadataLanguage,
            hint: l.settingsMetadataLanguageSubtitle,
            value: tag,
            options: const [null, ...AppServices.metadataLanguages],
            label: languageName,
            onChanged: services.setMetadataLanguage,
          ),
        ),
        // META-030: the anime database and its daily update.
        const AnimeDbSettings(),
        // META-012: the optional TheTVDB and Fanart.tv keys.
        const _ExtraProviderKeys(),
      ],
    );
  }
}

/// The user's own TheTVDB key and personal Fanart.tv key (META-012). Both
/// are optional: TVmaze, Fanart.tv (with the app's project key) and the
/// OpenSubtitles fingerprint lookup work without them.
class _ExtraProviderKeys extends StatefulWidget {
  const _ExtraProviderKeys();

  @override
  State<_ExtraProviderKeys> createState() => _ExtraProviderKeysState();
}

class _ExtraProviderKeysState extends State<_ExtraProviderKeys> {
  final _tvdb = TextEditingController();
  final _tvdbPin = TextEditingController();
  final _fanart = TextEditingController();
  bool _loaded = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_loaded) return;
    _loaded = true;
    final services = Services.read(context);
    services.extraProviderKeys().then((k) {
      if (!mounted) return;
      setState(() {
        _tvdb.text = k.tvdb ?? '';
        _tvdbPin.text = k.tvdbPin ?? '';
        _fanart.text = k.fanart ?? '';
      });
    });
  }

  @override
  void dispose() {
    _tvdb.dispose();
    _tvdbPin.dispose();
    _fanart.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final l = context.l10n;
    Widget field(TextEditingController c, String label, String hint, Key key) =>
        Padding(
          padding: const EdgeInsets.only(top: 10),
          child: TextField(
            key: key,
            controller: c,
            obscureText: true,
            decoration: InputDecoration(
              labelText: label,
              helperText: hint,
              helperMaxLines: 4,
            ),
            maxLines: 1,
          ),
        );
    return Padding(
      key: const ValueKey('settings-extra-providers'),
      padding: const EdgeInsets.only(top: 16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            l.settingsExtraProvidersTitle,
            style: theme.textTheme.titleSmall,
          ),
          const SizedBox(height: 4),
          Text(l.settingsExtraProvidersHint, style: theme.textTheme.labelSmall),
          field(
            _tvdb,
            l.settingsTvdbKeyLabel,
            l.settingsTvdbHint,
            const ValueKey('settings-tvdb-key'),
          ),
          field(
            _tvdbPin,
            l.settingsTvdbPinLabel,
            l.settingsTvdbPinHint,
            const ValueKey('settings-tvdb-pin'),
          ),
          field(
            _fanart,
            l.settingsFanartKeyLabel,
            l.settingsFanartHint,
            const ValueKey('settings-fanart-key'),
          ),
          const SizedBox(height: 10),
          KButton(
            key: const ValueKey('settings-extra-keys-save'),
            label: l.commonSave,
            onPressed: () async {
              await Services.read(context).setExtraProviderKeys(
                tvdb: _tvdb.text,
                tvdbPin: _tvdbPin.text,
                fanart: _fanart.text,
              );
              if (!context.mounted) return;
              _say(context, l.settingsExtraKeysSaved);
            },
          ),
        ],
      ),
    );
  }
}

// ---------------------------------------------------------------- language

/// Interface language: the system locale by default, or a manual choice that
/// is kept in Preferences (UI-020).
class _LanguageSection extends StatelessWidget {
  const _LanguageSection();

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    return ValueListenableBuilder<Locale?>(
      valueListenable: services.locale,
      builder: (context, current, _) {
        final options = <(Locale?, String)>[
          (null, l.settingsLanguageSystem),
          (const Locale('en'), l.settingsLanguageEnglish),
          (const Locale('ru'), l.settingsLanguageRussian),
          if (kDebugMode || current == kPseudoLocale)
            (kPseudoLocale, l.settingsLanguagePseudo),
        ];
        return SettingsSection(
          key: const ValueKey('settings-section-language'),
          icon: Icons.language,
          title: l.settingsLanguageTitle,
          subtitle: l.settingsLanguageBasicSubtitle,
          children: [
            for (final (locale, name) in options)
              SettingsTile(
                icon: locale == current
                    ? Icons.radio_button_checked
                    : Icons.radio_button_unchecked,
                title: name,
                selected: locale == current,
                onTap: () => services.setLocale(locale),
                trailing: const SizedBox.shrink(),
              ),
            // PLAY-004: audio and subtitle languages and the sound
            // processing of the profile (UI-041: basic, next to the
            // interface language).
            SettingsTile(
              key: const ValueKey('settings-languages-sound'),
              icon: Icons.translate,
              title: l.settingsLanguagesSound,
              subtitle: l.settingsLanguagesSoundSubtitle,
              onTap: () => _push(context, const TrackSettingsScreen()),
            ),
          ],
        );
      },
    );
  }
}

// ----------------------------------------------------------------- network

/// How the app connects to network sources (UI-041, advanced): the RSA keys
/// of SFTP sources (NET-006, RUSTSEC-2023-0071).
class _NetworkSection extends StatelessWidget {
  const _NetworkSection();

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    return SettingsSection(
      key: const ValueKey('settings-section-network'),
      icon: Icons.lan_outlined,
      title: l.settingsNetworkTitle,
      subtitle: l.settingsNetworkSubtitle,
      children: [
        ValueListenableBuilder<bool>(
          valueListenable: services.sshRsaKeys,
          builder: (context, on, _) => SettingsSwitchTile(
            key: const ValueKey('settings-ssh-rsa-keys'),
            icon: Icons.key_outlined,
            title: l.settingsSshRsaKeys,
            subtitle: l.settingsSshRsaKeysSubtitle,
            value: on,
            onChanged: services.setSshRsaKeys,
          ),
        ),
      ],
    );
  }
}

// ------------------------------------------------------------------ twitch

/// Twitch: the navigation toggle (UI-002) and the chat's "pause animated
/// emotes" (TWI-012).
class _TwitchSection extends StatelessWidget {
  const _TwitchSection();

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    return SettingsSection(
      key: const ValueKey('settings-section-twitch'),
      icon: Icons.live_tv_outlined,
      title: l.settingsTwitchTitle,
      subtitle: l.settingsTwitchSubtitle,
      children: [
        ValueListenableBuilder<bool>(
          valueListenable: services.showTwitch,
          builder: (context, show, _) => SettingsSwitchTile(
            key: const ValueKey('settings-show-twitch'),
            icon: Icons.view_sidebar_outlined,
            title: l.settingsShowTwitch,
            subtitle: l.settingsShowTwitchSubtitle,
            value: show,
            onChanged: services.setShowTwitch,
          ),
        ),
        // TWI-009: "X is live" for favourites, with what the platform can
        // honestly promise.
        ListenableBuilder(
          listenable: services.liveNotifications,
          builder: (context, _) {
            final live = services.liveNotifications;
            final signedIn = services.twitch.account != null || live.running;
            return SettingsSwitchTile(
              key: const ValueKey('settings-twitch-live-notifications'),
              icon: Icons.notifications_active_outlined,
              title: l.settingsTwitchLiveNotifications,
              subtitle: !signedIn
                  ? l.settingsTwitchLiveSignIn
                  : switch (defaultTargetPlatform) {
                      TargetPlatform.android => l.settingsTwitchLiveAndroid,
                      TargetPlatform.iOS => l.settingsTwitchLiveIos,
                      _ => l.settingsTwitchLiveDesktop,
                    },
              value: live.enabled,
              onChanged: live.setEnabled,
            );
          },
        ),
        // TWI-019: folders, quotas, file names and import of downloads and
        // recordings (full builds; never for a profile without Streams).
        if (AppFlavor.twitchArchive &&
            services.visibleArchive != null &&
            !services.profileControl.twitchHidden)
          SettingsTile(
            key: const ValueKey('settings-twitch-archive'),
            icon: Icons.video_library_outlined,
            title: l.twitchArchiveSettings,
            subtitle: l.twitchArchiveStorageSubtitle,
            onTap: () => Navigator.of(context).push(
              MaterialPageRoute<void>(
                builder: (_) => const TwitchArchiveSettingsScreen(),
              ),
            ),
          ),
        ValueListenableBuilder<bool>(
          valueListenable: services.twitchChatPauseAnimations,
          builder: (context, pause, _) => SettingsSwitchTile(
            key: const ValueKey('settings-twitch-chat-pause-animations'),
            icon: Icons.motion_photos_paused_outlined,
            title: l.twitchChatPauseAnimations,
            subtitle: l.settingsTwitchChatPauseAnimationsSubtitle,
            value: pause,
            onChanged: services.setTwitchChatPauseAnimations,
          ),
        ),
      ],
    );
  }
}

// ----------------------------------------------------------------- storage

/// `256 MB`, `1.5 GB`: binary units, as the cache limits are.
String formatCacheSize(AppLocalizations l, Locale locale, int bytes) {
  final tag = locale.toLanguageTag();
  const gib = 1 << 30;
  if (bytes >= gib) {
    return l.detailSizeGb(NumberFormat('0.#', tag).format(bytes / gib));
  }
  return l.detailSizeMb(NumberFormat('0', tag).format(bytes / (1 << 20)));
}

/// The image cache (LIB-013): usage, limit, clearing.
class _StorageSection extends StatelessWidget {
  const _StorageSection();

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    final locale = Localizations.localeOf(context);
    final cache = services.imageCache;
    ImageCacheUsage? usage;
    try {
      usage = cache?.usage();
    } catch (e) {
      debugPrint('Image cache usage not read: $e');
    }
    String size(int b) => formatCacheSize(l, locale, b);
    final limits = {...imageCacheLimits, ?usage?.maxBytes}.toList()..sort();
    return SettingsSection(
      key: const ValueKey('settings-section-storage'),
      icon: Icons.storage_outlined,
      title: l.settingsStorageTitle,
      subtitle: l.settingsStorageSubtitle,
      children: [
        if (cache == null || usage == null)
          SettingsTile(
            icon: Icons.image_outlined,
            title: l.settingsImageCache,
            subtitle: l.settingsImageCacheUnavailable,
            subtitleLines: 2,
          )
        else ...[
          SettingsTile(
            key: const ValueKey('settings-image-cache-usage'),
            icon: Icons.image_outlined,
            title: l.settingsImageCache,
            subtitle: l.settingsImageCacheUsage(
              usage.files,
              size(usage.bytes),
              size(usage.maxBytes),
            ),
          ),
          SettingsChoiceTile<int>(
            key: const ValueKey('settings-image-cache-limit'),
            icon: Icons.data_usage,
            title: l.settingsImageCacheLimit,
            value: usage.maxBytes,
            options: limits,
            label: size,
            onChanged: services.setImageCacheLimit,
          ),
          SettingsTile(
            key: const ValueKey('settings-image-cache-clear'),
            icon: Icons.delete_sweep_outlined,
            title: l.settingsImageCacheClear,
            onTap: () async {
              await services.clearImageCache();
              if (context.mounted) _say(context, l.settingsImageCacheCleared);
            },
            trailing: const SizedBox.shrink(),
          ),
        ],
      ],
    );
  }
}

// ----------------------------------------------------------------- privacy

class _PrivacySection extends StatelessWidget {
  const _PrivacySection();

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    return SettingsSection(
      key: const ValueKey('settings-section-privacy'),
      icon: Icons.shield_outlined,
      title: l.settingsPrivacyTitle,
      children: [
        Text(
          l.settingsPrivacyBody(kAppName),
          style: Theme.of(context).textTheme.bodySmall,
        ),
        const SizedBox(height: 8),
        // CORE-007: OS keychain or the encrypted fallback file.
        if (services.secretStorage.known)
          SettingsTile(
            key: const ValueKey('settings-secret-storage'),
            icon: services.secretStorage.isFallback
                ? Icons.lock_open_outlined
                : Icons.lock_outline,
            title: l.settingsPrivacySecrets,
            subtitle: secretStorageText(l, services.secretStorage),
            subtitleLines: 8,
            trailing: const SizedBox.shrink(),
          ),
        SettingsTile(
          key: const ValueKey('settings-clear-search'),
          icon: Icons.manage_search,
          title: l.settingsPrivacyClearSearch,
          onTap: () async {
            await services.clearSearchHistory();
            if (context.mounted) _say(context, l.settingsPrivacySearchCleared);
          },
          trailing: const SizedBox.shrink(),
        ),
        SettingsLinkTile(
          icon: Icons.policy_outlined,
          title: l.settingsPrivacyPolicy,
          url: kPrivacyPolicyUrl,
        ),
      ],
    );
  }
}

// ------------------------------------------------------------------- about

class _AboutSection extends StatelessWidget {
  const _AboutSection();

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return SettingsSection(
      key: const ValueKey('settings-section-about'),
      icon: Icons.info_outline,
      title: l.settingsUpdatesAboutTitle,
      children: [
        SettingsTile(
          icon: Icons.info_outline,
          title: l.settingsAboutApp(kAppName),
          subtitle: l.settingsAboutSubtitle(kAppVersion),
          onTap: () => _push(context, const AboutScreen()),
        ),
        // UI-004: the first-run setup, again.
        SettingsTile(
          key: const ValueKey('settings-onboarding'),
          icon: Icons.assistant_outlined,
          title: l.settingsOnboarding,
          subtitle: l.settingsOnboardingSubtitle,
          onTap: () => openOnboarding(context),
        ),
      ],
    );
  }
}
