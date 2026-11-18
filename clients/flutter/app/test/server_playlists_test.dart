// SPDX-License-Identifier: MPL-2.0
//
// MUS-012: music of a Navidrome server in the music views — its playlist
// (mirrored by the scan) is marked with the server, opens with the
// server's overline, is not offered in "Add to playlist", and downloads
// for offline with the album like any network tracks (NET-019). In
// Russian with real fonts; with KINOLUNE_SCREENSHOTS_DIR set the frames go
// to docs/screenshots/progress/mus-012:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/mus-012 \
//     flutter test test/server_playlists_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_offline.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/offline/offline_downloads.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/music/music_pages.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));
final _now = DateTime(2026, 11, 18, 20);

LibraryItem _track(String id, int n, String title) => LibraryItem(
  id: id,
  kind: ItemKind.track,
  title: title,
  year: 2024,
  artist: 'Neon Rivers',
  album: 'Night Drive',
  trackNumber: n,
  durationMs: 200000 + n * 7000,
  filePath: 'subsonic://nas:4533/Neon Rivers/Night Drive/0$n - $title.flac',
  streamUri:
      'http://127.0.0.1:1/t/3/Neon%20Rivers/Night%20Drive/0$n%20-%20$title.flac',
  addedAt: _now,
);

final _tracks = [
  _track('t1', 1, 'Coastline'),
  _track('t2', 2, 'Tail Lights'),
  _track('t3', 3, 'Midnight Exit'),
];

Future<(AppServices, FakeLibrary, FakeOfflineBackend)> _services() async {
  final lib = FakeLibrary.items(_tracks);
  await lib.createMusicPlaylist('Мой плейлист');
  final server = lib.addServerMusicPlaylist('В дорогу', 'Navidrome', [
    't3',
    't1',
  ]);
  expect(server.fromServer, isTrue);
  final backend = FakeOfflineBackend(
    titles: {for (final t in _tracks) t.id: t.title},
  );
  final s = AppServices(
    library: lib,
    secureStore: MemorySecureStore(),
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    clock: () => _now,
    offline: OfflineController(backend),
  );
  await s.init();
  s.locale.value = const Locale('ru');
  return (s, lib, backend);
}

void main() {
  setUpAll(loadAppFonts);

  test('the app does not edit a server playlist', () async {
    final lib = FakeLibrary.items(_tracks);
    final p = lib.addServerMusicPlaylist('Road', 'Navidrome', ['t1']);
    expect(() => lib.addToMusicPlaylist(p.id, 't2'), throwsStateError);
    expect(() => lib.renameMusicPlaylist(p.id, 'x'), throwsStateError);
    final own = await lib.createMusicPlaylist('Mine');
    expect(own.fromServer, isFalse);
    expect(await lib.addToMusicPlaylist(own.id, 't2'), isTrue);
  });

  for (final (name, ff, size) in <(String, FormFactor, Size)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('phone', FormFactor.phone, const Size(390, 844)),
  ]) {
    testWidgets('server playlist and offline album on $name', (t) async {
      setSurface(t, size, dpr: name == 'phone' ? 2 : 1);
      final (services, lib, backend) = await _services();
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.music),
      );
      await settle(t);
      await t.tap(find.byKey(const ValueKey('music-tab-library')));
      await settle(t, 6);
      await t.tap(find.byKey(const ValueKey('music-section-playlists')));
      await settle(t, 6);

      // The server's playlist: cloud icon, "<server> · N tracks".
      final server = (await lib.musicPlaylists().first).firstWhere(
        (p) => p.fromServer,
      );
      final row = find.byKey(ValueKey('music-playlist-${server.id}'));
      expect(row, findsOneWidget);
      expect(
        find.descendant(of: row, matching: find.byIcon(Icons.cloud_queue)),
        findsOneWidget,
      );
      expect(
        find.text(
          ru.musicServerPlaylistSubtitle(
            'Navidrome',
            ru.musicPlaylistTrackCount(2),
          ),
        ),
        findsOneWidget,
      );
      await capture(t, '01-server-playlist-list-$name');

      // Its page: the server's order and the server in the overline.
      await t.tap(row);
      await settle(t, 8);
      expect(
        find.text(ru.musicServerPlaylistLabel('Navidrome')),
        findsOneWidget,
      );
      expect(
        t.getTopLeft(find.text('Midnight Exit')).dy,
        lessThan(t.getTopLeft(find.text('Coastline')).dy),
      );

      // "Add to playlist" offers only the user's own playlists.
      final context = t.element(find.byType(MusicPlaylistScreen));
      final track = (await lib.musicTracks().first).first;
      // ignore: unawaited_futures
      showAddToMusicPlaylist(context, lib, track);
      await settle(t, 4);
      expect(find.text('Мой плейлист'), findsOneWidget);
      expect(
        find.descendant(
          of: find.byType(AlertDialog),
          matching: find.text('В дорогу'),
        ),
        findsNothing,
      );
      await t.tap(find.text(ru.commonCancel).last);
      await settle(t, 4);

      // Download the playlist: its tracks go to the offline queue.
      final download = find.byKey(const ValueKey('music-playlist-download'));
      expect(download, findsOneWidget);
      await t.tap(download);
      await settle(t, 6);
      expect(
        backend.requested.map((r) => r.id),
        containsAll(<String>['t1', 't3']),
      );
      for (var i = 0; i < 2; i++) {
        backend.advance(1);
        await settle(t, 4);
      }
      expect(find.text(ru.offlineDownloaded), findsOneWidget);
      await capture(t, '02-server-playlist-downloaded-$name');

      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
