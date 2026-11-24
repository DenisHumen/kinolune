// SPDX-License-Identifier: MPL-2.0
//
// Settings of MUS-015: the music equalizer (a tile in «Музыка» and its
// page) and the scrobbling section (Last.fm, ListenBrainz).

import 'dart:async';

import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart' show Equalizer, EqualizerPreset;

import '../../l10n/l10n.dart';
import '../../services/app_services.dart';
import '../../services/external_links.dart';
import '../../widgets/settings_tiles.dart';
import 'scrobbling.dart';

/// The name of [p] in the UI language.
String equalizerPresetName(AppLocalizations l, EqualizerPreset p) =>
    switch (p) {
      EqualizerPreset.flat => l.eqPresetFlat,
      EqualizerPreset.bassBoost => l.eqPresetBassBoost,
      EqualizerPreset.trebleBoost => l.eqPresetTrebleBoost,
      EqualizerPreset.vocal => l.eqPresetVocal,
      EqualizerPreset.rock => l.eqPresetRock,
      EqualizerPreset.pop => l.eqPresetPop,
      EqualizerPreset.jazz => l.eqPresetJazz,
      EqualizerPreset.classical => l.eqPresetClassical,
      EqualizerPreset.electronic => l.eqPresetElectronic,
      EqualizerPreset.loudness => l.eqPresetLoudness,
      EqualizerPreset.custom => l.eqPresetCustom,
    };

String _band(int hz) => hz >= 1000 ? '${hz ~/ 1000}k' : '$hz';

/// «Эквалайзер» in the music settings: on/off and the preset; opens
/// [EqualizerScreen].
class EqualizerTile extends StatelessWidget {
  const EqualizerTile({super.key});

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    final eq = services.music.audioProfile.equalizer;
    return SettingsTile(
      key: const ValueKey('settings-music-equalizer'),
      icon: Icons.graphic_eq,
      title: l.eqTitle,
      subtitle: eq.enabled ? equalizerPresetName(l, eq.preset) : l.eqOff,
      trailing: const Icon(Icons.chevron_right),
      onTap: () => Navigator.of(context).push(
        MaterialPageRoute<void>(builder: (_) => const EqualizerScreen()),
      ),
    );
  }
}

/// The 10-band equalizer of the active profile.
class EqualizerScreen extends StatelessWidget {
  const EqualizerScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    final eq = services.music.audioProfile.equalizer;
    void set(Equalizer e) => unawaited(services.setMusicEqualizer(e));
    return Scaffold(
      appBar: AppBar(title: Text(l.eqTitle)),
      body: ListView(
        padding: const EdgeInsets.symmetric(vertical: 8),
        children: [
          SwitchListTile(
            key: const ValueKey('eq-enabled'),
            title: Text(l.eqEnabled),
            subtitle: Text(l.eqBitPerfectNote),
            value: eq.enabled,
            onChanged: (v) => set(eq.copyWith(enabled: v)),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
            child: Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                for (final p in EqualizerPreset.values)
                  if (p != EqualizerPreset.custom || eq.preset == p)
                    ChoiceChip(
                      key: ValueKey('eq-preset-${p.name}'),
                      label: Text(equalizerPresetName(l, p)),
                      selected: eq.preset == p,
                      onSelected: eq.enabled
                          ? (_) => set(eq.withPreset(p))
                          : null,
                    ),
              ],
            ),
          ),
          _Slider(
            key: const ValueKey('eq-preamp'),
            label: l.eqPreamp,
            value: eq.preampDb,
            enabled: eq.enabled,
            onChanged: (v) => set(eq.copyWith(preampDb: v)),
          ),
          const Divider(),
          for (var i = 0; i < Equalizer.bands.length; i++)
            _Slider(
              key: ValueKey('eq-band-$i'),
              label: l.eqBandHz(_band(Equalizer.bands[i])),
              value: i < eq.gainsDb.length ? eq.gainsDb[i] : 0,
              enabled: eq.enabled,
              onChanged: (v) => set(eq.withBand(i, v)),
            ),
        ],
      ),
    );
  }
}

