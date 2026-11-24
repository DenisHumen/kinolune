// SPDX-License-Identifier: MPL-2.0
//
// META-030: the anime database in Settings → Metadata — the daily update
// switch (on by default), what the device uses and a manual check.
// META-031: the switch for recognising anime by its frames (off by
// default: frames go to trace.moe).

import 'package:intl/intl.dart';
import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../widgets/settings_tiles.dart';

/// The status line of [status]: origin, release and date, titles, the last
/// check.
String animeDbStatusText(
  AppLocalizations l,
  AnimeDbStatus? status,
  String locale,
) {
  if (status == null) return l.settingsAnimeDbNone;
  final release = status.origin == AnimeDbOrigin.downloaded
      ? l.settingsAnimeDbDownloaded(status.tag, status.lastUpdate)
      : l.settingsAnimeDbBundled(status.tag, status.lastUpdate);
  final entries = status.entries;
  final checked = status.checkedAt;
  return [
    release,
    if (entries != null) l.settingsAnimeDbEntries(entries),
    checked == null
        ? l.settingsAnimeDbNeverChecked
        : l.settingsAnimeDbChecked(
            DateFormat.yMMMd(locale).add_Hm().format(checked),
          ),
  ].join(' · ');
}

/// Switch, status and "Check now" of the anime database.
class AnimeDbSettings extends StatelessWidget {
  const AnimeDbSettings({super.key});

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final locale = Localizations.localeOf(context).toLanguageTag();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        ValueListenableBuilder<bool>(
          valueListenable: services.animeDataUpdates,
          builder: (context, on, _) => SettingsSwitchTile(
            key: const ValueKey('settings-anime-db-updates'),
            icon: Icons.cloud_sync_outlined,
            title: l.settingsAnimeDbUpdates,
            subtitle: l.settingsAnimeDbUpdatesSubtitle,
            value: on,
            onChanged: services.setAnimeDataUpdates,
          ),
        ),
        ValueListenableBuilder<AnimeDbStatus?>(
          valueListenable: services.animeDbStatus,
          builder: (context, status, _) => SettingsTile(
            key: const ValueKey('settings-anime-db-status'),
            icon: Icons.dataset_outlined,
            title: l.settingsAnimeDbTitle,
            subtitle: animeDbStatusText(l, status, locale),
            subtitleLines: 4,
            trailing: const SizedBox.shrink(),
          ),
        ),
        // Below the status, not beside it: the line stays readable on
        // phones.
        Padding(
          padding: const EdgeInsetsDirectional.only(start: 50),
          child: Align(
            alignment: AlignmentDirectional.centerStart,
            child: ValueListenableBuilder<bool>(
              valueListenable: services.animeDbChecking,
              builder: (context, checking, _) => checking
                  ? const Padding(
                      padding: EdgeInsets.all(10),
                      child: SizedBox.square(
                        dimension: 20,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      ),
                    )
                  : TextButton.icon(
                      key: const ValueKey('settings-anime-db-check'),
                      onPressed: () => _check(context),
                      icon: const Icon(Icons.refresh),
                      label: Text(l.settingsAnimeDbCheckNow),
                    ),
            ),
          ),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(12, 4, 12, 0),
          child: Text(l.settingsAnimeDbHint, style: theme.textTheme.labelSmall),
        ),
        // META-031: frames of the user's files go to an outside service,
        // so it is the user's choice, off until turned on.
        ValueListenableBuilder<bool>(
          valueListenable: services.animeFrameSearch,
          builder: (context, on, _) => SettingsSwitchTile(
            key: const ValueKey('settings-anime-frame-search'),
            icon: Icons.image_search_outlined,
            title: l.settingsAnimeFrameSearch,
            subtitle: l.settingsAnimeFrameSearchSubtitle,
            value: on,
            onChanged: services.setAnimeFrameSearch,
          ),
        ),
      ],
    );
  }

  Future<void> _check(BuildContext context) async {
    final services = Services.read(context);
    final l = context.l10n;
    final messenger = ScaffoldMessenger.maybeOf(context);
    String text;
    try {
      final r = await services.checkAnimeDbNow();
      text = r == AnimeDbRefresh.updated
          ? l.settingsAnimeDbUpdated
          : l.settingsAnimeDbUpToDate;
    } catch (_) {
      // BUG-010: with no database on the device there is no "current" one
      // to keep using.
      text = services.animeDbStatus.value == null
          ? l.settingsAnimeDbFailedNoDb
          : l.settingsAnimeDbFailed;
    }
    messenger?.showSnackBar(SnackBar(content: Text(text)));
  }
}
