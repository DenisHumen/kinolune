// SPDX-License-Identifier: MPL-2.0
//
// Trakt in the app (SYNC-011, ADR-0154): sign-in by a device code, the
// player's scrobbles, the two-way sync of watched marks and ratings, the
// watchlist shelf and the item menu's Trakt actions. The work is done by
// the Rust core (`CoreTrakt`); [TraktLink] is what the screens use, so
// tests can give a fake.

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../integration/trakt.dart';

/// A Trakt call failed. [code]: `not_configured`, `signed_out`,
/// `expired`, `denied`, `rate_limited`, `network`, `library`, `other`.
class TraktException implements Exception {
  const TraktException(this.code, [this.message = '']);

  final String code;
  final String message;

  @override
  String toString() => 'TraktException($code): $message';
}

/// A device sign-in in progress: show [userCode], [verificationUrl] and a
/// QR code of [qrUrl]; poll every [interval].
class TraktDeviceLogin {
  const TraktDeviceLogin({
    required this.userCode,
    required this.verificationUrl,
    required this.qrUrl,
    required this.expiresIn,
    required this.interval,
  });

  final String userCode;
  final String verificationUrl;
  final String qrUrl;
  final Duration expiresIn;
  final Duration interval;
}

/// What a sync changed.
class TraktSyncResult {
  const TraktSyncResult({
    this.watchedImported = 0,
    this.watchedExported = 0,
    this.ratingsImported = 0,
    this.ratingsExported = 0,
  });

  final int watchedImported;
  final int watchedExported;
  final int ratingsImported;
  final int ratingsExported;
}

/// What the app does with Trakt.
abstract class TraktLink {
  /// The signed-in user name ('' if unknown); null when signed out.
  ValueListenable<String?> get account;

  /// A client id is set (in the build or by the user).
  bool get configured;

  /// The user entered their own client id (advanced settings).
  bool get ownClient;

  Future<void> setOwnClient(String? clientId, String? clientSecret);

  Future<TraktDeviceLogin> startLogin();

  /// The user name once confirmed, null while waiting.
  Future<String?> pollLogin();

  void cancelLogin();

  Future<void> signOut();

  /// [action]: `start`, `pause` or `stop`; [progress] 0–100.
  Future<bool> scrobble(String itemId, String action, double progress);

  Future<TraktSyncResult> sync();

  Future<bool> setWatched(String itemId, bool watched);

  Future<bool> setRating(String itemId, int? stars);

  /// Library item ids on the watchlist, newest first.
  Future<List<String>> watchlistItems({bool refresh = false});

  Future<bool> inWatchlist(String itemId);

  Future<bool> setWatchlisted(String itemId, bool listed);
}

/// [TraktLink] on the Rust core.
class CoreTraktLink implements TraktLink {
  CoreTraktLink(this._core) : _account = ValueNotifier(_core.account());

  final CoreTrakt _core;
  final ValueNotifier<String?> _account;

  /// Opens Trakt over [library] with the secret store whose fallback file
  /// is in [dataDir].
  static Future<CoreTraktLink> open({
    required CoreLibrary library,
    required String dataDir,
  }) async =>
      CoreTraktLink(await CoreTrakt.open(library_: library, dataDir: dataDir));

  static Future<T> _guard<T>(Future<T> Function() call) async {
    try {
      return await call();
    } on TraktError catch (e) {
      throw TraktException(e.code, e.message);
    }
  }

  void _refreshAccount() => _account.value = _core.account();

  static int _now() => DateTime.now().millisecondsSinceEpoch;

  @override
  ValueListenable<String?> get account => _account;

  @override
  bool get configured => _core.isConfigured();

  @override
  bool get ownClient => _core.hasOwnClient();

  @override
  Future<void> setOwnClient(String? clientId, String? clientSecret) => _guard(
    () async {
      await _core.setOwnClient(clientId: clientId, clientSecret: clientSecret);
      _refreshAccount();
    },
  );

  @override
  Future<TraktDeviceLogin> startLogin() => _guard(() async {
    final l = await _core.startLogin();
    return TraktDeviceLogin(
      userCode: l.userCode,
      verificationUrl: l.verificationUrl,
      qrUrl: l.qrUrl,
      expiresIn: Duration(seconds: l.expiresInSecs),
      interval: Duration(seconds: l.intervalSecs),
    );
  });

  @override
  Future<String?> pollLogin() => _guard(() async {
    final name = await _core.pollLogin();
    if (name != null) _refreshAccount();
    return name;
  });

  @override
  void cancelLogin() => _core.cancelLogin();

  @override
  Future<void> signOut() => _guard(() async {
    try {
      await _core.signOut();
    } finally {
      _refreshAccount();
    }
  });

