// SPDX-License-Identifier: MPL-2.0
//
// Last.fm and ListenBrainz scrobbling of music (MUS-015, ADR-NNNN). The core
// (`pa-scrobble`) decides when a play counts (half the track or 4 minutes,
// tracks of 30 s and more), sends «now playing» and keeps the offline queue;
// this file follows the music player, keeps the accounts of each profile in
// the system keychain and runs the sign-in. Off until the user connects an
// account; nothing goes anywhere else.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:pa_core/pa_core.dart'
    show CoreScrobbler, CoreScrobbleTrack, CoreScrobbleStatus, ScrobbleError;
import 'package:pa_player/pa_player.dart'
    show MusicPlayerController, PlayRequest;

import '../../services/secure_store.dart';

/// Pending scrobbles and whether an account must sign in again.
@immutable
class ScrobbleStatus {
  const ScrobbleStatus({
    this.lastfmPending = 0,
    this.listenbrainzPending = 0,
    this.lastfmNeedsAuth = false,
    this.listenbrainzNeedsAuth = false,
  });

  final int lastfmPending;
  final int listenbrainzPending;
  final bool lastfmNeedsAuth;
  final bool listenbrainzNeedsAuth;
}

/// Step 1 of the Last.fm sign-in.
typedef LastFmAuthStart = ({String token, String url});

/// A track for the services.
typedef ScrobbleTrack = ({
  String artist,
  String title,
  String? album,
  int? durationMs,
});

/// Why a sign-in step failed: `no_key`, `unavailable`, `refused`,
/// `malformed`.
class ScrobbleException implements Exception {
  const ScrobbleException(this.code);
  final String code;
  @override
  String toString() => 'ScrobbleException($code)';
}

/// The core's scrobbler; tests use a fake.
abstract class ScrobbleBackend {
  /// The build has its own Last.fm API key.
  bool get hasProjectLastfmKey;

  Future<ScrobbleStatus> configure({
    String? lastfmSessionKey,
    String? lastfmApiKey,
    String? lastfmSecret,
    String? listenbrainzToken,
  });

  Future<void> trackStarted(String id, ScrobbleTrack track, int positionMs);
  Future<void> progress(int positionMs, bool playing);
  Future<void> stopped();
  Future<ScrobbleStatus> flush();

  Future<LastFmAuthStart> lastfmBegin({String? apiKey, String? secret});

  /// The account name and session key once the token is approved.
  Future<({String name, String sessionKey})> lastfmFinish(
    String token, {
    String? apiKey,
    String? secret,
  });

  Future<String> listenbrainzUser(String token);
}

/// [ScrobbleBackend] on `pa-scrobble`.
class CoreScrobbleBackend implements ScrobbleBackend {
  CoreScrobbleBackend(this._core);

  /// Opens the core's scrobbler with its queues in [dataDir].
  static CoreScrobbleBackend? open(String dataDir) {
    try {
      return CoreScrobbleBackend(CoreScrobbler.open(dataDir: dataDir));
    } catch (e) {
      debugPrint('Scrobbling unavailable: $e');
      return null;
    }
  }

  final CoreScrobbler _core;

  static ScrobbleStatus _status(CoreScrobbleStatus s) => ScrobbleStatus(
    lastfmPending: s.lastfmPending.toInt(),
    listenbrainzPending: s.listenbrainzPending.toInt(),
    lastfmNeedsAuth: s.lastfmNeedsAuth,
    listenbrainzNeedsAuth: s.listenbrainzNeedsAuth,
  );

  static Future<T> _call<T>(Future<T> Function() f) async {
    try {
      return await f();
    } on ScrobbleError catch (e) {
      // Only the code leaves here (the message is for logs).
      throw ScrobbleException(e.code);
    }
  }

  @override
  bool get hasProjectLastfmKey => CoreScrobbler.hasProjectLastfmKey();

  @override
  Future<ScrobbleStatus> configure({
    String? lastfmSessionKey,
    String? lastfmApiKey,
    String? lastfmSecret,
    String? listenbrainzToken,
  }) async => _status(
    await _core.configure(
      lastfmSessionKey: lastfmSessionKey,
      lastfmApiKey: lastfmApiKey,
      lastfmSecret: lastfmSecret,
      listenbrainzToken: listenbrainzToken,
    ),
  );

