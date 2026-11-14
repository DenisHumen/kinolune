// SPDX-License-Identifier: MPL-2.0
//
// The file browser (UI-006): listing, sorting, "folders first", file names,
// breadcrumbs and Back, and — the "done when" — a file opens the same
// Detail page as the library does (movie; show at the episode's season),
// an unrecognized file goes straight to the player. Also the v0 fallback
// for local folders (dart:io + items matched by path).

import 'dart:async';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/library/library_queries.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/file_browser_screen.dart';
import 'package:kinolune/screens/movie_detail_screen.dart';
import 'package:kinolune/screens/show_detail_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/library_source_manager.dart';
import 'package:kinolune/sources/source_browser.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:kinolune/widgets/format.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

BrowseEntry _f(String name, {int? ms, int? at, String dir = ''}) => BrowseEntry(
  name: name,
  relPath: dir.isEmpty ? name : '$dir/$name',
  isFolder: false,
  durationMs: ms,
  modified: at == null ? null : DateTime.fromMillisecondsSinceEpoch(at),
);

BrowseEntry _d(String name) =>
    BrowseEntry(name: name, relPath: name, isFolder: true);

List<String> _names(List<BrowseEntry> e) => [for (final x in e) x.name];

final _en = lookupAppLocalizations(const Locale('en'));

Finder _entry(String relPath) => find.byKey(ValueKey('browser-entry-$relPath'));

/// Opens the Sources screen and the file browser of the demo source.
Future<void> _openBrowser(WidgetTester t, AppServices services) async {
  await t.pumpWidget(testApp(services, section: Section.settings));
  await settle(t);
  await t.ensureVisible(find.text(_en.sourcesManage));
  await settle(t, 4);
  await t.tap(find.text(_en.sourcesManage));
  await settle(t);
  final browse = find.byKey(const ValueKey('source-browse-src-demo'));
  await t.ensureVisible(browse);
  await settle(t, 4);
  await t.tap(browse);
  await settle(t);
  expect(find.byType(FileBrowserScreen), findsOneWidget);
}

/// Taps through the folders of [filePath] (under the demo's `/media`) and
/// then the file.
Future<void> _walkTo(WidgetTester t, String filePath) async {
  final parts = filePath.substring('/media/'.length).split('/');
  for (var i = 1; i <= parts.length; i++) {
    final row = _entry(parts.take(i).join('/'));
    await t.scrollUntilVisible(
      row,
      200,
      scrollable: find
          .descendant(
            of: find.byKey(const ValueKey('browser-scroll')),
            matching: find.byType(Scrollable),
          )
          .first,
    );
    await settle(t, 2);
    await t.tap(row);
    await settle(t);
  }
}

