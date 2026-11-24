// SPDX-License-Identifier: MPL-2.0
//
// NET-024 (ADR-0157 §8): availability of servers and sources. The core
// runs one monitor for every source, however it was added, plus the
// devices of the family and the sync folder; this is its view for the
// UI. The full indicator, banners and the status screen are UI-053.

import 'dart:async';

import 'package:connectivity_plus/connectivity_plus.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:pa_core/pa_core.dart';

import '../offline/network_watch.dart' show listenConnectivity;
import '../sync/sync_pace.dart' show syncForeground;

/// Status of a server or source.
enum Availability {
  /// Not checked yet.
  unknown,

  /// Answers.
  available,

  /// Answers slowly.
  slow,

  /// Answers but refuses the password, token or rights.
  noAccess,

  /// Does not answer (see [TargetAvailability.since]).
  unavailable,

  /// This device has no network.
  noNetwork;

  /// From the core's name.
  static Availability parse(String s) => switch (s) {
    'available' => available,
    'slow' => slow,
    'no-access' => noAccess,
    'unavailable' => unavailable,
    'no-network' => noNetwork,
    _ => unknown,
  };

  /// Whether it can be used now.
  bool get isUp => this == available || this == slow;
}

/// One watched server or source.
@immutable
class TargetAvailability {
  const TargetAvailability({
    required this.id,
    required this.kind,
    required this.name,
    required this.status,
    this.origin = 'added',
    this.syncChannel = false,
    this.since,
    this.lastOk,
    this.rttMs,
    this.reason,
  });

  factory TargetAvailability.fromCore(CoreAvailabilityTarget t) =>
      TargetAvailability(
        id: t.id,
        kind: t.kind,
        name: t.name,
        status: Availability.parse(t.status),
        origin: t.origin,
        syncChannel: t.syncChannel,
        since: _time(t.sinceMs),
        lastOk: _time(t.lastOkMs),
        rttMs: t.rttMs,
        reason: t.reason,
      );

  static DateTime? _time(int? ms) =>
      ms == null ? null : DateTime.fromMillisecondsSinceEpoch(ms);

  /// `source:<id>`, `peer:<device id>` or `endpoint:<id>`.
  final String id;

  /// `smb`, `webdav`, `jellyfin`, `sync-peer`, … (see the core).
  final String kind;
  final String name;
  final Availability status;

  /// `added`, `synced` or `auto`.
  final String origin;

  /// Sync goes through it.
  final bool syncChannel;

  /// Since when it has [status].
  final DateTime? since;

  /// Last time it answered.
  final DateTime? lastOk;
  final int? rttMs;

  /// Why it is not available.
  final String? reason;

  /// The source id of a `source:<id>` target.
  String? get sourceId => id.startsWith('source:') ? id.substring(7) : null;
}

/// Availability of every server and source, for the UI.
abstract class AvailabilityService implements Listenable {
  /// Every watched target.
  List<TargetAvailability> get targets;

  /// The status of source [sourceId], if it is watched.
  TargetAvailability? ofSource(String sourceId);

  /// Probe [targetId] (or everything) now: the "retry" action.
  void retry([String? targetId]);
}

/// No monitor (v0 fallback, tests, demo): nothing is watched.
class NoAvailability extends ChangeNotifier implements AvailabilityService {
  NoAvailability();

  @override
  List<TargetAvailability> get targets => const [];

  @override
  TargetAvailability? ofSource(String sourceId) => null;

  @override
  void retry([String? targetId]) {}
}

/// A fixed list, for widget tests and the demo.
class FakeAvailability extends ChangeNotifier implements AvailabilityService {
  FakeAvailability([List<TargetAvailability> targets = const []])
    : _targets = targets;

  List<TargetAvailability> _targets;
  final retried = <String?>[];

  @override
  List<TargetAvailability> get targets => _targets;

  set targets(List<TargetAvailability> v) {
    _targets = v;
    notifyListeners();
  }

  @override
  TargetAvailability? ofSource(String sourceId) =>
      _targets.where((t) => t.sourceId == sourceId).firstOrNull;

  @override
  void retry([String? targetId]) => retried.add(targetId);
}

/// The core's monitor (NET-024): tells it the app state and network
/// changes, keeps the latest statuses.
class CoreAvailabilityService extends ChangeNotifier
    implements AvailabilityService {
  CoreAvailabilityService._(this._core) {
    _refresh();
    _sub = _core.watch().listen((_) => _refresh(), onError: (_) {});
    var foreground = syncForeground(WidgetsBinding.instance.lifecycleState);
    _lifecycle = AppLifecycleListener(
      onStateChange: (s) {
        final now = syncForeground(s);
        if (now == foreground) return;
        foreground = now;
        _core.setAppState(foreground: now);
      },
    );
    _core.setAppState(foreground: foreground);
    // Any change of connections (Wi-Fi to mobile, another Wi-Fi, back
    // online) probes everything at once.
    listenConnectivity((r) {
      final online =
          r.isNotEmpty && !r.every((c) => c == ConnectivityResult.none);
      _core.networkChanged(online: online);
    }, what: 'availability');
  }

  /// Opens the monitor over the sources of the library database at
  /// [dbPath]; [mobile]: a phone or tablet, which never probes in the
  /// background.
  static Future<CoreAvailabilityService> open({
    required String dbPath,
    required String dataDir,
    required bool mobile,
  }) async => CoreAvailabilityService._(
    await CoreAvailability.open(
      dbPath: dbPath,
      dataDir: dataDir,
      mobile: mobile,
    ),
  );

  final CoreAvailability _core;
  StreamSubscription<CoreAvailabilityEvent>? _sub;
  AppLifecycleListener? _lifecycle;
  List<TargetAvailability> _targets = const [];

  /// Also watches the devices of the family and the sync folder.
  void attachSync(CoreSync sync) => _core.attachSync(sync_: sync);

  /// Servers that are not sources: the own Kinolune server, a sync hub.
  void setEndpoints(List<CoreEndpoint> endpoints) =>
      _core.setEndpoints(endpoints: endpoints);

  /// A real operation on [targetId] worked or failed (passive signal).
  void report(String targetId, {bool ok = true, bool denied = false}) {
    if (ok) {
      _core.reportOk(targetId: targetId);
    } else {
      _core.reportFailure(targetId: targetId, denied: denied, reason: '');
    }
  }

  /// Recent status changes, oldest first.
  List<CoreAvailabilityTransition> history() => _core.history();

  void _refresh() {
    _targets = [
      for (final t in _core.targets()) TargetAvailability.fromCore(t),
    ];
    notifyListeners();
  }

  @override
  List<TargetAvailability> get targets => _targets;

  @override
  TargetAvailability? ofSource(String sourceId) =>
      _targets.where((t) => t.sourceId == sourceId).firstOrNull;

  @override
  void retry([String? targetId]) => _core.probeNow(targetId: targetId);

  @override
  void dispose() {
    unawaited(_sub?.cancel());
    _lifecycle?.dispose();
    super.dispose();
  }
}
