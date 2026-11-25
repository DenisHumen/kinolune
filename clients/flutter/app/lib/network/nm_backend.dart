// SPDX-License-Identifier: MPL-2.0
//
// NetworkManager over D-Bus (DIST-032, ADR-NNNN): the system bus on Linux,
// with no root. Kinolune OS lets its user manage connections through a
// polkit rule (distro/rootfs/usr/share/polkit-1/rules.d); on other
// desktops NetworkManager's own rules allow the active local session.
//
// The API used (NetworkManager ≥ 1.20): the root object (devices,
// connectivity, Wi-Fi switch), Device / Device.Wireless / Device.Wired,
// AccessPoint, IP4Config, Settings and Settings.Connection. A password is
// sent once, in AddAndActivateConnection or Update, and stays with
// NetworkManager; GetSettings never returns it.

import 'dart:async';
import 'dart:convert';

import 'package:dbus/dbus.dart';

import 'network_backend.dart';

const _nm = 'org.freedesktop.NetworkManager';
const _nmPath = '/org/freedesktop/NetworkManager';
const _settingsPath = '/org/freedesktop/NetworkManager/Settings';
const _device = '$_nm.Device';
const _wireless = '$_nm.Device.Wireless';
const _wired = '$_nm.Device.Wired';
const _ap = '$_nm.AccessPoint';
const _ip4 = '$_nm.IP4Config';
const _settings = '$_nm.Settings';
const _connection = '$_nm.Settings.Connection';
const _activeConn = '$_nm.Connection.Active';

// NMDeviceType
const _typeEthernet = 1;
const _typeWifi = 2;

// NMDeviceState
const _stateActivated = 100;
const _statePrepare = 40;

// NMActiveConnectionState
const _acActivated = 2;
const _acDeactivated = 4;

// NM80211ApFlags / NM80211ApSecurityFlags
const _apPrivacy = 0x1;
const _keyPsk = 0x100;
const _key8021x = 0x200;
const _keySae = 0x400;

// NMDeviceStateReason: the network turned the secret down.
const _reasonNoSecrets = 7;
const _reasonSupplicantDisconnect = 8;
const _reasonSsidNotFound = 53;

/// NetworkManager on [client] (the system bus by default).
class NmBackend implements NetworkBackend {
  NmBackend(this._client, {Duration? connectTimeout})
    : _connectTimeout = connectTimeout ?? const Duration(seconds: 45);

  /// The system bus; null where there is none (not Linux).
  static NmBackend? system() {
    try {
      return NmBackend(DBusClient.system());
    } catch (_) {
      return null;
    }
  }

  final DBusClient _client;
  final Duration _connectTimeout;
  String? _connecting;

  DBusRemoteObject _obj(String path) =>
      DBusRemoteObject(_client, name: _nm, path: DBusObjectPath(path));

  DBusRemoteObject get _root => _obj(_nmPath);

  @override
  Stream<void> get changes => DBusSignalStream(
    _client,
    sender: _nm,
    interface: 'org.freedesktop.DBus.Properties',
    name: 'PropertiesChanged',
    pathNamespace: DBusObjectPath(_nmPath),
  ).map((_) {});

  // ------------------------------------------------------------------ read

  @override
  Future<NetworkSnapshot> read() async {
    try {
      return await _read();
    } on DBusMethodResponseException {
      return NetworkSnapshot.unavailable;
    } on DBusClosedException {
      return NetworkSnapshot.unavailable;
    } catch (_) {
      // No system bus (a container, another OS): nothing to show.
      return NetworkSnapshot.unavailable;
    }
  }

