// SPDX-License-Identifier: MPL-2.0
//
// LIB-025 (ADR-0099): the core decides what files hold and the app shows
// it — «Аудиокниги» inside «Музыка», the «Мои видео» row on Home and its
// full list (a tap plays the clip), and «Это не то» on a folder of the
// file browser with «Вернуть». With KINOLUNE_SCREENSHOTS_DIR set the frames
// are written (in Russian):
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/lib-025 \
//     flutter test test/content_kinds_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/file_browser_screen.dart' show openFileBrowser;
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:kinolune/widgets/artwork.dart' show kDemoArtScheme;
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

Finder _key(String k) => find.byKey(ValueKey(k));

Future<void> _tapKey(WidgetTester t, String key) async {
  final f = _key(key);
  await t.ensureVisible(f);
  await t.pump();
  await t.tap(f);
  await settle(t, 6);
}

/// A source manager with numeric ids (as the core's) and a folder tree.
class _Manager extends FakeSourceManager {
  _Manager(super.library) : super(numericIds: true);

  static const tree = {
    '': ['Аудиокниги', 'Кино', 'Телефон'],
  };

  @override
  Future<List<SourceFolderEntry>> listFolder(String id, String dir) async => [
    for (final name in tree[dir] ?? const <String>[])
      SourceFolderEntry(
        name: name,
        relPath: dir.isEmpty ? name : '$dir/$name',
        isFolder: true,
      ),
  ];
}

/// A book of three chapters by invented authors.
List<MusicTrack> _book() => [
  for (var n = 1; n <= 3; n++)
    MusicTrack(
      item: LibraryItem(
        id: 'track-book-$n',
        kind: ItemKind.track,
        title: 'Глава $n',
        artist: 'Анна Северова',
        album: 'Дом у маяка',
        trackNumber: n,
        posterUrl: '${kDemoArtScheme}album',
        filePath: '/media/Аудиокниги/Анна Северова - Дом у маяка/$n.mp3',
        durationMs: (40 + n) * 60 * 1000,
        addedAt: DateTime(2026, 11, 10),
      ),
      albumArtists: const ['Анна Северова'],
      artists: const ['Анна Северова'],
      genres: const ['Audiobook'],
      codec: 'mp3',
      audiobook: true,
    ),
];

List<OwnVideo> _clips() => [
  for (final (i, name, minutes) in [
    (1, 'VID_20240612_183000', 2),
    (2, 'VID_20240613_101500', 1),
    (3, 'PXL_20240701_090000123', 4),
    (4, 'IMG_4521', 1),
  ])
    OwnVideo(
      fileId: i,
      sourceId: 1,
      relPath: 'Телефон/DCIM/Camera/$name.mp4',
      localPath: '/media/Телефон/DCIM/Camera/$name.mp4',
      title: name,
      sizeBytes: 40 << 20,
      durationMs: minutes * 60 * 1000,
      takenAt: DateTime(2024, 6, 12 + i, 18, 30),
    ),
];

Future<(AppServices, FakeLibrary)> _open(
  WidgetTester t, {
  Size size = const Size(1280, 800),
  FormFactor formFactor = FormFactor.desktop,
  Section section = Section.home,
  double dpr = 1,
}) async {
  setSurface(t, size, dpr: dpr);
  final library = FakeLibrary.demo(scanStepDelay: Duration.zero)
    ..audiobookTracks.addAll(_book())
    ..ownVideoList.addAll(_clips())
    ..folderContents.addAll({
      'Аудиокниги': const FolderContent(
        path: 'Аудиокниги',
        kind: ContentKind.audiobook,
      ),
      'Кино': const FolderContent(path: 'Кино', kind: ContentKind.video),
      'Телефон': const FolderContent(
        path: 'Телефон',
        kind: ContentKind.ownVideo,
      ),
    });
  final s = await demoServices(library: library, sourceManager: _Manager.new);
  s.locale.value = const Locale('ru');
  await t.pumpWidget(testApp(s, formFactor: formFactor, section: section));
  await settle(t);
  return (s, library);
}

