// SPDX-License-Identifier: MPL-2.0
//
// Made-up networks for the demo and the widget tests (DIST-032): a few
// Wi-Fi networks around, one with WPA2 whose password is "kinolune123",
// an open café network with a captive portal, a hidden one and a cable.
// Like NetworkManager, it keeps the passwords only inside itself.

import 'dart:async';

import '../network/network_backend.dart';

class FakeNetworkBackend implements NetworkBackend {
  FakeNetworkBackend({
    List<WifiNetwork>? networks,
    Map<String, String>? passwords,
    Set<String>? hidden,
    this.available = true,
    this.wired = const WiredStatus(),
    String? connectedTo,
    this.portal = const {},
  }) : _networks = [...?networks],
       _passwords = {...?passwords},
       _hidden = {...?hidden} {
    if (connectedTo != null) {
      _saved[connectedTo] = (autoconnect: true, password: null);
      _active = connectedTo;
    }
  }

  /// Home Wi-Fi joined, a neighbour's, a café with a portal, a hidden one.
  factory FakeNetworkBackend.demo({bool connected = true}) =>
      FakeNetworkBackend(
        networks: const [
          WifiNetwork(
            ssid: 'Home 5G',
            strength: 82,
            security: WifiSecurity.wpaPersonal,
            frequencyMhz: 5180,
          ),
          WifiNetwork(
            ssid: 'Neighbours',
            strength: 41,
            security: WifiSecurity.wpa3,
            frequencyMhz: 2437,
          ),
          WifiNetwork(
            ssid: 'Cafe Free WiFi',
            strength: 63,
            security: WifiSecurity.open,
            frequencyMhz: 2412,
          ),
          WifiNetwork(
            ssid: 'Office',
            strength: 24,
            security: WifiSecurity.enterprise,
            frequencyMhz: 5240,
          ),
        ],
        passwords: const {
          'Home 5G': 'kinolune123',
          'Neighbours': 'neighbours-only',
          'Attic': 'attic-secret',
        },
        hidden: const {'Attic'},
        portal: const {'Cafe Free WiFi'},
        connectedTo: connected ? 'Home 5G' : null,
        wired: const WiredStatus(present: true),
      );

  /// No NetworkManager or no adapter.
  bool available;
  WiredStatus wired;

  /// Networks that ask to sign in on a page first.
  final Set<String> portal;

  final List<WifiNetwork> _networks;
  final Map<String, String> _passwords;
  final Set<String> _hidden;
  final Map<String, ({bool autoconnect, String? password})> _saved = {};
  String? _active;
  bool _wifiOn = true;
  final _changes = StreamController<void>.broadcast();

  /// How many times [scan] ran.
  int scans = 0;

  /// The calls the panel made, for tests (no passwords in them).
  final List<String> log = [];

  /// Saved networks (as NetworkManager would list them).
  Set<String> get saved => _saved.keys.toSet();
  String? get activeSsid => _active;

  /// The cable is pulled and Wi-Fi drops: "no network".
  void dropAll() {
    _active = null;
    wired = WiredStatus(present: wired.present);
    _changes.add(null);
  }

  @override
  Stream<void> get changes => _changes.stream;

  NetConnectivity get _connectivity {
    if (_active == null && !wired.connected) return NetConnectivity.none;
    if (_active != null && portal.contains(_active)) {
      return NetConnectivity.portal;
    }
    return NetConnectivity.full;
  }

  @override
  Future<NetworkSnapshot> read() async {
    if (!available) return NetworkSnapshot.unavailable;
    final list = _wifiOn
        ? [
            for (final n in _networks)
              n.copyWith(
                active: n.ssid == _active,
                saved: _saved.containsKey(n.ssid),
                autoconnect: _saved[n.ssid]?.autoconnect ?? true,
              ),
            if (_active != null && _hidden.contains(_active))
              WifiNetwork(
                ssid: _active!,
                strength: 58,
                security: WifiSecurity.wpaPersonal,
                active: true,
                saved: true,
                autoconnect: _saved[_active]?.autoconnect ?? true,
              ),
          ]
        : <WifiNetwork>[];
    list.sort((a, b) {
      if (a.active != b.active) return a.active ? -1 : 1;
      if (a.saved != b.saved) return a.saved ? -1 : 1;
      return b.strength.compareTo(a.strength);
    });
    return NetworkSnapshot(
      available: true,
      wifiEnabled: _wifiOn,
      networks: list,
      wired: wired,
      wifiAddress: _active == null ? null : '192.168.1.42',
      connectivity: _connectivity,
      checkAvailable: true,
      portalUrl: 'http://portal.example/',
    );
  }

  @override
  Future<void> scan() async {
    scans++;
    log.add('scan');
  }

  @override
  Future<ConnectResult> connect(WifiNetwork network, {String? password}) async {
    log.add('connect ${network.ssid}');
    if (network.security == WifiSecurity.enterprise) {
      return ConnectResult.unsupported;
    }
    return _join(network.ssid, network.security, password);
  }

  @override
  Future<ConnectResult> connectHidden(
    String ssid, {
    required WifiSecurity security,
    String? password,
  }) async {
    log.add('hidden $ssid');
    if (!_hidden.contains(ssid)) return ConnectResult.notFound;
    return _join(ssid, security, password);
  }

  ConnectResult _join(String ssid, WifiSecurity security, String? password) {
    final known = _saved[ssid];
    final needs = security.needsPassword;
    final given = password ?? known?.password;
    if (needs && (given == null || given != _passwords[ssid])) {
      return ConnectResult.wrongPassword;
    }
    _saved[ssid] = (
      autoconnect: known?.autoconnect ?? true,
      password: needs ? given : null,
    );
    _active = ssid;
    _changes.add(null);
    return ConnectResult.ok;
  }

  @override
  Future<void> forget(String ssid) async {
    log.add('forget $ssid');
    _saved.remove(ssid);
    if (_active == ssid) _active = null;
    _changes.add(null);
  }

  @override
  Future<void> setAutoconnect(String ssid, bool on) async {
    log.add('autoconnect $ssid $on');
    final s = _saved[ssid];
    if (s != null) _saved[ssid] = (autoconnect: on, password: s.password);
  }

  @override
  Future<void> setWifiEnabled(bool on) async {
    log.add('wifi $on');
    _wifiOn = on;
    if (!on) _active = null;
  }

  @override
  Future<NetConnectivity> checkInternet() async {
    log.add('check');
    return _connectivity;
  }

  @override
  Future<void> close() => _changes.close();
}
