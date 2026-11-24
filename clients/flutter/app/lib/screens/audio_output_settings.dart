// SPDX-License-Identifier: MPL-2.0
//
// «Звук: авто / всегда декодировать» (PLAY-AND-004, ADR-NNNN): Dolby and
// DTS go untouched to a receiver that takes them over HDMI, or are always
// decoded to PCM. Android only: there the player asks the HDMI output what
// it takes (desktop passthrough is not wired yet).

import 'package:flutter/foundation.dart';
import 'package:material_ui/material_ui.dart';

import '../integration/player.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../widgets/settings_tiles.dart';

/// Whether the player can pass sound through on this platform.
bool get audioPassthroughAvailable =>
    !kIsWeb && defaultTargetPlatform == TargetPlatform.android;

/// The «Звук» tile of the playback section, on Android.
List<Widget> audioOutputTiles(BuildContext context) {
  if (!audioPassthroughAvailable) return const [];
  final services = Services.of(context);
  final l = context.l10n;
  return [
    ValueListenableBuilder<AudioOutputMode>(
      valueListenable: services.audioOutputMode,
      builder: (context, mode, _) => SettingsChoiceTile<AudioOutputMode>(
        key: const ValueKey('settings-audio-output'),
        icon: Icons.surround_sound_outlined,
        title: l.settingsAudioOutput,
        hint: l.settingsAudioOutputHint,
        value: mode,
        options: AudioOutputMode.values,
        label: (m) => switch (m) {
          AudioOutputMode.auto => l.settingsAudioOutputAuto,
          AudioOutputMode.decode => l.settingsAudioOutputDecode,
        },
        onChanged: services.setAudioOutputMode,
      ),
    ),
  ];
}