  @override
  Future<void> trackStarted(String id, ScrobbleTrack track, int positionMs) =>
      _core.trackStarted(
        id: id,
        track: CoreScrobbleTrack(
          artist: track.artist,
          title: track.title,
          album: track.album,
          durationMs: track.durationMs == null
              ? null
              : BigInt.from(track.durationMs!),
        ),
        positionMs: BigInt.from(positionMs < 0 ? 0 : positionMs),
      );

  @override
  Future<void> progress(int positionMs, bool playing) => _core.progress(
    positionMs: BigInt.from(positionMs < 0 ? 0 : positionMs),
    playing: playing,
  );

  @override
  Future<void> stopped() => _core.stopped();

  @override
  Future<ScrobbleStatus> flush() async => _status(await _core.flush());

  @override
  Future<LastFmAuthStart> lastfmBegin({String? apiKey, String? secret}) =>
      _call(() async {
        final a = await _core.lastfmBegin(apiKey: apiKey, secret: secret);
        return (token: a.token, url: a.url);
      });

  @override
  Future<({String name, String sessionKey})> lastfmFinish(
    String token, {
    String? apiKey,
    String? secret,
  }) => _call(() async {
    final s = await _core.lastfmFinish(
      token: token,
      apiKey: apiKey,
      secret: secret,
    );
    return (name: s.name, sessionKey: s.sessionKey);
  });

  @override
  Future<String> listenbrainzUser(String token) =>
      _call(() => _core.listenbrainzUser(token: token));
}

/// The scrobbling accounts of one profile. Kept as one JSON entry in the
/// keychain ([SecureStore]); [toString] shows no secret.
@immutable
class ScrobbleAccounts {
  const ScrobbleAccounts({
    this.lastfmName,
    this.lastfmSession,
    this.lastfmApiKey,
    this.lastfmSecret,
    this.listenbrainzUser,
    this.listenbrainzToken,
  });

  final String? lastfmName;
  final String? lastfmSession;

  /// The user's own Last.fm API account (required when the build has no
  /// key of its own).
  final String? lastfmApiKey;
  final String? lastfmSecret;

  final String? listenbrainzUser;
  final String? listenbrainzToken;

  bool get lastfmConnected => lastfmSession != null;
  bool get listenbrainzConnected => listenbrainzToken != null;

  ScrobbleAccounts copyWith({
    ValueGetter<String?>? lastfmName,
    ValueGetter<String?>? lastfmSession,
    ValueGetter<String?>? lastfmApiKey,
    ValueGetter<String?>? lastfmSecret,
    ValueGetter<String?>? listenbrainzUser,
    ValueGetter<String?>? listenbrainzToken,
  }) => ScrobbleAccounts(
    lastfmName: lastfmName == null ? this.lastfmName : lastfmName(),
    lastfmSession: lastfmSession == null ? this.lastfmSession : lastfmSession(),
    lastfmApiKey: lastfmApiKey == null ? this.lastfmApiKey : lastfmApiKey(),
    lastfmSecret: lastfmSecret == null ? this.lastfmSecret : lastfmSecret(),
    listenbrainzUser: listenbrainzUser == null
        ? this.listenbrainzUser
        : listenbrainzUser(),
    listenbrainzToken: listenbrainzToken == null
        ? this.listenbrainzToken
        : listenbrainzToken(),
  );

  Map<String, Object?> toJson() => {
    'lastfmName': ?lastfmName,
    'lastfmSession': ?lastfmSession,
    'lastfmApiKey': ?lastfmApiKey,
    'lastfmSecret': ?lastfmSecret,
    'listenbrainzUser': ?listenbrainzUser,
    'listenbrainzToken': ?listenbrainzToken,
  };

