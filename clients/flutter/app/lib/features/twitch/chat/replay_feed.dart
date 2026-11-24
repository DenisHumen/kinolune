// SPDX-License-Identifier: MPL-2.0
//
// VOD chat replay (TWI-016, research 06 §3.4), full builds only: the chat
// of a recording in step with the player.
//
// * A [ChatReplaySource] gives rendered lines with their offsets in
//   stretches ([ReplayChunk]): the core fetches GQL pages
//   (`VideoCommentsByOffsetOrCursor`) or reads a TwitchDownloader file next
//   to a downloaded VOD.
// * [ReplayChatFeed.update] gets the player position: the chat time is the
//   position minus the user's delay (±N s). A binary search finds the last
//   line at or before it. Playing forward adds the lines passed since the
//   last tick; a seek (back, or a jump forward) draws again the last
//   [ReplayChatFeed.window] lines before the new position. Paused, the
//   position stands still and so does the chat; at 2× it runs at 2×.
// * The stretch ahead is asked for [ReplayChatFeed.prefetch] before the
//   playhead reaches its end, so pages come before they are needed.
// * The feed is a [ChatFeed]: the live chat panel, view and controller
//   (TWI-012, TWI-023 emotes, TWI-024 filters) show it unchanged. It is
//   read-only.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';

import 'chat_model.dart';
import 'chat_settings.dart';

/// A line and when the replay shows it.
class ReplayLine {
  const ReplayLine(this.offsetMs, this.line);

  /// Milliseconds from the VOD start.
  final int offsetMs;
  final ChatLine line;
}

/// Every line shown in `[fromMs, toMs)` (`toMs` null: to the end of the
/// VOD), oldest first.
class ReplayChunk {
  const ReplayChunk({required this.fromMs, this.toMs, required this.lines});

  final int fromMs;
  final int? toMs;
  final List<ReplayLine> lines;
}

/// Why the chat of a recording cannot be shown.
class ChatReplayUnavailable implements Exception {
  const ChatReplayUnavailable(this.code, [this.detail = '']);

  /// `flag_off`, `network`, `gql_hash_invalid`, `integrity_required`,
  /// `not_found`, `decode`, …
  final String code;
  final String detail;

  @override
  String toString() => 'ChatReplayUnavailable($code${detail.isEmpty ? '' : ': $detail'})';
}

/// Where the lines come from (the core, or a test).
abstract class ChatReplaySource {
  /// Channel login or name, for the header.
  String get channel;

  /// The lines from [lookbackMs] before [offsetMs] to the end of what can
  /// be had there now. Throws [ChatReplayUnavailable].
  Future<ReplayChunk> chunkAt(int offsetMs, {required int lookbackMs});

  /// Badge and emote images as they arrive.
  Stream<ChatImage> get images;

  /// Viewer filters for lines from now on; returns the skipped rules.
  List<String> setFilters(ChatSettings settings);

  void close();
}

/// State of the replay for the panel.
enum ChatReplayStatus {
  /// Waiting for the first lines.
  loading,

  /// Lines are shown.
  ready,

  /// The chat of this recording is not available («Чат записи
  /// недоступен»): the config keeps it off, Twitch refused, no network.
  unavailable,
}

/// The delay the user can set: ±[kChatReplayMaxDelay].
const kChatReplayMaxDelay = Duration(seconds: 30);

/// One step of the delay buttons.
const kChatReplayDelayStep = Duration(seconds: 1);

class ReplayChatFeed implements ChatFeed {
  ReplayChatFeed(
    this.source, {
    this.window = 150,
    this.lookback = const Duration(seconds: 20),
    this.prefetch = const Duration(seconds: 30),
    this.jump = const Duration(seconds: 10),
    this.retryAfter = const Duration(seconds: 10),
    DateTime Function()? clock,
  }) : _clock = clock ?? DateTime.now {
    _images = source.images.listen(
      (i) => _emit([ImageReady(i)]),
      onError: (Object _) {},
    );
  }

  final ChatReplaySource source;

  /// Lines drawn after a seek (the newest ones before the position).
  final int window;

  /// How far back a seek asks for lines.
  final Duration lookback;

  /// The next stretch is asked for this long before the playhead needs it.
  final Duration prefetch;

  /// A forward move longer than this is a seek, not playback.
  final Duration jump;

  /// After a failure, wait this long before asking again.
  final Duration retryAfter;

  final DateTime Function() _clock;
  late final StreamSubscription<ChatImage> _images;
  final _out = StreamController<List<ChatUpdate>>();