  Future<NetworkSnapshot> _read() async {
    final root = await _root.getAllProperties(_nm);
    final devices = await _devices(root);
    final wifi = devices.where((d) => d.type == _typeWifi).firstOrNull;
    if (wifi == null) return NetworkSnapshot.unavailable;
    final wiredDev = devices.where((d) => d.type == _typeEthernet).firstOrNull;

    final saved = await _savedWifi();
    final wifiProps = await _obj(wifi.path).getAllProperties(_wireless);
    final activeAp = _path(wifiProps['ActiveAccessPoint']);
    final wifiOn = _bool(root['WirelessEnabled']) ?? true;
    final networks = <String, WifiNetwork>{};
    final aps = wifiOn
        ? (wifiProps['AccessPoints']?.asObjectPathArray() ??
              const <DBusObjectPath>[])
        : const <DBusObjectPath>[];
    for (final apPath in aps) {
      final Map<String, DBusValue> ap;
      try {
        ap = await _obj(apPath.value).getAllProperties(_ap);
      } on DBusMethodResponseException {
        continue; // gone between the list and the read
      }
      final ssid = _ssid(ap['Ssid']);
      if (ssid == null || ssid.isEmpty) continue; // hidden
      final isActive =
          apPath.value == activeAp && wifi.state == _stateActivated;
      final known = saved[ssid];
      final n = WifiNetwork(
        ssid: ssid,
        strength: (ap['Strength']?.asByte() ?? 0).clamp(0, 100),
        security: _security(
          _u32(ap['Flags']) ?? 0,
          _u32(ap['WpaFlags']) ?? 0,
          _u32(ap['RsnFlags']) ?? 0,
        ),
        active: isActive,
        saved: known != null,
        autoconnect: known?.autoconnect ?? true,
        frequencyMhz: _u32(ap['Frequency']),
        id: apPath.value,
      );
      final prev = networks[ssid];
      if (prev == null ||
          n.active ||
          (!prev.active && n.strength > prev.strength)) {
        networks[ssid] = n;
      }
    }
    final list = networks.values.toList()
      ..sort((a, b) {
        if (a.active != b.active) return a.active ? -1 : 1;
        if (a.saved != b.saved) return a.saved ? -1 : 1;
        return b.strength.compareTo(a.strength);
      });

    final busy = wifi.state >= _statePrepare && wifi.state < _stateActivated;
    String? connecting = busy ? _connecting : null;
    if (busy && connecting == null && wifi.activeConnection != null) {
      connecting = await _activeId(wifi.activeConnection!);
    }

    WiredStatus wired = const WiredStatus();
    if (wiredDev != null) {
      bool cable = false;
      try {
        cable =
            _bool(await _obj(wiredDev.path).getProperty(_wired, 'Carrier')) ??
            false;
      } on DBusMethodResponseException {
        cable = false;
      }
      final up = wiredDev.state == _stateActivated;
      wired = WiredStatus(
        present: true,
        cable: cable,
        connected: up,
        address: up ? await _address(wiredDev.ip4) : null,
      );
    }

    final check = _bool(root['ConnectivityCheckAvailable']) ?? false;
    final uri = root['ConnectivityCheckUri']?.asString();
    return NetworkSnapshot(
      available: true,
      wifiEnabled: wifiOn,
      networks: list,
      wired: wired,
      wifiAddress: wifi.state == _stateActivated
          ? await _address(wifi.ip4)
          : null,
      connectivity: _connectivity(_u32(root['Connectivity'])),
      checkAvailable: check,
      portalUrl: (uri == null || uri.isEmpty) ? null : uri,
      connecting: connecting,
    );
  }

  Future<List<_Dev>> _devices(Map<String, DBusValue> root) async {
    final paths =
        root['Devices']?.asObjectPathArray() ?? const <DBusObjectPath>[];
    final out = <_Dev>[];
    for (final p in paths) {
      try {
        final props = await _obj(p.value).getAllProperties(_device);
        if (_bool(props['Managed']) == false) continue;
        final type = _u32(props['DeviceType']) ?? 0;
        if (type != _typeWifi && type != _typeEthernet) continue;
        out.add(
          _Dev(
            path: p.value,
            type: type,
            state: _u32(props['State']) ?? 0,
            ip4: _path(props['Ip4Config']),
            activeConnection: _path(props['ActiveConnection']),
          ),
        );
      } on DBusMethodResponseException {
        continue;
      }
    }
    return out;
  }

  Future<String?> _activeId(String path) async {
    try {
      return (await _obj(path).getProperty(_activeConn, 'Id')).asString();
    } on DBusMethodResponseException {
      return null;
    }
  }

