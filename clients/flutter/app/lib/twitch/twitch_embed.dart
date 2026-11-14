// SPDX-License-Identifier: MPL-2.0
//
// The official Twitch embed (TWI-005, ADR-0011 §5, research 06 §2.2): the
// fallback player of store builds and of a `player = embed` remote config.
// Twitch's interactive embed (`player.twitch.tv/js/embed/v1.js`) runs in a
// WebView on a small page of ours; the page posts the player's events and,
// for VODs, its position (`getCurrentTime`/`getDuration`) through a
// JavaScript channel. [TwitchEmbedSession] turns them into Kinolune
// progress (`twitch:vod:<id>`: Continue watching, sync between devices)
// and handoff heartbeats (SYNC-003).
//
// Twitch requires the `parent` parameter: the domain of the page that
// embeds the player, served over HTTPS. The page is loaded from a string
// with the project's HTTPS domain as its base URL ([twitchEmbedParent]),
// so the WebView reports that origin to the embed; nothing is fetched
// from that domain.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../integration/library.dart' show Progress;
import '../sync/handoff.dart';
import 'twitch_service.dart';

/// The `parent` domain of the embed: the project's HTTPS site (GitHub
/// Pages of the public repository). `--dart-define` overrides it.
const twitchEmbedParent = String.fromEnvironment(
  'KINOLUNE_TWITCH_EMBED_PARENT',
  defaultValue: 'denishumen.github.io', // l10n-ignore
);

/// Name of the JavaScript channel the page posts to.
const twitchEmbedChannel = 'KinoluneEmbed'; // l10n-ignore

/// Base URL the page is loaded with: its origin is the `parent`.
Uri twitchEmbedBaseUrl([String parent = twitchEmbedParent]) =>
    Uri.parse('https://$parent/kinolune/twitch-embed'); // l10n-ignore

/// `1h2m3s`: the `time` option of a VOD.
String twitchEmbedTime(Duration d) {
  final h = d.inHours;
  final m = d.inMinutes.remainder(60);
  final s = d.inSeconds.remainder(60);
  return '${h}h${m}m${s}s'; // l10n-ignore
}

/// Options of `new Twitch.Player(…)` for [target].
Map<String, Object> twitchEmbedOptions(
  TwitchPlayTarget target, {
  String parent = twitchEmbedParent,
}) => {
  'width': '100%', // l10n-ignore
  'height': '100%', // l10n-ignore
  'parent': [parent],
  'autoplay': true,
  'muted': false,
  if (target.isLive)
    'channel': target.login!
  else ...{
    'video': 'v${target.videoId}', // l10n-ignore
    if (target.startAt != null && target.startAt! > Duration.zero)
      'time': twitchEmbedTime(target.startAt!),
  },
};

/// The page of the embed. The options are JSON with `<` escaped, so no
/// value can close the script element.
String twitchEmbedHtml(
  TwitchPlayTarget target, {
  String parent = twitchEmbedParent,
}) {
  final options = jsonEncode(twitchEmbedOptions(target, parent: parent))
      .replaceAll('<', r'\u003c');
  return _page
      .replaceFirst('/*OPTIONS*/null', options)
      .replaceFirst('/*VOD*/false', target.isLive ? 'false' : 'true');
}

// Twitch: the embed must not be covered and must be at least 400×300; the
// page gives the player the whole WebView. Events go to Dart as JSON:
// {"event": "ready|play|pause|seek|ended|online|offline|time|error",
//  "position": s, "duration": s} (position and duration for VODs only).
const _page = r'''<!DOCTYPE html>
<html><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<style>html,body{margin:0;width:100%;height:100%;background:#000;overflow:hidden}
#player{width:100%;height:100%}</style></head>
<body><div id="player"></div>
<script src="https://player.twitch.tv/js/embed/v1.js"></script>
<script>
(function () {
  var options = /*OPTIONS*/null;
  var vod = /*VOD*/false;
  function post(m) {
    try { KinoluneEmbed.postMessage(JSON.stringify(m)); } catch (e) {}
  }
  if (typeof Twitch === 'undefined' || !Twitch.Player) {
    post({event: 'error', message: 'script'}); // l10n-ignore
    return;
  }
  var player = new Twitch.Player('player', options);
  function report(name) {
    var m = {event: name};
    if (vod) {
      try {
        m.position = player.getCurrentTime();
        m.duration = player.getDuration();
      } catch (e) {}
    }
    post(m);
  }
  var E = Twitch.Player;
  player.addEventListener(E.READY, function () { report('ready'); });
  player.addEventListener(E.PLAY, function () { report('play'); });
  player.addEventListener(E.PAUSE, function () { report('pause'); });
  player.addEventListener(E.SEEK, function () { report('seek'); });
  player.addEventListener(E.ENDED, function () { report('ended'); });
  player.addEventListener(E.ONLINE, function () { report('online'); });
  player.addEventListener(E.OFFLINE, function () { report('offline'); });
  setInterval(function () {
    var paused = true;
    try { paused = player.isPaused(); } catch (e) {}
    if (vod && !paused) report('time');
  }, 5000);
  window.kinolune = {
    seek: function (s) { player.seek(s); },
    pause: function () { player.pause(); },
    play: function () { player.play(); },
    report: function () { report('time'); }
  };
})();
</script></body></html>''';