  /// Lines by offset; ties keep the order they came in ([_seq]).
  final _lines = <ReplayLine>[];
  final _seq = <ReplayLine, int>{};
  final _ids = <String>{};
  var _nextSeq = 0;

  /// Stretches whose lines are all here: `[from, to)`, `to` null = end.
  final _covered = <(int, int?)>[];

  /// Chat time of the last update; null: draw the window again.
  int? _shownUntil;
  int _position = 0;
  var _closed = false;
  Future<void>? _loading;
  int? _loadingAt;
  DateTime? _failedAt;

  /// Bumped by [setFilters]: chunks asked for before belong to old rules.
  var _generation = 0;

  final ValueNotifier<ChatReplayStatus> status = ValueNotifier(
    ChatReplayStatus.loading,
  );

  /// Code of the last failure ([ChatReplayUnavailable.code]).
  String? failure;

  /// Chat delay: positive shows lines later than the video, negative
  /// earlier.
  final ValueNotifier<Duration> delay = ValueNotifier(Duration.zero);

  /// The chat time for a video [position].
  int chatTime(Duration position) =>
      math.max(0, position.inMilliseconds - delay.value.inMilliseconds);

  @override
  String get channel => source.channel;

  @override
  bool get canSend => false;

  @override
  String? get ownLogin => null;

  @override
  List<ChatEmote> get emotes => const [];

  @override
  Future<ChatSendResult> send(String text, {String? replyTo}) async =>
      const ChatSendFailed(ChatSendError.readOnly);

  @override
  List<String> setFilters(ChatSettings settings) {
    final errors = source.setFilters(settings);
    // Lines here were rendered with the old rules: ask again.
    _generation++;
    _lines.clear();
    _seq.clear();
    _ids.clear();
    _covered.clear();
    _shownUntil = null;
    _loading = null;
    _loadingAt = null;
    _failedAt = null;
    if (status.value != ChatReplayStatus.unavailable) {
      unawaited(_load(chatTime(Duration(milliseconds: _position))));
    }
    return errors;
  }

  @override
  Stream<List<ChatUpdate>> get updates => _out.stream;

  /// Lines held (tests).
  int get held => _lines.length;

  /// Sets the delay, clamped to ±[kChatReplayMaxDelay]; the chat is drawn
  /// again at the new time.
  void setDelay(Duration value) {
    final max = kChatReplayMaxDelay.inMilliseconds;
    final ms = value.inMilliseconds.clamp(-max, max);
    if (ms == delay.value.inMilliseconds) return;
    delay.value = Duration(milliseconds: ms);
    _shownUntil = null;
    update(Duration(milliseconds: _position));
  }

  /// The player is at [position] (call on every position change; a pause
  /// simply stops the calls or repeats the same position).
  void update(Duration position) {
    if (_closed) return;
    _position = position.inMilliseconds;
    final t = chatTime(position);
    final cover = _coverAt(t);
    if (cover == null) {
      unawaited(_load(t));
      return;
    }
    final prev = _shownUntil;
    if (prev == null || t < prev || t - prev > jump.inMilliseconds) {
      final end = _countUntil(t);
      final start = math.max(0, end - window);
      _emit([
        LinesReplaced([for (final r in _lines.sublist(start, end)) r.line]),
      ]);
    } else if (t > prev) {
      final from = _countUntil(prev);
      final to = _countUntil(t);
      if (to > from) {
        _emit([for (final r in _lines.sublist(from, to)) LineAdded(r.line)]);
      }
    }
    _shownUntil = t;
    // Prefetch the next stretch before the playhead gets there.
    final end = cover.$2;
    if (end != null && end - t < prefetch.inMilliseconds) {
      unawaited(_load(end, lookbackMs: 0));
    }
  }