  Future<String?> _address(String? ip4) async {
    if (ip4 == null) return null;
    try {
      final data = await _obj(ip4).getProperty(_ip4, 'AddressData');
      for (final a in data.asArray()) {
        final m = a.asStringVariantDict();
        final addr = m['address']?.asString();
        if (addr != null && addr.isNotEmpty) return addr;
      }
    } on DBusMethodResponseException {
      return null;
    }
    return null;
  }

  /// Saved Wi-Fi profiles by SSID.
  Future<Map<String, _Saved>> _savedWifi() async {
    final out = <String, _Saved>{};
    for (final s in await _savedProfiles()) {
      out.putIfAbsent(s.ssid, () => s);
    }
    return out;
  }

  Future<List<_Saved>> _savedProfiles() async {
    final DBusMethodSuccessResponse list;
    try {
      list = await _obj(_settingsPath)
          .callMethod(_settings, 'ListConnections', const []);
    } on DBusMethodResponseException {
      return const [];
    }
    final out = <_Saved>[];
    for (final p in list.returnValues.first.asObjectPathArray()) {
      final settings = await _getSettings(p.value);
      if (settings == null) continue;
      final conn = settings['connection'];
      if (conn?['type']?.asString() != '802-11-wireless') continue;
      final ssid = _ssid(settings['802-11-wireless']?['ssid']);
      if (ssid == null) continue;
      out.add(
        _Saved(
          path: p.value,
          ssid: ssid,
          autoconnect: _bool(conn?['autoconnect']) ?? true,
        ),
      );
    }
    return out;
  }

  Future<Map<String, Map<String, DBusValue>>?> _getSettings(String p) async {
    try {
      final r = await _obj(p).callMethod(_connection, 'GetSettings', const []);
      return _fromDBus(r.returnValues.first);
    } on DBusMethodResponseException {
      return null;
    }
  }

  // --------------------------------------------------------------- actions

  @override
  Future<void> scan() async {
    final wifi = await _wifiDevice();
    if (wifi == null) return;
    try {
      await _obj(wifi).callMethod(_wireless, 'RequestScan', [
        DBusDict.stringVariant(const {}),
      ]);
    } on DBusMethodResponseException {
      // Scanning too often, or while joining: the list stays as it is.
    }
  }

  Future<String?> _wifiDevice() async {
    try {
      final root = await _root.getAllProperties(_nm);
      final devices = await _devices(root);
      return devices.where((d) => d.type == _typeWifi).firstOrNull?.path;
    } on DBusMethodResponseException {
      return null;
    }
  }

  @override
  Future<ConnectResult> connect(WifiNetwork network, {String? password}) async {
    if (network.security == WifiSecurity.enterprise) {
      return ConnectResult.unsupported;
    }
    final dev = await _wifiDevice();
    if (dev == null) return ConnectResult.failed;
    final ap = network.id ?? '/';
    final saved = (await _savedProfiles())
        .where((s) => s.ssid == network.ssid)
        .firstOrNull;
    _connecting = network.ssid;
    try {
      if (saved != null) {
        if (password != null && network.security.needsPassword) {
          final s = await _getSettings(saved.path);
          if (s == null) return ConnectResult.failed;
          s['802-11-wireless-security'] = _security80211(
            network.security,
            password,
          );
          await _obj(saved.path)
              .callMethod(_connection, 'Update', [_toDBus(s)]);
        }
        final r = await _root.callMethod(_nm, 'ActivateConnection', [
          DBusObjectPath(saved.path),
          DBusObjectPath(dev),
          DBusObjectPath(ap),
        ]);
        return await _wait(
          r.returnValues.first.asObjectPath().value,
          dev,
          password: password != null,
        );
      }
      if (network.security.needsPassword && password == null) {
        return ConnectResult.wrongPassword;
      }
      return await _addAndActivate(
        _wifiSettings(network.ssid, network.security, password),
        dev,
        ap,
      );
    } on DBusMethodResponseException catch (e) {
      return _errorResult(e);
    } finally {
      _connecting = null;
    }
  }

