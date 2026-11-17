// SPDX-License-Identifier: MPL-2.0
//
// LIB-024 (ADR-0097): the library looks at its sources by itself when the
// app comes back to the foreground and when the network returns, so new
// files show up and get recognised without "Scan now". The network comes
// from connectivity_plus (BSD-3-Clause), as for downloads (NET-019).

import 'dart:async';

import 'package:connectivity_plus/connectivity_plus.dart';
import 'package:flutter/widgets.dart';

import '../offline/network_watch.dart' show offlineNetworkOf;
import '../offline/offline_downloads.dart' show OfflineNetwork;

/// Asks the library for a check: sources not listed for `minAge` are
/// listed again; `networkBack` asks again for works whose fetch failed.
typedef LibraryCheck = Future<void> Function({
  Duration minAge,
  bool networkBack,
});

/// Turns app and network events into library checks.
class LibraryAutoCheck {
  LibraryAutoCheck(this._check);

  final LibraryCheck _check;

  /// A source listed this recently is not listed again on resume: desktop
  /// windows go inactive and back on every focus change.
  static const resumeMinAge = Duration(minutes: 2);

  bool? _online;

  /// The app is in the foreground again (resume, window shown).
  void resumed() => unawaited(_check(minAge: resumeMinAge));

  /// The platform reports the network: a check when it comes back after
  /// being offline (not on the first report, not between Wi-Fi and
  /// Ethernet).
  void network({required bool online}) {
    final was = _online;
    _online = online;
    if (online && was == false) {
      unawaited(_check(minAge: Duration.zero, networkBack: true));
    }
  }

  /// Follows the app's lifecycle (Android, Android TV, iOS, macOS and the
  /// other desktops through [AppLifecycleListener]) and the platform's
  /// network reports. Without the connectivity plugin (an unsupported
  /// desktop) only the lifecycle is followed.
  static LibraryAutoCheck attach(LibraryCheck check) {
    final c = LibraryAutoCheck(check);
    AppLifecycleListener(onResume: c.resumed, onShow: c.resumed);
    void report(List<ConnectivityResult> r) =>
        c.network(online: offlineNetworkOf(r) != OfflineNetwork.offline);
    try {
      final connectivity = Connectivity();
      unawaited(connectivity.checkConnectivity().then(report, onError: (_) {}));
      connectivity.onConnectivityChanged.listen(report, onError: (_) {});
    } catch (e) {
      debugPrint('Network state unavailable for library checks: $e');
    }
    return c;
  }
}
