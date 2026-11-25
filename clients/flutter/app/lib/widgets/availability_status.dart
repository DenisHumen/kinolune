// SPDX-License-Identifier: MPL-2.0
//
// The availability indicator (UI-053, ADR-0157 §8) over the statuses of
// the core's monitor (NET-024): a dot at every source and server, a line
// «Нет доступа к NAS с 14:05» with the reason and one action (retry, enter
// the password, change the address), and the plate «Синхронизация через
// NAS на паузе: N изменений ждут на этом устройстве…» with the last
// successful sync of each channel. The banners are
// notices/availability_notices.dart, the screen «Состояние подключений»
// screens/connection_status_screen.dart.

import 'dart:async';

import 'package:intl/intl.dart';
import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../screens/add_source_dialog.dart' show showEditSourceDialog;
import '../screens/connection_status_screen.dart';
import '../services/app_services.dart';
import '../sources/availability.dart';
import '../sync/device_sync.dart';
import '../theme/kinolune_theme.dart';
import 'focusable.dart';

/// The colour of [status]: green, yellow, red or grey.
Color availabilityColor(Availability status) => switch (status) {
  Availability.available => KColors.statusOk,
  Availability.slow => KColors.statusWarning,
  Availability.noAccess || Availability.unavailable => KColors.statusProblem,
  Availability.noNetwork || Availability.unknown => KColors.textTertiary,
};

/// One or two words for [status] (chips, the status screen).
String availabilityWord(AppLocalizations l, Availability status) =>
    switch (status) {
      Availability.available => l.availabilityStatusAvailable,
      Availability.slow => l.availabilityStatusSlow,
      Availability.noAccess => l.availabilityStatusNoAccess,
      Availability.unavailable => l.availabilityStatusUnavailable,
      Availability.noNetwork => l.availabilityStatusNoNetwork,
      Availability.unknown => l.availabilityStatusUnknown,
    };

/// `14:05` today, `3 Dec 14:05` another day.
String availabilityTime(BuildContext context, DateTime t, DateTime now) {
  final sameDay =
      t.year == now.year && t.month == now.month && t.day == now.day;
  final locale = Localizations.localeOf(context).toLanguageTag();
  final time = MaterialLocalizations.of(context)
      .formatTimeOfDay(TimeOfDay.fromDateTime(t));
  if (sameDay) return time;
  return '${DateFormat.MMMd(locale).format(t)} $time';
}

/// "just now", "5 minutes ago", "at 21:40" or "not yet".
String availabilityWhen(BuildContext context, DateTime? t, DateTime now) {
  final l = context.l10n;
  if (t == null) return l.availabilityNever;
  final minutes = now.difference(t).inMinutes;
  if (minutes < 1) return l.devicesFolderJustNow;
  if (minutes < 60) return l.devicesFolderMinutesAgo(minutes);
  return l.devicesFolderAt(availabilityTime(context, t, now));
}

/// The line of a target: «Нет доступа к NAS с 14:05».
String availabilityLine(
  BuildContext context,
  TargetAvailability t,
  DateTime now,
) {
  final l = context.l10n;
  final since = t.since;
  final at = since == null ? null : availabilityTime(context, since, now);
  return switch (t.status) {
    Availability.available => l.availabilityLineAvailable(t.name),
    Availability.slow => l.availabilityLineSlow(t.name),
    Availability.noAccess =>
      at == null
          ? l.availabilityLineNoAccess(t.name)
          : l.availabilityLineNoAccessSince(t.name, at),
    Availability.unavailable =>
      at == null
          ? l.availabilityLineUnavailable(t.name)
          : l.availabilityLineUnavailableSince(t.name, at),
    Availability.noNetwork => l.availabilityLineNoNetwork,
    Availability.unknown => l.availabilityLineUnknown(t.name),
  };
}

/// Why a target cannot be used, in plain words.
enum AvailabilityReason {
  /// The user name or password was refused.
  password,

  /// No rights to the folder.
  rights,