  @override
  Future<ConnectResult> connectHidden(
    String ssid, {
    required WifiSecurity security,
    String? password,
  }) async {
    if (security == WifiSecurity.enterprise) return ConnectResult.unsupported;
    final dev = await _wifiDevice();
    if (dev == null) return ConnectResult.failed;
    _connecting = ssid;
    try {
      return await _addAndActivate(
        _wifiSettings(ssid, security, password, hidden: true),
        dev,
        '/',
      );
    } on DBusMethodResponseException catch (e) {
      return _errorResult(e);
    } finally {
      _connecting = null;
    }
  }

  ConnectResult _errorResult(DBusMethodResponseException e) {
    // A WPA password shorter than 8 or longer than 63 characters.
    if (e.errorName.endsWith('InvalidProperty') &&
        e.toString().contains('802-11-wireless-security')) {
      return ConnectResult.wrongPassword;
    }
    if (e.errorName.endsWith('UnknownConnection') ||
        e.errorName.endsWith('UnknownDevice')) {
      return ConnectResult.notFound;
    }
    return ConnectResult.failed;
  }

  /// A new profile: on failure it is removed again, so a wrong password
  /// is not kept.
  Future<ConnectResult> _addAndActivate(
    Map<String, Map<String, DBusValue>> settings,
    String dev,
    String ap,
  ) async {
    final r = await _root.callMethod(_nm, 'AddAndActivateConnection', [
      _toDBus(settings),
      DBusObjectPath(dev),
      DBusObjectPath(ap),
    ]);
    final profile = r.returnValues[0].asObjectPath().value;
    final active = r.returnValues[1].asObjectPath().value;
    final hasPassword = settings.containsKey('802-11-wireless-security');
    final result = await _wait(active, dev, password: hasPassword);
    if (result != ConnectResult.ok) {
      try {
        await _obj(profile).callMethod(_connection, 'Delete', const []);
      } on DBusMethodResponseException {
        // Already gone.
      }
    }
    return result;
  }

  /// Waits until the activation [active] is up or down.
  Future<ConnectResult> _wait(
    String active,
    String dev, {
    required bool password,
  }) async {
    final end = DateTime.now().add(_connectTimeout);
    while (DateTime.now().isBefore(end)) {
      int? state;
      try {
        state = _u32(await _obj(active).getProperty(_activeConn, 'State'));
      } on DBusMethodResponseException {
        state = _acDeactivated; // the activation object is gone: it failed
      }
      if (state == _acActivated) return ConnectResult.ok;
      if (state == _acDeactivated) return _failure(dev, password: password);
      await Future<void>.delayed(const Duration(milliseconds: 250));
    }
    return ConnectResult.failed;
  }

  Future<ConnectResult> _failure(String dev, {required bool password}) async {
    try {
      final r = (await _obj(
        dev,
      ).getProperty(_device, 'StateReason')).asStruct();
      final reason = _u32(r[1]) ?? 0;
      if (reason == _reasonSsidNotFound) return ConnectResult.notFound;
      if (password &&
          (reason == _reasonNoSecrets ||
              reason == _reasonSupplicantDisconnect)) {
        return ConnectResult.wrongPassword;
      }
    } on DBusMethodResponseException {
      // No reason to tell.
    }
    return ConnectResult.failed;
  }

  @override
  Future<void> forget(String ssid) async {
    for (final s in await _savedProfiles()) {
      if (s.ssid != ssid) continue;
      try {
        await _obj(s.path).callMethod(_connection, 'Delete', const []);
      } on DBusMethodResponseException {
        // Already gone.
      }
    }
  }

  @override
  Future<void> setAutoconnect(String ssid, bool on) async {
    for (final s in await _savedProfiles()) {
      if (s.ssid != ssid) continue;
      final settings = await _getSettings(s.path);
      if (settings == null) continue;
      (settings['connection'] ??= {})['autoconnect'] = DBusBoolean(on);
      // Update keeps the stored secrets when the new settings have none.
      await _obj(s.path).callMethod(_connection, 'Update', [_toDBus(settings)]);
    }
  }

  @override
  Future<void> setWifiEnabled(bool on) =>
      _root.setProperty(_nm, 'WirelessEnabled', DBusBoolean(on));

