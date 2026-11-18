// SPDX-License-Identifier: MPL-2.0
//
// NET-013: a long press on Play of an item of a media server (Plex,
// Jellyfin, Emby) offers a transcode on the server — the source quality or
// a lower one — and plays the server's HLS; a plain press plays directly.
// Items of other sources ignore the long press. Progress saves of a server
// item are its reports (Plex: timeline).

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/movie_detail_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/fake_jellyfin.dart';
import 'support/test_app.dart';

final _now = DateTime(2026, 11, 18, 20);

final _onPlex = LibraryItem(
  id: 'm-plex',
  kind: ItemKind.movie,
  title: 'Big Buck Bunny',
  year: 2008,
  filePath: '/Movies/Big Buck Bunny (2008)/Big Buck Bunny (2008).mkv',
  durationMs: 596000,
  addedAt: _now,
);

final _local = LibraryItem(
  id: 'm-local',
  kind: ItemKind.movie,
  title: 'Plain Film',
  year: 1999,
  filePath: '/media/Movies/plain.avi',
  addedAt: _now,
);

Future<(AppServices, FakeJellyfinLink, List<FakePlayerEngine>)>
_services() async {
  final link = FakeJellyfinLink()..serverItems.add('m-plex');
  final engines = <FakePlayerEngine>[];
  final s = AppServices(
    library: FakeLibrary.items([_onPlex, _local]),
    secureStore: MemorySecureStore(),
    playerEngineFactory: ({bool video = true}) {
      final e = FakePlayerEngine();
      engines.add(e);
      return e;
    },
    jellyfin: link,
    clock: () => _now,
  );
  await s.init();
  return (s, link, engines);
}

Future<void> _open(WidgetTester t, AppServices s, String title) async {
  setSurface(t, const Size(1280, 800));
  await t.pumpWidget(
    testApp(s, formFactor: FormFactor.desktop, section: Section.movies),
  );
  await settle(t, 10);
  await t.tap(find.text(title).first);
  await settle(t, 10);
  expect(find.byType(MovieDetailScreen), findsOneWidget);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

void main() {
  testWidgets('long press on Play transcodes a server item', (t) async {
    final (s, link, engines) = await _services();
    await _open(t, s, 'Big Buck Bunny');
    await t.longPress(find.byKey(detailPlayKey));
    await settle(t, 5);
    expect(
      find.byKey(const ValueKey('server-transcode-dialog')),
      findsOneWidget,
    );
    expect(find.text('Transcode on the server'), findsOneWidget);
    for (final label in [
      'Original quality',
      '1080p · 8 Mbit/s',
      '720p · 4 Mbit/s',
      '480p · 1.5 Mbit/s',
    ]) {
      expect(find.text(label), findsOneWidget);
    }
    await t.tap(find.text('720p · 4 Mbit/s'));
    await settle(t, 10);
    expect(link.calls, contains('playback m-plex 0 transcode 4000000'));
    // The player opened the server's HLS, not the file.
    expect(engines.last.uris.single, contains('/transcode/universal/'));
    await _close(t);
  });

  testWidgets('a press plays a server item directly', (t) async {
    final (s, link, engines) = await _services();
    await _open(t, s, 'Big Buck Bunny');
    await t.tap(find.byKey(detailPlayKey));
    await settle(t, 10);
    expect(link.calls, contains('playback m-plex 0'));
    expect(engines.last.uris.single, isNot(contains('/transcode/')));
    await _close(t);
  });

  testWidgets('long press on Play of a local file does nothing', (t) async {
    final (s, link, engines) = await _services();
    await _open(t, s, 'Plain Film');
    final before = engines.length;
    await t.longPress(find.byKey(detailPlayKey));
    await settle(t, 5);
    expect(find.byKey(const ValueKey('server-transcode-dialog')), findsNothing);
    expect(link.calls.where((c) => c.startsWith('playback')), isEmpty);
    expect(engines.length, before);
    await _close(t);
  });
}