  /// The saved password could not be read.
  secretStore,

  /// The name or address is not found on the network.
  hostNotFound,

  /// The device answers, the service does not (connection refused).
  refused,

  /// No answer in time.
  timeout,

  /// A device of the family is not seen on the network.
  notSeen,

  /// Anything else.
  noAnswer,
}

/// The reason of [t] from the core's text; null when it is up or the
/// device itself has no network.
AvailabilityReason? availabilityReason(TargetAvailability t) {
  final r = (t.reason ?? '').toLowerCase();
  switch (t.status) {
    case Availability.noAccess:
      if (r.contains('secret store')) return AvailabilityReason.secretStore;
      if (r.contains('permission') || r.contains('http 403')) {
        return AvailabilityReason.rights;
      }
      return AvailabilityReason.password;
    case Availability.unavailable:
      if (r.contains('host not found') || r.contains('no host')) {
        return AvailabilityReason.hostNotFound;
      }
      if (r.contains('refused')) return AvailabilityReason.refused;
      if (r.contains('timed out') || r.contains('timeout')) {
        return AvailabilityReason.timeout;
      }
      if (r.contains('not seen')) return AvailabilityReason.notSeen;
      return AvailabilityReason.noAnswer;
    case Availability.available ||
        Availability.slow ||
        Availability.noNetwork ||
        Availability.unknown:
      return null;
  }
}

String availabilityReasonText(AppLocalizations l, AvailabilityReason r) =>
    switch (r) {
      AvailabilityReason.password => l.availabilityReasonPassword,
      AvailabilityReason.rights => l.availabilityReasonRights,
      AvailabilityReason.secretStore => l.availabilityReasonSecretStore,
      AvailabilityReason.hostNotFound => l.availabilityReasonHostNotFound,
      AvailabilityReason.refused => l.availabilityReasonRefused,
      AvailabilityReason.timeout => l.availabilityReasonTimeout,
      AvailabilityReason.notSeen => l.availabilityReasonNotSeen,
      AvailabilityReason.noAnswer => l.availabilityReasonNoAnswer,
    };

/// The one action of a target that cannot be used.
enum AvailabilityFix {
  /// Probe again now.
  retry,

  /// The source's form, to type the password again.
  password,

  /// The source's form, to change the address.
  address,
}

/// What the button of [t] does: a refused password or rights are fixed in
/// the source's form, an address that is not found too; everything else is
/// tried again. Servers that are not sources are only tried again.
AvailabilityFix availabilityFix(TargetAvailability t) {
  if (t.sourceId == null) return AvailabilityFix.retry;
  return switch (availabilityReason(t)) {
    AvailabilityReason.password ||
    AvailabilityReason.rights ||
    AvailabilityReason.secretStore => AvailabilityFix.password,
    AvailabilityReason.hostNotFound => AvailabilityFix.address,
    _ => AvailabilityFix.retry,
  };
}

String availabilityFixLabel(AppLocalizations l, AvailabilityFix f) =>
    switch (f) {
      AvailabilityFix.retry => l.availabilityActionRetry,
      AvailabilityFix.password => l.availabilityActionPassword,
      AvailabilityFix.address => l.availabilityActionAddress,
    };

/// Does what the button of [t] promises.
Future<void> runAvailabilityFix(
  BuildContext context,
  TargetAvailability t,
) async {
  final services = Services.read(context);
  final fix = availabilityFix(t);
  final sourceId = t.sourceId;
  if (fix != AvailabilityFix.retry && sourceId != null) {
    final source = services.managedSource(sourceId);
    if (source != null) {
      final edited = await showEditSourceDialog(context, source);
      if (edited == null) return;
    }
  }
  services.availability.retry(t.id);
}

/// The dot of a status.
class AvailabilityDot extends StatelessWidget {
  const AvailabilityDot(this.status, {super.key, this.size = 10});

  final Availability status;
  final double size;

