// SPDX-License-Identifier: MPL-2.0
//
// The device's network for the job queue (NET-019): "Wi-Fi only"
// downloads wait on mobile data and every download waits offline. The
// platform reports the connection through connectivity_plus (BSD-3-Clause).

import 'dart:async';
import 'dart:io' show Platform;

import 'package:connectivity_plus/connectivity_plus.dart';
import 'package:flutter/foundation.dart';
import 'package:path_provider/path_provider.dart';

import 'offline_downloads.dart';

/// The queue's network for a set of connections: Wi-Fi or Ethernet is
/// unmetered, mobile data alone is metered, nothing is offline. A VPN or
/// another kind counts as unmetered (desktops, emulators).
OfflineNetwork offlineNetworkOf(List<ConnectivityResult> results) {
  if (results.isEmpty || results.every((r) => r == ConnectivityResult.none)) {
    return OfflineNetwork.offline;
  }
  if (results.any(
    (r) =>
        r == ConnectivityResult.wifi ||
        r == ConnectivityResult.ethernet ||
        r == ConnectivityResult.vpn ||
        r == ConnectivityResult.other,
  )) {
    return OfflineNetwork.unmetered;
  }
  if (results.contains(ConnectivityResult.mobile)) {
    return OfflineNetwork.metered;
  }
  return OfflineNetwork.unmetered;
}

/// Reports the network to [offline] now and on every change. Without the
/// plugin (an unsupported desktop) the queue keeps its default, online.
void watchNetwork(OfflineController offline) {
  Future<void> report(List<ConnectivityResult> r) async {
    try {
      await offline.setNetwork(offlineNetworkOf(r));
    } catch (e) {
      debugPrint('Network not reported: $e');
    }
  }

  try {
    final connectivity = Connectivity();
    unawaited(connectivity.checkConnectivity().then(report, onError: (_) {}));
    connectivity.onConnectivityChanged.listen(report, onError: (_) {});
  } catch (e) {
    debugPrint('Network state unavailable: $e');
  }
}

/// The default download folder: the app's external files on Android (no
/// permission, usually the larger volume), else `<data>/downloads/offline`.
Future<String> offlineFolder(String dataDir) async {
  if (Platform.isAndroid) {
    try {
      final external = await getExternalStorageDirectory();
      if (external != null) return '${external.path}/downloads/offline';
    } catch (e) {
      debugPrint('External storage unavailable: $e');
    }
  }
  return '$dataDir/downloads/offline';
}
