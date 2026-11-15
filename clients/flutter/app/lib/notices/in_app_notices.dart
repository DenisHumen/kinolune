// SPDX-License-Identifier: MPL-2.0
//
// In-app notice banners (UI-026, ADR-NNNN): a compact card over any screen,
// like the incoming-call banner of Apple TV, for events the app already
// has — a new movie or episode after a scan, a title playing on another
// device, a finished download, a favourite channel going live, an update.
// One card at a time from a short queue; the card hides by itself. This file
// is the model and the queue; the card is notice_host.dart, the event
// sources are notice_feeds.dart.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

import '../l10n/l10n.dart';

/// What a notice is about.
enum InAppNoticeKind {
  /// A new movie or episode in the library after a scan.
  library,

  /// A title plays on another device of the profile.
  handoff,

  /// A download (offline copy, Twitch VOD, recording) finished.
  download,

  /// A favourite Twitch channel went live.
  live,

  /// A new app version (PKG-018).
  update,
}

/// A text of a notice in the current UI language.
typedef NoticeText = String Function(AppLocalizations l);

/// What a notice button does; [context] is under the app's navigator.
typedef NoticeAction = FutureOr<void> Function(BuildContext context);

/// Whether [notice] may be shown. UI-025 (protected and personal libraries)
/// plugs its rule in here.
typedef NoticeFilter = bool Function(InAppNotice notice);

@immutable
class InAppNotice {
  const InAppNotice({
    required this.id,
    required this.kind,
    required this.title,
    this.body,
    this.actionLabel,
    this.onAction,
    this.dismissLabel,
    this.onDismiss,
    this.duration = defaultDuration,
    this.itemId,
  });

  /// How long a card stays when nobody touches it.
  static const defaultDuration = Duration(seconds: 8);

  /// Notices with the same id are one notice: posting it again replaces the
  /// text of the queued or shown one (a batch of new episodes that grows).
  final String id;
  final InAppNoticeKind kind;
  final NoticeText title;
  final NoticeText? body;

  /// The primary button ("Open", "Resume", "Watch", "Update").
  final NoticeText? actionLabel;
  final NoticeAction? onAction;

  /// The second button; "Dismiss" when null.
  final NoticeText? dismissLabel;

  /// Runs when the user closes the card (the button, Back, a swipe); not
  /// when it hides by itself.
  final VoidCallback? onDismiss;

  /// Shown this long while it has neither focus nor the pointer.
  final Duration duration;

  /// The library item it is about, for [NoticeFilter]s.
  final String? itemId;
}

/// The queue of notices: one is shown, a few wait.
class InAppNotices extends ChangeNotifier {
  InAppNotices({this.maxQueued = 5});

  /// Waiting notices beyond this drop the oldest one.
  final int maxQueued;

  /// Checked when a notice is posted; null shows everything.
  NoticeFilter? filter;

  InAppNotice? _current;
  final List<InAppNotice> _queue = [];

  /// Changes whenever another notice is shown (not when its text changes).
  int _serial = 0;

  /// The notice on screen.
  InAppNotice? get current => _current;

  /// Notices waiting after [current], oldest first.
  List<InAppNotice> get queued => List.unmodifiable(_queue);

  /// Grows by one whenever another notice takes the screen.
  int get serial => _serial;

  /// Whether a notice with [id] is shown or waits.
  bool contains(String id) =>
      _current?.id == id || _queue.any((n) => n.id == id);

  /// Shows [notice] now or after the ones before it; false when [filter]
  /// refused it.
  bool post(InAppNotice notice) {
    final f = filter;
    if (f != null && !f(notice)) return false;
    if (_current?.id == notice.id) {
      _current = notice;
    } else if (_queue.indexWhere((n) => n.id == notice.id) case final i
        when i >= 0) {
      _queue[i] = notice;
    } else if (_current == null) {
      _current = notice;
      _serial++;
    } else {
      _queue.add(notice);
      while (_queue.length > maxQueued) {
        _queue.removeAt(0);
      }
    }
    notifyListeners();
    return true;
  }

  /// Hides the current notice by itself (its time ran out).
  void expire() => _next();

  /// The user closed the current notice.
  void dismiss() {
    final n = _current;
    _next();
    n?.onDismiss?.call();
  }

  /// The user pressed the primary button of the current notice.
  Future<void> activate(BuildContext context) async {
    final n = _current;
    _next();
    await n?.onAction?.call(context);
  }

  /// Removes the notice [id], shown or waiting.
  void remove(String id) {
    if (_current?.id == id) {
      _next();
      return;
    }
    final before = _queue.length;
    _queue.removeWhere((n) => n.id == id);
    if (_queue.length != before) notifyListeners();
  }

  /// Drops everything (a profile switch).
  void clear() {
    if (_current == null && _queue.isEmpty) return;
    _queue.clear();
    _current = null;
    _serial++;
    notifyListeners();
  }

  void _next() {
    if (_current == null) return;
    _current = _queue.isEmpty ? null : _queue.removeAt(0);
    _serial++;
    notifyListeners();
  }
}