  @override
  Future<bool> scrobble(String itemId, String action, double progress) =>
      _guard(
        () =>
            _core.scrobble(itemId: itemId, action: action, progress: progress),
      );

  @override
  Future<TraktSyncResult> sync() => _guard(() async {
    try {
      final r = await _core.sync_();
      return TraktSyncResult(
        watchedImported: r.watchedImported,
        watchedExported: r.watchedExported,
        ratingsImported: r.ratingsImported,
        ratingsExported: r.ratingsExported,
      );
    } finally {
      // A refused refresh signs out.
      _refreshAccount();
    }
  });

  @override
  Future<bool> setWatched(String itemId, bool watched) => _guard(
    () => _core.setWatched(itemId: itemId, watched: watched, atMs: _now()),
  );

  @override
  Future<bool> setRating(String itemId, int? stars) =>
      _guard(() => _core.setRating(itemId: itemId, stars: stars, atMs: _now()));

  @override
  Future<List<String>> watchlistItems({bool refresh = false}) =>
      _guard(() => _core.watchlistItems(refresh: refresh));

  @override
  Future<bool> inWatchlist(String itemId) =>
      _guard(() => _core.inWatchlist(itemId: itemId));

  @override
  Future<bool> setWatchlisted(String itemId, bool listed) =>
      _guard(() => _core.setWatchlisted(itemId: itemId, listed: listed));
}

/// Scrobbles one player session to Trakt: `start` at the first progress
/// report and again after a pause, `pause` when the reports stop for
/// [idle] (the player reports every second while playing, SYNC-014),
/// `stop` with the last position when the player closes or moves on to
/// another item (Up Next). Trakt records a play at a `stop` of 80 % and
/// more. Errors are logged, never shown.
class TraktScrobbler {
  TraktScrobbler(
    this._link, {
    this.idle = const Duration(seconds: 4),
    DateTime Function()? clock,
    bool timer = true,
  }) : _clock = clock ?? DateTime.now {
    if (timer && _link != null) {
      _timer = Timer.periodic(const Duration(seconds: 2), (_) => tick());
    }
  }

  final TraktLink? _link;
  final Duration idle;
  final DateTime Function() _clock;
  Timer? _timer;

  String? _item;
  double _progress = 0;
  bool _playing = false;
  DateTime? _lastAt;

  /// Events sent, for tests: `start <id> <progress>` and so on.
  @visibleForTesting
  final List<String> sent = [];

  bool get _active => _link?.account.value != null;

  /// A progress report of the player.
  void report(String itemId, int positionMs, int durationMs) {
    if (!_active || durationMs <= 0) return;
    if (_item != null && _item != itemId) _send('stop');
    if (_item != itemId) {
      _item = itemId;
      _playing = false;
    }
    _progress = (positionMs * 100 / durationMs).clamp(0, 100).toDouble();
    _lastAt = _clock();
    if (!_playing) {
      _playing = true;
      _send('start');
    }
  }

  /// Checks for a pause: no report for [idle] while playing.
  void tick() {
    final at = _lastAt;
    if (!_playing || at == null || _item == null) return;
    if (_clock().difference(at) >= idle) {
      _playing = false;
      _send('pause');
    }
  }

  /// The player closed.
  void close() {
    _timer?.cancel();
    _timer = null;
    if (_item != null) _send('stop');
    _item = null;
    _playing = false;
  }

  void _send(String action) {
    final item = _item;
    final link = _link;
    if (item == null || link == null) return;
    final progress = _progress;
    sent.add('$action $item ${progress.toStringAsFixed(1)}');
    link
        .scrobble(item, action, progress)
        .then<void>(
          (_) {},
          onError: (Object e) => debugPrint('Trakt $action of $item: $e'),
        );
  }
}

/// Runs a sync now and then every [every] while someone is signed in
/// (started by the app after the library opens). Errors are logged.
class TraktAutoSync {
  TraktAutoSync(this._link, {this.every = const Duration(hours: 6)});

  final TraktLink _link;
  final Duration every;
  Timer? _timer;
  bool _running = false;

  void start({Duration delay = const Duration(seconds: 20)}) {
    _timer?.cancel();
    _timer = Timer(delay, () {
      unawaited(run());
      _timer = Timer.periodic(every, (_) => unawaited(run()));
    });
  }

  /// One sync, unless one runs or nobody is signed in.
  Future<TraktSyncResult?> run() async {
    if (_running || _link.account.value == null) return null;
    _running = true;
    try {
      return await _link.sync();
    } catch (e) {
      debugPrint('Trakt sync failed: $e');
      return null;
    } finally {
      _running = false;
    }
  }

  void dispose() => _timer?.cancel();
}
