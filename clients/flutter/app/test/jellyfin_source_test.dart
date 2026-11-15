// SPDX-License-Identifier: MPL-2.0
//
// NET-014 in the app: adding a Jellyfin server with a password or Quick
// Connect, addresses, and the playback reports of a player session.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/screens/jellyfin_source_form.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/jellyfin_link.dart';
import 'package:kinolune/sources/source_address.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));

/// A Jellyfin client that records the calls.
class FakeJellyfinLink implements JellyfinLink {
  final calls = <String>[];
  int pollsUntilApproved = 2;
  bool quickConnectOff = false;

  ManagedSource _added(String name) => ManagedSource(
    id: '42',
    type: SourceType.jellyfin,
    name: name,
    address: 'http://jf.test:8096',
    username: 'anna',
    hasPassword: true,
  );

  @override
  Future<JellyfinServerInfo> probe(String address) async =>
      const JellyfinServerInfo(name: 'Home', version: '10.10.7');

  @override
  Future<ManagedSource> signIn({
    required List<String> addresses,
    required String username,
    required String password,
    required String deviceName,
    String? name,
  }) async {
    calls.add('signIn ${addresses.join(' ')} $username ${password.length}');
    if (password != 'secret') {
      throw const SourceException(
        SourceErrorCode.authFailed,
        'jellyfin: authentication failed',
      );
    }
    return _added(name ?? 'Home');
  }

  @override
  Future<QuickConnectTicket> quickConnectStart({
    required List<String> addresses,
    required String deviceName,
  }) async {
    calls.add('qcStart ${addresses.join(' ')}');
    if (quickConnectOff) {
      throw const SourceException(SourceErrorCode.authFailed, 'off');
    }
    return QuickConnectTicket(
      handle: BigInt.one,
      code: '123456',
      serverName: 'Home',
    );
  }

  @override
  Future<bool> quickConnectPoll(BigInt handle) async {
    calls.add('qcPoll');
    return --pollsUntilApproved <= 0;
  }

  @override
  Future<ManagedSource> quickConnectFinish(
    BigInt handle, {
    String? name,
  }) async {
    calls.add('qcFinish');
    return _added(name ?? 'Home');
  }

  @override
  void quickConnectCancel(BigInt handle) => calls.add('qcCancel');

  @override
  Future<JellyfinPlay?> playback(
    String itemId, {
    int startMs = 0,
    bool forceTranscode = false,
  }) async => null;

  @override
  Future<void> report(
    String itemId,
    int positionMs, {
    bool paused = false,
  }) async => calls.add('report $itemId $positionMs');

  @override
  Future<void> stopped(String itemId, int positionMs) async =>
      calls.add('stopped $itemId $positionMs');

  @override
  Future<void> setFavorite(String itemId, bool favorite) async =>
      calls.add('favorite $itemId $favorite');

  @override
  Future<void> setWatched(String itemId, bool watched) async =>
      calls.add('watched $itemId $watched');

  @override
  Future<void> removeFromResume(String itemId) async =>
      calls.add('unresume $itemId');

  @override
  Future<int> importUserData(String sourceId) async {
    calls.add('import $sourceId');
    return 0;
  }
}

Future<AppServices> _openJellyfinForm(
  WidgetTester tester,
  FakeJellyfinLink link,
) async {
  setSurface(tester, const Size(1280, 900));
  final services = await demoServices(jellyfin: link);
  await tester.pumpWidget(testApp(services, section: Section.settings));
  await settle(tester);
  await tester.tap(find.text(en.addSourceTitle).first);
  await settle(tester, 4);
  await tester.tap(find.text(en.sourcesTypeJellyfin));
  await settle(tester, 4);
  return services;
}

Future<void> _enter(WidgetTester tester, String key, String text) async {
  await tester.ensureVisible(find.byKey(ValueKey(key)));
  await tester.enterText(find.byKey(ValueKey(key)), text);
  await tester.pump();
}

Future<void> _submit(WidgetTester tester) async {
  await tester.ensureVisible(find.byKey(const ValueKey('jellyfin-submit')));
  await tester.tap(find.byKey(const ValueKey('jellyfin-submit')));
  await settle(tester, 8);
}

