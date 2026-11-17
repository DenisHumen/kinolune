// SPDX-License-Identifier: MPL-2.0
//
// QA-015: fixes for what the test catalog (QA-014) found. Here: the store
// text of Updates, a tap on "X is live", deletions with «Отменить», a held
// OK opening the context menu, the «Все» tile of «Мои видео» on TV. Up
// Next without autoplay is in playback_session_test.dart, «Отметить
// непросмотренным» in library_grid_test.dart. With KINOLUNE_SCREENSHOTS_DIR
// set the visible ones are written (in Russian):
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/qa-015 \
//     flutter test test/catalog_fixes_test.dart

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_offline.dart';
import 'package:kinolune/fakes/fake_twitch_archive.dart';
import 'package:kinolune/features/music/smart_playlists.dart';
import 'package:kinolune/features/updates/update_section.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/offline/offline_downloads.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/downloads_screen.dart';
import 'package:kinolune/screens/music/smart_playlist_pages.dart';
import 'package:kinolune/screens/own_videos.dart' show OwnVideosScreen;
import 'package:kinolune/screens/twitch/twitch_archive_screens.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/notification_taps.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/theme/kinolune_theme.dart';
import 'package:kinolune/twitch/twitch_archive.dart';
import 'package:kinolune/twitch/twitch_downloads.dart';
import 'package:kinolune/widgets/focusable.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

Finder _key(String k) => find.byKey(ValueKey(k));