class _Slider extends StatelessWidget {
  const _Slider({
    super.key,
    required this.label,
    required this.value,
    required this.enabled,
    required this.onChanged,
  });

  final String label;
  final double value;
  final bool enabled;
  final ValueChanged<double> onChanged;

  @override
  Widget build(BuildContext context) {
    final v = value.clamp(Equalizer.minGainDb, Equalizer.maxGainDb).toDouble();
    final db = '${v > 0 ? '+' : ''}${v.toStringAsFixed(1)} dB';
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16),
      child: Row(
        children: [
          SizedBox(width: 88, child: Text(label)),
          Expanded(
            child: Slider(
              value: v,
              min: Equalizer.minGainDb,
              max: Equalizer.maxGainDb,
              divisions: 48,
              label: db,
              semanticFormatterCallback: (_) => '$label $db',
              onChanged: enabled ? onChanged : null,
            ),
          ),
          SizedBox(
            width: 72,
            child: Text(db, textAlign: TextAlign.end),
          ),
        ],
      ),
    );
  }
}

/// «Скробблинг»: Last.fm and ListenBrainz of the active profile.
class ScrobbleSettingsSection extends StatelessWidget {
  const ScrobbleSettingsSection({super.key});

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    final s = services.scrobbling;
    return ListenableBuilder(
      listenable: s,
      builder: (context, _) {
        final a = s.accounts;
        final st = s.status;
        String pending(int n) => n > 0 ? ' · ${l.scrobblePending(n)}' : '';
        return SettingsSection(
          key: const ValueKey('settings-section-scrobble'),
          icon: Icons.podcasts_outlined,
          title: l.scrobbleTitle,
          subtitle: s.available ? l.scrobbleSubtitle : l.scrobbleUnavailable,
          children: [
            if (s.available) ...[
              SettingsTile(
                key: const ValueKey('scrobble-lastfm'),
                icon: Icons.radio_outlined,
                title: 'Last.fm',
                subtitle: st.lastfmNeedsAuth && a.lastfmConnected
                    ? l.scrobbleReconnect
                    : a.lastfmConnected
                    ? l.scrobbleConnectedAs(a.lastfmName ?? '') +
                          pending(st.lastfmPending)
                    : l.scrobbleNotConnected,
                trailing: TextButton(
                  onPressed: () => a.lastfmConnected && !st.lastfmNeedsAuth
                      ? unawaited(s.disconnectLastfm())
                      : unawaited(_connectLastfm(context, s)),
                  child: Text(
                    a.lastfmConnected && !st.lastfmNeedsAuth
                        ? l.scrobbleDisconnect
                        : l.scrobbleConnect,
                  ),
                ),
              ),
              SettingsTile(
                key: const ValueKey('scrobble-lastfm-key'),
                icon: Icons.key_outlined,
                title: s.needsOwnLastfmKey
                    ? l.scrobbleLastfmKey
                    : l.scrobbleLastfmKeyOptional,
                subtitle: a.lastfmApiKey == null
                    ? l.scrobbleLastfmKeyHint
                    : l.scrobbleLastfmKeySet,
                subtitleLines: 2,
                onTap: () => unawaited(_editLastfmKey(context, s)),
              ),
              SettingsTile(
                key: const ValueKey('scrobble-listenbrainz'),
                icon: Icons.headphones_outlined,
                title: 'ListenBrainz',
                subtitle: st.listenbrainzNeedsAuth && a.listenbrainzConnected
                    ? l.scrobbleReconnect
                    : a.listenbrainzConnected
                    ? l.scrobbleConnectedAs(a.listenbrainzUser ?? '') +
                          pending(st.listenbrainzPending)
                    : l.scrobbleListenbrainzHint,
                subtitleLines: 2,
                trailing: TextButton(
                  onPressed: () =>
                      a.listenbrainzConnected && !st.listenbrainzNeedsAuth
                      ? unawaited(s.disconnectListenbrainz())
                      : unawaited(_connectListenbrainz(context, s)),
                  child: Text(
                    a.listenbrainzConnected && !st.listenbrainzNeedsAuth
                        ? l.scrobbleDisconnect
                        : l.scrobbleConnect,
                  ),
                ),
              ),
            ],
          ],
        );
      },
    );
  }
}

