// SPDX-License-Identifier: MPL-2.0
//
// «Состояние подключений» (UI-053, ADR-0157 §8), advanced settings: every
// source and server the core's monitor watches (NET-024) with its status,
// round trip and last answer; the sync channels with the changes waiting
// on this device and the last successful sync; the history of status
// changes. One column; on a TV every row takes the remote's focus.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../branding.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../sources/availability.dart';
import '../sources/source_text.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/availability_status.dart';
import '../widgets/focusable.dart';
import '../widgets/settings_tiles.dart';

/// Opens «Состояние подключений» over the current page.
Future<void> openConnectionStatus(BuildContext context) => Navigator.of(
  context,
).push(MaterialPageRoute<void>(builder: (_) => const ConnectionStatusScreen()));

/// How many status changes the history shows, newest first.
const connectionHistoryShown = 50;

class ConnectionStatusScreen extends StatefulWidget {
  const ConnectionStatusScreen({super.key});

  @override
  State<ConnectionStatusScreen> createState() => _ConnectionStatusScreenState();
}

class _ConnectionStatusScreenState extends State<ConnectionStatusScreen> {
  SyncChannelsModel? _channels;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final services = Services.of(context);
    final m = _channels;
    if (m == null ||
        m.availability != services.availability ||
        m.sync != services.sync) {
      m?.dispose();
      _channels = SyncChannelsModel(services.availability, services.sync);
    }
  }

  @override
  void dispose() {
    _channels?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final services = Services.of(context);
    final d = KDensity.of(context);
    final availability = services.availability;
    return Scaffold(
      key: const ValueKey('connection-status-page'),
      backgroundColor: KColors.bgBase,
      appBar: AppBar(
        backgroundColor: KColors.bgBase,
        title: Text(
          l.availabilityScreenTitle,
          style: Theme.of(context).textTheme.titleLarge,
        ),
      ),
      body: ListenableBuilder(
        listenable: Listenable.merge([availability, _channels]),
        builder: (context, _) {
          final targets = availability.targets;
          final channels = _channels!;
          final history = availability.history().reversed.take(
            connectionHistoryShown,
          );
          // A handful of rows and at most [connectionHistoryShown] lines:
          // one column, built at once.
          return FocusTraversalGroup(
            child: SingleChildScrollView(
              key: const ValueKey('connection-status-list'),
              padding: EdgeInsets.fromLTRB(
                d.pageInsets.left,
                4,
                d.pageInsets.right,
                d.pageInsets.bottom + 24,
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Padding(
                    padding: const EdgeInsets.only(bottom: 16),
                    child: Text(
                      l.availabilityScreenIntro,
                      style: Theme.of(context).textTheme.bodyMedium,
                    ),
                  ),
                  const SyncPausedPlate(openOnActivate: false),
                  SettingsSection(
                    key: const ValueKey('connection-status-targets'),
                    icon: Icons.lan_outlined,
                    title: l.availabilitySectionTargets,
                    children: [
                      if (targets.isEmpty)
                        Text(
                          l.availabilityNothing,
                          style: Theme.of(context).textTheme.bodySmall,
                        ),
                      for (final t in targets) _TargetTile(target: t),
                    ],
                  ),
                  SettingsSection(
                    key: const ValueKey('connection-status-sync'),
                    icon: Icons.sync,
                    title: l.availabilitySectionSync,
                    children: [
                      _Line(
                        key: const ValueKey('connection-status-outbox'),
                        text: l.availabilityOutbox(channels.outbox),
                      ),
                      if (channels.folder.enabled)
                        _Line(
                          key: const ValueKey(
                            'connection-status-folder-outbox',
                          ),
                          text: l.availabilityFolderOutbox(
                            channels.folder.pending,
                          ),
                        ),
                      for (final c in channels.channels)
                        _ChannelTile(channel: c),
                    ],
                  ),
                  SettingsSection(
                    key: const ValueKey('connection-status-history'),
                    icon: Icons.history,
                    title: l.availabilitySectionHistory,
                    children: [
                      if (history.isEmpty)
                        _Line(text: l.availabilityHistoryEmpty)
                      else
                        for (final h in history) _HistoryTile(transition: h),
                    ],
                  ),
                ],
              ),
            ),
          );
        },
      ),
    );
  }
}

/// A plain line; takes the focus on a TV so the remote scrolls through.
class _Line extends StatelessWidget {
  const _Line({super.key, required this.text, this.style});

  final String text;
  final TextStyle? style;

  @override
  Widget build(BuildContext context) {
    final tv = KDensity.of(context).isTv;
    final child = Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Text(text, style: style ?? Theme.of(context).textTheme.bodySmall),
    );
    if (!tv) return child;
    return KFocusable(light: false, builder: (context, _) => child);
  }
}

