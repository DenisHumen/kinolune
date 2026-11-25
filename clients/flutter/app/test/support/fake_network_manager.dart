// SPDX-License-Identifier: MPL-2.0
//
// A fake NetworkManager on a private D-Bus (DIST-032): our own DBusServer on
// a Unix socket in a temp folder, with the objects and methods NmBackend
// uses — the root object, a Wi-Fi and an Ethernet device, access points,
// IP4Config, Settings and Settings.Connection, active connections. Like
// the real one it keeps passwords to itself (GetSettings returns none) and
// refuses a wrong WPA password with NO_SECRETS.

import 'dart:io';

import 'package:dbus/dbus.dart';

const _nm = 'org.freedesktop.NetworkManager';

class FakeAp {
  FakeAp(
    this.ssid, {
    this.strength = 70,
    this.password,
    this.wpa3 = false,
    this.enterprise = false,
    this.hidden = false,
    this.frequency = 2437,
    this.portal = false,
  });

  final String ssid;
  int strength;

  /// The WPA password; null: an open network.
  final String? password;
  final bool wpa3;
  final bool enterprise;

  /// Not in the scan list; joins only by name with `hidden: true`.
  final bool hidden;
  final int frequency;

  /// A captive portal: connectivity is PORTAL while joined.
  final bool portal;
  late String path;
}

class _Saved {
  _Saved(this.path, this.settings);

  final String path;
  Map<String, Map<String, DBusValue>> settings;
  String get ssid =>
      String.fromCharCodes(settings['802-11-wireless']!['ssid']!.asByteArray());
}

class FakeNetworkManager {
  FakeNetworkManager._(this._server, this._dir, this.address);

  final DBusServer _server;
  final Directory _dir;
  final DBusAddress address;
  late final DBusClient _bus;

  final List<FakeAp> aps = [];
  bool wifiAdapter = true;
  bool wiredAdapter = true;
  bool carrier = true;
  bool wiredUp = false;
  bool wirelessEnabled = true;
  int scans = 0;

  /// Method calls in order (`Interface.Method`), for assertions.
  final List<String> calls = [];

  /// Every secret NetworkManager was given (the app keeps none).
  final List<String> secretsReceived = [];

  final Map<String, _Saved> _saved = {};
  FakeAp? _active;
  String? _activePath;
  int _wifiState = 30; // disconnected
  (int, int) _reason = (30, 0);
  int _next = 1;
  final Map<String, DBusObject> _objects = {};

  static Future<FakeNetworkManager> start() async {
    final dir = await Directory.systemTemp.createTemp('kinolune-nm-');
    final server = DBusServer();
    final address = await server.listenAddress(DBusAddress.unix(dir: dir));
    final nm = FakeNetworkManager._(server, dir, address);
    nm._bus = DBusClient(address);
    await nm._bus.requestName(_nm);
    await nm._register(_Obj('/org/freedesktop/NetworkManager', nm._root));
    await nm._register(
      _Obj('/org/freedesktop/NetworkManager/Settings', nm._settings),
    );
    await nm._register(
      _Obj('/org/freedesktop/NetworkManager/Devices/1', nm._wifiDev),
    );
    await nm._register(
      _Obj('/org/freedesktop/NetworkManager/Devices/2', nm._ethDev),
    );
    await nm._register(
      _Obj('/org/freedesktop/NetworkManager/IP4Config/1', nm._ip4('192.168.50.23')),
    );
    await nm._register(
      _Obj('/org/freedesktop/NetworkManager/IP4Config/2', nm._ip4('10.0.0.5')),
    );
    return nm;
  }

  /// A client of the fake bus for the code under test.
  DBusClient client() => DBusClient(address);

  Future<void> addAp(FakeAp ap) async {
    ap.path = '/org/freedesktop/NetworkManager/AccessPoint/${_next++}';
    aps.add(ap);
    await _register(_Obj(ap.path, _apHandler(ap)));
  }

  /// The SSIDs NetworkManager keeps profiles for.
  Set<String> get savedSsids => {for (final s in _saved.values) s.ssid};
  String? get activeSsid => _active?.ssid;

  /// The access point goes away (out of range): the link drops.
  void dropWifi() {
    _active = null;
    _wifiState = 30;
    _activePath = null;
  }

  Future<void> close() async {
    await _bus.close();
    await _server.close();
    await _dir.delete(recursive: true);
  }

  Future<void> _register(DBusObject o) async {
    _objects[o.path.value] = o;
    await _bus.registerObject(o);
  }

  Future<void> _unregister(String path) async {
    final o = _objects.remove(path);
    if (o != null) await _bus.unregisterObject(o);
  }

  // ----------------------------------------------------------- the model

  int get _connectivity {
    if (_active == null && !wiredUp) return 1;
    if (_active?.portal ?? false) return 2;
    return 4;
  }