String _error(AppLocalizations l, Object e) => switch (e) {
  ScrobbleException(code: 'no_key') => l.scrobbleErrorNoKey,
  ScrobbleException(code: 'refused') => l.scrobbleErrorRefused,
  _ => l.scrobbleErrorUnavailable,
};

void _snack(BuildContext context, String text) => ScaffoldMessenger.maybeOf(
  context,
)?.showSnackBar(SnackBar(content: Text(text)));

/// Desktop sign-in: the browser page, then «Готово» here.
Future<void> _connectLastfm(
  BuildContext context,
  MusicScrobbleController s,
) async {
  final l = context.l10n;
  if (s.needsOwnLastfmKey && s.accounts.lastfmApiKey == null) {
    _snack(context, l.scrobbleErrorNoKey);
    return _editLastfmKey(context, s);
  }
  LastFmAuthStart start;
  try {
    start = await s.beginLastfm();
  } catch (e) {
    if (context.mounted) _snack(context, _error(l, e));
    return;
  }
  final opened = await openExternalLink(Uri.parse(start.url));
  if (!context.mounted) return;
  final done = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      title: const Text('Last.fm'),
      content: SelectableText(
        opened ? l.scrobbleLastfmAllow : l.scrobbleLastfmOpenManually(start.url),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: Text(l.commonCancel),
        ),
        FilledButton(
          key: const ValueKey('scrobble-lastfm-done'),
          onPressed: () => Navigator.pop(context, true),
          child: Text(l.scrobbleLastfmDone),
        ),
      ],
    ),
  );
  if (done != true) return;
  try {
    await s.finishLastfm(start.token);
    if (context.mounted) {
      _snack(context, l.scrobbleConnectedAs(s.accounts.lastfmName ?? ''));
    }
  } catch (e) {
    if (context.mounted) _snack(context, _error(l, e));
  }
}

Future<void> _editLastfmKey(
  BuildContext context,
  MusicScrobbleController s,
) async {
  final l = context.l10n;
  final key = TextEditingController(text: s.accounts.lastfmApiKey);
  final secret = TextEditingController(text: s.accounts.lastfmSecret);
  final ok = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      title: Text(l.scrobbleLastfmKey),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(l.scrobbleLastfmKeyHint),
          TextField(
            key: const ValueKey('scrobble-lastfm-api-key'),
            controller: key,
            decoration: InputDecoration(labelText: l.scrobbleLastfmApiKey),
          ),
          TextField(
            key: const ValueKey('scrobble-lastfm-secret'),
            controller: secret,
            obscureText: true,
            decoration: InputDecoration(labelText: l.scrobbleLastfmSecret),
          ),
        ],
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: Text(l.commonCancel),
        ),
        FilledButton(
          onPressed: () => Navigator.pop(context, true),
          child: Text(l.commonSave),
        ),
      ],
    ),
  );
  final k = key.text;
  final sec = secret.text;
  key.dispose();
  secret.dispose();
  if (ok == true) await s.setLastfmKey(k, sec);
}

Future<void> _connectListenbrainz(
  BuildContext context,
  MusicScrobbleController s,
) async {
  final l = context.l10n;
  final token = TextEditingController();
  final ok = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      title: const Text('ListenBrainz'),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(l.scrobbleListenbrainzHint),
          TextField(
            key: const ValueKey('scrobble-listenbrainz-token'),
            controller: token,
            obscureText: true,
            decoration: InputDecoration(labelText: l.scrobbleListenbrainzToken),
          ),
        ],
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: Text(l.commonCancel),
        ),
        FilledButton(
          onPressed: () => Navigator.pop(context, true),
          child: Text(l.scrobbleConnect),
        ),
      ],
    ),
  );
  final t = token.text;
  token.dispose();
  if (ok != true || t.trim().isEmpty) return;
  try {
    await s.connectListenbrainz(t);
    if (context.mounted) {
      _snack(context, l.scrobbleConnectedAs(s.accounts.listenbrainzUser ?? ''));
    }
  } catch (e) {
    if (context.mounted) _snack(context, _error(l, e));
  }
}
