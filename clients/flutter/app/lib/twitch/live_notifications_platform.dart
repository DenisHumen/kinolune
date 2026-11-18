// SPDX-License-Identifier: MPL-2.0
//
// Platform side of "X is live" (TWI-009): local notifications through
// flutter_local_notifications (BSD-3-Clause) and, on Android, a periodic
// WorkManager check (workmanager, MIT) for when the app process is gone.
//
// Android's limits, honestly: WorkManager runs periodic work at most every
// 15 minutes, and Doze, App Standby buckets and battery saver delay it
// further (hours for a rarely used app). Only a foreground service with a
// permanent notification would do better; we do not run one. So on Android
// a notification arrives within ~2 minutes while the app process lives
// (also in the background until Android stops it), and otherwise with the
// next WorkManager run.

import 'dart:convert';
import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:pa_core/pa_core.dart' as core;
import 'package:pa_platform_secrets/pa_platform_secrets.dart';
import 'package:path_provider/path_provider.dart';
import 'package:workmanager/workmanager.dart';

import '../config.dart';
import '../integration/library.dart' show RustLibrary, coreDbFileName;
import '../l10n/l10n.dart';
import '../services/notification_taps.dart';
import '../services/preferences.dart';
import 'core_twitch_backend.dart';
import 'live_notifications.dart';
import 'twitch_models.dart';

/// Android notification channel of "X is live".
const kLiveChannelId = 'twitch_live';

/// Unique name of the WorkManager job.
const kLiveCheckTask = 'app.kinolune.twitch.live_check';

/// WorkManager period (Android's minimum).
const kLiveCheckPeriod = Duration(minutes: 15);

/// A background check announces broadcasts started at most this long ago:
/// the period plus what Doze usually adds.
const kLiveCheckMaxAge = Duration(minutes: 45);

/// Notification id of a broadcast: stable, 31-bit.
int liveNotificationId(String streamId) {
  var h = 0x811c9dc5;
  for (final c in utf8.encode(streamId)) {
    h = ((h ^ c) * 0x01000193) & 0x7fffffff;
  }
  return h;
}

/// Notifications through flutter_local_notifications: Android, Linux
/// (freedesktop notifications over D-Bus) and macOS. Windows needs an app
/// GUID registration that the app does not have yet: no notifications
/// there.
class PluginLiveNotifier implements LiveNotifier {
  PluginLiveNotifier({required this.l10n});

  /// Strings in the current UI language (channel name, "Open").
  final AppLocalizations Function() l10n;

  final _plugin = FlutterLocalNotificationsPlugin();
  bool? _ready;

  @override
  Future<bool> init() async {
    final ready = _ready;
    if (ready != null) return ready;
    if (kIsWeb || Platform.isWindows) return _ready = false;
    try {
      final ok = await _plugin.initialize(
        settings: InitializationSettings(
          android: const AndroidInitializationSettings('@mipmap/ic_launcher'),
          linux: LinuxInitializationSettings(
            defaultActionName: l10n().twitchLiveNotificationOpen,
          ),
          macOS: const DarwinInitializationSettings(),
          iOS: const DarwinInitializationSettings(),
        ),
        // QA-015: a tap opens the channel (the payload below).
        onDidReceiveNotificationResponse: _onTap,
      );
      // The app was started by a tap on one of ours.
      final launch = await _plugin.getNotificationAppLaunchDetails();
      if (launch?.didNotificationLaunchApp ?? false) {
        NotificationTaps.instance.handle(launch?.notificationResponse?.payload);
      }
      var permitted = ok ?? false;
      if (Platform.isAndroid) {
        final android = _plugin
            .resolvePlatformSpecificImplementation<
              AndroidFlutterLocalNotificationsPlugin
            >();
        permitted =
            await android?.requestNotificationsPermission() ?? permitted;
      }
      return _ready = permitted;
    } catch (e) {
      debugPrint('Live notifications unavailable: $e');
      return _ready = false;
    }
  }