/// An event of the embed page.
enum TwitchEmbedEventKind {
  ready,
  play,
  pause,
  seek,
  ended,
  online,
  offline,
  time,
  error,
}

class TwitchEmbedEvent {
  const TwitchEmbedEvent(this.kind, {this.position, this.duration});

  final TwitchEmbedEventKind kind;

  /// VODs only.
  final Duration? position;
  final Duration? duration;

  /// Parses a message of the page; null for anything else.
  static TwitchEmbedEvent? parse(String raw) {
    Object? json;
    try {
      json = jsonDecode(raw);
    } on FormatException {
      return null;
    }
    if (json is! Map) return null;
    final event = json['event'];
    final kind = TwitchEmbedEventKind.values
        .where((k) => k.name == event)
        .firstOrNull;
    if (kind == null) return null;
    Duration? secs(Object? v) => v is num && v.isFinite && v >= 0
        ? Duration(milliseconds: (v * 1000).round())
        : null;
    return TwitchEmbedEvent(
      kind,
      position: secs(json['position']),
      duration: secs(json['duration']),
    );
  }
}

/// What the embed screen shows.
enum TwitchEmbedState { loading, playing, paused, ended, offline, failed }

/// One embed playback: the player's events become the screen state, VOD
/// positions become progress (every report of the page: about every 5 s
/// while playing, on pause, seek and end) and handoff heartbeats.
class TwitchEmbedSession {
  TwitchEmbedSession({
    required this.target,
    required this.saveProgress,
    this.handoff,
    DateTime Function()? clock,
  }) : clock = clock ?? DateTime.now;

  final TwitchPlayTarget target;
  final Future<void> Function(Progress progress) saveProgress;
  final HandoffController? handoff;
  final DateTime Function() clock;

  final ValueNotifier<TwitchEmbedState> state = ValueNotifier(
    TwitchEmbedState.loading,
  );

  /// `twitch:vod:<id>` (the media key of the VOD), null for live.
  String? get itemId =>
      target.isLive ? null : 'twitch:vod:${target.videoId}'; // l10n-ignore

  Duration _position = Duration.zero;
  Duration _duration = Duration.zero;
  Duration? _saved;
  bool _begun = false;
  bool _closed = false;

  Duration get position => _position;
  Duration get duration => _duration;

  /// A message of the page.
  Future<void> handle(String raw) async {
    if (_closed) return;
    final e = TwitchEmbedEvent.parse(raw);
    if (e == null) return;
    if (e.duration != null && e.duration! > Duration.zero) {
      _duration = e.duration!;
    }
    final known = e.position != null;
    if (known) _position = e.position!;
    switch (e.kind) {
      case TwitchEmbedEventKind.ready:
        // The position is not applied yet: begin from where we asked.
        _begin(target.startAt ?? Duration.zero);
      case TwitchEmbedEventKind.play || TwitchEmbedEventKind.online:
        state.value = TwitchEmbedState.playing;
        _begin(_position);
      case TwitchEmbedEventKind.time:
        state.value = TwitchEmbedState.playing;
        if (known) await _save();
      case TwitchEmbedEventKind.pause:
        state.value = TwitchEmbedState.paused;
        if (known) await _save();
      case TwitchEmbedEventKind.seek:
        if (known) await _save();
      case TwitchEmbedEventKind.ended:
        state.value = TwitchEmbedState.ended;
        if (!target.isLive && _duration > Duration.zero) {
          _position = _duration;
          await _save(watched: true);
        }
      case TwitchEmbedEventKind.offline:
        state.value = TwitchEmbedState.offline;
      case TwitchEmbedEventKind.error:
        state.value = TwitchEmbedState.failed;
    }
  }

  void _begin(Duration at) {
    final id = itemId;
    if (_begun || id == null) return;
    _begun = true;
    handoff?.begin(id, at.inMilliseconds, durationMs: _duration.inMilliseconds);
  }

  Future<void> _save({bool watched = false}) async {
    final id = itemId;
    // Before READY applied `time`, the page may report 0: keep the old
    // position rather than resetting it.
    if (id == null || _duration <= Duration.zero) return;
    if (_position <= Duration.zero && !watched) return;
    if (_saved == _position && !watched) return;
    _saved = _position;
    _begin(_position);
    handoff?.report(id, _position.inMilliseconds, _duration.inMilliseconds);
    try {
      await saveProgress(
        Progress(
          itemId: id,
          positionMs: _position.inMilliseconds,
          durationMs: _duration.inMilliseconds,
          updatedAt: clock(),
          watched: watched,
        ),
      );
    } catch (e) {
      debugPrint('Twitch embed progress not saved: $e');
    }
  }

  /// The screen closes: the last position and the end of the handoff
  /// session.
  Future<void> close() async {
    if (_closed) return;
    if (state.value != TwitchEmbedState.ended) await _save();
    _closed = true;
    if (_begun) handoff?.end();
  }
}
