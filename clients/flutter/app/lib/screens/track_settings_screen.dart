// SPDX-License-Identifier: MPL-2.0

import 'package:material_ui/material_ui.dart';

import '../integration/player.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/settings_tiles.dart';

/// Languages and sound (PLAY-004, design 01 §1.4 "Language" and "Audio"),
/// per profile: the audio and subtitle languages the player picks (Auto,
/// Original, chosen languages), when subtitles come on, SDH, and the sound
/// processing — stereo downmix, its normalization, LFE in stereo and
/// Volume Boost. The OSD audio panel has quick toggles for the sound.
class TrackSettingsScreen extends StatelessWidget {
  const TrackSettingsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final d = KDensity.of(context);
    final l = context.l10n;
    return Scaffold(
      body: SafeArea(
        child: ListenableBuilder(
          listenable: Listenable.merge([
            services.trackPolicy,
            services.audioProcessing,
          ]),
          builder: (context, _) {
            final p = services.trackPolicy.value;
            final a = services.audioProcessing.value;
            return ListView(
              padding: EdgeInsets.fromLTRB(
                d.pageInsets.left,
                d.pageInsets.top,
                d.pageInsets.right,
                d.pageInsets.bottom + 90,
              ),
              children: [
                Row(
                  children: [
                    if (!d.isTv)
                      IconButton(
                        tooltip: l.commonBack,
                        onPressed: () => Navigator.of(context).maybePop(),
                        icon: const Icon(Icons.arrow_back),
                      ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        l.settingsLanguagesSound,
                        style: Theme.of(context).textTheme.headlineSmall,
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 16),
                ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 760),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      SettingsSection(
                        key: const ValueKey('tracks-section-languages'),
                        icon: Icons.translate,
                        title: l.tracksSectionLanguages,
                        children: _languages(context, services, p, l),
                      ),
                      SettingsSection(
                        key: const ValueKey('tracks-section-sound'),
                        icon: Icons.volume_up_outlined,
                        title: l.tracksSectionSound,
                        children: _sound(services, a, l),
                      ),
                    ],
                  ),
                ),
              ],
            );
          },
        ),
      ),
    );
  }

  List<Widget> _languages(
    BuildContext context,
    AppServices services,
    TrackPolicy p,
    AppLocalizations l,
  ) {
    String mode(LanguageMode m, {required bool audio}) => switch (m) {
      LanguageMode.auto => audio ? l.tracksModeAutoAudio : l.tracksModeAuto,
      LanguageMode.original => l.tracksModeOriginal,
      LanguageMode.custom => l.tracksModeCustom,
    };
    return [
      SettingsChoiceTile<LanguageMode>(
        key: const ValueKey('tracks-audio-mode'),
        icon: Icons.record_voice_over_outlined,
        title: l.tracksAudioLanguage,
        hint: l.tracksAudioHint,
        value: p.audio.mode,
        options: LanguageMode.values,
        label: (m) => mode(m, audio: true),
        onChanged: (m) => services.setTrackPolicy(
          p.copyWith(audio: p.audio.copyWith(mode: m)),
        ),
      ),
      if (p.audio.mode == LanguageMode.custom)
        SettingsTile(
          key: const ValueKey('tracks-audio-languages'),
          icon: Icons.format_list_numbered,
          title: l.tracksLanguagesTitle,
          subtitle: _names(p.audio.languages, l),
          onTap: () async {
            final picked = await pickLanguages(context, p.audio.languages);
            if (picked == null) return;
            final now = services.trackPolicy.value;
            await services.setTrackPolicy(
              now.copyWith(audio: now.audio.copyWith(languages: picked)),
            );
          },
        ),
      SettingsChoiceTile<LanguageMode>(
        key: const ValueKey('tracks-subtitle-mode-lang'),
        icon: Icons.subtitles_outlined,
        title: l.tracksSubtitleLanguage,
        hint: l.tracksSubtitleHint,
        value: p.subtitles.mode,
        options: LanguageMode.values,
        label: (m) => mode(m, audio: false),
        onChanged: (m) => services.setTrackPolicy(
          p.copyWith(subtitles: p.subtitles.copyWith(mode: m)),
        ),
      ),
      if (p.subtitles.mode == LanguageMode.custom)
        SettingsTile(
          key: const ValueKey('tracks-subtitle-languages'),
          icon: Icons.format_list_numbered,
          title: l.tracksLanguagesTitle,
          subtitle: _names(p.subtitles.languages, l),
          onTap: () async {
            final picked = await pickLanguages(context, p.subtitles.languages);
            if (picked == null) return;
            final now = services.trackPolicy.value;
            await services.setTrackPolicy(
              now.copyWith(
                subtitles: now.subtitles.copyWith(languages: picked),
              ),
            );
          },
        ),
      SettingsChoiceTile<SubtitleMode>(
        key: const ValueKey('tracks-subtitle-when'),
        icon: Icons.closed_caption_outlined,
        title: l.tracksSubtitleMode,
        hint: l.tracksSubtitleModeHint,
        value: p.subtitleMode,
        options: SubtitleMode.values,
        label: (m) => switch (m) {
          SubtitleMode.auto => l.tracksSubtitleModeAuto,
          SubtitleMode.always => l.tracksSubtitleModeAlways,
          SubtitleMode.forcedOnly => l.tracksSubtitleModeForced,
          SubtitleMode.off => l.tracksSubtitleModeOff,
        },
        onChanged: (m) => services.setTrackPolicy(p.copyWith(subtitleMode: m)),
      ),
      SettingsChoiceTile<SdhPreference>(
        key: const ValueKey('tracks-sdh'),
        icon: Icons.hearing_outlined,
        title: l.tracksSdh,
        value: p.sdh,
        options: SdhPreference.values,
        label: (m) => switch (m) {
          SdhPreference.auto => l.tracksSdhAuto,
          SdhPreference.prefer => l.tracksSdhPrefer,
          SdhPreference.avoid => l.tracksSdhAvoid,
        },
        onChanged: (m) => services.setTrackPolicy(p.copyWith(sdh: m)),
      ),
    ];
  }

  List<Widget> _sound(
    AppServices services,
    AudioProcessing a,
    AppLocalizations l,
  ) => [
    SettingsChoiceTile<DownmixMode>(
      key: const ValueKey('tracks-downmix'),
      icon: Icons.headphones_outlined,
      title: l.tracksDownmix,
      hint: l.tracksDownmixHint,
      value: a.downmix,
      options: DownmixMode.values,
      label: (m) => switch (m) {
        DownmixMode.auto => l.tracksDownmixAuto,
        DownmixMode.stereo => l.tracksDownmixStereo,
      },
      onChanged: (m) => services.setAudioProcessing(a.copyWith(downmix: m)),
    ),
    SettingsSwitchTile(
      key: const ValueKey('tracks-normalize'),
      icon: Icons.compress,
      title: l.tracksNormalize,
      subtitle: l.tracksNormalizeSubtitle,
      value: a.normalizeDownmix,
      onChanged: (v) =>
          services.setAudioProcessing(a.copyWith(normalizeDownmix: v)),
    ),
    SettingsSwitchTile(
      key: const ValueKey('tracks-lfe'),
      icon: Icons.speaker_outlined,
      title: l.tracksLfe,
      subtitle: l.tracksLfeSubtitle,
      value: a.lfeToStereo,
      onChanged: (v) => services.setAudioProcessing(a.copyWith(lfeToStereo: v)),
    ),
    SettingsChoiceTile<int>(
      key: const ValueKey('tracks-boost'),
      icon: Icons.volume_up_outlined,
      title: l.tracksBoost,
      hint: l.tracksBoostHint,
      value: a.clampedBoost,
      options: AudioProcessing.boostSteps,
      label: (v) => v == 100 ? l.tracksBoostOff : l.tracksBoostValue(v),
      onChanged: (v) => services.setAudioProcessing(a.copyWith(boost: v)),
    ),
  ];

  static String _names(List<String> codes, AppLocalizations l) => codes.isEmpty
      ? l.tracksLanguagesNone
      : codes.map(languageName).join(', ');
}

