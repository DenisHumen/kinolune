// SPDX-License-Identifier: MPL-2.0
//
// The music queue survives an app restart (MUS-003, ADR-NNNN): the
// player's snapshot (tracks, order, position, shuffle, repeat, source,
// history) is kept per profile in Preferences as JSON and put back,
// paused, on the next start. Tracks are found again by their library id,
// so a network stream gets its current loopback address and a deleted
// file drops out.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../../integration/library.dart';
import '../../integration/player.dart';
import '../../services/preferences.dart';

/// What the music player gets for [t]; [PlayRequest.itemId] lets the queue
/// find the track again after a restart.
PlayRequest musicRequest(MusicTrack t) => PlayRequest(
  uri: t.item.playbackUri,
  itemId: t.id,
  title: t.title,
  artist: t.artist,
  album: t.item.album,
  albumId: t.albumId,
  artworkUri: t.item.posterUrl,
  durationMs: t.item.durationMs,
);

/// Keeps the queue of [player] for the active profile.
class MusicQueueStore {
  MusicQueueStore({
    required this.player,
    required this.preferences,
    required this.profileId,
    required this.libraryTracks,
    this.positionEvery = const Duration(seconds: 15),
    DateTime Function()? clock,
  }) : _clock = clock ?? DateTime.now;

  final MusicPlayerController player;
  final Preferences preferences;
  /// `syncId` of the active profile; null without profiles.
  final Future<String?> Function() profileId;

  /// Present library tracks by id; null when they cannot be read (the
  /// queue is then restored as saved).
  final Future<Map<String, PlayRequest>?> Function() libraryTracks;

  /// The playhead of a playing track is saved this often.
  final Duration positionEvery;
  final DateTime Function() _clock;

  String? _key;
  int _savedRevision = -1;
  bool _savedPlaying = false;
  DateTime? _savedAt;
  Future<void>? _inFlight;
  bool _dirty = false;
  bool _listening = false;
  Future<void>? _loading;

  /// Completes when the queue of the active profile is restored.
  Future<void> get ready => _loading ?? Future.value();

  Future<String> _keyFor() async {
    String? id;
    try {
      id = await profileId();
    } catch (e) {
      debugPrint('Music queue: no active profile: $e');
    }
    return Preferences.musicQueue(id ?? 'default');
  }

  /// Restores the saved queue of the active profile (paused) and starts
  /// saving changes.
  Future<void> load() => _loading = _load();

  Future<void> _load() async {
    final key = await _keyFor();
    _key = key;
    if (!_listening) {
      _listening = true;
      player.addListener(_onChange);
    }
    final raw = await preferences.read(key);
    if (raw == null || raw.isEmpty) {
      _markSaved();
      return;
    }
    try {
      final json = jsonDecode(raw);
      if (json is! Map<String, Object?>) return;
      Map<String, PlayRequest>? tracks;
      try {
        tracks = await libraryTracks();
      } catch (e) {
        debugPrint('Music queue: library tracks not read: $e');
      }
      // A key switch while the library was read: that profile wins.
      if (_key != key) return;
      await player.restore(
        json,
        // No tracks yet (a library still loading): keep the queue as saved.
        resolve: tracks == null || tracks.isEmpty
            ? null
            : (r) {
                final id = r.itemId;
                if (id == null) return r;
                return tracks![id];
              },
      );
    } catch (e) {
      debugPrint('Music queue: saved queue not restored: $e');
    }
    _markSaved();
  }

  void _markSaved() {
    _savedRevision = player.queue.revision;
    _savedPlaying = player.playing;
    _savedAt = _clock();
  }

  void _onChange() {
    if (_key == null) return;
    final rev = player.queue.revision;
    final playing = player.playing;
    if (rev != _savedRevision || playing != _savedPlaying) {
      unawaited(save());
      return;
    }
    final at = _savedAt;
    if (playing && (at == null || _clock().difference(at) >= positionEvery)) {
      unawaited(save());
    }
  }

  /// Writes the queue now (also when the app goes to the background).
  /// Saves do not overlap: one asked for during a write runs right after
  /// it, with the state of then.
  Future<void> save() {
    final running = _inFlight;
    if (running != null) {
      _dirty = true;
      return running;
    }
    return _inFlight = _saveLoop().whenComplete(() => _inFlight = null);
  }

  Future<void> _saveLoop() async {
    do {
      _dirty = false;
      final key = _key;
      if (key == null) return;
      _markSaved();
      final value = player.hasQueue ? jsonEncode(player.snapshot()) : null;
      await preferences.write(key, value);
    } while (_dirty);
  }

  /// Another profile became active: keeps the old queue, stops the
  /// music and restores the queue of the new profile.
  Future<void> switchProfile() async {
    await save();
    _key = null;
    await player.stop();
    await load();
  }

  void dispose() {
    if (_listening) player.removeListener(_onChange);
    _listening = false;
  }
}

/// Present tracks of [library] by id as the player gets them.
Future<Map<String, PlayRequest>?> libraryTrackRequests(Library library) async {
  final tracks = await libraryMusic(library).musicTracks().first;
  return {for (final t in tracks) t.id: musicRequest(t)};
}
