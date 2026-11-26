// SPDX-License-Identifier: MPL-2.0
//
// BUG-030: the signed remote config (CORE-014, ADR-0004) is checked soon
// after start, then on a timer and when the app comes back to the
// foreground. The core decides whether a check is due (the first one of a
// run always is; later ones follow `refresh.interval_secs`/`retry_secs`)
// and sends `If-None-Match`, so most checks are a `304` or no request at
// all. Offline or a bad signature keeps the config in use.

import 'dart:async';

import 'package:flutter/widgets.dart';

/// One check: true when a newer config is in use now.
typedef RemoteConfigCheck = Future<bool> Function();

/// Runs [check] [firstDelay] after [start], every [every] and on resume;
/// [onUpdated] after a check that brought a newer config. Checks never
/// overlap and never throw.
class RemoteConfigRefresher {
  RemoteConfigRefresher({
    required this.check,
    required this.onUpdated,
    this.firstDelay = const Duration(seconds: 10),
    this.every = const Duration(minutes: 30),
    this.watchLifecycle = true,
  });

  final RemoteConfigCheck check;
  final VoidCallback onUpdated;
  final Duration firstDelay;
  final Duration every;

  /// Check on `AppLifecycleState.resumed` too (off in unit tests).
  final bool watchLifecycle;

  Timer? _first;
  Timer? _periodic;
  AppLifecycleListener? _lifecycle;
  Future<void>? _running;

  /// Schedules the checks; does not wait for the network.
  void start() {
    _first ??= Timer(firstDelay, () => unawaited(run()));
    _periodic ??= Timer.periodic(every, (_) => unawaited(run()));
    if (watchLifecycle) {
      _lifecycle ??= AppLifecycleListener(onResume: () => unawaited(run()));
    }
  }

  /// One check now, joined with one already running.
  Future<void> run() => _running ??= _run().whenComplete(() => _running = null);

  Future<void> _run() async {
    try {
      if (await check()) onUpdated();
    } catch (e) {
      // Offline, bad signature, rollback: the config in use stays.
      debugPrint('Remote config check failed: $e');
    }
  }

  void dispose() {
    _first?.cancel();
    _periodic?.cancel();
    _lifecycle?.dispose();
    _first = null;
    _periodic = null;
    _lifecycle = null;
  }
}