  @override
  Future<void> show(
    LiveAlertInfo alert, {
    required String title,
    String? body,
  }) async {
    if (!await init()) return;
    try {
      await _plugin.show(
        id: liveNotificationId(alert.streamId),
        title: title,
        body: body,
        notificationDetails: NotificationDetails(
          android: AndroidNotificationDetails(
            kLiveChannelId,
            l10n().twitchLiveNotificationChannel,
            importance: Importance.high,
            priority: Priority.high,
            category: AndroidNotificationCategory.recommendation,
            when: alert.startedAt.millisecondsSinceEpoch,
            showWhen: true,
          ),
          linux: const LinuxNotificationDetails(),
          macOS: const DarwinNotificationDetails(),
        ),
        payload: 'twitch/channel/${alert.login}',
      );
    } catch (e) {
      debugPrint('Live notification not shown: $e');
    }
  }
}

/// A tap on any of the app's notifications (the plugin keeps one callback).
void _onTap(NotificationResponse r) =>
    NotificationTaps.instance.handle(r.payload);

/// Turns the Android background check on or off (no-op elsewhere).
Future<void> scheduleBackgroundLiveChecks({required bool on}) async {
  if (kIsWeb || !Platform.isAndroid) return;
  try {
    final wm = Workmanager();
    await wm.initialize(liveCheckDispatcher);
    if (!on) {
      await wm.cancelByUniqueName(kLiveCheckTask);
      return;
    }
    await wm.registerPeriodicTask(
      kLiveCheckTask,
      kLiveCheckTask,
      frequency: kLiveCheckPeriod,
      constraints: Constraints(
        networkType: NetworkType.connected,
        requiresBatteryNotLow: true,
      ),
      existingWorkPolicy: ExistingPeriodicWorkPolicy.keep,
    );
  } catch (e) {
    debugPrint('Background live checks not scheduled: $e');
  }
}

/// WorkManager's entry point in a background isolate.
@pragma('vm:entry-point')
void liveCheckDispatcher() {
  Workmanager().executeTask((task, input) => runBackgroundLiveCheck());
}

/// One background check: favourites with a bell, one `GET /streams`
/// through the core, notifications for new broadcasts. Returns true when
/// done (also when there is nothing to do), false to let WorkManager retry.
Future<bool> runBackgroundLiveCheck() async {
  WidgetsFlutterBinding.ensureInitialized();
  if (AppConfig.twitchClientId.isEmpty) return true;
  try {
    final dir = await getApplicationSupportDirectory();
    final prefs = FilePreferences('${dir.path}/preferences.json');
    if (await prefs.read(Preferences.twitchLiveNotifications) == 'false') {
      return true;
    }
    final raw = await prefs.read(Preferences.twitchFavorites);
    final favorites = raw == null
        ? const <TwitchFavorite>[]
        : [
            for (final j in jsonDecode(raw) as List)
              ?TwitchFavorite.fromJson(j),
          ];
    final channels = [
      for (final f in favorites)
        if (f.userId case final id?)
          LiveWatch(
            userId: id,
            login: f.login,
            displayName: f.displayName ?? f.login,
            notify: f.notify,
          ),
    ];
    if (!channels.any((c) => c.notify)) return true;
    await RustLibrary.ensureInitialized();
    // The token is in the Keystore (CORE-015), as in the app.
    await openPlatformSecrets(dataDir: dir.path) ??
        await core.secretsOpenDefault(dataDir: dir.path);
    final backend = await CoreTwitchBackend.open(
      dbPath: '${dir.path}/$coreDbFileName',
      dataDir: dir.path,
      clientId: AppConfig.twitchClientId,
    );
    if (backend.account == null) return true;
    final alerts = await backend.checkLiveOnce(
      channels,
      maxAge: kLiveCheckMaxAge,
    );
    final locale =
        parseLocaleTag(await prefs.read(Preferences.locale)) ??
        const Locale('en');
    final l10n = lookupAppLocalizations(locale);
    final notifier = PluginLiveNotifier(l10n: () => l10n);
    for (final a in alerts) {
      final t = liveTexts(l10n, a);
      await notifier.show(a, title: t.title, body: t.body);
    }
    return true;
  } catch (e) {
    debugPrint('Background live check failed: $e');
    return false;
  }
}
