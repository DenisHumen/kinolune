// SPDX-License-Identifier: MPL-2.0
//
// TWI-035 (full builds only): the state behind "● Записать" of the live
// players. The recording itself is a job of the core's queue (TWI-018): it
// goes on when the player closes, as long as the app runs, and ends with
// the broadcast (or "Stop", or a full disk). This model only watches the
// queue for the channel on screen — polling `TwitchArchive.recordings()`
// while a button listens — and starts and stops recordings.

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'twitch_archive.dart';
import 'twitch_downloads.dart';

/// Recordings of the live players, one per [TwitchArchive].
class TwitchLiveRecordings extends ChangeNotifier {
  TwitchLiveRecordings(this.archive, {this.poll = defaultPoll});

  /// How often the queue is read while a player shows the button: size and
  /// state; the elapsed time ticks in the button itself.
  static const defaultPoll = Duration(seconds: 2);

  static final _all = Expando<TwitchLiveRecordings>();

  /// The shared model of [archive].
  static TwitchLiveRecordings of(TwitchArchive archive) =>
      _all[archive] ??= TwitchLiveRecordings(archive);

  final TwitchArchive archive;
  final Duration poll;

  List<TwitchRecordingItem> _items = const [];
  Timer? _timer;
  int _listeners = 0;
  bool _loaded = false;

  /// Whether the queue was read at least once.
  bool get loaded => _loaded;

  @override
  void addListener(VoidCallback listener) {
    super.addListener(listener);
    if (_listeners++ == 0) {
      unawaited(refresh());
      _timer = Timer.periodic(poll, (_) => unawaited(refresh()));
    }
  }

  @override
  void removeListener(VoidCallback listener) {
    super.removeListener(listener);
    if (--_listeners == 0) {
      _timer?.cancel();
      _timer = null;
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  /// Reads the queue again.
  Future<void> refresh() async {
    try {
      _items = await archive.recordings();
      _loaded = true;
      notifyListeners();
    } catch (e) {
      debugPrint('Recordings not read: $e');
    }
  }

  /// The running (or paused) recording of a channel, by user id or login.
  TwitchRecordingItem? activeFor({String? userId, required String login}) {
    final name = login.toLowerCase();
    for (final r in _items) {
      if (r.activity != RecordingActivity.active &&
          r.activity != RecordingActivity.paused) {
        continue;
      }
      if ((userId != null && userId.isNotEmpty && r.userId == userId) ||
          r.login?.toLowerCase() == name) {
        return r;
      }
    }
    return null;
  }

  /// Starts recording [login] from now; throws [TwitchRecordError].
  Future<int> start({
    required int profileId,
    required String login,
    String? userId,
  }) async {
    final id = await archive.recordNow(profileId, userId ?? '', login: login);
    await refresh();
    return id;
  }

  /// Ends a recording now and keeps it (finalised like the end of the
  /// broadcast).
  Future<void> stop(int jobId) async {
    await archive.controlRecording(jobId, TwitchDownloadAction.stop);
    await refresh();
  }
}