  @override
  Widget build(BuildContext context) => Semantics(
    label: availabilityWord(context.l10n, status),
    child: Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: availabilityColor(status),
        shape: BoxShape.circle,
      ),
    ),
  );
}

/// The dot of source [sourceId], following the monitor; nothing while the
/// source is not watched.
class SourceAvailabilityDot extends StatelessWidget {
  const SourceAvailabilityDot({super.key, required this.sourceId});

  final String sourceId;

  @override
  Widget build(BuildContext context) {
    final availability = Services.of(context).availability;
    return ListenableBuilder(
      listenable: availability,
      builder: (context, _) {
        final t = availability.ofSource(sourceId);
        if (t == null) return const SizedBox.shrink();
        return AvailabilityDot(
          t.status,
          key: ValueKey('availability-dot-${t.id}'),
        );
      },
    );
  }
}

/// A target in one block: the dot and the line, the reason, and the one
/// action while it cannot be used. [compact] leaves out the reason.
class AvailabilityStatusRow extends StatelessWidget {
  const AvailabilityStatusRow({
    super.key,
    required this.target,
    this.compact = false,
  });

  final TargetAvailability target;
  final bool compact;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final now = Services.of(context).clock();
    final t = target;
    final reason = availabilityReason(t);
    final down =
        t.status == Availability.noAccess ||
        t.status == Availability.unavailable;
    final fix = availabilityFix(t);
    return Padding(
      key: ValueKey('availability-row-${t.id}'),
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: FocusTraversalGroup(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              crossAxisAlignment: CrossAxisAlignment.center,
              children: [
                AvailabilityDot(t.status),
                const SizedBox(width: 10),
                Expanded(
                  child: Text(
                    availabilityLine(context, t, now),
                    style: theme.textTheme.bodyMedium,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ],
            ),
            if (!compact && reason != null)
              Padding(
                padding: const EdgeInsets.only(left: 20, top: 2),
                child: Text(
                  availabilityReasonText(l, reason),
                  style: theme.textTheme.bodySmall,
                ),
              ),
            if (down)
              Padding(
                padding: const EdgeInsets.only(left: 20, top: 8),
                child: KButton(
                  key: ValueKey('availability-fix-${t.id}'),
                  label: availabilityFixLabel(l, fix),
                  icon: switch (fix) {
                    AvailabilityFix.retry => Icons.refresh,
                    AvailabilityFix.password => Icons.key_outlined,
                    AvailabilityFix.address => Icons.edit_location_alt_outlined,
                  },
                  onPressed: () => unawaited(runAvailabilityFix(context, t)),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// A sync channel and what waits for it.
@immutable
class SyncChannelState {
  const SyncChannelState({
    required this.target,
    required this.pending,
    this.lastSync,
  });

  final TargetAvailability target;

  /// Own changes waiting on this device for this channel.
  final int pending;

  /// The last successful sync through it.
  final DateTime? lastSync;

  bool get paused => target.syncPaused;
}

/// The sync channels of [targets] with their waiting changes: the folder
/// counts its own journal ([FolderSyncState.pending]), the devices of the
/// family, a hub and the own server the LAN outbox ([outbox]).
List<SyncChannelState> syncChannels(
  List<TargetAvailability> targets, {
  required FolderSyncState folder,
  required int outbox,
}) => [
  for (final t in targets)
    if (t.syncChannel)
      folder.enabled && t.sourceId == folder.sourceId
          ? SyncChannelState(
              target: t,
              pending: folder.pending,
              lastSync: folder.lastSync,
            )
          : SyncChannelState(target: t, pending: outbox, lastSync: t.lastOk),
];

/// Keeps the sync channels and their waiting changes fresh: reads the
/// folder status and the outbox whenever the monitor changes.
class SyncChannelsModel extends ChangeNotifier {
  SyncChannelsModel(this.availability, this.sync) {
    availability.addListener(_changed);
    unawaited(refresh());
  }

  final AvailabilityService availability;
  final DeviceSync sync;

  FolderSyncState _folder = FolderSyncState.none;
  int _outbox = 0;
  bool _disposed = false;
  int _seq = 0;

  List<SyncChannelState> get channels =>
      syncChannels(availability.targets, folder: _folder, outbox: _outbox);

  /// The channels sync through which is paused.
  List<SyncChannelState> get paused => [
    for (final c in channels)
      if (c.paused) c,
  ];

  /// Changes waiting on this device across all channels.
  int get outbox => _outbox;
  FolderSyncState get folder => _folder;

  void _changed() {
    notifyListeners();
    unawaited(refresh());
  }

  /// Reads the counts again.
  Future<void> refresh() async {
    final seq = ++_seq;
    var folder = FolderSyncState.none;
    var outbox = 0;
    try {
      folder = await sync.folderSync();
    } catch (_) {}
    try {
      outbox = await sync.outboxPending();
    } catch (_) {}
    if (_disposed || seq != _seq) return;
    if (folder.pending == _folder.pending &&
        folder.lastSync == _folder.lastSync &&
        folder.sourceId == _folder.sourceId &&
        outbox == _outbox) {
      return;
    }
    _folder = folder;
    _outbox = outbox;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    availability.removeListener(_changed);
    super.dispose();
  }
}

/// «Синхронизация через NAS на паузе: 12 изменений ждут на этом устройстве
/// и уйдут сами, когда связь появится», with the last successful sync of
/// every channel. Nothing while every channel works. Focusable on a TV;
/// opens «Состояние подключений».
class SyncPausedPlate extends StatefulWidget {
  const SyncPausedPlate({super.key, this.openOnActivate = true});

  /// Opens the status screen when activated (off on that screen itself).
  final bool openOnActivate;

  @override
  State<SyncPausedPlate> createState() => _SyncPausedPlateState();
}

class _SyncPausedPlateState extends State<SyncPausedPlate> {
  SyncChannelsModel? _model;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final services = Services.of(context);
    final m = _model;
    if (m == null ||
        m.availability != services.availability ||
        m.sync != services.sync) {
      m?.dispose();
      _model = SyncChannelsModel(services.availability, services.sync);
    }
  }

  @override
  void dispose() {
    _model?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: _model!,
    builder: (context, _) {
      final model = _model!;
      final paused = model.paused;
      if (paused.isEmpty) return const SizedBox.shrink();
      final l = context.l10n;
      final theme = Theme.of(context);
      final now = Services.of(context).clock();
      final d = KDensity.of(context);
      final color = KColors.statusWarning;
      final body = Container(
        padding: const EdgeInsets.all(14),
        decoration: BoxDecoration(
          color: color.withValues(alpha: .10),
          borderRadius: BorderRadius.circular(d.panelRadius * .6),
          border: Border.all(color: color.withValues(alpha: .6)),
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Icon(Icons.sync_problem, color: color),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  for (final c in paused)
                    Padding(
                      padding: const EdgeInsets.only(bottom: 4),
                      child: Text(
                        l.availabilitySyncPaused(c.target.name, c.pending),
                        key: ValueKey(
                          'availability-sync-paused-${c.target.id}',
                        ),
                        style: theme.textTheme.bodyMedium,
                      ),
                    ),
                  const SizedBox(height: 4),
                  for (final c in model.channels)
                    Text(
                      l.availabilitySyncLast(
                        c.target.name,
                        availabilityWhen(context, c.lastSync, now),
                      ),
                      style: theme.textTheme.bodySmall,
                    ),
                ],
              ),
            ),
          ],
        ),
      );
      return Padding(
        key: const ValueKey('availability-sync-plate'),
        padding: const EdgeInsets.only(bottom: 16),
        child: widget.openOnActivate
            ? KFocusable(
                light: false,
                radius: d.panelRadius * .6,
                onActivate: () => unawaited(openConnectionStatus(context)),
                builder: (context, _) => body,
              )
            : body,
      );
    },
  );
}
