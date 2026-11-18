// SPDX-License-Identifier: MPL-2.0
//
// NET-018 in the app: DLNA servers found on the network (SSDP) next to the
// mDNS shares, the DLNA and M3U playlist forms without a login, and the
// address rules of both kinds.

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/core_discovery.dart';
import 'package:kinolune/sources/discovery.dart';
import 'package:kinolune/sources/source_address.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_core/pa_core.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));

const _nas = DiscoveredSource(
  type: SourceType.dlna,
  name: 'Living-room NAS',
  host: '192.168.1.20',
  url: 'http://192.168.1.20:8200/rootDesc.xml',
  secure: false,
);

class _FakeCore implements DiscoveryCore {
  final _lists = StreamController<List<CoreDiscoveredService>>.broadcast();

  void emit(List<CoreDiscoveredService> list) => _lists.add(list);

  @override
  Stream<List<CoreDiscoveredService>> watch() => _lists.stream;

  @override
  Future<void> browse(List<CoreServiceKind> kinds) async {}

  @override
  Future<void> stop(List<CoreServiceKind> kinds) async {}

  @override
  Future<void> close() async {}
}

Future<FakeSourceManager> _open(
  WidgetTester tester, {
  SourceDiscovery discovery = const StaticSourceDiscovery([_nas]),
}) async {
  setSurface(tester, const Size(1280, 900));
  late FakeSourceManager manager;
  final services = await demoServices(
    discovery: discovery,
    sourceManager: (library) => manager = FakeSourceManager(library),
  );
  await tester.pumpWidget(testApp(services, section: Section.settings));
  await settle(tester);
  await tester.tap(find.text(en.addSourceTitle).first);
  await settle(tester, 4);
  return manager;
}

Future<void> _submit(WidgetTester tester) async {
  await tester.ensureVisible(find.byKey(const ValueKey('source-submit')));
  await tester.tap(find.byKey(const ValueKey('source-submit')));
  await settle(tester, 8);
}

void main() {
  setUpAll(loadAppFonts);

  test('DLNA servers and playlists are sources without a login', () {
    for (final t in [SourceType.dlna, SourceType.playlist]) {
      expect(SourceType.available, contains(t));
      expect(t.indexable, isTrue);
      expect(t.openSource, isTrue);
      expect(SourceType.fromCoreKind(t.coreKind), t);
    }
    expect(SourceType.dlna.coreKind, 'dlna');
    expect(SourceType.playlist.coreKind, 'playlist');
    expect(SourceType.webdav.openSource, isFalse);
  });

  test('DLNA and playlist addresses', () {
    expect(
      dlnaAddress('192.168.1.20:8200/rootDesc.xml').address,
      'http://192.168.1.20:8200/rootDesc.xml',
    );
    expect(dlnaAddress('').error, SourceFormError.urlEmpty);
    expect(
      dlnaAddress('http://u:p@nas/rootDesc.xml').error,
      SourceFormError.credentialsInAddress,
    );
    expect(
      playlistAddress('https://cdn.example/films.m3u8').address,
      'https://cdn.example/films.m3u8',
    );
    expect(
      playlistAddress('/home/media/films.m3u').address,
      '/home/media/films.m3u',
    );
    expect(
      playlistAddress(r'C:\Media\films.m3u').address,
      r'C:\Media\films.m3u',
    );
    expect(playlistAddress('films.m3u').error, SourceFormError.playlistAddress);
    expect(
      playlistAddress('ftp://h/films.m3u').error,
      SourceFormError.urlScheme,
    );
    expect(
      playlistDisplay('m3u+file:///home/media/films.m3u'),
      '/home/media/films.m3u',
    );
    expect(playlistDisplay('https://a/b.m3u'), 'https://a/b.m3u');
  });

  test('DLNA servers join the mDNS shares while the list is open', () async {
    final core = _FakeCore();
    var searches = 0;
    final discovery = CoreSourceDiscovery(
      core,
      kinds: const [CoreServiceKind.smb],
      lanSearch: () async {
        searches++;
        return [_nas];
      },
      searchInterval: const Duration(milliseconds: 50),
    );
    final seen = <List<DiscoveredSource>>[];
    final sub = discovery.watch().listen(seen.add);
    await Future<void>.delayed(const Duration(milliseconds: 10));
    core.emit([
      CoreDiscoveredService(
        kind: CoreServiceKind.smb,
        instance: 'Share',
        host: 'nas.local',
        port: 445,
        addresses: const ['192.168.1.20'],
        url: 'smb://192.168.1.20/',
      ),
    ]);
    await Future<void>.delayed(const Duration(milliseconds: 120));
    final last = seen.last;
    expect(last.map((d) => d.type), [SourceType.smb, SourceType.dlna]);
    expect(last.last.url, _nas.url);
    expect(searches, greaterThan(1), reason: 'searched again while open');
    await sub.cancel();
    final after = searches;
    await Future<void>.delayed(const Duration(milliseconds: 120));
    expect(searches, after, reason: 'no searches after the list closed');
  });

  testWidgets('a DLNA server found on the network is added in one step', (
    tester,
  ) async {
    final manager = await _open(tester);
    expect(find.text('Living-room NAS'), findsOneWidget);
    expect(find.text('${en.sourcesTypeDlna} · 192.168.1.20'), findsOneWidget);
    await tester.tap(find.text('Living-room NAS'));
    await settle(tester, 4);
    // No login fields: DLNA is open to the home network.
    expect(find.byKey(const ValueKey('source-user')), findsNothing);
    expect(find.byKey(const ValueKey('source-password')), findsNothing);
    expect(find.byKey(const ValueKey('source-sync')), findsNothing);
    expect(find.text(en.sourcesDlnaNote), findsOneWidget);
    await _submit(tester);
    final d = manager.drafts.single;
    expect(d.type, SourceType.dlna);
    expect(d.name, 'Living-room NAS');
    expect(d.address, 'http://192.168.1.20:8200/rootDesc.xml');
    expect(d.username, isNull);
    expect(d.password, isNull);
    expect(d.sync, SourceSyncMode.off);
  });

  testWidgets('an M3U playlist is added by link', (tester) async {
    final manager = await _open(tester);
    await tester.ensureVisible(find.text(en.sourcesTypePlaylist));
    await tester.tap(find.text(en.sourcesTypePlaylist));
    await settle(tester, 4);
    expect(find.text(en.sourcesPlaylistAddress), findsOneWidget);
    expect(find.byKey(const ValueKey('source-playlist-pick')), findsOneWidget);
    expect(find.byKey(const ValueKey('source-password')), findsNothing);

    await tester.enterText(
      find.byKey(const ValueKey('source-url')),
      'films.m3u',
    );
    await _submit(tester);
    expect(find.text(en.sourcesFormErrorPlaylist), findsOneWidget);
    expect(manager.drafts, isEmpty);

    await tester.enterText(
      find.byKey(const ValueKey('source-url')),
      'https://cdn.example/lists/films.m3u8',
    );
    await _submit(tester);
    final d = manager.drafts.single;
    expect(d.type, SourceType.playlist);
    expect(d.address, 'https://cdn.example/lists/films.m3u8');
    expect(d.name, 'films.m3u8');
    expect(d.password, isNull);
  });
}