/// Languages offered in the list (ISO 639-2/B) with their own names: a
/// name in its own language needs no translation.
const List<(String, String)> kTrackLanguages = [
  ('rus', 'Русский'),
  ('eng', 'English'),
  ('ukr', 'Українська'),
  ('bel', 'Беларуская'),
  ('kaz', 'Қазақша'),
  ('ger', 'Deutsch'),
  ('fre', 'Français'),
  ('spa', 'Español'),
  ('ita', 'Italiano'),
  ('por', 'Português'),
  ('pol', 'Polski'),
  ('cze', 'Čeština'),
  ('dut', 'Nederlands'),
  ('swe', 'Svenska'),
  ('nor', 'Norsk'),
  ('dan', 'Dansk'),
  ('fin', 'Suomi'),
  ('tur', 'Türkçe'),
  ('jpn', '日本語'),
  ('kor', '한국어'),
  ('chi', '中文'),
  ('ara', 'العربية'),
  ('heb', 'עברית'),
  ('hin', 'हिन्दी'),
];

/// The own name of [code] (any form [normalizeLanguage] reads), else the
/// code itself.
String languageName(String code) {
  final c = normalizeLanguage(code) ?? code;
  for (final (k, name) in kTrackLanguages) {
    if (k == c) return name;
  }
  return code;
}