void main() {
  setUpAll(loadAppFonts);

  for (final (name, ff, size, dpr) in <(String, FormFactor, Size, double)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800), 1),
    ('phone', FormFactor.phone, const Size(390, 844), 2),
  ]) {
    testWidgets('audiobooks are a section of Music on $name', (t) async {
      await _open(
        t,
        size: size,
        formFactor: ff,
        section: Section.music,
        dpr: dpr,
      );
      expect(_key('music-tab-audiobooks'), findsOneWidget);
      // Music leaves the book out.
      expect(find.text('Дом у маяка'), findsNothing);
      await _tapKey(t, 'music-tab-audiobooks');
      expect(_key('music-audiobooks'), findsOneWidget);
      expect(find.text('Дом у маяка'), findsOneWidget);
      expect(find.text('Nocturne Avenue'), findsNothing);
      await capture(t, '01-audiobooks-$name');
      await _tapKey(t, 'music-tab-library');
      expect(find.text('Дом у маяка'), findsNothing);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('«Мои видео» on Home and in full on $name', (t) async {
      await _open(t, size: size, formFactor: ff, dpr: dpr);
      final row = _key('home-own-videos');
      await t.scrollUntilVisible(
        row,
        300,
        scrollable: find.byType(Scrollable).first,
      );
      await settle(t, 4);
      expect(find.text(ru.homeOwnVideosTitle), findsWidgets);
      expect(find.text('VID_20240612_183000'), findsOneWidget);
      await capture(t, '02-own-videos-home-$name');
      await t.tap(
        find.descendant(of: row, matching: find.text(ru.commonSeeAll)).first,
      );
      await settle(t, 8);
      expect(_key('own-videos-grid'), findsOneWidget);
      await capture(t, '03-own-videos-all-$name');
      await t.scrollUntilVisible(
        find.text('IMG_4521'),
        300,
        scrollable: find.descendant(
          of: _key('own-videos-grid'),
          matching: find.byType(Scrollable),
        ),
      );
      await settle(t, 2);
      await t.tap(find.text('IMG_4521'));
      await settle(t, 8);
      final player = t.widget<PlayerScreen>(find.byType(PlayerScreen));
      expect(player.request.uri, '/media/Телефон/DCIM/Camera/IMG_4521.mp4');
      expect(player.request.title, 'IMG_4521');
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }

  testWidgets('«Это не то» on a folder flips it and «Вернуть» undoes', (
    t,
  ) async {
    final (s, library) = await _open(t, section: Section.settings);
    final source = await s.createSource(
      const SourceDraft(type: SourceType.local, name: 'NAS', address: '/media'),
    );
    await settle(t, 4);
    // ignore: unawaited_futures
    openFileBrowser(t.element(find.byType(AppShell)), source);
    await settle(t, 8);
    expect(_key('browser-entry-Телефон'), findsOneWidget);
    // Only the kinds a folder does not usually hold are named.
    expect(
      find.descendant(
        of: _key('browser-entry-Телефон'),
        matching: find.textContaining(ru.browserKindOwnVideo),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: _key('browser-entry-Кино'),
        matching: find.textContaining(ru.browserKindVideo),
      ),
      findsNothing,
    );
    await capture(t, '04-browser-kinds-desktop');

    // «Кино» is the user's own footage: one tap, no form.
    await _tapKey(t, 'browser-folder-not-this-Кино');
    expect(library.notThisCalls, [(int.parse(source.id), 'Кино')]);
    expect(
      find.text(ru.browserNotThisDone(ru.browserKindOwnVideo)),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: _key('browser-entry-Кино'),
        matching: find.textContaining(
          ru.browserKindChanged(ru.browserKindOwnVideo),
        ),
      ),
      findsOneWidget,
    );
    await capture(t, '05-browser-not-this-desktop');

    // «Вернуть» brings back what was detected.
    await t.tap(find.text(ru.browserNotThisUndo));
    await settle(t, 6);
    expect(library.notThisCalls.length, 2);
    expect(library.folderContents['Кино']!.kind, ContentKind.video);
    expect(library.folderContents['Кино']!.overridden, isFalse);
  });
}
