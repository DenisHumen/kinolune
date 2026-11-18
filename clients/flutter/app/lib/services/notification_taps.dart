// SPDX-License-Identifier: MPL-2.0
//
// Taps on the app's own notifications (QA-015). The notification plugin
// is one per process and keeps one tap callback, the one of its last
// `initialize`: every place that initializes it passes [NotificationTaps.
// handle], and the app listens here. A tap that started the app arrives
// before anyone listens and waits for the listener.
//
// Payloads: `twitch/channel/<login>` (TWI-009 "X is live") opens the
// channel; others are ignored.

/// The router of notification taps.
class NotificationTaps {
  NotificationTaps._();

  static final instance = NotificationTaps._();

  void Function(String payload)? _listener;
  String? _pending;

  /// A tap with [payload] (the plugin's callback, or its launch details).
  void handle(String? payload) {
    if (payload == null || payload.isEmpty) return;
    final listener = _listener;
    if (listener == null) {
      _pending = payload;
    } else {
      listener(payload);
    }
  }

  /// Sets the one listener; a tap that came before it is delivered now.
  void listen(void Function(String payload) listener) {
    _listener = listener;
    final pending = _pending;
    _pending = null;
    if (pending != null) listener(pending);
  }

  /// Removes [listener] if it is the current one.
  void unlisten(void Function(String payload) listener) {
    if (_listener == listener) _listener = null;
  }
}

/// The Twitch login of a "X is live" payload `twitch/channel/<login>`.
String? twitchChannelOfPayload(String payload) {
  const prefix = 'twitch/channel/';
  if (!payload.startsWith(prefix)) return null;
  final login = payload.substring(prefix.length).trim().toLowerCase();
  return RegExp(r'^[a-z0-9_]{1,25}$').hasMatch(login) ? login : null;
}
