// SPDX-License-Identifier: MPL-2.0
//
// Which network backend the app opens (DIST-032): NetworkManager on the
// Linux system bus, the made-up networks in demo mode, nothing on Android,
// iOS, macOS and Windows (their own system settings handle Wi-Fi).

import 'dart:async';
import 'dart:io' show Platform;

import '../fakes/fake_network.dart';
import 'network_backend.dart';
import 'network_controller.dart';
import 'nm_backend.dart';

/// The controller for Home's network icon; null where there is none. It
/// stays hidden until NetworkManager answers with a Wi-Fi adapter.
NetworkController? openNetwork({required bool demo, bool? linux}) {
  if (!(linux ?? Platform.isLinux)) return null;
  final NetworkBackend? backend = demo
      ? FakeNetworkBackend.demo()
      : NmBackend.system();
  if (backend == null) return null;
  final c = NetworkController(backend);
  unawaited(c.start());
  return c;
}