/// Asks for an ordered list of languages: tapping a language adds it at
/// the end or removes it. Null if cancelled.
Future<List<String>?> pickLanguages(
  BuildContext context,
  List<String> current,
) => showDialog<List<String>>(
  context: context,
  builder: (context) => _LanguagesDialog(initial: current),
);

class _LanguagesDialog extends StatefulWidget {
  const _LanguagesDialog({required this.initial});

  final List<String> initial;

  @override
  State<_LanguagesDialog> createState() => _LanguagesDialogState();
}

class _LanguagesDialogState extends State<_LanguagesDialog> {
  late final List<String> _picked = [
    for (final c in widget.initial) ?normalizeLanguage(c),
  ];

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return AlertDialog(
      key: const ValueKey('tracks-languages-dialog'),
      title: Text(
        l.tracksLanguagesTitle,
        style: Theme.of(context).textTheme.titleLarge,
      ),
      content: SizedBox(
        width: 360,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(l.tracksLanguagesHint),
            const SizedBox(height: 8),
            Flexible(
              child: ListView(
                shrinkWrap: true,
                children: [
                  // The chosen ones first, in their order.
                  for (final (code, name) in [
                    for (final c in _picked) (c, languageName(c)),
                    for (final e in kTrackLanguages)
                      if (!_picked.contains(e.$1)) e,
                  ])
                    ListTile(
                      key: ValueKey('tracks-language-$code'),
                      dense: true,
                      leading: SizedBox(
                        width: 28,
                        child: _picked.contains(code)
                            ? CircleAvatar(
                                radius: 12,
                                child: Text('${_picked.indexOf(code) + 1}'),
                              )
                            : const Icon(Icons.add, size: 20),
                      ),
                      title: Text(name),
                      selected: _picked.contains(code),
                      onTap: () => setState(() {
                        _picked.contains(code)
                            ? _picked.remove(code)
                            : _picked.add(code);
                      }),
                    ),
                ],
              ),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.commonCancel),
        ),
        FilledButton(
          key: const ValueKey('tracks-languages-done'),
          onPressed: () => Navigator.of(context).pop(List.of(_picked)),
          child: Text(l.tracksLanguagesDone),
        ),
      ],
    );
  }
}
