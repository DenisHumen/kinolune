// SPDX-License-Identifier: MPL-2.0
//
// SYNC-014 (ADR-0137): folder sync looks at the folder every 1.5 s while
// the app is in front, every 30 s in the background, and slower on mobile
// data. The app tells the core which of these it is; there is no setting.

import 'package:connectivity_plus/connectivity_plus.dart';
import 'package:flutter/widgets.dart';

import '../offline/network_watch.dart'
    show listenConnectivity, offlineNetworkOf;
import '../offline/offline_downloads.dart' show OfflineNetwork;
import 'device_sync.dart';

/// Whether the app counts as in front in [state]: a desktop window that
/// lost the focus (inactive) still shows the library.
bool syncForeground(AppLifecycleState? state) =>
    state == null ||
    state == AppLifecycleState.resumed ||
    state == AppLifecycleState.inactive;

/// Tells [sync] whether the app is in front and its network metered, now
/// and on every change.
void watchSyncPace(DeviceSync sync) {
  var foreground = syncForeground(WidgetsBinding.instance.lifecycleState);
  var metered = false;
  void tell() => sync.setAppState(foreground: foreground, metered: metered);
  AppLifecycleListener(
    onStateChange: (s) {
      final now = syncForeground(s);
      if (now == foreground) return;
      foreground = now;
      tell();
    },
  );
  void network(List<ConnectivityResult> r) {
    final now = offlineNetworkOf(r) == OfflineNetwork.metered;
    if (now == metered) return;
    metered = now;
    tell();
  }

  listenConnectivity(network, what: 'sync');
  tell();
}
