// SPDX-License-Identifier: MPL-2.0
import 'dart:async';
import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// What plays, as the system media session shows it (PLAY-AND-008).
enum MediaSessionKind { music, video, live }

/// A command from the system: the notification, the lock screen, a
/// Bluetooth or wired headset, or audio focus.
enum MediaSessionCommand {
  play,
  pause,
  next,
  previous,
  seek,
  stop,

  /// Another app plays a short sound (navigation, notification): play
  /// quieter until [unduck].
  duck,
  unduck,
}

/// The state the system media session shows: notification, lock screen,
/// Bluetooth (AVRCP) metadata, the PiP window controls.
@immutable
class MediaSessionInfo {
  const MediaSessionInfo({
    required this.kind,
    required this.title,
    this.subtitle,
    this.album,
    this.artworkUri,
    this.duration = Duration.zero,
    this.position = Duration.zero,
    this.playing = false,
    this.buffering = false,
    this.ended = false,
    this.rate = 1.0,
    this.hasNext = false,
    this.hasPrevious = false,
    this.seekable = true,
    this.id,
  });

  final MediaSessionKind kind;
  final String title;

  /// Artist of a track, the show of an episode, the channel of a stream.
  final String? subtitle;
  final String? album;

  /// Cover, poster or thumbnail: a file path, `file://`, `content://` or
  /// http(s) URI.
  final String? artworkUri;

  /// Zero: unknown (and a live stream).
  final Duration duration;
  final Duration position;
  final bool playing;
  final bool buffering;
  final bool ended;
  final double rate;
  final bool hasNext;
  final bool hasPrevious;

  /// The system may seek (a scrubber on the notification and lock screen).
  final bool seekable;

  /// Identifies the item (library id or URI); a new id is a new item.
  final String? id;

  bool get live => kind == MediaSessionKind.live;

  Map<String, Object?> toMap() => {
    'kind': kind.name,
    'id': id ?? title,
    'title': title,
    'subtitle': subtitle,
    'album': album,
    'artworkUri': artworkUri,
    'durationMs': duration.inMilliseconds,
    'positionMs': position.inMilliseconds,
    'playing': playing,
    'buffering': buffering,
    'ended': ended,
    'rate': rate,
    'hasNext': hasNext,
    'hasPrevious': hasPrevious,
    'seekable': seekable && !live && duration > Duration.zero,
  };

  /// Same item and state, ignoring where the playhead is.
  bool sameExceptPosition(MediaSessionInfo o) =>
      kind == o.kind &&
      title == o.title &&
      subtitle == o.subtitle &&
      album == o.album &&
      artworkUri == o.artworkUri &&
      duration == o.duration &&
      playing == o.playing &&
      buffering == o.buffering &&
      ended == o.ended &&
      rate == o.rate &&
      hasNext == o.hasNext &&
      hasPrevious == o.hasPrevious &&
      seekable == o.seekable &&
      id == o.id;
}

/// Handles a [MediaSessionCommand]; [position] is set for
/// [MediaSessionCommand.seek].
typedef MediaSessionHandler =
    void Function(MediaSessionCommand command, Duration? position);

/// The app's single system media session (PLAY-AND-008, ADR-NNNN).
///
/// On Android it drives a Media3 `MediaSessionService` whose player is a
/// `SimpleBasePlayer` bridge over our libmpv engine: the media notification
/// with artwork and controls, the lock screen, Bluetooth and headset
/// buttons, audio focus (ducking, pause on a call) and "becoming noisy"
/// (headphones unplugged). The music player and the video player [claim] it;
/// the newest claim is shown and gets the commands, so a video on top of
/// paused music takes over the notification and gives it back on close.
///
/// Elsewhere (and in tests, unless [enabled]) it does nothing.
class SystemMediaSession {
  SystemMediaSession({
    MethodChannel? channel,
    bool? enabled,
    Duration Function()? clock,
  }) : _channel = channel ?? const MethodChannel(channelName),
       enabled = enabled ?? (!kIsWeb && Platform.isAndroid),
       _clock = clock ?? _stopwatchClock() {
    if (this.enabled) _channel.setMethodCallHandler(_onCall);
  }

  static const channelName = 'app.kinolune/media_session';

  /// The app-wide session.
  static final SystemMediaSession instance = SystemMediaSession();

  /// Position drift that is sent again; the system extrapolates the
  /// playhead from the last update while playing.
  static const positionTolerance = Duration(milliseconds: 1500);

