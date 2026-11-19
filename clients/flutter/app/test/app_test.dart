// SPDX-License-Identifier: MPL-2.0

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_smb_server.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/widgets/logo.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

void main() {
  setUpAll(loadAppFonts);

  group('desktop', () {
    testWidgets('home shows the rows and the sidebar', (tester) async {
      setSurface(tester, const Size(1280, 800));
      await tester.pumpWidget(testApp(await demoServices()));
      await settle(tester);
      expect(find.text('Continue Watching'), findsOneWidget);
      await scrollHomeTo(tester, find.text('Recently Added Movies'));
      expect(find.text('Recently Added Movies'), findsOneWidget);
      expect(find.text('TV Shows'), findsWidgets);
      expect(find.byType(KinoluneLogo), findsWidgets);
      expect(find.text('Settings'), findsOneWidget); // sidebar item
    });

    testWidgets('movies grid sorts, filters and opens a detail page', (
      tester,
    ) async {
      setSurface(tester, const Size(1280, 800));
      await tester.pumpWidget(
        testApp(await demoServices(), section: Section.movies),
      );
      await settle(tester);
      expect(find.text('14 titles'), findsOneWidget);
      await tester.tap(find.text('Sort: Recently added'));
      await settle(tester, 3);
      await tester.tap(find.text('Title').last);
      await settle(tester, 3);
      expect(find.text('Sort: Title'), findsOneWidget);
      await tester.tap(find.text('In progress'));
      await settle(tester, 3);
      expect(find.text('The Quiet Orbit'), findsOneWidget);
      expect(find.text('Harbor Lights'), findsNothing);
      await tester.tap(find.text('The Quiet Orbit'));
      await settle(tester);
      expect(find.textContaining('Resume'), findsOneWidget);
      expect(find.text('Fix match'), findsOneWidget);
    });

    testWidgets('fix match updates the title', (tester) async {
      setSurface(tester, const Size(1280, 800));
      await tester.pumpWidget(
        testApp(await demoServices(), section: Section.movies),
      );
      await settle(tester);
      await tester.tap(find.text('Midnight Cartographer'));
      await settle(tester);
      expect(find.textContaining('Uncertain match'), findsOneWidget);
      await tester.tap(find.text('Fix match'));
      await settle(tester, 4);
      await tester.enterText(
        find.widgetWithText(TextField, 'Title'),
        'The Midnight Cartographer',
      );
      await tester.tap(find.text('Save'));
      await settle(tester, 4);
      expect(find.text('Match updated'), findsOneWidget);
    });

    testWidgets('show → seasons → episodes', (tester) async {
      setSurface(tester, const Size(1280, 800));
      await tester.pumpWidget(
        testApp(await demoServices(), section: Section.shows),
      );
      await settle(tester);
      await tester.tap(find.text('Moonlit Bureau'));
      await settle(tester);
      expect(find.text('Season 1'), findsOneWidget);
      expect(find.text('Season 3'), findsOneWidget);
      expect(find.text('Resume S2 · E4'), findsOneWidget);
      await tester.tap(find.text('Season 3'));
      await settle(tester, 4);
      expect(find.textContaining('S3 · E1'), findsWidgets);
    });

    testWidgets('music plays an album and shows the mini player', (
      tester,
    ) async {
      setSurface(tester, const Size(1280, 800));
      final services = await demoServices();
      await tester.pumpWidget(testApp(services, section: Section.music));
      await settle(tester);
      await tester.tap(find.text('Nocturne Avenue'));
      await settle(tester);
      await tester.tap(find.text('Play'));
      await settle(tester, 3);
      expect(services.music.current?.title, 'Streetlamp Waltz');
      expect(find.byType(NowPlayingBar), findsOneWidget);
      services.music.pause();
      await settle(tester, 2);
    });

    testWidgets('search finds titles and artists', (tester) async {
      setSurface(tester, const Size(1280, 800));
      await tester.pumpWidget(
        testApp(await demoServices(), section: Section.search),
      );
      await settle(tester);
      await tester.enterText(find.byType(TextField), 'orbit');
      await settle(tester, 3);
      expect(find.text('The Quiet Orbit'), findsOneWidget);
      expect(find.text('Harbor Lights'), findsNothing);
      expect(find.text('Music'), findsWidgets); // "Orbit Kids" tracks
    });

    testWidgets('onboarding → add folder source → scan fills the library', (
      tester,
    ) async {
      setSurface(tester, const Size(1280, 800));
      final services = await demoServices(empty: true);
      await tester.pumpWidget(testApp(services));
      await settle(tester);
      // UI-042: welcome → sources.
      await tester.tap(find.byKey(const ValueKey('onboarding-next')));
      await settle(tester, 4);
      await tester.tap(find.byKey(const ValueKey('onboarding-local')));
      await settle(tester, 4);
      await tester.enterText(
        find.widgetWithText(TextField, 'Folder'),
        '/srv/media',
      );
      await tester.tap(find.text('Add and scan'));
      await settle(tester, 20);
      expect(services.sources.single.path, '/srv/media');
      expect(services.scanProgress?.isDone, isTrue);
      // Pairing is optional: Next, then Open home.
      await tester.tap(find.byKey(const ValueKey('onboarding-next')));
      await settle(tester, 4);
      await tester.tap(find.text('Open home'));
      await settle(tester, 6);
      await scrollHomeTo(tester, find.text('Recently Added Movies'));
      expect(find.text('Recently Added Movies'), findsOneWidget);
    });

    testWidgets('SMB source password goes to the core, not the app store', (
      tester,
    ) async {
      setSurface(tester, const Size(1280, 800));
      final services = await demoServices(advancedSettings: true);
      await tester.pumpWidget(testApp(services, section: Section.settings));
      await settle(tester);
      await tester.tap(find.text('Add source'));
      await settle(tester, 4);
      await tester.tap(find.text('SMB share (Windows, NAS)'));
      await settle(tester, 3);
      // UI-028: server, sign in, pick the folder.
      final fake = services.sourceManager as FakeSourceManager;
      fake.smbServers = [
        FakeSmbServer(address: 'nas.local', users: const {'viewer': 's3cret'}),
      ];
      await tester.enterText(
        find.byKey(const ValueKey('smb-server')),
        'nas.local',
      );
      await tester.tap(find.byKey(const ValueKey('smb-next')));
      await settle(tester, 4);
      await tester.enterText(find.byKey(const ValueKey('smb-user')), 'viewer');
      await tester.enterText(
        find.byKey(const ValueKey('smb-password')),
        's3cret',
      );
      await tester.tap(find.byKey(const ValueKey('smb-connect')));
      await settle(tester, 6);
      await tester.tap(find.byKey(const ValueKey('smb-share-Media')));
      await settle(tester, 6);
      await tester.tap(find.byKey(const ValueKey('smb-use-folder')));
      await settle(tester, 20);
      final smb = services.sources.firstWhere((s) => s.kind == SourceKind.smb);
      expect(smb.path, 'smb://nas.local/Media');
      expect(smb.username, 'viewer');
      // LIB-018: one copy, in the core's secret store.
      expect(services.managedSource(smb.id)!.hasPassword, isTrue);
      expect(fake.drafts.single.password, 's3cret');
      expect(await services.secureStore.read('smb_password_${smb.id}'), isNull);
    });

    testWidgets('TMDB key is saved', (tester) async {
      setSurface(tester, const Size(1280, 800));
      final services = await demoServices(advancedSettings: true);
      await tester.pumpWidget(testApp(services, section: Section.settings));
      await settle(tester);
      final field = find.widgetWithText(
        TextField,
        'TMDB API key (v3) or read access token',
      );
      // Below the fold of the Settings list: scroll it to the middle.
      await Scrollable.ensureVisible(tester.element(field), alignment: .5);
      await tester.pump();
      await tester.enterText(field, 'abc123');
      // The first «Save» is TMDB's; the second saves the META-012 keys.
      await tester.tap(find.text('Save').first);
      await settle(tester, 3);
      expect(await services.tmdbKey(), 'abc123');
      expect(await services.library.tmdbApiKey(), 'abc123');
    });

    testWidgets('TheTVDB and Fanart.tv keys are saved (META-012)', (
      tester,
    ) async {
      setSurface(tester, const Size(1280, 800));
      final services = await demoServices(advancedSettings: true);
      await tester.pumpWidget(testApp(services, section: Section.settings));
      await settle(tester);
      final tvdb = find.byKey(const ValueKey('settings-tvdb-key'));
      await Scrollable.ensureVisible(tester.element(tvdb), alignment: .5);
      await tester.pump();
      await tester.enterText(tvdb, ' tvdb-key ');
      await tester.enterText(
        find.byKey(const ValueKey('settings-fanart-key')),
        'fanart-key',
      );
      await tester.tap(find.byKey(const ValueKey('settings-extra-keys-save')));
      await settle(tester, 3);
      final keys = await services.extraProviderKeys();
      expect(keys.tvdb, 'tvdb-key');
      expect(keys.tvdbPin, isNull);
      expect(keys.fanart, 'fanart-key');
      expect(find.text('Keys saved'), findsOneWidget);
    });
  });

  group('phone', () {
    testWidgets('uses bottom tabs', (tester) async {
      setSurface(tester, const Size(400, 860), dpr: 2.5);
      await tester.pumpWidget(
        testApp(await demoServices(), formFactor: FormFactor.phone),
      );
      await settle(tester);
      expect(find.byType(NavigationBar), findsOneWidget);
      await tester.tap(find.text('Library'));
      await settle(tester);
      expect(find.text('14 titles'), findsOneWidget);
    });
  });

  group('tv', () {
    testWidgets('top menu, D-pad focus and Back', (tester) async {
      setSurface(tester, const Size(960, 540), dpr: 2);
      await tester.pumpWidget(
        testApp(await demoServices(), formFactor: FormFactor.tv),
      );
      await settle(tester);
      expect(find.byType(NavigationBar), findsNothing);
      // The Home tab has initial focus; moving right switches sections.
      expect(FocusManager.instance.primaryFocus?.debugLabel, 'menu-home');
      await pressKey(tester, LogicalKeyboardKey.arrowRight);
      expect(find.text('14 titles'), findsOneWidget);
      // Down enters the content, Back returns to the menu.
      await pressKey(tester, LogicalKeyboardKey.arrowDown);
      expect(
        FocusManager.instance.primaryFocus?.debugLabel,
        isNot(startsWith('menu-')),
      );
      await pressKey(tester, LogicalKeyboardKey.escape);
      expect(FocusManager.instance.primaryFocus?.debugLabel, 'menu-movies');
    });
  });

  group('fake library', () {
    test(
      'continue watching is limited to 2..95 % and sorted by recency',
      () async {
        final lib = FakeLibrary.demo();
        final list = await lib.continueWatching().first;
        expect(list.first.$1.title, 'The Quiet Orbit');
        await lib.saveProgress(
          Progress(
            itemId: 'movie-the-quiet-orbit',
            positionMs: 99,
            durationMs: 100,
            updatedAt: DateTime.now(),
          ),
        );
        final after = await lib.continueWatching().first;
        expect(after.any((e) => e.$1.id == 'movie-the-quiet-orbit'), isFalse);
      },
    );
  });
}