  _Handler get _root => _Handler(
    props: (i) => i == _nm
        ? {
            'Devices': DBusArray.objectPath([
              if (wifiAdapter)
                DBusObjectPath('/org/freedesktop/NetworkManager/Devices/1'),
              if (wiredAdapter)
                DBusObjectPath('/org/freedesktop/NetworkManager/Devices/2'),
            ]),
            'WirelessEnabled': DBusBoolean(wirelessEnabled),
            'Connectivity': DBusUint32(_connectivity),
            'ConnectivityCheckAvailable': const DBusBoolean(true),
            'ConnectivityCheckUri': const DBusString(
              'http://check.example/hotspot.txt',
            ),
          }
        : null,
    set: (i, name, v) {
      if (name != 'WirelessEnabled') return false;
      wirelessEnabled = v.asBoolean();
      if (!wirelessEnabled) dropWifi();
      return true;
    },
    methods: (call) async {
      calls.add('NM.${call.name}');
      switch (call.name) {
        case 'CheckConnectivity':
          return DBusMethodSuccessResponse([DBusUint32(_connectivity)]);
        case 'ActivateConnection':
          final conn = call.values[0].asObjectPath().value;
          final saved = _saved.values.where((s) => s.path == conn).firstOrNull;
          if (saved == null) {
            return DBusMethodErrorResponse('$_nm.UnknownConnection');
          }
          return DBusMethodSuccessResponse([
            DBusObjectPath(await _activate(saved)),
          ]);
        case 'AddAndActivateConnection':
          final settings = _settingsOf(call.values[0]);
          final psk = settings['802-11-wireless-security']?['psk'];
          if (psk != null) {
            secretsReceived.add(psk.asString());
            final len = psk.asString().length;
            if (len < 8 || len > 63) {
              return DBusMethodErrorResponse(
                '$_nm.Settings.Connection.InvalidProperty',
                [const DBusString('802-11-wireless-security.psk: invalid')],
              );
            }
          }
          final s = await _addProfile(settings);
          return DBusMethodSuccessResponse([
            DBusObjectPath(s.path),
            DBusObjectPath(await _activate(s)),
          ]);
      }
      return DBusMethodErrorResponse.unknownMethod();
    },
  );

  Future<_Saved> _addProfile(Map<String, Map<String, DBusValue>> settings) async {
    final path = '/org/freedesktop/NetworkManager/Settings/${_next++}';
    final s = _Saved(path, settings);
    _saved[path] = s;
    await _register(_Obj(path, _connHandler(s)));
    return s;
  }

  /// Joins [s]'s network now (the fake has no delays) and returns the
  /// active connection.
  Future<String> _activate(_Saved s) async {
    final path = '/org/freedesktop/NetworkManager/ActiveConnection/${_next++}';
    final ssid = s.ssid;
    final hidden = _bool(s.settings['802-11-wireless']?['hidden']);
    final ap = aps
        .where((a) => a.ssid == ssid && (!a.hidden || hidden))
        .firstOrNull;
    var state = 2;
    if (ap == null) {
      state = 4;
      _reason = (30, 53); // SSID_NOT_FOUND
    } else {
      final psk = s.settings['802-11-wireless-security']?['psk']?.asString();
      if (ap.password != null && psk != ap.password) {
        state = 4;
        _reason = (30, 7); // NO_SECRETS
      }
    }
    if (state == 2) {
      _active = ap;
      _activePath = path;
      _wifiState = 100;
    } else {
      dropWifi();
    }
    final id = s.settings['connection']?['id']?.asString() ?? ssid;
    await _register(
      _Obj(
        path,
        _Handler(
          props: (i) => i == '$_nm.Connection.Active'
              ? {'State': DBusUint32(state), 'Id': DBusString(id)}
              : null,
        ),
      ),
    );
    return path;
  }

  _Handler get _settings => _Handler(
    methods: (call) async {
      calls.add('Settings.${call.name}');
      if (call.name == 'ListConnections') {
        return DBusMethodSuccessResponse([
          DBusArray.objectPath([
            for (final p in _saved.keys) DBusObjectPath(p),
          ]),
        ]);
      }
      return DBusMethodErrorResponse.unknownMethod();
    },
  );

  _Handler _connHandler(_Saved s) => _Handler(
    methods: (call) async {
      calls.add('Connection.${call.name}');
      switch (call.name) {
        case 'GetSettings':
          // Like NetworkManager: no secrets here.
          final copy = {
            for (final e in s.settings.entries)
              e.key: {
                for (final f in e.value.entries)
                  if (f.key != 'psk' && f.key != 'wep-key0') f.key: f.value,
              },
          };
          return DBusMethodSuccessResponse([_toDBus(copy)]);
        case 'Update':
          final next = _settingsOf(call.values[0]);
          final psk = next['802-11-wireless-security']?['psk'];
          if (psk != null) {
            secretsReceived.add(psk.asString());
          } else if (s.settings['802-11-wireless-security']?['psk']
              case final old?) {
            // Keeps the stored secret when the update has none.
            (next['802-11-wireless-security'] ??= {})['psk'] = old;
          }
          s.settings = next;
          return DBusMethodSuccessResponse();
        case 'Delete':
          _saved.remove(s.path);
          if (_active?.ssid == s.ssid) dropWifi();
          await _unregister(s.path);
          return DBusMethodSuccessResponse();
      }
      return DBusMethodErrorResponse.unknownMethod();
    },
  );

