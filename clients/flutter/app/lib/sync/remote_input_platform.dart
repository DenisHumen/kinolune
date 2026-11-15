// SPDX-License-Identifier: MPL-2.0
//
// The phone's "Type from your phone" notification (UI-027) through
// flutter_local_notifications (BSD-3-Clause), the plugin TWI-009 already
// uses. Android and iOS only: only phones answer input requests.
//
// It is shown only while the app process lives: in the background until
// the system stops it (Android), or for the short time iOS keeps a
// backgrounded app running. Tapping it opens the app, which then opens the
// input sheet for the request that is still open.

import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';

import '../l10n/l10n.dart';
import 'remote_input.dart';

/// Android notification channel of remote input.
const kRemoteInputChannelId = 'remote_input';

/// The one notification id of remote input (one request at a time).
const kRemoteInputNotificationId = 0x027027;

/// The notifier of this platform; null where phones do not run the app.
RemoteInputNotifier? platformRemoteInputNotifier(
  AppLocalizations Function() l10n,
) {
  if (kIsWeb || !(Platform.isAndroid || Platform.isIOS)) return null;
  return PluginRemoteInputNotifier(l10n: l10n);
}

class PluginRemoteInputNotifier implements RemoteInputNotifier {
  PluginRemoteInputNotifier({required this.l10n});

  final AppLocalizations Function() l10n;
  final _plugin = FlutterLocalNotificationsPlugin();
  bool _initialized = false;
  bool _asked = false;

  Future<bool> _init() async {
    if (_initialized) return true;
    try {
      await _plugin.initialize(
        settings: const InitializationSettings(
          android: AndroidInitializationSettings('@mipmap/ic_launcher'),
          // Asked when the user is looking (ensurePermission), not here.
          iOS: DarwinInitializationSettings(
            requestAlertPermission: false,
            requestBadgePermission: false,
            requestSoundPermission: false,
          ),
        ),
      );
      return _initialized = true;
    } catch (e) {
      debugPrint('Remote input notifications unavailable: $e');
      return false;
    }
  }

  @override
  Future<void> ensurePermission() async {
    if (_asked || !await _init()) return;
    _asked = true;
    try {
      if (Platform.isAndroid) {
        await _plugin
            .resolvePlatformSpecificImplementation<
              AndroidFlutterLocalNotificationsPlugin
            >()
            ?.requestNotificationsPermission();
      } else if (Platform.isIOS) {
        await _plugin
            .resolvePlatformSpecificImplementation<
              IOSFlutterLocalNotificationsPlugin
            >()
            ?.requestPermissions(alert: true, sound: true);
      }
    } catch (e) {
      debugPrint('Remote input notification permission: $e');
    }
  }

  @override
  Future<void> show({required String title, required String body}) async {
    if (!await _init()) return;
    try {
      await _plugin.show(
        id: kRemoteInputNotificationId,
        title: title,
        body: body,
        notificationDetails: NotificationDetails(
          android: AndroidNotificationDetails(
            kRemoteInputChannelId,
            l10n().remoteInputNotificationChannel,
            importance: Importance.high,
            priority: Priority.high,
            category: AndroidNotificationCategory.message,
            // Gone after 5 minutes: the request is closed by then.
            timeoutAfter: 5 * 60 * 1000,
          ),
          iOS: const DarwinNotificationDetails(
            interruptionLevel: InterruptionLevel.timeSensitive,
          ),
        ),
      );
    } catch (e) {
      debugPrint('Remote input notification not shown: $e');
    }
  }

  @override
  Future<void> cancel() async {
    if (!_initialized) return;
    try {
      await _plugin.cancel(id: kRemoteInputNotificationId);
    } catch (_) {}
  }
}
