// SPDX-License-Identifier: MPL-2.0
//
// Music continues on another device (MUS-015, ADR-0156), like Spotify
// Connect but through the family's own sync (SYNC-003, ADR-0044), no
// cloud: «Продолжить на…» sends the whole queue, the current track and the
// playhead to one device (the `music` message, sealed with the family
// key); it plays from the same second and this device pauses. «Забрать
// сюда» asks a device for its queue: it answers with a push and pauses.
//
// Library ids differ between devices, so tracks travel with their tags
// and are found again in the target's own library by artist, album, title
// and length; a track it does not have drops out (a public http(s) stream
// is kept as is).

import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:pa_player/pa_player.dart'
    show MusicPlayerController, MusicQueue, PlayRequest, musicRequestFromJson;

import '../../notices/in_app_notices.dart';
import '../../sync/device_sync.dart';

/// Version of the handoff JSON.
const musicHandoffVersion = 1;

/// Largest handoff JSON, bytes (the core takes 128 KiB).
const maxMusicHandoffBytes = 120 * 1024;

/// Tracks kept before the current one when a long queue is trimmed.
const handoffHistoryWindow = 20;

/// A playing source is at most this far ahead when the target opens the
/// queue (clock skew between devices is not trusted beyond it).
const maxHandoffLagMs = 60 * 1000;

/// The handoff JSON of [player] at [now]: its queue snapshot (no history:
/// that stays on this device), whether it plays and when it was taken.
/// A queue too long for one message keeps [handoffHistoryWindow] tracks
/// before the current one and as many after it as fit.
Map<String, Object?> musicHandoffPayload(
  MusicPlayerController player, {
  required DateTime now,
}) {
  final snap = player.snapshot();
  final payload = <String, Object?>{
    ...snap,
    'recent': const <Object?>[],
    'handoff': musicHandoffVersion,
    'playing': player.playing,
    'sentAtMs': now.millisecondsSinceEpoch,
  };
  return trimMusicHandoff(payload);
}

/// [payload] cut down to [maxBytes] of JSON around its current track.
Map<String, Object?> trimMusicHandoff(
  Map<String, Object?> payload, {
  int maxBytes = maxMusicHandoffBytes,
}) {
  if (utf8.encode(jsonEncode(payload)).length <= maxBytes) return payload;
  final tracks = (payload['tracks'] as List?) ?? const [];
  final pos = payload['position'] is int ? payload['position'] as int : 0;
  final from = max(0, pos - handoffHistoryWindow);
  var after = tracks.length - pos;
  while (after > 1) {
    after = (after / 2).ceil();
    final end = min(tracks.length, pos + after);
    final out = {
      ...payload,
      'tracks': tracks.sublist(from, end),
      'position': pos - from,
    }..remove('unshuffled');
    if (utf8.encode(jsonEncode(out)).length <= maxBytes) return out;
  }
  return {
    ...payload,
    'tracks': pos < tracks.length ? [tracks[pos]] : const [],
    'position': 0,
  }..remove('unshuffled');
}

/// Where the target starts the current track at [now]: the sent playhead,
/// plus the time since it was sent while the source played (at most
/// [maxHandoffLagMs]), and not past the end of the track.
Duration musicHandoffStart(Map<String, Object?> payload, DateTime now) {
  final at = payload['positionMs'] is int ? payload['positionMs'] as int : 0;
  var ms = at;
  final sent = payload['sentAtMs'];
  if (payload['playing'] == true && sent is int) {
    ms += (now.millisecondsSinceEpoch - sent).clamp(0, maxHandoffLagMs);
  }
  final tracks = payload['tracks'];
  final pos = payload['position'];
  if (tracks is List && pos is int && pos >= 0 && pos < tracks.length) {
    final d = musicRequestFromJson(tracks[pos])?.durationMs;
    if (d != null && d > 0) ms = min(ms, max(0, d - 1000));
  }
  return Duration(milliseconds: max(0, ms));
}

String _norm(String? s) => (s ?? '').toLowerCase().replaceAll(
  RegExp(r'[^\p{L}\p{N}]+', unicode: true),
  '',
);

/// Finds tracks of another device in this device's library.
class HandoffTrackIndex {
  HandoffTrackIndex(Iterable<PlayRequest> tracks) {
    for (final t in tracks) {
      _full.putIfAbsent(_key(t.artist, t.album, t.title), () => t);
      (_loose[_key(t.artist, null, t.title)] ??= []).add(t);
    }
  }

  final _full = <String, PlayRequest>{};
  final _loose = <String, List<PlayRequest>>{};

  static String _key(String? artist, String? album, String title) =>
      '${_norm(artist)}\u0001${_norm(album)}\u0001${_norm(title)}';

  /// The local track for [remote]: same artist, album and title; else the
  /// same artist and title of about the same length (±3 s); else a public
  /// stream as is; else null.
  PlayRequest? resolve(PlayRequest remote) {
    final exact = _full[_key(remote.artist, remote.album, remote.title)];
    if (exact != null) return exact;
    final loose = _loose[_key(remote.artist, null, remote.title)];
    if (loose != null) {
      final d = remote.durationMs;
      for (final t in loose) {
        final ld = t.durationMs;
        if (d == null || ld == null || (d - ld).abs() <= 3000) return t;
      }
    }
    final uri = Uri.tryParse(remote.uri);
    if (uri != null &&
        (uri.scheme == 'http' || uri.scheme == 'https') &&
        !_isLocal(uri.host)) {
      return remote;
    }
    return null;
  }