  /// Binary search: how many lines are shown at or before [t].
  int _countUntil(int t) {
    var lo = 0;
    var hi = _lines.length;
    while (lo < hi) {
      final mid = (lo + hi) >> 1;
      if (_lines[mid].offsetMs <= t) {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }
    return lo;
  }

  (int, int?)? _coverAt(int t) {
    for (final c in _covered) {
      if (t >= c.$1 && (c.$2 == null || t < c.$2!)) return c;
    }
    return null;
  }

  Future<void> _load(int at, {int? lookbackMs}) {
    final busy = _loading;
    if (busy != null) {
      // One request at a time; a seek elsewhere asks again afterwards.
      if (_loadingAt != at) {
        return busy.then((_) {
          if (!_closed) update(Duration(milliseconds: _position));
        });
      }
      return busy;
    }
    final failed = _failedAt;
    if (failed != null && _clock().difference(failed) < retryAfter) {
      return Future.value();
    }
    final generation = _generation;
    _loadingAt = at;
    final future = _loading = () async {
      try {
        final chunk = await source.chunkAt(
          at,
          lookbackMs: lookbackMs ?? lookback.inMilliseconds,
        );
        if (_closed || generation != _generation) return;
        _merge(chunk);
        _failedAt = null;
        if (status.value != ChatReplayStatus.ready) {
          status.value = ChatReplayStatus.ready;
          _emit(const [ConnectionChanged(ChatConnection.connected)]);
        }
      } on ChatReplayUnavailable catch (e) {
        if (_closed || generation != _generation) return;
        failure = e.code;
        _failedAt = _clock();
        // Lines already here stay; with none the panel says so.
        if (_lines.isEmpty) status.value = ChatReplayStatus.unavailable;
      } catch (e) {
        if (_closed || generation != _generation) return;
        failure = 'error';
        _failedAt = _clock();
        if (_lines.isEmpty) status.value = ChatReplayStatus.unavailable;
      } finally {
        if (generation == _generation) {
          _loading = null;
          _loadingAt = null;
        }
      }
    }();
    return future.then((_) {
      if (!_closed && generation == _generation && _failedAt == null) {
        update(Duration(milliseconds: _position));
      }
    });
  }

  void _merge(ReplayChunk chunk) {
    for (final r in chunk.lines) {
      if (r.line.id.isNotEmpty && !_ids.add(r.line.id)) continue;
      _seq[r] = _nextSeq++;
      _lines.add(r);
    }
    _lines.sort((a, b) {
      final c = a.offsetMs.compareTo(b.offsetMs);
      return c != 0 ? c : _seq[a]!.compareTo(_seq[b]!);
    });
    if (chunk.toMs == null || chunk.toMs! > chunk.fromMs) {
      _covered.add((chunk.fromMs, chunk.toMs));
      _covered.sort((a, b) => a.$1.compareTo(b.$1));
      final merged = <(int, int?)>[];
      for (final c in _covered) {
        if (merged.isNotEmpty) {
          final last = merged.last;
          if (last.$2 == null || c.$1 <= last.$2!) {
            final end = last.$2 == null || c.$2 == null
                ? null
                : math.max(last.$2!, c.$2!);
            merged[merged.length - 1] = (last.$1, end);
            continue;
          }
        }
        merged.add(c);
      }
      _covered
        ..clear()
        ..addAll(merged);
    }
  }

  void _emit(List<ChatUpdate> batch) {
    if (!_closed && !_out.isClosed) _out.add(batch);
  }

  @override
  void close() {
    if (_closed) return;
    _closed = true;
    unawaited(_images.cancel());
    source.close();
    // The notifiers are not disposed: the panel may still listen while
    // it goes away.
    if (!_out.isClosed) unawaited(_out.close());
  }
}

/// A source with fixed lines (tests, previews): every chunk is the whole
/// chat, or fails with [failure].
class ManualReplaySource implements ChatReplaySource {
  ManualReplaySource(
    this.all, {
    this.channel = 'test_channel',
    this.failure,
    this.step,
  });

  /// All lines, oldest first.
  final List<ReplayLine> all;

  @override
  final String channel;

  /// Every [chunkAt] fails with it when set.
  ChatReplayUnavailable? failure;

  /// Pages of this length instead of everything: `[at - lookback,
  /// at + step)`.
  final Duration? step;

  final requests = <(int, int)>[];
  final _images = StreamController<ChatImage>.broadcast();
  ChatSettings? filters;
  var closed = false;

  @override
  Future<ReplayChunk> chunkAt(int offsetMs, {required int lookbackMs}) async {
    requests.add((offsetMs, lookbackMs));
    final f = failure;
    if (f != null) throw f;
    final s = step;
    if (s == null) return ReplayChunk(fromMs: 0, lines: all);
    final from = math.max(0, offsetMs - lookbackMs);
    final to = offsetMs + s.inMilliseconds;
    final last = all.isEmpty ? 0 : all.last.offsetMs;
    return ReplayChunk(
      fromMs: from,
      toMs: to > last ? null : to,
      lines: [
        for (final r in all)
          if (r.offsetMs >= from && r.offsetMs < to) r,
      ],
    );
  }

  void addImage(ChatImage image) => _images.add(image);

  @override
  Stream<ChatImage> get images => _images.stream;

  @override
  List<String> setFilters(ChatSettings settings) {
    filters = settings;
    return const [];
  }

  @override
  void close() {
    closed = true;
    unawaited(_images.close());
  }
}