void main() {
  group('sortBrowseEntries', () {
    final entries = [
      _f('Episode 10.mkv', ms: 100, at: 3),
      _d('Season 2'),
      _f('episode 2.mkv', at: 1),
      _d('Extras'),
      _f('Episode 1.mkv', ms: 300, at: 2),
    ];

    test('folders first, natural and case-insensitive by name', () {
      final s = sortBrowseEntries(
        entries,
        sort: const LibrarySort(),
        foldersFirst: true,
      );
      expect(_names(s), [
        'Extras',
        'Season 2',
        'Episode 1.mkv',
        'episode 2.mkv',
        'Episode 10.mkv',
      ]);
    });

    test('without folders first the names mix; descending reverses', () {
      final s = sortBrowseEntries(
        entries,
        sort: const LibrarySort(descending: true),
        foldersFirst: false,
      );
      expect(_names(s), [
        'Season 2',
        'Extras',
        'Episode 10.mkv',
        'episode 2.mkv',
        'Episode 1.mkv',
      ]);
    });

    test('duration and date keep missing values last both ways', () {
      for (final desc in [false, true]) {
        final s = sortBrowseEntries(
          entries,
          sort: LibrarySort(field: SortField.duration, descending: desc),
          foldersFirst: true,
        );
        expect(_names(s).sublist(2, 4), desc
            ? ['Episode 1.mkv', 'Episode 10.mkv']
            : ['Episode 10.mkv', 'Episode 1.mkv']);
        expect(s.last.name, 'episode 2.mkv');
      }
      final byDate = sortBrowseEntries(
        entries,
        sort: const LibrarySort(field: SortField.added, descending: true),
        foldersFirst: true,
      );
      expect(_names(byDate).sublist(2), [
        'Episode 10.mkv',
        'Episode 1.mkv',
        'episode 2.mkv',
      ]);
    });

    test('titles sort by the label shown', () {
      final s = sortBrowseEntries(
        [_f('b.mkv'), _f('a.mkv')],
        sort: const LibrarySort(),
        foldersFirst: true,
        labelOf: (e) => e.name == 'a.mkv' ? 'Zulu' : 'Alpha',
      );
      expect(_names(s), ['b.mkv', 'a.mkv']);
    });
  });

  test('a live listing hides system entries and non-media files', () {
    final merged = mergeListing(
      const [
        SourceFolderEntry(name: 'Movies', relPath: 'Movies', isFolder: true),
        SourceFolderEntry(name: '@eaDir', relPath: '@eaDir', isFolder: true),
        SourceFolderEntry(name: '.hidden.mkv', relPath: '.hidden.mkv', isFolder: false),
        SourceFolderEntry(name: 'notes.txt', relPath: 'notes.txt', isFolder: false),
        SourceFolderEntry(
          name: 'a.mkv',
          relPath: 'a.mkv',
          isFolder: false,
          sizeBytes: 7,
        ),
        SourceFolderEntry(name: 'disc.iso', relPath: 'disc.iso', isFolder: false),
      ],
      {
        'a.mkv': const BrowseEntry(
          name: 'a.mkv',
          relPath: 'a.mkv',
          isFolder: false,
          itemId: 'm1',
          durationMs: 60000,
        ),
        'Movies': const BrowseEntry(
          name: 'Movies',
          relPath: 'Movies',
          isFolder: true,
          fileCount: 3,
        ),
      },
    );
    expect(_names(merged), ['Movies', 'a.mkv', 'disc.iso']);
    expect(merged[0].fileCount, 3);
    expect((merged[1].itemId, merged[1].sizeBytes, merged[1].durationMs), (
      'm1',
      7,
      60000,
    ));
    expect(merged[2].isDiscImage, isTrue);
  });

  test('tree listing of library paths counts files in folders', () {
    final e = treeListing('/media', 'Shows', [
      '/media/Shows/A/Season 01/a1.mkv',
      '/media/Shows/A/Season 01/a2.mkv',
      '/media/Shows/B/b1.mkv',
      '/media/Shows/pilot.mp4',
      '/media/Movies/x.mkv',
    ]);
    expect([for (final x in e) (x.relPath, x.isFolder, x.fileCount)], [
      ('Shows/A', true, 2),
      ('Shows/B', true, 1),
      ('Shows/pilot.mp4', false, null),
    ]);
  });

  test('local roots: plain paths and file URIs; network has none', () {
    ManagedSource src(SourceType t, String a) =>
        ManagedSource(id: '1', type: t, name: 'S', address: a);
    expect(localRootOf(src(SourceType.local, '/data/films')), '/data/films');
    expect(
      localRootOf(src(SourceType.local, 'file:///data/my%20films')),
      '/data/my films',
    );
    expect(localRootOf(src(SourceType.smb, 'smb://nas/media')), isNull);
    expect(joinLocal('/data/', 'a/b.mkv'), '/data/a/b.mkv');
  });

  group('v0 fallback: local folders through dart:io', () {
    late Directory tmp;
    setUp(() => tmp = Directory.systemTemp.createTempSync('ui006-'));
    tearDown(() => tmp.deleteSync(recursive: true));

    test('lists the folder and matches files to items by path', () async {
      final movieDir = Directory('${tmp.path}/Movies/Heat (1995)')
        ..createSync(recursive: true);
      final file = File('${movieDir.path}/Heat.1995.mkv')
        ..writeAsBytesSync([1, 2, 3]);
      File('${movieDir.path}/clip.mp4').writeAsBytesSync([1]);
      File('${movieDir.path}/info.nfo').writeAsStringSync('x');
      Directory('${tmp.path}/.trash').createSync();

      final library = FakeLibrary.items([
        LibraryItem(
          id: 'movie-heat',
          kind: ItemKind.movie,
          title: 'Heat',
          year: 1995,
          filePath: file.path,
          durationMs: 170 * 60000,
        ),
      ]);
      final added = await library.addSource(
        MediaSource(
          id: '',
          kind: SourceKind.localFolder,
          name: 'Disk',
          path: tmp.path,
        ),
      );
      final manager = LibrarySourceManager(library);
      final source = (await manager.list()).firstWhere((s) => s.id == added.id);
      final browser = SourceBrowser.of(library, manager);
      expect(browser, isA<MemorySourceBrowser>());

      final root = await browser.list(source, '');
      expect(_names(root.entries), ['Movies']);
      expect(root.liveError, isNull);

      final inside = await browser.list(source, 'Movies/Heat (1995)');
      final sorted = sortBrowseEntries(
        inside.entries,
        sort: const LibrarySort(),
        foldersFirst: true,
      );
      expect(_names(sorted), ['clip.mp4', 'Heat.1995.mkv']);
      final heat = sorted.last;
      expect((heat.itemId, heat.sizeBytes, heat.durationMs), (
        'movie-heat',
        3,
        170 * 60000,
      ));
      expect(sorted.first.itemId, isNull);
      expect(
        browser.fileUri(source, 'Movies/Heat (1995)/clip.mp4'),
        '${tmp.path}/Movies/Heat (1995)/clip.mp4',
      );

      await expectLater(
        manager.listFolder(source.id, '../etc'),
        throwsA(isA<SourceException>()),
      );
      final gone = await browser.list(source, 'Missing');
      expect(gone.liveError?.code, SourceErrorCode.pathNotFound);
    });
  });

  group('screen', () {
    testWidgets('a movie file opens the same Detail as the library', (
      t,
    ) async {
      setSurface(t, const Size(1280, 800));
      final services = await demoServices();
      final movie = (await services.library.item('movie-the-quiet-orbit'))!;

      // The library's path: Movies → the poster.
      await t.pumpWidget(testApp(services, section: Section.movies));
      await settle(t);
      await t.tap(find.text('The Quiet Orbit').first);
      await settle(t);
      final fromLibrary = t.widget<MovieDetailScreen>(
        find.byType(MovieDetailScreen),
      );

      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
      // The browser's path: Sources → Browse files → folders → the file.
      await _openBrowser(t, services);
      await _walkTo(t, movie.filePath);
      final fromBrowser = t.widget<MovieDetailScreen>(
        find.byType(MovieDetailScreen),
      );
      expect(fromBrowser.item.id, movie.id);
      expect(fromBrowser.item.id, fromLibrary.item.id);
      expect(find.byType(PlayerScreen), findsNothing);
    });

    testWidgets('an episode file opens its show at the episode season', (
      t,
    ) async {
      setSurface(t, const Size(1280, 800));
      final services = await demoServices();
      final episode =
          (await services.library.item('show-moonlit-bureau-s2e3'))!;
      await _openBrowser(t, services);
      await _walkTo(t, episode.filePath);
      final page = t.widget<ShowDetailScreen>(find.byType(ShowDetailScreen));
      expect(page.show.id, 'show-moonlit-bureau');
      expect(page.initialSeason, 2);
    });

    testWidgets('titles or file names, folders first, breadcrumbs, Back', (
      t,
    ) async {
      setSurface(t, const Size(1280, 800));
      final services = await demoServices();
      final episode =
          (await services.library.item('show-moonlit-bureau-s1e2'))!;
      final fileName = episode.filePath.split('/').last;
      await _openBrowser(t, services);
      await _walkTo(t, '/media/Shows/Moonlit Bureau');
      await t.tap(_entry('Shows/Moonlit Bureau/Season 01'));
      await settle(t);

      // Titles by default, the file name below.
      final title =
          'Moonlit Bureau · ${episodeCode(_en, episode)} · ${episode.title}';
      expect(find.text(title), findsOneWidget);
      expect(find.textContaining(fileName), findsOneWidget);

      // File names instead; remembered.
      await t.tap(find.byKey(const ValueKey('browser-file-names')));
      await settle(t, 4);
      expect(find.text(fileName), findsOneWidget);
      expect(find.text(title), findsNothing);
      expect(
        (services.preferences as MemoryPreferences)
            .values[Preferences.browserShowFileNames],
        'true',
      );

      // Breadcrumbs: back to the show folder.
      await t.tap(find.byKey(const ValueKey('browser-crumb-Shows/Moonlit Bureau')));
      await settle(t);
      expect(_entry('Shows/Moonlit Bureau/Season 02'), findsOneWidget);

      // Back goes up a folder, then leaves the browser at the root.
      await t.tap(find.byKey(const ValueKey('browser-back')));
      await settle(t);
      expect(_entry('Shows/Moonlit Bureau'), findsOneWidget);
      await pressKey(t, LogicalKeyboardKey.backspace);
      expect(_entry('Shows'), findsOneWidget);
      await t.tap(find.byKey(const ValueKey('browser-back')));
      await settle(t);
      expect(find.byType(FileBrowserScreen), findsNothing);
    });

    testWidgets('sort and folders first are saved per source', (t) async {
      setSurface(t, const Size(1280, 800));
      final services = await demoServices();
      await _openBrowser(t, services);
      // Root: Home Videos, Movies, Music, Shows — all folders.
      await t.tap(_entry('Home Videos'));
      await settle(t);
      await t.tap(find.byKey(const ValueKey('browser-sort')));
      await settle(t, 4);
      await t.tap(find.byKey(const ValueKey('browser-sort-title')));
      await settle(t, 4);
      final rows = find.byWidgetPredicate(
        (w) =>
            w.key is ValueKey<String> &&
            (w.key! as ValueKey<String>).value.startsWith('browser-entry-'),
      );
      final order = [
        for (final w in t.widgetList(rows))
          (w.key! as ValueKey<String>).value.split('/').last,
      ];
      expect(order, [
        'Old camcorder tapes.iso',
        'Birthday party.mkv',
        'Beach day 2019.mp4',
      ]);
      await t.tap(find.byKey(const ValueKey('browser-folders-first')));
      await settle(t, 4);
      final prefs = await services.queries.viewPrefs('folder:src-demo');
      expect(prefs.sort, const LibrarySort(descending: true));
      expect(prefs.foldersFirst, isFalse);
    });

    testWidgets('an unrecognized file plays at once; a disc image says why', (
      t,
    ) async {
      setSurface(t, const Size(1280, 800));
      final services = await demoServices();
      await _openBrowser(t, services);
      await _walkTo(t, '/media/Home Videos/Old camcorder tapes.iso');
      expect(find.byKey(const ValueKey('unplayable-dialog')), findsOneWidget);
      await t.tap(find.text(_en.libraryUnplayableOk));
      await settle(t);
      expect(find.text(_en.browserNotRecognized), findsWidgets);
      await t.tap(_entry('Home Videos/Beach day 2019.mp4'));
      await settle(t);
      final player = t.widget<PlayerScreen>(find.byType(PlayerScreen));
      expect(player.request.uri, '/media/Home Videos/Beach day 2019.mp4');
      expect(player.request.title, 'Beach day 2019.mp4');
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('remote: arrows and select open folders on TV', (t) async {
      setSurface(t, const Size(960, 540), dpr: 2);
      final services = await demoServices();
      await t.pumpWidget(
        testApp(services, formFactor: FormFactor.tv, section: Section.settings),
      );
      await settle(t);
      final settings = t.element(find.byType(Scaffold).last);
      unawaited(
        openFileBrowser(settings, services.managedSources.first),
      );
      await settle(t);
      expect(_entry('Home Videos'), findsOneWidget);
      // The first row has focus; down twice and select opens "Music".
      await pressKey(t, LogicalKeyboardKey.arrowDown, times: 2);
      await pressKey(t, LogicalKeyboardKey.select);
      await settle(t);
      expect(find.byKey(const ValueKey('browser-crumb-Music')), findsOneWidget);
    });
  });
}