/// What a target is: the kind of a source, a device of the family, the
/// own server or a hub.
String availabilityKindText(BuildContext context, TargetAvailability t) {
  final l = context.l10n;
  final services = Services.of(context);
  final sourceId = t.sourceId;
  if (sourceId != null) {
    final s = services.managedSource(sourceId);
    if (s != null) return sourceTypeLabel(l, s.type);
  }
  return switch (t.kind) {
    'sync-peer' => l.availabilityKindDevice, // l10n-ignore
    'kinolune' => l.availabilityKindServer(kAppName), // l10n-ignore
    'sync-hub' => l.availabilityKindHub, // l10n-ignore
    _ => t.kind.toUpperCase(),
  };
}

class _TargetTile extends StatelessWidget {
  const _TargetTile({required this.target});

  final TargetAvailability target;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final services = Services.of(context);
    final now = services.clock();
    final t = target;
    final down =
        t.status == Availability.noAccess ||
        t.status == Availability.unavailable;
    final details = [
      availabilityKindText(context, t),
      availabilityWord(l, t.status),
      if (t.origin == 'synced') l.availabilityOriginSynced, // l10n-ignore
      if (t.origin == 'auto') l.availabilityOriginAuto, // l10n-ignore
      if (t.syncChannel) l.availabilityChannelMark,
      if (t.rttMs != null && t.status.isUp) l.availabilityRtt(t.rttMs!),
      l.availabilityLastOk(availabilityWhen(context, t.lastOk, now)),
    ].join(' · ');
    final body = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        AvailabilityStatusRow(target: t),
        Padding(
          padding: const EdgeInsets.only(left: 20, bottom: 6),
          child: Text(
            details,
            key: ValueKey('connection-status-details-${t.id}'),
            style: theme.textTheme.labelSmall,
          ),
        ),
      ],
    );
    // A row that cannot be used holds its own button; a working one takes
    // the focus itself and probes again on OK.
    return KFocusable(
      light: false,
      focusable: !down,
      semanticLabel: availabilityLine(context, t, now),
      onActivate: down ? null : () => services.availability.retry(t.id),
      builder: (context, _) => body,
    );
  }
}

class _ChannelTile extends StatelessWidget {
  const _ChannelTile({required this.channel});

  final SyncChannelState channel;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final now = Services.of(context).clock();
    final c = channel;
    return Padding(
      key: ValueKey('connection-status-channel-${c.target.id}'),
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        children: [
          AvailabilityDot(c.target.status),
          const SizedBox(width: 10),
          Expanded(
            child: _Line(
              text: [
                l.availabilitySyncLast(
                  c.target.name,
                  availabilityWhen(context, c.lastSync, now),
                ),
                if (c.paused) l.availabilityChannelWaiting(c.pending),
              ].join(' · '),
            ),
          ),
        ],
      ),
    );
  }
}

class _HistoryTile extends StatelessWidget {
  const _HistoryTile({required this.transition});

  final AvailabilityTransition transition;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final now = Services.of(context).clock();
    final h = transition;
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: const EdgeInsets.only(top: 8),
          child: AvailabilityDot(h.to, size: 8),
        ),
        const SizedBox(width: 10),
        Expanded(
          child: _Line(
            text: [
              availabilityTime(context, h.at, now),
              l.availabilityHistoryLine(
                h.name,
                availabilityWord(l, h.from),
                availabilityWord(l, h.to),
              ),
              if (h.passive) l.availabilityPassive,
            ].join(' · '),
          ),
        ),
      ],
    );
  }
}

/// Settings (advanced) → «Подключения»: the servers that are not sources
/// (the own server, a hub) with their status, and the way to «Состояние
/// подключений».
class ConnectionSettingsSection extends StatelessWidget {
  const ConnectionSettingsSection({super.key});

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final availability = Services.of(context).availability;
    return ListenableBuilder(
      listenable: availability,
      builder: (context, _) {
        final servers = [
          for (final t in availability.targets)
            if (t.sourceId == null && !t.isPeer) t,
        ];
        return SettingsSection(
          key: const ValueKey('settings-section-connections'),
          icon: Icons.network_check,
          title: l.availabilitySettingsTitle,
          subtitle: l.availabilitySettingsSubtitle,
          children: [
            for (final t in servers) AvailabilityStatusRow(target: t),
            SettingsTile(
              key: const ValueKey('settings-connection-status'),
              icon: Icons.monitor_heart_outlined,
              title: l.availabilitySettingsTile,
              subtitle: l.availabilitySettingsTileSubtitle,
              onTap: () => unawaited(openConnectionStatus(context)),
            ),
          ],
        );
      },
    );
  }
}
