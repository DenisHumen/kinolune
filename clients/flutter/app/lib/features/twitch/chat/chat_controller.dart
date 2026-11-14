// SPDX-License-Identifier: MPL-2.0
//
// State of one chat view (TWI-012, research 06 §3.3): a ring buffer of the
// last [ChatController.capacity] lines, a pause while the user reads older
// lines ("N new messages"), moderation applied in place, TV thinning and
// the images the core has downloaded.

import 'dart:async';
import 'dart:collection';

import 'package:flutter/foundation.dart';

import 'chat_model.dart';

class ChatController extends ChangeNotifier {
  ChatController(
    this.feed, {
    this.capacity = 500,
    this.maxLinesPerSecond,
    DateTime Function()? clock,
  }) : assert(capacity > 0),
       _clock = clock ?? DateTime.now {
    _sub = feed.updates.listen(apply, onDone: _onDone);
  }

  /// Lines kept on screen; older ones drop out (ring buffer).
  final int capacity;

  /// TV thinning: at most this many new lines per second are shown, the
  /// rest are counted in [skipped] (`+N`). `null`: everything.
  final int? maxLinesPerSecond;

  final ChatFeed feed;
  final DateTime Function() _clock;
  late final StreamSubscription<List<ChatUpdate>> _sub;

  final _lines = ListQueue<ChatLine>();
  final _pending = ListQueue<ChatLine>();
  final _byId = <String, ChatLine>{};
  final _images = <String, ChatImage>{};
  var _following = true;
  var _skipped = 0;
  var _skippedNow = 0;
  var _imagesVersion = 0;
  var _windowStart = DateTime.fromMillisecondsSinceEpoch(0);
  var _windowCount = 0;
  var _systemSeq = 0;
  var _disposed = false;

  ChatConnection _connection = ChatConnection.connecting;

  /// Lines on screen, oldest first.
  int get length => _lines.length;
  ChatLine operator [](int index) => _lines.elementAt(index);
  Iterable<ChatLine> get lines => _lines;

  /// Lines that arrived while the view was paused ("N new messages").
  int get unseen => _pending.length;

  /// `true` while new lines go straight to the list.
  bool get following => _following;

  /// Lines dropped by TV thinning in the last full second (`+N`).
  int get skipped => _skipped;

  /// Bumped when an image arrives: lines with emotes redraw.
  int get imagesVersion => _imagesVersion;

  ChatConnection get connection => _connection;

  /// Image of [key] when the core has it on disk.
  ChatImage? image(String key) => _images[key];

  /// The user scrolled up: keep the list still, collect new lines.
  void pause() {
    if (!_following) return;
    _following = false;
    notifyListeners();
  }

  /// Back at the newest line: show what was collected.
  void resume() {
    if (_following && _pending.isEmpty) return;
    _following = true;
    while (_pending.isNotEmpty) {
      _push(_pending.removeFirst());
    }
    notifyListeners();
  }

  /// Applies one batch from the feed (public for tests).
  void apply(List<ChatUpdate> batch) {
    if (_disposed) return;
    for (final u in batch) {
      switch (u) {
        case LineAdded(:final line):
          _add(line);
        case LineDeleted(:final messageId):
          final line = _byId[messageId];
          if (line != null && line.fate == ChatLineFate.shown) {
            line.fate = ChatLineFate.deleted;
          }
        case UserCleared(:final userId, :final login, :final seconds, :final banned):
          final fate = banned
              ? ChatLineFate.banned
              : (seconds != null ? ChatLineFate.timedOut : ChatLineFate.deleted);
          for (final l in [..._lines, ..._pending]) {
            if (l.kind == ChatLineKind.message &&
                (l.userId == userId || (userId.isEmpty && l.login == login))) {
              l.fate = fate;
            }
          }
          if (banned) {
            _addSystem(UserBannedText(login));
          } else if (seconds != null) {
            _addSystem(UserTimedOutText(login, seconds));
          }
        case ChatCleared():
          _lines.clear();
          _pending.clear();
          _byId.clear();
          _addSystem(const ChatClearedText());
        case ServerNotice(:final text):
          _addSystem(ServerNoticeText(text));
        case ConnectionChanged(:final state):
          _connection = state;
        case ImageReady(:final image):
          _images[image.key] = image;
          _imagesVersion++;
      }
    }
    notifyListeners();
  }

  void _addSystem(ChatSystemText text) {
    _systemSeq++;
    _add(ChatLine.system('system-$_systemSeq', text), thin: false);
  }

  void _add(ChatLine line, {bool thin = true}) {
    if (thin && _thinned()) return;
    if (_following) {
      _push(line);
    } else {
      _pending.addLast(line);
      _byId[line.id] = line;
      while (_pending.length > capacity) {
        _byId.remove(_pending.removeFirst().id);
      }
    }
  }

  bool _thinned() {
    final max = maxLinesPerSecond;
    if (max == null) return false;
    final now = _clock();
    if (now.difference(_windowStart) >= const Duration(seconds: 1)) {
      _skipped = _skippedNow;
      _skippedNow = 0;
      _windowStart = now;
      _windowCount = 0;
    }
    _windowCount++;
    if (_windowCount <= max) return false;
    _skippedNow++;
    return true;
  }

  void _push(ChatLine line) {
    _lines.addLast(line);
    _byId[line.id] = line;
    while (_lines.length > capacity) {
      final old = _lines.removeFirst();
      if (identical(_byId[old.id], old)) _byId.remove(old.id);
    }
  }

  void _onDone() {
    if (_disposed) return;
    _connection = ChatConnection.stopped;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _sub.cancel();
    feed.close();
    super.dispose();
  }
}
