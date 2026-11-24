// SPDX-License-Identifier: MPL-2.0
//
// TV display settings of the player (PLAY-AND-005, ADR-0147): «Частота
// кадров» (auto frame rate), «Разрешение как у видео» and «Совпадение
// динамического диапазона». Shown on TV only: phones and desktops leave
// their display alone.

import 'package:material_ui/material_ui.dart';

import '../integration/player.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../widgets/settings_tiles.dart';

/// Whether the video plays through the native Android player
/// (pa_player_android, Media3 on a SurfaceView), the only one that passes
/// HDR and Dolby Vision to the TV. The 0.x player (media_kit) draws into a
/// Flutter texture, always SDR (ADR-0008), so «Совпадение динамического
/// диапазона» stays hidden until the app plays through Media3 (UI-013).
const kNativeHdrPlayback = false;

/// The display tiles of the playback section, on TV.
List<Widget> displayMatchTiles(BuildContext context) {
  final services = Services.of(context);
  final l = context.l10n;
  return [
    ValueListenableBuilder<DisplayMatchSettings>(
      valueListenable: services.displayMatch,
      builder: (context, s, _) => SettingsChoiceTile<FrameRateMatch>(
        key: const ValueKey('settings-frame-rate'),
        icon: Icons.slow_motion_video_outlined,
        title: l.settingsFrameRateMatch,
        hint: l.settingsFrameRateMatchHint,
        value: s.frameRate,
        options: FrameRateMatch.values,
        label: (m) => switch (m) {
          FrameRateMatch.auto => l.settingsFrameRateAuto,
          FrameRateMatch.off => l.settingsFrameRateOff,
        },
        onChanged: (m) => services.setDisplayMatch(s.copyWith(frameRate: m)),
      ),
    ),
    ValueListenableBuilder<DisplayMatchSettings>(
      valueListenable: services.displayMatch,
      builder: (context, s, _) => s.frameRate == FrameRateMatch.off
          ? const SizedBox.shrink()
          : SettingsSwitchTile(
              key: const ValueKey('settings-resolution-match'),
              icon: Icons.aspect_ratio_outlined,
              title: l.settingsResolutionMatch,
              subtitle: l.settingsResolutionMatchSubtitle,
              value: s.resolution,
              onChanged: (v) =>
                  services.setDisplayMatch(s.copyWith(resolution: v)),
            ),
    ),
    if (kNativeHdrPlayback)
      ValueListenableBuilder<DisplayMatchSettings>(
        valueListenable: services.displayMatch,
        builder: (context, s, _) => SettingsSwitchTile(
          key: const ValueKey('settings-dynamic-range-match'),
          icon: Icons.hdr_on_outlined,
          title: l.settingsDynamicRangeMatch,
          subtitle: l.settingsDynamicRangeMatchSubtitle,
          value: s.dynamicRange,
          onChanged: (v) =>
              services.setDisplayMatch(s.copyWith(dynamicRange: v)),
        ),
      ),
  ];
}
