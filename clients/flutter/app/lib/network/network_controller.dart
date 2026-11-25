// SPDX-License-Identifier: MPL-2.0
//
// The state behind the network icon on Home and its panel (DIST-032): the
// latest [NetworkSnapshot], kept fresh by the system's change signals and
// a slow poll, and the actions of the panel. Hidden (`visible == false`)
// when the system has no NetworkManager or no Wi-Fi adapter.

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'network_backend.dart';

class NetworkController extends ChangeNotifier {
  NetworkController(
    this.backend, {
    this.poll = const Duration(seconds: 20),
    this.panelPoll = const Duration(seconds: 4),
    this.scanSettle = const Duration(milliseconds: 1500),
  });

  final NetworkBackend backend;

  /// How often the state is re-read with the panel closed and open.
  final Duration poll;
  final Duration panelPoll;

  /// How long scan results take to come in.
  final Duration scanSettle;

  NetworkSnapshot _snapshot = NetworkSnapshot.unavailable;
  NetworkSnapshot get snapshot => _snapshot;

  /// The icon and the panel are shown at all.
  bool get visible => _snapshot.available;

  /// The SSID being joined from the panel; null when idle.
  String? get joining => _joining;
  String? _joining;

  /// "Проверить интернет" is running.
  bool get checking => _checking;
  bool _checking = false;

  /// The panel is looking for networks.
  bool get scanning => _scanning;
  bool _scanning = false;

  StreamSubscription<void>? _changes;
  Timer? _timer;
  Timer? _debounce;
  int _panels = 0;
  bool _disposed = false;
  bool _started = false;

  /// Reads the state and starts following it.
  Future<void> start() async {
    if (_started) return;
    _started = true;
    await refresh();
    try {
      _changes = backend.changes.listen((_) => _soon(), onError: (_) {});
    } catch (_) {
      // No change signals: the poll still runs.
    }
    _schedule();
  }

  void _schedule() {
    _timer?.cancel();
    _timer = null;
    final every = _panels > 0 ? panelPoll : poll;
    // Zero: no polling (tests), the change signals only.
    if (_disposed || every <= Duration.zero) return;
    _timer = Timer.periodic(every, (_) => refresh());
  }

  void _soon() {
    _debounce?.cancel();
    _debounce = Timer(const Duration(milliseconds: 300), refresh);
  }

  Future<void> refresh() async {
    final s = await backend.read();
    if (_disposed) return;
    _snapshot = s;
    notifyListeners();
  }

  /// The panel opened: re-read more often and look for networks.
  void panelOpened() {
    _panels++;
    _schedule();
    unawaited(scan());
  }

  void panelClosed() {
    if (_panels > 0) _panels--;
    _schedule();
  }

  Future<void> scan() async {
    if (_scanning) return;
    _scanning = true;
    _notify();
    try {
      await backend.scan();
      // Results come in over a few seconds.
      if (scanSettle > Duration.zero) {
        await Future<void>.delayed(scanSettle);
      }
      await refresh();
    } finally {
      _scanning = false;
      _notify();
    }
  }

  Future<ConnectResult> connect(WifiNetwork n, {String? password}) =>
      _join(n.ssid, () => backend.connect(n, password: password));

  Future<ConnectResult> connectHidden(
    String ssid, {
    required WifiSecurity security,
    String? password,
  }) => _join(
    ssid,
    () => backend.connectHidden(ssid, security: security, password: password),
  );

  Future<ConnectResult> _join(
    String ssid,
    Future<ConnectResult> Function() run,
  ) async {
    _joining = ssid;
    _notify();
    try {
      return await run();
    } finally {
      _joining = null;
      await refresh();
    }
  }

  Future<void> forget(String ssid) async {
    await backend.forget(ssid);
    await refresh();
  }

  Future<void> setAutoconnect(String ssid, bool on) async {
    await backend.setAutoconnect(ssid, on);
    await refresh();
  }

  Future<void> setWifiEnabled(bool on) async {
    try {
      await backend.setWifiEnabled(on);
    } catch (_) {
      // No permission: the switch stays as the system has it.
    }
    await refresh();
  }

  Future<NetConnectivity> checkInternet() async {
    _checking = true;
    _notify();
    try {
      final c = await backend.checkInternet();
      await refresh();
      return c;
    } finally {
      _checking = false;
      _notify();
    }
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _timer?.cancel();
    _debounce?.cancel();
    unawaited(_changes?.cancel());
    unawaited(backend.close());
    super.dispose();
  }
}
