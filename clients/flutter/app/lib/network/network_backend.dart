// SPDX-License-Identifier: MPL-2.0
//
// Wi-Fi and network right from Home (DIST-032, ADR-NNNN). What the network
// icon and its panel see of the system's network: Wi-Fi networks around,
// the wired link, the address and the internet check. On Linux this is
// NetworkManager over D-Bus (nm_backend.dart); there is nothing elsewhere
// (Android, iOS, macOS, Windows keep their own system settings).
//
// Passwords pass through [NetworkBackend.connect] straight to the system;
// nothing here keeps, logs or syncs them.

import 'dart:async';

/// What the system's connectivity check says (NetworkManager's
/// `NMConnectivityState`).
enum NetConnectivity {
  unknown,

  /// No network at all.
  none,

  /// A captive portal: the network wants a sign-in page first.
  portal,

  /// A network but no internet.
  limited,

  /// Internet works.
  full,
}

/// How a Wi-Fi network is protected.
enum WifiSecurity {
  open,

  /// WEP: old; joined with a key like WPA.
  wep,

  /// WPA/WPA2 Personal (a password).
  wpaPersonal,

  /// WPA3 Personal (SAE, a password).
  wpa3,

  /// 802.1X (a company network): needs a user name and certificates, which
  /// the panel does not ask; the system settings do.
  enterprise;

  /// Asks a password to join.
  bool get needsPassword => this != open && this != enterprise;
}

/// A Wi-Fi network around (the strongest access point of an SSID).
class WifiNetwork {
  const WifiNetwork({
    required this.ssid,
    required this.strength,
    required this.security,
    this.active = false,
    this.saved = false,
    this.autoconnect = true,
    this.frequencyMhz,
    this.id,
  });

  final String ssid;

  /// Signal quality, 0–100.
  final int strength;
  final WifiSecurity security;

  /// This device is connected to it now.
  final bool active;

  /// The system remembers it (a saved profile): joins without a password.
  final bool saved;

  /// The saved profile joins it by itself.
  final bool autoconnect;
  final int? frequencyMhz;

  /// The backend's own handle (the access point's D-Bus path).
  final String? id;

  bool get is5GHz => (frequencyMhz ?? 0) >= 4900;

  WifiNetwork copyWith({
    int? strength,
    bool? active,
    bool? saved,
    bool? autoconnect,
  }) => WifiNetwork(
    ssid: ssid,
    strength: strength ?? this.strength,
    security: security,
    active: active ?? this.active,
    saved: saved ?? this.saved,
    autoconnect: autoconnect ?? this.autoconnect,
    frequencyMhz: frequencyMhz,
    id: id,
  );

  @override
  String toString() => 'WifiNetwork($ssid, $strength%, $security)';
}

/// The wired (Ethernet) link.
class WiredStatus {
  const WiredStatus({
    this.present = false,
    this.cable = false,
    this.connected = false,
    this.address,
  });

  /// The computer has an Ethernet port the system manages.
  final bool present;

  /// A cable is plugged in.
  final bool cable;

  /// The link is up with an address.
  final bool connected;

  /// Its IPv4 address.
  final String? address;
}

/// Everything the icon and the panel show, read at once.
class NetworkSnapshot {
  const NetworkSnapshot({
    required this.available,
    this.wifiEnabled = true,
    this.networks = const [],
    this.wired = const WiredStatus(),
    this.wifiAddress,
    this.connectivity = NetConnectivity.unknown,
    this.checkAvailable = false,
    this.portalUrl,
    this.connecting,
  });

  /// No NetworkManager or no Wi-Fi adapter: nothing is shown.
  static const unavailable = NetworkSnapshot(available: false);

  /// NetworkManager runs and the computer has a Wi-Fi adapter.
  final bool available;

  /// Wi-Fi is on (a radio switch can turn it off).
  final bool wifiEnabled;

  /// Networks around, strongest first; the connected one first of all.
  final List<WifiNetwork> networks;
  final WiredStatus wired;

  /// The Wi-Fi link's IPv4 address.
  final String? wifiAddress;
  final NetConnectivity connectivity;

  /// The system checks the internet by itself (a check URL is set).
  final bool checkAvailable;

  /// The address that opens a captive portal's sign-in page (the system's
  /// own check URL); null when not set.
  final String? portalUrl;

  /// The SSID the system joins right now, if any.
  final String? connecting;

  WifiNetwork? get active {
    for (final n in networks) {
      if (n.active) return n;
    }
    return null;
  }

  /// The first address to show: Wi-Fi, then the cable.
  String? get address => wifiAddress ?? wired.address;
}

/// How joining a network ended.
enum ConnectResult {
  ok,

  /// The network turned the password down.
  wrongPassword,

  /// The network is gone (out of range, or the hidden name is wrong).
  notFound,

  /// The panel cannot join it (802.1X): use the system settings.
  unsupported,

  /// The system refused (no permission) or anything else failed.
  failed,
}

/// The system's network as the app sees it.
abstract class NetworkBackend {
  /// Reads everything; [NetworkSnapshot.unavailable] when there is no
  /// NetworkManager or no Wi-Fi adapter. Never throws.
  Future<NetworkSnapshot> read();

  /// Something changed (a signal from the system); the caller re-reads.
  Stream<void> get changes;

  /// Asks the adapter to look for networks again (best effort).
  Future<void> scan();

  /// Joins [network]; [password] for a protected one that is not saved
  /// yet (or to replace a saved password). It goes only to the system.
  Future<ConnectResult> connect(WifiNetwork network, {String? password});

  /// Joins a hidden network by its name.
  Future<ConnectResult> connectHidden(
    String ssid, {
    required WifiSecurity security,
    String? password,
  });

  /// Removes the saved profiles of [ssid] (disconnects if connected).
  Future<void> forget(String ssid);

  /// Joins [ssid] by itself or not.
  Future<void> setAutoconnect(String ssid, bool on);

  /// Turns Wi-Fi on or off.
  Future<void> setWifiEnabled(bool on);

  /// Runs the system's internet check now.
  Future<NetConnectivity> checkInternet();

  Future<void> close();
}

/// No network management on this system.
class NoNetworkBackend implements NetworkBackend {
  const NoNetworkBackend();

  @override
  Future<NetworkSnapshot> read() async => NetworkSnapshot.unavailable;

  @override
  Stream<void> get changes => const Stream.empty();

  @override
  Future<void> scan() async {}

  @override
  Future<ConnectResult> connect(
    WifiNetwork network, {
    String? password,
  }) async => ConnectResult.failed;

  @override
  Future<ConnectResult> connectHidden(
    String ssid, {
    required WifiSecurity security,
    String? password,
  }) async => ConnectResult.failed;

  @override
  Future<void> forget(String ssid) async {}

  @override
  Future<void> setAutoconnect(String ssid, bool on) async {}

  @override
  Future<void> setWifiEnabled(bool on) async {}

  @override
  Future<NetConnectivity> checkInternet() async => NetConnectivity.unknown;

  @override
  Future<void> close() async {}
}
