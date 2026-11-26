// SPDX-License-Identifier: MPL-2.0
//
// PLAY-017 in the app: a Blu-ray image plays its main title with the
// disc's chapters, angles and stream languages; its other playlists are
// extras of the film; a protected disc says why instead of opening the
// player.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/screens/playback.dart';
import 'package:kinolune/shell/app_shell.dart' show Section;
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart' show PlayerScreen, PlayerScreenState;
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

const _iso = LibraryItem(
  id: 'm1',
  kind: ItemKind.movie,
  title: 'Dune',
  year: 2021,
  filePath: '/media/Movies/Dune (2021).iso',
);

const _locked = LibraryItem(
  id: 'm2',
  kind: ItemKind.movie,
  title: 'Tenet',
  year: 2020,
  filePath: '/media/Movies/Tenet (2020).iso',
);

String _clip(String n) => '/media/Movies/Dune (2021).iso/BDMV/STREAM/$n.m2ts';

final _main = DiscTitle(
  id: '00001',
  durationMs: 5400000,
  angles: [
    [_clip('00001'), _clip('00002')],
    [_clip('00001'), _clip('00003')],
  ],
  chaptersMs: const [0, 1800000, 3600000],
  streams: const [
    DiscStream(subtitle: false, id: 0x1100, codec: 'TrueHD', language: 'eng'),
    DiscStream(subtitle: false, id: 0x1101, codec: 'AC-3', language: 'rus'),
    DiscStream(subtitle: false, id: 0x1102, codec: 'AC-3', language: 'ukr'),
    DiscStream(subtitle: true, id: 0x1200, codec: 'PGS', language: 'eng'),
    DiscStream(subtitle: true, id: 0x1201, codec: 'PGS', language: ''),
  ],
);

/// A library whose `.iso` items are Blu-ray images.
class _DiscLibrary extends FakeLibrary implements LibraryParts, LibraryDisc {
  _DiscLibrary() : super.items(const [_iso, _locked]);

  @override
  Future<List<String>> playParts(LibraryItem item) async =>
      item.id == _iso.id ? _main.angles.first : [item.playbackUri];

  @override
  Future<DiscPlayback?> discPlayback(LibraryItem item) async =>
      switch (item.id) {
        'm1' => DiscPlayback(
          bluRay: true,
          volumeId: 'DUNE',
          main: _main,
          extras: [
            DiscTitle(
              id: '00002',
              durationMs: 600000,
              angles: [
                [_clip('00010')],
              ],
            ),
          ],
        ),
        'm2' => const DiscPlayback(bluRay: true, protection: 'aacs'),
        _ => null,
      };
}

Future<(FakePlayerEngine, BuildContext)> _app(WidgetTester t) async {
  setSurface(t, const Size(1280, 800));
  final engine = FakePlayerEngine(loadedDuration: const Duration(minutes: 90));
  final services = AppServices(
    library: _DiscLibrary(),
    secureStore: MemorySecureStore(),
    playerEngineFactory: ({bool video = true}) => engine,
  );
  await services.init();
  await t.pumpWidget(testApp(services));
  await settle(t);
  return (engine, t.element(find.byType(Scaffold).first));
}

void main() {
  testWidgets('a protected disc says why and does not play', (t) async {
    final (engine, context) = await _app(t);
    playVideo(context, _locked);
    await settle(t);
    expect(find.byKey(const ValueKey('unplayable-dialog')), findsOneWidget);
    expect(find.textContaining('copy-protected'), findsOneWidget);
    expect(find.byType(PlayerScreen), findsNothing);
    expect(engine.calls, isEmpty);
    await t.tap(find.text('OK'));
    await settle(t);
  });

  testWidgets('an image plays its main title with chapters and angles', (
    t,
  ) async {
    final (engine, context) = await _app(t);
    playVideo(context, _iso);
    await settle(t);
    expect(find.byType(PlayerScreen), findsOneWidget);
    final angles = _main.angleUris;
    expect(engine.uris, [angles.first]);
    final request = t
        .state<PlayerScreenState>(find.byType(PlayerScreen))
        .request;
    expect(request.angles, angles);
    expect(request.chapters!.map((c) => c.start.inMinutes), [0, 30, 60]);
    expect(request.streamLanguages, {
      0x1100: 'eng',
      0x1101: 'rus',
      0x1102: 'ukr',
      0x1200: 'eng',
    });
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });

  testWidgets('the other playlists are extras of the film', (t) async {
    setSurface(t, const Size(1280, 800));
    final services = AppServices(
      library: _DiscLibrary(),
      secureStore: MemorySecureStore(),
      playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    );
    await services.init();
    await t.pumpWidget(testApp(services, section: Section.movies));
    await settle(t, 10);
    await t.tap(find.text('Dune').first);
    await settle(t, 10);
    expect(find.byKey(const ValueKey('detail-extras')), findsOneWidget);
    expect(find.text('00002'), findsOneWidget);
    expect(find.textContaining('From the disc'), findsOneWidget);
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });

  test('the texts in Russian and Ukrainian', () async {
    final ru = await AppLocalizations.delegate.load(const Locale('ru'));
    expect(ru.libraryUnplayableProtectedDisc, contains('защищён'));
    expect(ru.detailExtraDiscTitle, 'С диска');
    final uk = await AppLocalizations.delegate.load(const Locale('uk'));
    expect(uk.libraryUnplayableProtectedDisc, contains('захищений'));
    expect(uk.detailExtraDiscTitle, 'З диска');
  });
}
