// SPDX-License-Identifier: MPL-2.0
//
// Background downloads on Android (NET-019, ADR-NNNN). The offline copies
// run in the Rust job queue of the process; this binding tells the Android
// host what is in the queue (channel `app.kinolune/downloads`, `update`),
// so it keeps the process going in a foreground service of type dataSync
// with a progress notification, keeps this engine without an activity and
// schedules a WorkManager kick. Pause, resume and cancel from the
// notification come back as `command`. Battery Saver holds the downloads
// (pauses them, remembering which) while no UI of the app is visible.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../services/preferences.dart';
import 'offline_downloads.dart';

/// What the Android host needs to know about the queue.
@immutable
class BackgroundDownloadsSnapshot {
  const BackgroundDownloadsSnapshot({
    this.active = 0,
    this.runnable = 0,
    this.blockedByNetwork = 0,
    this.paused = 0,
    this.held = 0,
    this.current,
    this.wifiOnly = true,
  });

  /// Unfinished downloads.
  final int active;

  /// Running, or will run without the user (queued, retrying, waiting for
  /// space, paused by the queue for resources).
  final int runnable;

  /// Waiting for a network: "Wi-Fi only" on mobile data, or offline.
  final int blockedByNetwork;

  /// Paused by the user.
  final int paused;

  /// Paused by this binding for Battery Saver.
  final int held;

  /// The download the notification shows.
  final OfflineDownload? current;
  final bool wifiOnly;

  Map<String, Object?> toMap() => {
    'active': active,
    'runnable': runnable,
    'blockedByNetwork': blockedByNetwork,
    'paused': paused,
    'held': held,
    'jobId': current?.jobId,
    'title': current?.title,
    'bytesDone': current?.bytesDone ?? 0,
    'bytesTotal': current?.bytesTotal,
    'wifiOnly': wifiOnly,
  };

  /// Same counts and download, progress aside.
  bool sameShape(BackgroundDownloadsSnapshot o) =>
      active == o.active &&
      runnable == o.runnable &&
      blockedByNetwork == o.blockedByNetwork &&
      paused == o.paused &&
      held == o.held &&
      current?.jobId == o.current?.jobId &&
      current?.title == o.current?.title &&
      wifiOnly == o.wifiOnly;

  static BackgroundDownloadsSnapshot of(
    List<OfflineDownload> downloads, {
    required bool wifiOnly,
    Set<int> held = const {},
  }) {
    var active = 0, runnable = 0, blocked = 0, paused = 0, heldN = 0;
    OfflineDownload? running, waiting, rest;
    for (final d in downloads) {
      if (!d.state.isActive) continue;
      active++;
      if (d.state == OfflineState.paused && held.contains(d.jobId)) {
        heldN++;
        rest ??= d;
      } else if (d.waitingFor == 'network') {
        blocked++;
        waiting ??= d;
      } else if (d.state == OfflineState.paused &&
          (d.waitingFor == null || d.waitingFor == 'user')) {
        paused++;
        rest ??= d;
      } else {
        runnable++;
        if (d.state == OfflineState.running) {
          running ??= d;
        } else {
          waiting ??= d;
        }
      }
    }
    return BackgroundDownloadsSnapshot(
      active: active,
      runnable: runnable,
      blockedByNetwork: blocked,
      paused: paused,
      held: heldN,
      current: running ?? waiting ?? rest,
      wifiOnly: wifiOnly,
    );
  }
}

/// Binds an [OfflineController] to the Android host.
class BackgroundDownloads with WidgetsBindingObserver {
  BackgroundDownloads(
    this.offline, {
    required this.preferences,
    MethodChannel? channel,
    this.progressInterval = const Duration(seconds: 1),
  }) : _channel = channel ?? const MethodChannel(channelName);

  static const channelName = 'app.kinolune/downloads';

  final OfflineController offline;
  final Preferences preferences;
  final Duration progressInterval;
  final MethodChannel _channel;

  bool _started = false;
  bool _powerSave = false;

  /// "Resume" from the notification during Battery Saver: no hold until it
  /// is turned off and on again.
  bool _override = false;
  bool _visible = false;
  final Set<int> _held = {};
  BackgroundDownloadsSnapshot? _sent;
  DateTime _sentAt = DateTime.fromMillisecondsSinceEpoch(0);
  Timer? _progressTimer;
  Future<void> _holding = Future.value();

  /// Job ids held for Battery Saver (tests).
  Set<int> get held => Set.unmodifiable(_held);

  Future<void> start() async {
    if (_started) return;
    _started = true;
    _held.addAll(
      _decodeHeld(await preferences.read(Preferences.offlineHeldJobs)),
    );
    _channel.setMethodCallHandler(_onCall);
    final binding = WidgetsBinding.instance;
    binding.addObserver(this);
    _visible = _isVisible(binding.lifecycleState);
    offline.addListener(_changed);
    try {
      final r = await _channel.invokeMapMethod<String, Object?>('ready');
      _powerSave = r?['powerSave'] == true;
    } on MissingPluginException {
      return;
    } on PlatformException catch (e) {
      debugPrint('Background downloads unavailable: $e');
      return;
    }
    await _evaluateHold();
    _send(force: true);
  }

