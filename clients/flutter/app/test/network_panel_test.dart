// SPDX-License-Identifier: MPL-2.0
//
// DIST-032: the network icon on Home and its panel on a computer, a phone
// and a TV (focus with the arrow keys), in Russian, English and Ukrainian:
// the list with signal and lock, a password (wrong, then right), an open
// café with a captive portal, a hidden network, "Forget", auto-connect,
// "Проверить интернет", "no network"; no icon without NetworkManager or
// without a Wi-Fi adapter.
//
// Screenshots:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/dist-032 \
//     flutter test test/network_panel_test.dart

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_network.dart';
import 'package:kinolune/network/network_backend.dart';
import 'package:kinolune/network/network_controller.dart';
import 'package:kinolune/network/network_setup.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

NetworkController _controller(FakeNetworkBackend b) => NetworkController(
  b,
  poll: Duration.zero,
  panelPoll: Duration.zero,
  scanSettle: Duration.zero,
);

Finder _key(String k) => find.byKey(ValueKey(k));

Future<NetworkController> _pump(
  WidgetTester tester,
  FakeNetworkBackend backend, {
  FormFactor formFactor = FormFactor.desktop,
  Size size = const Size(1440, 900),
  String lang = 'ru',
}) async {
  setSurface(tester, size);
  final net = _controller(backend);
  await net.start();
  final services = await demoServices(network: net);
  services.locale.value = Locale(lang);
  await tester.pumpWidget(testApp(services, formFactor: formFactor));
  await settle(tester);
  return net;
}

Future<void> _open(WidgetTester tester) async {
  await tester.tap(_key('network-icon'));
  await settle(tester);
  expect(_key('network-panel'), findsOneWidget);
}

Future<void> _done(WidgetTester tester, NetworkController net) async {
  await tester.pumpWidget(const SizedBox());
  net.dispose();
  await settle(tester, 4);
}