Future<void> _push(WidgetTester t, Widget page) async {
  final nav = t.state<NavigatorState>(find.byType(Navigator).first);
  // Not awaited: the route completes when popped.
  // ignore: unawaited_futures
  nav.push(MaterialPageRoute<void>(builder: (_) => page));
  await settle(t);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

/// A bare Russian app around [child] (no services).
Widget _bare(Widget child, {FormFactor ff = FormFactor.desktop}) =>
    RepaintBoundary(
      key: screenshotKey,
      child: FormFactorScope(
        formFactor: ff,
        child: MaterialApp(
          locale: const Locale('ru'),
          supportedLocales: AppLocalizations.supportedLocales,
          localizationsDelegates: kLocalizationsDelegates,
          theme: buildKinoluneTheme(ff),
          home: Scaffold(backgroundColor: KColors.bgBase, body: child),
        ),
      ),
    );

void main() {
  setUpAll(loadAppFonts);

  group('updates of store builds name their store', () {
    for (final (os, store) in [
      ('android', 'Google Play'),
      ('ios', 'App Store'),
      ('macos', 'App Store'),
      ('windows', 'Microsoft Store'),
    ]) {
      testWidgets(os, (t) async {
        setSurface(t, const Size(900, 400));
        await t.pumpWidget(
          _bare(ListView(children: [NoSelfUpdateSection(os: os)])),
        );
        await settle(t, 4);
        expect(
          find.text(ru.updatesStoreNamedBody('Kinolune', store)),
          findsOne,
        );
        expect(find.textContaining('Flatpak'), findsNothing);
        // Only Google Play has a known page yet.
        expect(
          _key('updates-store'),
          os == 'android' ? findsOne : findsNothing,
        );
        if (os == 'ios') await capture(t, 'qa-015-updates-ios');
      });
    }

    testWidgets('Linux and full builds updated by the system', (t) async {
      for (final section in const [
        NoSelfUpdateSection(os: 'linux'),
        NoSelfUpdateSection(storeBuild: false, os: 'android'),
      ]) {
        await t.pumpWidget(_bare(ListView(children: [section])));
        await settle(t, 2);
        expect(find.text(ru.updatesManagedBody('Kinolune')), findsOne);
        expect(_key('updates-store'), findsNothing);
      }
    });
  });

  group('a tap on "X is live"', () {
    test('payloads', () {
      expect(twitchChannelOfPayload('twitch/channel/LunaForge'), 'lunaforge');
      expect(twitchChannelOfPayload('twitch/channel/'), isNull);
      expect(twitchChannelOfPayload('twitch/channel/a/b'), isNull);
      expect(twitchChannelOfPayload('remote-input'), isNull);
    });

    testWidgets('opens the channel, also when it started the app', (t) async {
      setSurface(t, const Size(1280, 800));
      final s = await demoServices();
      // Before the app listens: as when the tap launched it.
      NotificationTaps.instance.handle('twitch/channel/lunaforge');
      await t.pumpWidget(testApp(s));
      await settle(t, 8);
      expect(_key('twitch-channel-page-lunaforge'), findsOneWidget);

      Navigator.of(t.element(_key('twitch-channel-page-lunaforge'))).pop();
      await settle(t, 6);
      NotificationTaps.instance.handle('something/else');
      await settle(t, 4);
      expect(_key('twitch-channel-page-lunaforge'), findsNothing);
      NotificationTaps.instance.handle('twitch/channel/novamarsh');
      await settle(t, 8);
      expect(_key('twitch-channel-page-novamarsh'), findsOneWidget);
      await _close(t);
    });
  });

  group('deletions go at once with «Отменить»', () {
    testWidgets('a smart playlist', (t) async {
      setSurface(t, const Size(1280, 800));
      final s = await demoServices();
      s.locale.value = const Locale('ru');
      const p = SmartPlaylist(id: 'smart-1', name: 'Всё подряд');
      await s.smartPlaylists.save(p);
      await t.pumpWidget(testApp(s, section: Section.music));
      await settle(t);
      await t.tap(_key('music-tab-library'));
      await settle(t, 4);
      await t.tap(_key('music-section-playlists'));
      await settle(t, 4);
      await t.tap(_key('music-smart-smart-1'));
      await settle(t, 4);
      expect(find.byType(SmartPlaylistScreen), findsOneWidget);

      await t.tap(_key('smart-delete'));
      await settle(t, 6);
      // No dialog: gone, the page closed, the message offers it back.
      expect(find.byType(AlertDialog), findsNothing);
      expect(s.smartPlaylists.playlists.value, isEmpty);
      expect(find.byType(SmartPlaylistScreen), findsNothing);
      expect(find.text(ru.musicSmartDeleted('Всё подряд')), findsOneWidget);
      await capture(t, 'qa-015-smart-playlist-undo');
      await t.tap(_key('snack-undo'));
      await settle(t, 6);
      expect(s.smartPlaylists.playlists.value.single.name, 'Всё подряд');
      await _close(t);
    });

    testWidgets('a recording rule', (t) async {
      setSurface(t, const Size(1280, 900));
      final archive = FakeTwitchArchive.demo(demoNow);
      final s = await demoServices(twitchArchive: archive);
      const rule = TwitchRecordRule(userId: '7002', keepLast: 5);
      await archive.setRule(s.archiveProfileId, rule);
      await t.pumpWidget(testApp(s, section: Section.twitch));
      await settle(t);
      await _push(
        t,
        const TwitchRecordRuleScreen(userId: '7002', displayName: 'LunaForge'),
      );
      await t.ensureVisible(_key('twitch-rule-delete'));
      await settle(t, 2);
      await t.tap(_key('twitch-rule-delete'));
      await settle(t, 6);
      expect(find.byType(AlertDialog), findsNothing);
      expect(await archive.rules(s.archiveProfileId), isEmpty);
      expect(_key('twitch-rule-page-7002'), findsNothing);
      await t.tap(_key('snack-undo'));
      await settle(t, 6);
      final back = (await archive.rules(s.archiveProfileId)).single;
      expect((back.userId, back.keepLast), ('7002', 5));
      await _close(t);
    });

    testWidgets('a failed recording leaves the list when not undone', (
      t,
    ) async {
      setSurface(t, const Size(1280, 800));
      final archive = FakeTwitchArchive(
        recordings: [
          TwitchRecordingItem(
            jobId: 50,
            userId: '7002',
            login: 'lunaforge',
            streamId: '90050',
            state: TwitchDownloadState.failed,
            createdAt: demoNow,
          ),
        ],
      );
      final s = await demoServices(twitchArchive: archive);
      await t.pumpWidget(testApp(s, section: Section.twitch));
      await settle(t);
      await _push(t, const TwitchArchiveScreen(refresh: Duration(hours: 1)));
      await t.tap(_key('twitch-recording-50-remove'));
      await settle(t, 4);
      expect(_key('twitch-recording-50'), findsNothing);
      expect(archive.actions, isEmpty, reason: 'waits for «Отменить»');
      await t.pump(const Duration(seconds: 9));
      await settle(t, 4);
      expect(archive.actions, [(50, TwitchDownloadAction.remove)]);
      await _close(t);
    });

    testWidgets('a failed download, undone', (t) async {
      setSurface(t, const Size(1280, 900));
      final item = LibraryItem(
        id: 'm1',
        kind: ItemKind.movie,
        title: 'Arrival',
        filePath: 'smb://nas/media/Arrival.mkv',
        streamUri: 'http://127.0.0.1:1/t/1/Arrival.mkv',
        addedAt: demoNow,
      );
      final backend = FakeOfflineBackend(titles: {'m1': 'Arrival'});
      final s = AppServices(
        library: FakeLibrary.items([item]),
        secureStore: MemorySecureStore(),
        playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
        offline: OfflineController(backend),
      );
      await s.init();
      s.locale.value = const Locale('ru');
      await s.offline!.download(const OfflineTarget.item('m1'));
      backend.fail();
      await t.pumpWidget(testApp(s));
      await settle(t, 4);
      await _push(t, const DownloadsScreen());
      expect(find.text('Arrival'), findsOneWidget);
      await t.tap(_key('download-j100-remove'));
      await settle(t, 4);
      expect(find.text('Arrival'), findsNothing);
      expect(find.text(ru.offlineRemoved), findsOneWidget);
      await capture(t, 'qa-015-download-removed-undo');
      await t.tap(_key('snack-undo'));
      await settle(t, 6);
      expect(find.text('Arrival'), findsOneWidget);
      expect(backend.items.single.state, OfflineState.failed);
      await _close(t);
    });
  });

  group('a held OK opens the context menu (TV-002)', () {
    testWidgets('short press activates, long press opens the menu', (t) async {
      var activated = 0;
      var menus = 0;
      await t.pumpWidget(
        _bare(
          Center(
            child: KFocusable(
              autofocus: true,
              onActivate: () => activated++,
              onContextMenu: (_) => menus++,
              builder: (_, _) => const SizedBox(width: 120, height: 80),
            ),
          ),
          ff: FormFactor.tv,
        ),
      );
      await settle(t, 2);
      for (final key in [
        LogicalKeyboardKey.select,
        LogicalKeyboardKey.enter,
        LogicalKeyboardKey.gameButtonA,
      ]) {
        await t.sendKeyDownEvent(key);
        await t.pump(const Duration(milliseconds: 120));
        await t.sendKeyUpEvent(key);
        await t.pump();
      }
      expect((activated, menus), (3, 0));

      await t.sendKeyDownEvent(LogicalKeyboardKey.select);
      await t.pump(const Duration(milliseconds: 300));
      await t.sendKeyRepeatEvent(LogicalKeyboardKey.select);
      await t.pump(const Duration(milliseconds: 300));
      expect(menus, 1, reason: 'opens while still held');
      await t.sendKeyUpEvent(LogicalKeyboardKey.select);
      await t.pump();
      expect((activated, menus), (3, 1), reason: 'no activation on release');
    });

    testWidgets('without a menu OK activates on the press', (t) async {
      var activated = 0;
      await t.pumpWidget(
        _bare(
          Center(
            child: KFocusable(
              autofocus: true,
              onActivate: () => activated++,
              builder: (_, _) => const SizedBox(width: 120, height: 80),
            ),
          ),
          ff: FormFactor.tv,
        ),
      );
      await settle(t, 2);
      await t.sendKeyDownEvent(LogicalKeyboardKey.select);
      await t.pump();
      expect(activated, 1);
      await t.sendKeyUpEvent(LogicalKeyboardKey.select);
    });

    testWidgets('a poster in the library on TV', (t) async {
      setSurface(t, const Size(1920, 1080));
      final s = await demoServices();
      s.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(s, formFactor: FormFactor.tv, section: Section.movies),
      );
      await settle(t, 10);
      final poster = find
          .byType(KFocusable)
          .evaluate()
          .firstWhere((e) => (e.widget as KFocusable).onContextMenu != null);
      final inner = find.descendant(
        of: find.byWidget(poster.widget),
        matching: find.byType(GestureDetector),
      );
      Focus.of(t.element(inner.first)).requestFocus();
      await settle(t, 4);
      await t.sendKeyDownEvent(LogicalKeyboardKey.select);
      await t.pump(const Duration(milliseconds: 600));
      await settle(t, 6);
      expect(_key('item-menu-play'), findsOneWidget);
      await capture(t, 'qa-015-tv-long-ok-menu');
      await t.sendKeyUpEvent(LogicalKeyboardKey.select);
      await settle(t, 2);
      await t.sendKeyEvent(LogicalKeyboardKey.escape);
      await settle(t, 4);
      await _close(t);
    });
  });

  testWidgets('«Мои видео» on TV ends with a «Все» tile', (t) async {
    setSurface(t, const Size(1920, 1080));
    final library = FakeLibrary.demo(scanStepDelay: Duration.zero)
      ..ownVideoList.addAll([
        for (var i = 1; i <= 3; i++)
          OwnVideo(
            fileId: i,
            sourceId: 1,
            relPath: 'Телефон/DCIM/VID_$i.mp4',
            localPath: '/media/Телефон/DCIM/VID_$i.mp4',
            title: 'VID_$i',
            sizeBytes: 40 << 20,
            durationMs: 60000,
            takenAt: DateTime(2024, 6, i),
          ),
      ]);
    final s = await demoServices(library: library);
    s.locale.value = const Locale('ru');
    await t.pumpWidget(testApp(s, formFactor: FormFactor.tv));
    await settle(t, 8);
    final row = _key('home-own-videos');
    await t.scrollUntilVisible(
      row,
      300,
      scrollable: find.byType(Scrollable).first,
    );
    await settle(t, 4);
    // No header link on TV; the tile ends the row instead.
    expect(
      find.descendant(of: row, matching: find.byType(TextButton)),
      findsNothing,
    );
    final tile = find.descendant(of: row, matching: _key('row-see-all-tile'));
    expect(tile, findsOneWidget);
    await capture(t, 'qa-015-tv-own-videos-see-all');
    final gesture = find.descendant(
      of: tile,
      matching: find.byType(GestureDetector),
    );
    Focus.of(t.element(gesture.first)).requestFocus();
    await settle(t, 4);
    await t.sendKeyEvent(LogicalKeyboardKey.select);
    await settle(t, 8);
    expect(find.byType(OwnVideosScreen), findsOneWidget);
    await _close(t);
  });
}