  void dispose() {
    if (!_started) return;
    _started = false;
    offline.removeListener(_changed);
    WidgetsBinding.instance.removeObserver(this);
    _channel.setMethodCallHandler(null);
    _progressTimer?.cancel();
  }

  static bool _isVisible(AppLifecycleState? s) =>
      s == AppLifecycleState.resumed || s == AppLifecycleState.inactive;

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    final v = _isVisible(state);
    if (v == _visible) return;
    _visible = v;
    unawaited(_evaluateHold());
  }

  BackgroundDownloadsSnapshot _snapshot() => BackgroundDownloadsSnapshot.of(
    offline.downloads,
    wifiOnly: offline.settings.wifiOnly,
    held: _held,
  );

  void _changed() {
    // A download the hold paused may have been cancelled or done. Not by
    // state: right after the pause a running job still reports `running`
    // until its runner stops; a held id counts only while it is paused.
    _held.removeWhere(
      (id) => !offline.downloads.any((d) => d.jobId == id && d.state.isActive),
    );
    _send();
  }

  /// Sends a change of the counts at once and progress at most once per
  /// [progressInterval].
  void _send({bool force = false}) {
    final s = _snapshot();
    final last = _sent;
    if (!force && last != null && last.sameShape(s)) {
      if (last.current?.bytesDone == s.current?.bytesDone) return;
      final wait = progressInterval - DateTime.now().difference(_sentAt);
      if (wait > Duration.zero) {
        _progressTimer ??= Timer(wait, () {
          _progressTimer = null;
          _send();
        });
        return;
      }
    }
    _progressTimer?.cancel();
    _progressTimer = null;
    _sent = s;
    _sentAt = DateTime.now();
    unawaited(
      _channel.invokeMethod<void>('update', s.toMap()).catchError((Object e) {
        debugPrint('Downloads not reported: $e');
      }),
    );
  }

  Future<Object?> _onCall(MethodCall call) async {
    switch (call.method) {
      case 'command':
        final args = (call.arguments as Map?) ?? const {};
        await command(
          args['action'] as String?,
          (args['jobId'] as num?)?.toInt(),
        );
      case 'powerSave':
        _powerSave = call.arguments == true;
        if (!_powerSave) _override = false;
        await _evaluateHold();
    }
    return null;
  }

  /// A notification action: `pause` (every download that runs or waits),
  /// `resume` (every paused one, also those held for Battery Saver) or
  /// `cancel` ([jobId], else the one the notification shows).
  Future<void> command(String? action, int? jobId) async {
    await offline.refresh();
    final active = offline.downloads.where((d) => d.state.isActive).toList();
    switch (action) {
      case 'pause':
        for (final d in active) {
          if (d.state != OfflineState.paused) {
            await _control(d, OfflineAction.pause);
          }
        }
      case 'resume':
        if (_powerSave) _override = true;
        for (final d in active) {
          if (d.state == OfflineState.paused) {
            await _control(d, OfflineAction.resume);
          }
        }
        await _setHeld(const {});
      case 'cancel':
        final id = jobId ?? _snapshot().current?.jobId;
        for (final d in active) {
          if (d.jobId == id) await _control(d, OfflineAction.cancel);
        }
    }
    _send(force: true);
  }

  Future<void> _control(OfflineDownload d, OfflineAction a) async {
    try {
      await offline.control(d, a);
    } catch (e) {
      debugPrint('Download ${d.jobId} not ${a.name}d: $e');
    }
  }

  /// Battery Saver with no visible UI pauses the downloads that run or
  /// wait; they resume when either ends. Serialised: lifecycle and power
  /// changes can come quickly.
  Future<void> _evaluateHold() =>
      _holding = _holding.then((_) => _applyHold()).catchError((Object e) {
        debugPrint('Battery Saver hold not applied: $e');
      });

  Future<void> _applyHold() async {
    final hold = _powerSave && !_visible && !_override;
    if (hold) {
      await offline.refresh();
      final add = <int>{};
      for (final d in offline.downloads) {
        final id = d.jobId;
        if (id == null || !d.state.isActive) continue;
        if (d.state == OfflineState.paused) continue;
        await _control(d, OfflineAction.pause);
        add.add(id);
      }
      if (add.isNotEmpty) await _setHeld({..._held, ...add});
    } else if (_held.isNotEmpty) {
      final ids = Set.of(_held);
      await _setHeld(const {});
      await offline.refresh();
      for (final d in offline.downloads) {
        if (ids.contains(d.jobId) && d.state == OfflineState.paused) {
          await _control(d, OfflineAction.resume);
        }
      }
    }
    _send(force: true);
  }

  Future<void> _setHeld(Set<int> ids) async {
    _held
      ..clear()
      ..addAll(ids);
    await preferences.write(
      Preferences.offlineHeldJobs,
      ids.isEmpty ? null : jsonEncode(ids.toList()..sort()),
    );
  }

  static Set<int> _decodeHeld(String? raw) {
    if (raw == null) return {};
    try {
      return {for (final v in jsonDecode(raw) as List) (v as num).toInt()};
    } catch (_) {
      return {};
    }
  }
}