  factory ScrobbleAccounts.fromJson(Object? json) {
    if (json is! Map) return const ScrobbleAccounts();
    String? s(String k) =>
        json[k] is String && (json[k] as String).isNotEmpty
        ? json[k] as String
        : null;
    return ScrobbleAccounts(
      lastfmName: s('lastfmName'),
      lastfmSession: s('lastfmSession'),
      lastfmApiKey: s('lastfmApiKey'),
      lastfmSecret: s('lastfmSecret'),
      listenbrainzUser: s('listenbrainzUser'),
      listenbrainzToken: s('listenbrainzToken'),
    );
  }

  @override
  String toString() =>
      'ScrobbleAccounts(lastfm: $lastfmName, listenbrainz: $listenbrainzUser)';
}

/// Keychain entry of a profile's accounts.
String scrobbleAccountsKey(String profile) => 'scrobble_accounts_$profile';

/// Follows the music player and keeps the accounts of the active profile.
class MusicScrobbleController extends ChangeNotifier {
  MusicScrobbleController({
    required this.backend,
    required this.player,
    required this.secureStore,
    required this.profileId,
    this.progressEvery = const Duration(seconds: 5),
    this.flushEvery = const Duration(minutes: 1),
    DateTime Function()? clock,
  }) : _clock = clock ?? DateTime.now;

  /// Null where the core is not there (the v0 fallback): nothing to set.
  final ScrobbleBackend? backend;
  final MusicPlayerController player;
  final SecureStore secureStore;

  /// `syncId` of the active profile; null without profiles.
  final Future<String?> Function() profileId;

  /// The playhead goes to the core at most this often while playing.
  final Duration progressEvery;

  /// The offline queues are retried this often.
  final Duration flushEvery;
  final DateTime Function() _clock;

  ScrobbleAccounts _accounts = const ScrobbleAccounts();
  ScrobbleStatus _status = const ScrobbleStatus();
  String _profile = 'default';
  bool _listening = false;
  Timer? _flushTimer;

  String? _currentId;
  bool _lastPlaying = false;
  DateTime? _lastReport;

  /// Calls to the core, one after another.
  Future<void> _chain = Future.value();

  ScrobbleAccounts get accounts => _accounts;
  ScrobbleStatus get status => _status;
  bool get available => backend != null;

  /// Whether the user must enter their own Last.fm key.
  bool get needsOwnLastfmKey => !(backend?.hasProjectLastfmKey ?? false);

  bool get enabled =>
      _accounts.lastfmConnected || _accounts.listenbrainzConnected;

  void _enqueue(Future<void> Function() f) {
    _chain = _chain.then((_) => f()).catchError((Object e) {
      debugPrint('Scrobbling: ${e.runtimeType}');
    });
  }

  /// Loads the accounts of the active profile and starts following the
  /// player.
  Future<void> load() async {
    final b = backend;
    if (b == null) return;
    try {
      _profile = await profileId() ?? 'default';
    } catch (_) {
      _profile = 'default';
    }
    final raw = await secureStore.read(scrobbleAccountsKey(_profile));
    try {
      _accounts = ScrobbleAccounts.fromJson(raw == null ? null : jsonDecode(raw));
    } catch (_) {
      _accounts = const ScrobbleAccounts();
    }
    await _apply();
    if (!_listening) {
      _listening = true;
      player.addListener(_onPlayer);
    }
    _flushTimer ??= Timer.periodic(flushEvery, (_) => unawaited(flush()));
  }

  /// Another profile became active.
  Future<void> switchProfile() async {
    _currentId = null;
    _enqueue(() => backend?.stopped() ?? Future.value());
    await load();
  }

  Future<void> _apply() async {
    final b = backend;
    if (b == null) return;
    final a = _accounts;
    final done = Completer<void>();
    _enqueue(() async {
      try {
        _status = await b.configure(
          lastfmSessionKey: a.lastfmSession,
          lastfmApiKey: a.lastfmApiKey,
          lastfmSecret: a.lastfmSecret,
          listenbrainzToken: a.listenbrainzToken,
        );
      } finally {
        done.complete();
      }
    });
    await done.future;
    notifyListeners();
  }

