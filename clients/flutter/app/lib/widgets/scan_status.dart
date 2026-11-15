// SPDX-License-Identifier: MPL-2.0
//
// The progress line of a running library scan (LIB-023): which step runs,
// with counts ("Распознано 120 из 200"), and which provider makes the scan
// wait ("AniList просит подождать 40 с"), so a long step never looks like a
// frozen bar. Used by the sidebar, Settings, Sources and onboarding.

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';

/// Human name of a metadata provider.
String providerName(String provider) => switch (provider) {
  'tmdb' => 'TMDB',
  'anilist' => 'AniList',
  'shikimori' => 'Shikimori',
  _ => provider,
};

/// The step of [p] with its counts.
String scanStepText(AppLocalizations l, ScanProgress p) {
  final total = p.total ?? 0;
  final step =
      p.step ??
      switch (p.phase) {
        ScanPhase.connecting => ScanStep.connecting,
        ScanPhase.scanning => ScanStep.listing,
        ScanPhase.metadata => ScanStep.titles,
        ScanPhase.done => ScanStep.done,
      };
  return switch (step) {
    ScanStep.connecting => l.settingsScanConnecting,
    ScanStep.listing => l.settingsScanStepListing(p.filesFound),
    ScanStep.musicTags => l.settingsScanStepMusicTags(p.processed, total),
    ScanStep.folders => l.settingsScanStepFolders,
    ScanStep.probing => l.settingsScanStepProbing(p.processed, total),
    ScanStep.fingerprinting => l.settingsScanStepFingerprinting(
      p.processed,
      total,
    ),
    ScanStep.recognizing => l.settingsScanStepRecognizing(p.processed, total),
    ScanStep.finishing => l.settingsScanStepFinishing,
    ScanStep.titles => l.settingsScanStepTitles(p.processed, total),
    ScanStep.anime => l.settingsScanStepAnime(p.processed, total),
    ScanStep.done => l.settingsScanUpToDate(p.filesFound),
  };
}

/// The longest provider wait of [p], as a sentence; `null` when nothing
/// waits.
String? scanWaitText(AppLocalizations l, ScanProgress p) {
  if (p.waiting.isEmpty) return null;
  final w = p.waiting.reduce((a, b) => a.seconds >= b.seconds ? a : b);
  final name = providerName(w.provider);
  return w.providerAsked
      ? l.settingsScanWaitProviderAsked(name, w.seconds)
      : l.settingsScanWaitRateLimit(name, w.seconds);
}

/// Step and wait in one line.
String scanStatusText(AppLocalizations l, ScanProgress p) {
  final wait = scanWaitText(l, p);
  final step = scanStepText(l, p);
  return wait == null ? step : '$step · $wait';
}

/// Share of the current step that is done, or `null` (an animated bar)
/// while the step has no count yet.
double? scanFraction(ScanProgress p) {
  final total = p.total ?? 0;
  if (total <= 0) return null;
  return (p.processed / total).clamp(0.0, 1.0);
}

/// The bar and the line of a running scan.
class ScanStatusView extends StatelessWidget {
  const ScanStatusView({super.key, required this.progress, this.dense = false});

  final ScanProgress progress;

  /// Small text under a thin bar (sidebar).
  final bool dense;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final wait = scanWaitText(l, progress);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        ClipRRect(
          borderRadius: BorderRadius.circular(4),
          child: LinearProgressIndicator(
            value: scanFraction(progress),
            minHeight: dense ? 4 : 6,
          ),
        ),
        const SizedBox(height: 6),
        Text(
          scanStepText(l, progress),
          key: const ValueKey('scan-step'),
          style: dense ? theme.textTheme.labelSmall : theme.textTheme.bodySmall,
          maxLines: 2,
          overflow: TextOverflow.ellipsis,
        ),
        if (wait != null)
          Padding(
            padding: const EdgeInsets.only(top: 2),
            child: Text(
              wait,
              key: const ValueKey('scan-wait'),
              style:
                  (dense
                          ? theme.textTheme.labelSmall
                          : theme.textTheme.bodySmall)
                      ?.copyWith(color: theme.colorScheme.tertiary),
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
            ),
          ),
      ],
    );
  }
}