  @override
  Future<NetConnectivity> checkInternet() async {
    try {
      final r = await _root.callMethod(_nm, 'CheckConnectivity', const []);
      return _connectivity(_u32(r.returnValues.first));
    } on DBusMethodResponseException {
      return NetConnectivity.unknown;
    }
  }

  @override
  Future<void> close() => _client.close();
}

// ------------------------------------------------------------------ helpers

class _Dev {
  const _Dev({
    required this.path,
    required this.type,
    required this.state,
    this.ip4,
    this.activeConnection,
  });

  final String path;
  final int type;
  final int state;
  final String? ip4;
  final String? activeConnection;
}

class _Saved {
  const _Saved({
    required this.path,
    required this.ssid,
    required this.autoconnect,
  });

  final String path;
  final String ssid;
  final bool autoconnect;
}

/// Settings for a new Wi-Fi profile, as NetworkManager's `nmcli dev wifi
/// connect` makes them.
Map<String, Map<String, DBusValue>> _wifiSettings(
  String ssid,
  WifiSecurity security,
  String? password, {
  bool hidden = false,
}) => {
  'connection': {
    'id': DBusString(ssid),
    'type': const DBusString('802-11-wireless'),
    'autoconnect': const DBusBoolean(true),
  },
  '802-11-wireless': {
    'ssid': DBusArray.byte(_utf8(ssid)),
    'mode': const DBusString('infrastructure'),
    if (hidden) 'hidden': const DBusBoolean(true),
  },
  if (security != WifiSecurity.open && password != null)
    '802-11-wireless-security': _security80211(security, password),
};

Map<String, DBusValue> _security80211(WifiSecurity s, String password) =>
    switch (s) {
      WifiSecurity.wep => {
        'key-mgmt': const DBusString('none'),
        'wep-key0': DBusString(password),
        // 1: a hex or ASCII key, 2: a passphrase.
        'wep-key-type': DBusUint32(
          password.length == 5 ||
                  password.length == 13 ||
                  password.length == 10 ||
                  password.length == 26
              ? 1
              : 2,
        ),
      },
      WifiSecurity.wpa3 => {
        'key-mgmt': const DBusString('sae'),
        'psk': DBusString(password),
      },
      _ => {
        'key-mgmt': const DBusString('wpa-psk'),
        'psk': DBusString(password),
      },
    };

WifiSecurity _security(int flags, int wpa, int rsn) {
  final all = wpa | rsn;
  if (all & _key8021x != 0) return WifiSecurity.enterprise;
  if (all & _keyPsk != 0) return WifiSecurity.wpaPersonal;
  if (all & _keySae != 0) return WifiSecurity.wpa3;
  if (all != 0) return WifiSecurity.wpaPersonal;
  if (flags & _apPrivacy != 0) return WifiSecurity.wep;
  return WifiSecurity.open;
}

NetConnectivity _connectivity(int? v) => switch (v) {
  1 => NetConnectivity.none,
  2 => NetConnectivity.portal,
  3 => NetConnectivity.limited,
  4 => NetConnectivity.full,
  _ => NetConnectivity.unknown,
};

int? _u32(DBusValue? v) => v is DBusUint32 ? v.value : null;
bool? _bool(DBusValue? v) => v is DBusBoolean ? v.value : null;

String? _path(DBusValue? v) =>
    v is DBusObjectPath && v.value != '/' ? v.value : null;

String? _ssid(DBusValue? v) {
  if (v is! DBusArray) return null;
  final bytes = v.asByteArray().toList();
  try {
    return const Utf8Codec().decode(bytes);
  } on FormatException {
    return String.fromCharCodes(bytes);
  }
}

List<int> _utf8(String s) => const Utf8Codec().encode(s);

Map<String, Map<String, DBusValue>> _fromDBus(DBusValue v) => {
  for (final e in v.asDict().entries)
    e.key.asString(): {
      for (final f in e.value.asDict().entries)
        f.key.asString(): f.value.asVariant(),
    },
};

DBusDict _toDBus(Map<String, Map<String, DBusValue>> s) =>
    DBusDict(DBusSignature('s'), DBusSignature('a{sv}'), {
      for (final e in s.entries)
        DBusString(e.key): DBusDict.stringVariant(e.value),
    });
