// SPDX-License-Identifier: MPL-2.0
//
// DIST-032: the app's NetworkManager client (lib/network/nm_backend.dart)
// as a command-line tool for the live check with virtual Wi-Fi
// (distro/tests/wifi-hwsim.sh). Pure Dart, no Flutter:
//
//   dart compile exe tool/nm_probe.dart -o nm_probe
//   nm_probe status | scan | check
//   nm_probe connect SSID [PASSWORD]
//   nm_probe hidden SSID open|wpa|wpa3 [PASSWORD]
//   nm_probe forget SSID
//   nm_probe autoconnect SSID on|off
//   nm_probe wifi on|off
//
// Prints one JSON object; the exit code is 0 unless the call itself broke.
// Passwords go to NetworkManager only and are never printed.

import 'dart:convert';
import 'dart:io';

import 'package:kinolune/network/network_backend.dart';
import 'package:kinolune/network/nm_backend.dart';

Future<void> main(List<String> args) async {
  if (args.isEmpty) {
    stderr.writeln('usage: nm_probe status|scan|check|connect|hidden|forget|'
        'autoconnect|wifi …');
    exit(64);
  }
  final nm = NmBackend.system();
  if (nm == null) {
    stdout.writeln(jsonEncode({'available': false}));
    return;
  }
  try {
    final out = await _run(nm, args);
    stdout.writeln(jsonEncode(out));
  } finally {
    await nm.close();
  }
}

Future<Map<String, Object?>> _run(NmBackend nm, List<String> a) async {
  switch (a.first) {
    case 'status':
      return _status(await nm.read());
    case 'scan':
      await nm.scan();
      return {'ok': true};
    case 'check':
      return {'connectivity': (await nm.checkInternet()).name};
    case 'connect':
      final s = await nm.read();
      final n = s.networks.where((n) => n.ssid == a[1]).firstOrNull;
      if (n == null) return {'result': ConnectResult.notFound.name};
      final r = await nm.connect(n, password: a.length > 2 ? a[2] : null);
      return {'result': r.name};
    case 'hidden':
      final sec = switch (a[2]) {
        'open' => WifiSecurity.open,
        'wpa3' => WifiSecurity.wpa3,
        _ => WifiSecurity.wpaPersonal,
      };
      final r = await nm.connectHidden(
        a[1],
        security: sec,
        password: a.length > 3 ? a[3] : null,
      );
      return {'result': r.name};
    case 'forget':
      await nm.forget(a[1]);
      return {'ok': true};
    case 'autoconnect':
      await nm.setAutoconnect(a[1], a[2] == 'on');
      return {'ok': true};
    case 'wifi':
      await nm.setWifiEnabled(a[1] == 'on');
      return {'ok': true};
  }
  return {'error': 'unknown command ${a.first}'};
}

Map<String, Object?> _status(NetworkSnapshot s) => {
  'available': s.available,
  'wifiEnabled': s.wifiEnabled,
  'active': s.active?.ssid,
  'address': s.address,
  'connectivity': s.connectivity.name,
  'wired': {
    'present': s.wired.present,
    'cable': s.wired.cable,
    'connected': s.wired.connected,
  },
  'networks': [
    for (final n in s.networks)
      {
        'ssid': n.ssid,
        'strength': n.strength,
        'security': n.security.name,
        'saved': n.saved,
        'active': n.active,
        'autoconnect': n.autoconnect,
      },
  ],
};
