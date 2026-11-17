// SPDX-License-Identifier: MPL-2.0
//
// Settings → «Трейлеры» (UI-036): when the banner of a movie or show page
// plays the trailer by itself, and whether internet trailers are used.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../widgets/settings_tiles.dart';
import 'trailer_settings.dart';

class TrailerSettingsSection extends StatefulWidget {
  const TrailerSettingsSection({super.key});

  @override
  State<TrailerSettingsSection> createState() => _TrailerSettingsSectionState();
}

class _TrailerSettingsSectionState extends State<TrailerSettingsSection> {
  TrailerSettings _settings = const TrailerSettings();

  @override
  void initState() {
    super.initState();
    unawaited(_read());
  }

  Future<void> _read() async {
    final s = await TrailerSettings.read(Services.read(context).preferences);
    if (mounted) setState(() => _settings = s);
  }

  Future<void> _setAutoplay(TrailerAutoplay v) async {
    setState(
      () => _settings = TrailerSettings(
        autoplay: v,
        internet: _settings.internet,
      ),
    );
    await TrailerSettings.writeAutoplay(Services.read(context).preferences, v);
  }

  Future<void> _setInternet(bool on) async {
    setState(
      () => _settings = TrailerSettings(
        autoplay: _settings.autoplay,
        internet: on,
      ),
    );
    await TrailerSettings.writeInternet(Services.read(context).preferences, on);
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return SettingsSection(
      key: const ValueKey('settings-section-trailers'),
      icon: Icons.movie_outlined,
      title: l.settingsTrailersTitle,
      subtitle: l.settingsTrailersSubtitle,
      children: [
        SettingsChoiceTile<TrailerAutoplay>(
          key: const ValueKey('settings-trailers-autoplay'),
          icon: Icons.play_circle_outline,
          title: l.settingsTrailersAutoplay,
          hint: l.settingsTrailersAutoplayHint,
          value: _settings.autoplay,
          options: TrailerAutoplay.values,
          label: (v) => switch (v) {
            TrailerAutoplay.always => l.settingsTrailersAlways,
            TrailerAutoplay.wifiOnly => l.settingsTrailersWifiOnly,
            TrailerAutoplay.never => l.settingsTrailersNever,
          },
          onChanged: (v) => unawaited(_setAutoplay(v)),
        ),
        SettingsSwitchTile(
          key: const ValueKey('settings-trailers-internet'),
          icon: Icons.public,
          title: l.settingsTrailersInternet,
          subtitle: l.settingsTrailersInternetSubtitle,
          value: _settings.internet,
          onChanged: (v) => unawaited(_setInternet(v)),
        ),
      ],
    );
  }
}