void main() {
  setUpAll(loadAppFonts);

  testWidgets('desktop: icon on Home, list, wrong then right password', (
    tester,
  ) async {
    final backend = FakeNetworkBackend.demo();
    final net = await _pump(tester, backend);
    expect(_key('network-icon'), findsOneWidget);
    expect(
      find.byKey(ValueKey('network-icon-${Icons.wifi_rounded.codePoint}')),
      findsOneWidget,
    );
    await capture(tester, 'desktop-home-icon-ru');

    await _open(tester);
    expect(backend.scans, 1, reason: 'opening the panel looks for networks');
    expect(find.text('Wi-Fi: Home 5G'), findsWidgets);
    expect(find.text('IP-адрес: 192.168.1.42'), findsOneWidget);
    for (final ssid in ['Home 5G', 'Cafe Free WiFi', 'Neighbours', 'Office']) {
      expect(_key('network-ap-$ssid'), findsOneWidget);
    }
    expect(find.byIcon(Icons.lock_rounded), findsNWidgets(3));
    await capture(tester, 'desktop-panel-ru');

    // A protected network: the password dialog, a wrong one first.
    await tester.tap(_key('network-ap-Neighbours'));
    await settle(tester);
    expect(find.text('Пароль для «Neighbours»'), findsOneWidget);
    await tester.enterText(_key('network-password'), 'guess-1234');
    await tester.tap(_key('network-password-connect'));
    await settle(tester);
    expect(find.text('Неверный пароль. Попробуйте ещё раз.'), findsOneWidget);
    await capture(tester, 'desktop-wrong-password-ru');
    await tester.enterText(_key('network-password'), 'neighbours-only');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await settle(tester);
    expect(find.text('Подключено к «Neighbours»'), findsOneWidget);
    expect(backend.activeSsid, 'Neighbours');
    expect(backend.saved, containsAll(['Home 5G', 'Neighbours']));

    // The panel never keeps the password; the log has none either.
    expect(backend.log.join(' '), isNot(contains('neighbours-only')));
    await _done(tester, net);
  });

  testWidgets('desktop: café with a captive portal and the internet check', (
    tester,
  ) async {
    final backend = FakeNetworkBackend.demo(connected: false);
    final net = await _pump(tester, backend, lang: 'en');
    expect(
      find.byKey(ValueKey('network-icon-${Icons.wifi_off_rounded.codePoint}')),
      findsOneWidget,
      reason: '"No network" on the icon',
    );
    await _open(tester);
    expect(find.text('No network'), findsWidgets);
    await tester.tap(_key('network-ap-Cafe Free WiFi'));
    await settle(tester);
    expect(find.text('Connected to “Cafe Free WiFi”'), findsOneWidget);
    expect(_key('network-portal'), findsOneWidget);
    await tester.tap(_key('network-check'));
    await settle(tester);
    expect(
      find.text('The network asks you to sign in on its page'),
      findsWidgets,
    );
    await capture(tester, 'desktop-portal-en');
    await tester.tap(_key('network-close'));
    await settle(tester);
    expect(
      find.byKey(
        ValueKey('network-icon-${Icons.wifi_password_rounded.codePoint}'),
      ),
      findsOneWidget,
    );
    await _done(tester, net);
  });

  testWidgets('desktop: hidden network, auto-connect, forget, no network', (
    tester,
  ) async {
    final backend = FakeNetworkBackend.demo();
    final net = await _pump(tester, backend, lang: 'uk');
    await _open(tester);

    await tester.tap(_key('network-hidden'));
    await settle(tester);
    await tester.enterText(_key('network-hidden-ssid'), 'Attic');
    await tester.enterText(_key('network-hidden-password'), 'attic-secret');
    await tester.tap(_key('network-hidden-connect'));
    await settle(tester);
    expect(find.text('Підключено до «Attic»'), findsOneWidget);
    expect(backend.activeSsid, 'Attic');

    // Saved Home 5G: switch auto-connect off, then forget it.
    await tester.tap(_key('network-details-Home 5G'));
    await settle(tester);
    await tester.tap(_key('network-autoconnect'));
    await settle(tester);
    expect(backend.log, contains('autoconnect Home 5G false'));
    await capture(tester, 'desktop-details-uk');
    await tester.tap(_key('network-forget'));
    await settle(tester);
    expect(find.text('Мережу «Home 5G» забуто'), findsOneWidget);
    expect(backend.saved, isNot(contains('Home 5G')));

    // Everything drops: "no network".
    backend.dropAll();
    await net.refresh();
    await settle(tester);
    expect(find.text('Немає мережі'), findsWidgets);
    await tester.tap(_key('network-check'));
    await settle(tester);
    expect(find.text('Немає підключення до мережі'), findsOneWidget);
    await _done(tester, net);
  });

  testWidgets('phone: the panel is a sheet', (tester) async {
    final backend = FakeNetworkBackend.demo();
    final net = await _pump(
      tester,
      backend,
      formFactor: FormFactor.phone,
      size: const Size(390, 844),
    );
    await _open(tester);
    expect(find.byType(BottomSheet), findsOneWidget);
    expect(_key('network-ap-Home 5G'), findsOneWidget);
    await capture(tester, 'phone-panel-ru');
    await tester.tap(_key('network-ap-Cafe Free WiFi'));
    await settle(tester);
    expect(backend.activeSsid, 'Cafe Free WiFi');
    await _done(tester, net);
  });

  testWidgets('TV: the arrow keys reach the icon and the panel', (
    tester,
  ) async {
    final backend = FakeNetworkBackend.demo();
    final net = await _pump(
      tester,
      backend,
      formFactor: FormFactor.tv,
      size: const Size(1920, 1080),
    );
    bool focusedIn(String key) {
      final ctx = FocusManager.instance.primaryFocus?.context;
      if (ctx == null) return false;
      var found = false;
      ctx.visitAncestorElements((e) {
        if (e.widget.key == ValueKey(key)) found = true;
        return !found;
      });
      return found;
    }

    for (var i = 0; i < 14 && !focusedIn('network-icon'); i++) {
      await pressKey(tester, LogicalKeyboardKey.arrowRight);
    }
    expect(focusedIn('network-icon'), isTrue);
    await pressKey(tester, LogicalKeyboardKey.enter);
    expect(_key('network-panel'), findsOneWidget);
    await pressKey(tester, LogicalKeyboardKey.arrowDown, times: 2);
    expect(focusedIn('network-panel'), isTrue);
    await capture(tester, 'tv-panel-ru');
    await pressKey(tester, LogicalKeyboardKey.escape);
    expect(_key('network-panel'), findsNothing);
    await _done(tester, net);
  });

  testWidgets('no icon without a Wi-Fi adapter or NetworkManager', (
    tester,
  ) async {
    final backend = FakeNetworkBackend.demo()..available = false;
    final net = await _pump(tester, backend);
    expect(_key('network-icon'), findsNothing);
    await _done(tester, net);

    // No controller at all (Android, iOS, macOS, Windows).
    setSurface(tester, const Size(1440, 900));
    await tester.pumpWidget(testApp(await demoServices()));
    await settle(tester);
    expect(_key('network-icon'), findsNothing);
  });

  test('openNetwork: Linux only', () async {
    expect(openNetwork(demo: true, linux: false), isNull);
    final c = openNetwork(demo: true, linux: true)!;
    await c.refresh();
    expect(c.visible, isTrue);
    expect(c.snapshot.connectivity, NetConnectivity.full);
    c.dispose();
  });
}