  final MethodChannel _channel;
  final bool enabled;
  final Duration Function() _clock;
  final List<MediaSessionClaim> _claims = [];
  MediaSessionInfo? _sent;
  Duration _sentAt = Duration.zero;
  bool _cleared = true;

  static Duration Function() _stopwatchClock() {
    final w = Stopwatch()..start();
    return () => w.elapsed;
  }

  /// The claim the system shows now, if any.
  MediaSessionClaim? get active => _claims.isEmpty ? null : _claims.last;

  /// Starts showing [handler]'s playback; call [MediaSessionClaim.update]
  /// with its state and [MediaSessionClaim.release] when it is gone.
  MediaSessionClaim claim(MediaSessionHandler handler, {String? debugLabel}) {
    final c = MediaSessionClaim._(this, handler, debugLabel);
    _claims.add(c);
    _sent = null; // the new top is sent in full
    return c;
  }

  void _update(MediaSessionClaim c, MediaSessionInfo info) {
    c._info = info;
    if (!identical(active, c)) return;
    _push(info);
  }

  void _release(MediaSessionClaim c) {
    final wasTop = identical(active, c);
    _claims.remove(c);
    if (!wasTop) return;
    _sent = null;
    final top = active;
    final info = top?._info;
    if (info != null) {
      _push(info);
    } else {
      _clear();
    }
  }

  /// Where the system thinks the playhead is now.
  Duration _expectedPosition() {
    final s = _sent!;
    if (!s.playing || s.buffering || s.ended) return s.position;
    final dt = _clock() - _sentAt;
    return s.position + dt * s.rate;
  }

  void _push(MediaSessionInfo info) {
    final last = _sent;
    if (last != null && last.sameExceptPosition(info)) {
      final drift = (info.position - _expectedPosition()).abs();
      if (drift < positionTolerance) return;
    }
    _sent = info;
    _sentAt = _clock();
    _cleared = false;
    if (!enabled) return;
    unawaited(_invoke('update', info.toMap()));
  }

  void _clear() {
    _sent = null;
    if (_cleared) return;
    _cleared = true;
    if (!enabled) return;
    unawaited(_invoke('clear', null));
  }

  Future<void> _invoke(String method, Object? args) async {
    try {
      await _channel.invokeMethod<void>(method, args);
    } on MissingPluginException {
      // No native side (a test host, another platform).
    } on PlatformException catch (e) {
      debugPrint('media session: $method failed: ${e.message}');
    }
  }

  Future<Object?> _onCall(MethodCall call) async {
    if (call.method != 'command') return null;
    final args = (call.arguments as Map?)?.cast<String, Object?>() ?? const {};
    final name = args['action'];
    final command = MediaSessionCommand.values
        .where((c) => c.name == name)
        .firstOrNull;
    if (command == null) return null;
    final ms = args['positionMs'];
    dispatch(command, ms is int ? Duration(milliseconds: ms) : null);
    return null;
  }

  /// Delivers [command] to the active claim (the native side, tests).
  @visibleForTesting
  void dispatch(MediaSessionCommand command, [Duration? position]) =>
      send(command, position);

  /// Sends [command] to whatever plays now, as the system would: the macOS
  /// menu bar's Playback menu (UI-022). Nothing plays: does nothing.
  void send(MediaSessionCommand command, [Duration? position]) {
    final top = active;
    if (top == null) return;
    top._handler(command, position);
  }

  /// Play/Pause: pauses what plays, plays what is paused (UI-022).
  void togglePlay() {
    final info = active?.info;
    send(
      info != null && info.playing
          ? MediaSessionCommand.pause
          : MediaSessionCommand.play,
    );
  }
}

/// One player's hold on the [SystemMediaSession].
class MediaSessionClaim {
  MediaSessionClaim._(this._session, this._handler, this.debugLabel);

  final SystemMediaSession _session;
  final MediaSessionHandler _handler;
  final String? debugLabel;
  MediaSessionInfo? _info;
  bool _released = false;

  /// The last state passed to [update].
  MediaSessionInfo? get info => _info;
  bool get released => _released;

  /// Shows [info] (if this claim is on top); cheap to call on every tick.
  void update(MediaSessionInfo info) {
    if (_released) return;
    _session._update(this, info);
  }

  /// Gives the session back to the previous claim, or removes the
  /// notification when there is none.
  void release() {
    if (_released) return;
    _released = true;
    _session._release(this);
  }

  @override
  String toString() => 'MediaSessionClaim(${debugLabel ?? ''})';
}