  _Handler get _wifiDev => _Handler(
    props: (i) => switch (i) {
      '$_nm.Device' => {
        'DeviceType': const DBusUint32(2),
        'Interface': const DBusString('wlan0'),
        'Managed': const DBusBoolean(true),
        'State': DBusUint32(_wifiState),
        'StateReason': DBusStruct([DBusUint32(_reason.$1), DBusUint32(_reason.$2)]),
        'Ip4Config': DBusObjectPath(
          _wifiState == 100 ? '/org/freedesktop/NetworkManager/IP4Config/1' : '/',
        ),
        'ActiveConnection': DBusObjectPath(_activePath ?? '/'),
      },
      '$_nm.Device.Wireless' => {
        'AccessPoints': DBusArray.objectPath([
          for (final a in aps)
            if (!a.hidden) DBusObjectPath(a.path),
        ]),
        'ActiveAccessPoint': DBusObjectPath(_active?.path ?? '/'),
      },
      _ => null,
    },
    methods: (call) async {
      calls.add('Wireless.${call.name}');
      if (call.name == 'RequestScan') {
        scans++;
        return DBusMethodSuccessResponse();
      }
      return DBusMethodErrorResponse.unknownMethod();
    },
  );

  _Handler get _ethDev => _Handler(
    props: (i) => switch (i) {
      '$_nm.Device' => {
        'DeviceType': const DBusUint32(1),
        'Interface': const DBusString('enp1s0'),
        'Managed': const DBusBoolean(true),
        'State': DBusUint32(wiredUp ? 100 : (carrier ? 30 : 20)),
        'StateReason': DBusStruct([const DBusUint32(30), const DBusUint32(0)]),
        'Ip4Config': DBusObjectPath(
          wiredUp ? '/org/freedesktop/NetworkManager/IP4Config/2' : '/',
        ),
        'ActiveConnection': DBusObjectPath('/'),
      },
      '$_nm.Device.Wired' => {'Carrier': DBusBoolean(carrier)},
      _ => null,
    },
  );

  _Handler _apHandler(FakeAp a) => _Handler(
    props: (i) => i == '$_nm.AccessPoint'
        ? {
            'Ssid': DBusArray.byte(a.ssid.codeUnits),
            'Strength': DBusByte(a.strength),
            'Flags': DBusUint32(a.password != null || a.enterprise ? 1 : 0),
            'WpaFlags': const DBusUint32(0),
            'RsnFlags': DBusUint32(
              a.enterprise
                  ? 0x200 | 0x8
                  : a.password == null
                  ? 0
                  : (a.wpa3 ? 0x400 | 0x8 : 0x100 | 0x8),
            ),
            'Frequency': DBusUint32(a.frequency),
          }
        : null,
  );

  _Handler _ip4(String address) => _Handler(
    props: (i) => i == '$_nm.IP4Config'
        ? {
            'AddressData': DBusArray(DBusSignature('a{sv}'), [
              DBusDict.stringVariant({
                'address': DBusString(address),
                'prefix': const DBusUint32(24),
              }),
            ]),
          }
        : null,
  );
}

bool _bool(DBusValue? v) => v is DBusBoolean && v.value;

Map<String, Map<String, DBusValue>> _settingsOf(DBusValue v) => {
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

class _Handler {
  _Handler({this.props, this.methods, this.set});

  final Map<String, DBusValue>? Function(String interface)? props;
  final Future<DBusMethodResponse> Function(DBusMethodCall call)? methods;
  final bool Function(String interface, String name, DBusValue value)? set;
}

class _Obj extends DBusObject {
  _Obj(String path, this.h) : super(DBusObjectPath(path));

  final _Handler h;

  @override
  Future<DBusMethodResponse> handleMethodCall(DBusMethodCall call) async =>
      h.methods?.call(call) ?? DBusMethodErrorResponse.unknownMethod();

  @override
  Future<DBusMethodResponse> getProperty(String interface, String name) async {
    final v = h.props?.call(interface)?[name];
    return v == null
        ? DBusMethodErrorResponse.unknownProperty()
        : DBusGetPropertyResponse(v);
  }

  @override
  Future<DBusMethodResponse> getAllProperties(String interface) async {
    final all = h.props?.call(interface);
    return all == null
        ? DBusMethodErrorResponse.unknownInterface()
        : DBusGetAllPropertiesResponse(all);
  }

  @override
  Future<DBusMethodResponse> setProperty(
    String interface,
    String name,
    DBusValue value,
  ) async => (h.set?.call(interface, name, value) ?? false)
      ? DBusMethodSuccessResponse()
      : DBusMethodErrorResponse.propertyReadOnly();
}
