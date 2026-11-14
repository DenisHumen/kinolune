// SPDX-License-Identifier: MPL-2.0
//
// Settings (UI-016): one scrolling page of sections — Library & sources,
// Playback, Shows, Music, Devices, Metadata, Language, Twitch, Storage,
// Privacy, About. Every row is focusable for the TV remote.

import 'package:flutter/foundation.dart';
import 'package:intl/intl.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart'
    show ReplayGainMode, kSkipStepsSeconds;

import '../branding.dart';
import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../services/image_cache_control.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/cards.dart';
import '../widgets/focusable.dart';
import '../widgets/settings_tiles.dart';
import 'about_screen.dart';
import 'add_source_dialog.dart';
import 'devices_screen.dart';
import 'onboarding_screen.dart';
import 'show_detail_screen.dart' show orderLabel;
import 'sources_screen.dart';
import 'subtitle_settings_screen.dart';

class SettingsScreen extends StatelessWidget {
  const SettingsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    const sections = <Widget>[
      _LibrarySection(),
      _PlaybackSection(),
      _ShowsSection(),
      _MusicSection(),
      _DevicesSection(),
      _MetadataSection(),
      _LanguageSection(),
      _TwitchSection(),
      _StorageSection(),
      _PrivacySection(),
      _AboutSection(),
    ];
    // Not lazy: a few panels only, and every row stays reachable by focus
    // traversal (TV remote) and by `ensureVisible`.
    return SingleChildScrollView(
      padding: EdgeInsets.only(bottom: d.pageInsets.bottom + 90),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          PageTitle(l.navSettings),
          for (final s in sections)
            Padding(
              padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
              child: Align(
                alignment: Alignment.topLeft,
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 760),
                  child: s,
                ),
              ),
            ),
        ],
      ),
    );
  }
}

void _push(BuildContext context, Widget screen) =>
    Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => screen));

void _say(BuildContext context, String text) =>
    ScaffoldMessenger.maybeOf(context)
        ?.showSnackBar(SnackBar(content: Text(text)));

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
      status = switch (p.phase) {
        ScanPhase.connecting => l.settingsScanConnecting,
        ScanPhase.metadata => l.settingsScanMetadata(
          p.processed,
          p.total ?? p.filesFound,
        ),
        _ => l.settingsScanFiles(p.processed, p.total ?? p.filesFound),
      };
    } else if (p.error != null) {
      status = l.settingsScanFailed(p.error!);
    } else {
      status = l.settingsScanUpToDate(p.filesFound);
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(l.settingsLibrarySubtitle, style: theme.textTheme.bodySmall),
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

/// Subtitle style (PLAY-007), continuous playback and the takeover pause
/// (SYNC-003).
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
      ],
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
          subtitle: l.settingsLanguageSubtitle,
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
          ],
        );
      },
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
      title: l.settingsAboutTitle,
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
