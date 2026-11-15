// SPDX-License-Identifier: MPL-2.0
//
// Subsonic and OpenSubsonic music servers (NET-016) for the app. A
// Subsonic source (Navidrome, Gonic, Airsonic, …) is added with the
// source form like WebDAV — address, user name and password, or an
// OpenSubsonic API key without a user name — and the core scans its songs
// into the music section and streams them through the proxy. What only a
// Subsonic server has is here: checking the server before it is added,
// and scrobbles of what the music player plays ([MusicScrobbler]).

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:pa_player/pa_player.dart';

import '../integration/subsonic.dart';
import 'source_manager.dart';

/// What a Subsonic server said about itself.
class SubsonicServerInfo {
  const SubsonicServerInfo({
    required this.apiVersion,
    this.serverType = '',
    this.serverVersion = '',
    this.openSubsonic = false,
  });

  /// Subsonic API version (`1.16.1`).
  final String apiVersion;

  /// Server software (`navidrome`, …); empty for a plain Subsonic server.
  final String serverType;
  final String serverVersion;
  final bool openSubsonic;
}

/// The Subsonic side of the app. [scrobble] does nothing for tracks of
/// other sources, so callers need not know where a track comes from.
abstract class SubsonicLink {
  /// Checks the server at [address] and the credentials: [username] and
  /// the password in [secret], or no [username] and an API key. Throws a
  /// [SourceException] (`authFailed`, `unreachable`, …).
  Future<SubsonicServerInfo> probe(
    String address, {
    String? username,
    required String secret,
  });

  /// Scrobbles library track [itemId]: [submission] false is "now
  /// playing", true "played" (at [at], default now). False if the track is
  /// not on a Subsonic source.
  Future<bool> scrobble(
    String itemId, {
    required bool submission,
    DateTime? at,
  });
}

/// [SubsonicLink] on the core (`CoreSubsonic`).
class CoreSubsonicLink implements SubsonicLink {
  CoreSubsonicLink(this._core);

  final CoreSubsonic _core;

  static Future<CoreSubsonicLink> open({
    required CoreLibrary library,
    required String dataDir,
  }) async => CoreSubsonicLink(
    await CoreSubsonic.open(library_: library, dataDir: dataDir),
  );

  static Future<T> _guard<T>(Future<T> Function() call) async {
    try {
      return await call();
    } on SourcesError catch (e) {
      throw SourceException.fromCore(e.code, e.message);
    }
  }

  @override
  Future<SubsonicServerInfo> probe(
    String address, {
    String? username,
    required String secret,
  }) => _guard(() async {
    final s = await _core.probe(
      address: address,
      username: username,
      secret: secret,
    );
    return SubsonicServerInfo(
      apiVersion: s.apiVersion,
      serverType: s.serverType,
      serverVersion: s.serverVersion,
      openSubsonic: s.openSubsonic,
    );
  });

  @override
  Future<bool> scrobble(
    String itemId, {
    required bool submission,
    DateTime? at,
  }) => _guard(
    () => _core.scrobble(
      itemId: itemId,
      submission: submission,
      timeMs: at?.millisecondsSinceEpoch,
    ),
  );
}

/// Reports what [player] plays to the Subsonic server of each track:
/// "now playing" when a track starts playing, "played" once it has
/// played for half its length or four minutes, whichever comes first —
/// the rule of Last.fm that Subsonic servers pass scrobbles on to —
/// for tracks of 30 seconds or more. Time skipped by seeking does not
/// count. Tracks of other sources cost one call that does nothing.
class MusicScrobbler {
  MusicScrobbler({
    required this.player,
    required this.link,
    DateTime Function()? clock,
  }) : _clock = clock ?? DateTime.now;

  final MusicPlayerController player;
  final SubsonicLink link;
  final DateTime Function() _clock;

  static const minLength = Duration(seconds: 30);
  static const enough = Duration(minutes: 4);

  /// A position step larger than this is a seek, not playback.
  static const _maxStep = Duration(seconds: 5);

  String? _id;
  DateTime? _startedAt;
  Duration? _lastPos;
  Duration _played = Duration.zero;
  bool _nowSent = false;
  bool _submitted = false;
  bool _listening = false;

  /// Scrobbles sent, for tests: `now <id>` and `played <id>`.
  @visibleForTesting
  final List<String> sent = [];

  void start() {
    if (_listening) return;
    _listening = true;
    player.addListener(_onChange);
  }

  void dispose() {
    if (!_listening) return;
    _listening = false;
    player.removeListener(_onChange);
  }

  void _reset(String? id) {
    _id = id;
    _startedAt = null;
    _lastPos = null;
    _played = Duration.zero;
    _nowSent = false;
    _submitted = false;
  }

  void _onChange() {
    final id = player.current?.itemId;
    if (id != _id) _reset(id);
    if (id == null) return;
    final pos = player.position;
    final last = _lastPos;
    // The same track again (repeat one, or back to its start).
    if (last != null && pos + _maxStep < last && pos < _maxStep) {
      _reset(id);
    }
    if (!player.playing) {
      _lastPos = pos;
      return;
    }
    if (!_nowSent) {
      _nowSent = true;
      _startedAt = _clock();
      _send(id, submission: false);
    }
    final prev = _lastPos;
    if (prev != null) {
      final step = pos - prev;
      if (step > Duration.zero && step <= _maxStep) _played += step;
    }
    _lastPos = pos;
    final length = player.duration;
    if (!_submitted &&
        length >= minLength &&
        (_played * 2 >= length || _played >= enough)) {
      _submitted = true;
      _send(id, submission: true, at: _startedAt);
    }
  }

  void _send(String id, {required bool submission, DateTime? at}) {
    sent.add('${submission ? 'played' : 'now'} $id'); // l10n-ignore
    unawaited(
      link
          .scrobble(id, submission: submission, at: at)
          .then<void>(
            (_) {},
            onError: (Object e) => debugPrint('Scrobble of $id not sent: $e'),
          ),
    );
  }
}