  Future<void> _save(ScrobbleAccounts a) async {
    _accounts = a;
    await secureStore.write(
      scrobbleAccountsKey(_profile),
      a.toJson().isEmpty ? null : jsonEncode(a.toJson()),
    );
    await _apply();
  }

  /// What the services get for [r]; null for a track without an artist.
  static ScrobbleTrack? trackOf(PlayRequest r) {
    final artist = r.artist?.trim();
    if (artist == null || artist.isEmpty || r.title.trim().isEmpty) {
      return null;
    }
    return (
      artist: artist,
      title: r.title.trim(),
      album: r.album,
      durationMs: r.durationMs,
    );
  }

  void _onPlayer() {
    final b = backend;
    if (b == null || !enabled) return;
    final t = player.current;
    final now = _clock();
    final pos = player.position.inMilliseconds;
    final playing = player.playing;
    if (t == null) {
      if (_currentId != null) {
        _currentId = null;
        _enqueue(b.stopped);
      }
      return;
    }
    final id = t.itemId ?? t.uri;
    if (id != _currentId && playing) {
      final track = trackOf(t);
      _currentId = id;
      _lastPlaying = true;
      _lastReport = now;
      if (track == null) {
        _enqueue(b.stopped);
      } else {
        _enqueue(() => b.trackStarted(id, track, pos));
      }
      return;
    }
    if (id != _currentId) return;
    final last = _lastReport;
    if (playing != _lastPlaying ||
        last == null ||
        now.difference(last) >= progressEvery) {
      _lastPlaying = playing;
      _lastReport = now;
      _enqueue(() => b.progress(pos, playing));
    }
  }

  /// Sends what waits in the offline queues.
  Future<void> flush() async {
    final b = backend;
    if (b == null || !enabled) return;
    final done = Completer<void>();
    _enqueue(() async {
      try {
        _status = await b.flush();
      } finally {
        done.complete();
      }
    });
    await done.future;
    notifyListeners();
  }

  // -- Last.fm -----------------------------------------------------------------

  /// Saves the user's own Last.fm API key and secret (blank: none).
  Future<void> setLastfmKey(String? apiKey, String? secret) {
    String? v(String? s) => s == null || s.trim().isEmpty ? null : s.trim();
    return _save(
      _accounts.copyWith(
        lastfmApiKey: () => v(apiKey),
        lastfmSecret: () => v(secret),
      ),
    );
  }

  /// Step 1: the page where the user allows Kinolune.
  Future<LastFmAuthStart> beginLastfm() {
    final b = backend;
    if (b == null) throw const ScrobbleException('unavailable');
    return b.lastfmBegin(
      apiKey: _accounts.lastfmApiKey,
      secret: _accounts.lastfmSecret,
    );
  }

  /// Step 2, after the user allowed access in the browser.
  Future<void> finishLastfm(String token) async {
    final b = backend;
    if (b == null) throw const ScrobbleException('unavailable');
    final s = await b.lastfmFinish(
      token,
      apiKey: _accounts.lastfmApiKey,
      secret: _accounts.lastfmSecret,
    );
    await _save(
      _accounts.copyWith(
        lastfmName: () => s.name,
        lastfmSession: () => s.sessionKey,
      ),
    );
  }

  Future<void> disconnectLastfm() => _save(
    _accounts.copyWith(lastfmName: () => null, lastfmSession: () => null),
  );

  // -- ListenBrainz ------------------------------------------------------------

  /// Checks [token] and connects its user.
  Future<void> connectListenbrainz(String token) async {
    final b = backend;
    if (b == null) throw const ScrobbleException('unavailable');
    final t = token.trim();
    final user = await b.listenbrainzUser(t);
    await _save(
      _accounts.copyWith(
        listenbrainzUser: () => user,
        listenbrainzToken: () => t,
      ),
    );
  }

  Future<void> disconnectListenbrainz() => _save(
    _accounts.copyWith(
      listenbrainzUser: () => null,
      listenbrainzToken: () => null,
    ),
  );

  @override
  void dispose() {
    _flushTimer?.cancel();
    if (_listening) player.removeListener(_onPlayer);
    _listening = false;
    super.dispose();
  }
}