void main() {
  setUpAll(loadAppFonts);

  group('addresses', () {
    test('scheme added, trailing slash dropped, bad input refused', () {
      expect(jellyfinAddress('jf.local:8096/').address, 'http://jf.local:8096');
      expect(
        jellyfinAddress(' https://media.example/jellyfin/ ').address,
        'https://media.example/jellyfin',
      );
      expect(jellyfinAddress('').error, SourceFormError.urlEmpty);
      expect(jellyfinAddress('ftp://x').error, SourceFormError.urlScheme);
      expect(
        jellyfinAddress('http://a:b@x').error,
        SourceFormError.credentialsInAddress,
      );
      expect(jellyfinAltAddresses('a:1\n\nhttps://b/\nftp://c'), [
        'http://a:1',
        'https://b',
      ]);
    });
  });

  group('add form', () {
    testWidgets('signs in with a password, both addresses go to the core', (
      tester,
    ) async {
      final link = FakeJellyfinLink();
      await _openJellyfinForm(tester, link);
      expect(find.text(en.sourcesJellyfinTokenNote), findsOneWidget);
      // No address: a field error, nothing sent.
      await _submit(tester);
      expect(link.calls, isEmpty);
      await _enter(tester, 'jellyfin-address', '192.168.1.10:8096');
      await _enter(
        tester,
        'jellyfin-alt-addresses',
        'https://media.example.org',
      );
      await _enter(tester, 'jellyfin-user', 'anna');
      await _enter(tester, 'jellyfin-password', 'wrong');
      await _submit(tester);
      expect(link.calls.last, startsWith('signIn http://192.168.1.10:8096 '));
      expect(link.calls.last, contains('https://media.example.org anna'));
      // A wrong password stays in the form with a readable error.
      expect(find.byType(JellyfinSourceForm), findsOneWidget);
      expect(find.text(en.sourcesErrorAuth), findsOneWidget);

      await _enter(tester, 'jellyfin-password', 'secret');
      await _submit(tester);
      expect(find.byType(JellyfinSourceForm), findsNothing);
      // The added source is scanned, then the server's state imported
      // (the fake manager has no such source, so nothing to import).
      expect(link.calls.where((c) => c.startsWith('signIn')).length, 2);
    });

    testWidgets('Quick Connect shows the code and finishes once approved', (
      tester,
    ) async {
      final link = FakeJellyfinLink();
      await _openJellyfinForm(tester, link);
      await _enter(tester, 'jellyfin-address', 'http://jf.test:8096');
      await tester.tap(find.text(en.sourcesJellyfinModeQuickConnect));
      await settle(tester, 2);
      expect(find.byKey(const ValueKey('jellyfin-password')), findsNothing);
      await _submit(tester);
      expect(find.text('123456'), findsOneWidget);
      expect(find.text(en.sourcesJellyfinQuickConnectWaiting), findsOneWidget);
      await tester.pump(const Duration(seconds: 2));
      await settle(tester, 2);
      expect(find.byType(JellyfinSourceForm), findsOneWidget);
      await tester.pump(const Duration(seconds: 2));
      await settle(tester, 8);
      expect(link.calls, [
        'qcStart http://jf.test:8096',
        'qcPoll',
        'qcPoll',
        'qcFinish',
      ]);
      expect(find.byType(JellyfinSourceForm), findsNothing);
    });

    testWidgets('Quick Connect switched off says so', (tester) async {
      final link = FakeJellyfinLink()..quickConnectOff = true;
      await _openJellyfinForm(tester, link);
      await _enter(tester, 'jellyfin-address', 'http://jf.test:8096');
      await tester.tap(find.text(en.sourcesJellyfinModeQuickConnect));
      await settle(tester, 2);
      await _submit(tester);
      expect(
        find.byKey(const ValueKey('jellyfin-quick-connect-off')),
        findsOneWidget,
      );
    });

    testWidgets('cancelling a pending code cancels it at the core', (
      tester,
    ) async {
      final link = FakeJellyfinLink()..pollsUntilApproved = 100;
      await _openJellyfinForm(tester, link);
      await _enter(tester, 'jellyfin-address', 'http://jf.test:8096');
      await tester.tap(find.text(en.sourcesJellyfinModeQuickConnect));
      await settle(tester, 2);
      await _submit(tester);
      await tester.tap(find.text(en.commonCancel).last);
      await settle(tester, 4);
      expect(link.calls.last, 'qcCancel');
    });
  });

  test('a player session reports start, progress, Up Next and stop', () async {
    final link = FakeJellyfinLink();
    final r = JellyfinReporter(link)
      ..report('e1', 0)
      ..report('e1', 5000)
      ..report('e2', 0)
      ..close();
    await Future<void>.delayed(Duration.zero);
    expect(link.calls, [
      'report e1 0',
      'report e1 5000',
      'stopped e1 5000',
      'report e2 0',
      'stopped e2 0',
    ]);
    // Closing twice or without a link does nothing.
    r.close();
    JellyfinReporter(null)
      ..report('x', 1)
      ..close();
    expect(link.calls.length, 5);
  });
}