  static bool _isLocal(String host) =>
      host == 'localhost' ||
      host == '::1' ||
      host.startsWith('127.') ||
      host.endsWith('.localhost');
}

/// Who plays what: sends and takes music between this device and the
/// others of the family.
class MusicHandoffController extends ChangeNotifier {
  MusicHandoffController({
    required this.sync,
    required this.player,
    required this.libraryTracks,
    this.notices,
    DateTime Function()? clock,
    this.pullTimeout = const Duration(seconds: 5),
  }) : _clock = clock ?? DateTime.now;

  final DeviceSync sync;
  final MusicPlayerController player;

  /// This device's music tracks as the player gets them; null when they
  /// cannot be read.
  final Future<Iterable<PlayRequest>?> Function() libraryTracks;

  final InAppNotices? notices;
  final DateTime Function() _clock;

  /// How long «Забрать сюда» waits for the other device.
  final Duration pullTimeout;

  StreamSubscription<SyncEvent>? _sub;
  final _pulls = <String, Completer<bool>>{};

  /// Devices music can go to: online, of this family, not this one.
  List<SyncDevice> targets() {
    try {
      return sync
          .devices()
          .where((d) => !d.thisDevice && d.online)
          .toList(growable: false);
    } catch (_) {
      return const [];
    }
  }

  /// Whether «Продолжить на…» makes sense: sync runs and music is loaded.
  bool get canContinue => sync.available && player.hasQueue;

  void start() => _sub ??= sync.events.listen(_onEvent, onError: (_) {});

  @override
  void dispose() {
    unawaited(_sub?.cancel());
    _sub = null;
    for (final c in _pulls.values) {
      if (!c.isCompleted) c.complete(false);
    }
    super.dispose();
  }

  /// «Продолжить на [device]»: sends the queue and pauses here. False
  /// when nothing was sent (no queue, sync not running).
  Future<bool> continueOn(SyncDevice device, {String? session}) async {
    if (!player.hasQueue) return false;
    final payload = musicHandoffPayload(player, now: _clock());
    final id = await sync.sendMusic(
      to: device.id,
      queue: jsonEncode(payload),
      session: session,
    );
    if (id == null) return false;
    player.pause();
    notices?.post(
      InAppNotice(
        id: 'music-handoff',
        kind: InAppNoticeKind.handoff,
        title: (l) => l.musicHandoffSentTitle(device.name),
      ),
    );
    return true;
  }

  /// «Забрать сюда»: asks [device] for its queue. True when it answered
  /// in time (the music then plays here).
  Future<bool> pullFrom(SyncDevice device) async {
    final id = await sync.sendMusic(to: device.id);
    if (id == null) return false;
    final done = Completer<bool>();
    _pulls[id] = done;
    final timer = Timer(pullTimeout, () {
      if (!done.isCompleted) done.complete(false);
    });
    try {
      return await done.future;
    } finally {
      timer.cancel();
      _pulls.remove(id);
    }
  }

  void _onEvent(SyncEvent e) {
    final f = e.music;
    if (e.kind != SyncEventKind.music || f == null) return;
    if (f.pull) {
      unawaited(_answerPull(f));
    } else {
      unawaited(receive(f));
    }
  }

  Future<void> _answerPull(MusicHandoffFrame f) async {
    if (!player.hasQueue) return;
    final target =
        targets().where((d) => d.id == f.deviceId).firstOrNull ??
        SyncDevice(
          id: f.deviceId,
          name: f.deviceName ?? f.deviceId,
          kind: SyncDeviceKind.phone,
          fingerprint: '',
        );
    await continueOn(target, session: f.session);
  }

  /// Plays a queue another device handed over. Returns false for one that
  /// does not read or has no track this device can play.
  Future<bool> receive(MusicHandoffFrame f) async {
    final raw = f.queue;
    if (raw == null) return false;
    Map<String, Object?> payload;
    try {
      final json = jsonDecode(raw);
      if (json is! Map<String, Object?>) return false;
      payload = json;
    } catch (_) {
      return false;
    }
    if (payload['v'] != MusicQueue.jsonVersion) return false;
    Iterable<PlayRequest>? local;
    try {
      local = await libraryTracks();
    } catch (e) {
      debugPrint('Music handoff: library tracks not read: $e');
    }
    final index = HandoffTrackIndex(local ?? const []);
    final tracks = (payload['tracks'] as List?) ?? const [];
    final pos = payload['position'] is int ? payload['position'] as int : 0;
    final current = pos >= 0 && pos < tracks.length
        ? musicRequestFromJson(tracks[pos])
        : null;
    // Nothing here to play: keep this device's own queue.
    final playable = tracks.any((t) {
      final r = musicRequestFromJson(t);
      return r != null && index.resolve(r) != null;
    });
    if (!playable) return false;
    final currentKept = current != null && index.resolve(current) != null;
    final start = musicHandoffStart(payload, _clock());
    final ok = await player.restore(payload, resolve: index.resolve);
    if (!ok || !player.hasQueue) return false;
    if (currentKept) player.seek(start);
    player.resume();
    final from = f.deviceName ?? f.deviceId;
    notices?.post(
      InAppNotice(
        id: 'music-handoff',
        kind: InAppNoticeKind.handoff,
        title: (l) => l.musicHandoffReceivedTitle(from),
      ),
    );
    _pulls[f.session]?.complete(true);
    notifyListeners();
    return true;
  }
}
