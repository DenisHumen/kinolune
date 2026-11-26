// SPDX-License-Identifier: MPL-2.0
//
// LIB-028: «Закинуть файл — и он сразу виден на всех устройствах». The
// cards of the family's uploads (this device's queue and the others'
// steps and ticks), the grey card in the library on a phone, a computer
// and a TV in Russian, English and Ukrainian (grey, live bar, cannot be
// opened, offline, done, cancel), the guess right after a drop with
// «Не то». With KINOLUNE_SCREENSHOTS_DIR set the frames are written:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/lib-028 \
//     flutter test test/live_upload_test.dart

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_personal_media.dart';
import 'package:kinolune/fakes/fake_source_files.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/files/files_controller.dart';
import 'package:kinolune/files/library_upload.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/library/live_upload_card.dart';
import 'package:kinolune/library/live_uploads.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

Finder _key(String k) => find.byKey(ValueKey(k));

class _Fs implements LocalFs {
  @override
  bool isDirectory(String path) => false;

  @override
  List<String> filesUnder(String dir) => const [];

  @override
  String join(String dir, String rel) => '$dir/$rel';
}

const _gb = 1000 * 1000 * 1000;

/// Another device's upload: «Дюна: Часть вторая» from «Desktop».
LiveUploadRecord _remote({
  String id = 'u1',
  LiveUploadPhase phase = LiveUploadPhase.started,
  LiveUploadKind kind = LiveUploadKind.movie,
}) => LiveUploadRecord(
  id: id,
  phase: phase,
  deviceId: 'pc',
  info: LiveUploadInfo(
    name: kind == LiveUploadKind.series
        ? 'Severance.S02E03.mkv'
        : 'Dune.Part.Two.2024.2160p.mkv',
    kind: kind,
    size: 2900 * 1000 * 1000,
    dir: kind == LiveUploadKind.series ? 'TV Shows/Severance' : 'Movies',
    deviceName: 'Desktop',
    guess: kind == LiveUploadKind.series
        ? const LiveUploadGuess(title: 'Severance', season: 2, episode: 3)
        : const LiveUploadGuess(title: 'Dune: Part Two', year: 2024),
  ).toJson(),
);

LiveUploadTick _tick(
  int done, {
  String id = 'u1',
  LiveUploadState state = LiveUploadState.copying,
}) => LiveUploadTick(
  id: id,
  deviceId: 'pc',
  progress: LiveUploadProgress(
    state: state,
    done: done,
    size: 2900 * 1000 * 1000,
    rate: 85 * 1000 * 1000,
  ),
);

Future<void> _drain(FilesController c) async {
  for (var i = 0; i < 200 && c.pendingUploads > 0; i++) {
    await Future<void>.delayed(const Duration(milliseconds: 2));
  }
}

FilesController _controller(FakeSourceFiles files) => FilesController(
  files: files,
  preferences: MemoryPreferences(),
  sources: () => const [
    ManagedSource(
      id: '7',
      type: SourceType.smb,
      name: 'NAS',
      address: 'smb://nas/media',
    ),
  ],
  fileSize: (_) async => 1000,
  deleteLocal: (_) async {},
);

void main() {
  tearDown(() => libraryLocalFs = const IoLocalFs());

  group('the cards', () {
    test('a drop: started, live progress, done; the guess is made on the '
        'destination', () async {
      final files = FakeSourceFiles(sizeOf: (_) => 1000)
        ..addFolder(7, 'Movies');
      final c = _controller(files);
      final link = MemoryLiveUploadLink(deviceId: 'kpc');
      final live = LiveUploads(link: link, files: c, deviceName: () => 'PC');
      await live.start();
      final hold = Completer<void>();
      files.holdUploads = hold;
      final jobs = await c.enqueue(
        [const UploadRequest('/home/u/Dune.Part.Two.2024.mkv')],
        sourceId: 7,
        dir: 'Movies',
        toLibrary: true,
      );
      final card = await live.began(
        jobs: jobs,
        name: 'Dune.Part.Two.2024.mkv',
        kind: LiveUploadKind.movie,
        dest: 'Movies/Dune.Part.Two.2024.mkv',
      );
      expect(card.info.value.guess.title, 'Dune Part Two');
      expect(card.info.value.guess.year, 2024);
      expect(card.info.value.deviceName, 'PC');
      expect((await link.records()).single.phase, LiveUploadPhase.started);
      await Future<void>.delayed(const Duration(milliseconds: 20));
      // Half way (the fake holds there): the core got every report.
      expect(link.reports[card.id]!.state, LiveUploadState.copying);
      expect(link.reports[card.id]!.done, 500);
      expect(live.forPage(shows: false), [card]);
      expect(live.forPage(shows: true), isEmpty);
      hold.complete();
      files.holdUploads = null;
      await _drain(c);
      await Future<void>.delayed(const Duration(milliseconds: 20));
      expect(link.reports[card.id]!.state, LiveUploadState.done);
      expect((await link.records()).single.phase, LiveUploadPhase.done);
      expect(card.progress.value.percent, 100);
    });

    test('cancel here, and a cancel asked by another device', () async {
      final files = FakeSourceFiles(sizeOf: (_) => 1000);
      final c = _controller(files);
      final link = MemoryLiveUploadLink(deviceId: 'kpc');
      final live = LiveUploads(link: link, files: c);
      await live.start();
      files.holdUploads = Completer<void>();
      Future<LiveUpload> drop(String name) async => live.began(
        jobs: await c.enqueue(
          [UploadRequest('/home/u/$name')],
          sourceId: 7,
          dir: 'Movies',
          toLibrary: true,
        ),
        name: name,
        kind: LiveUploadKind.movie,
        dest: 'Movies/$name',
      );
      final a = await drop('A.2001.mkv');
      final b = await drop('B.2002.mkv');
      await live.cancel(b);
      expect(link.reports[b.id]!.state, LiveUploadState.cancelled);
      expect(
        (await link.records()).firstWhere((r) => r.id == b.id).phase,
        LiveUploadPhase.cancelled,
      );
      expect(live.byId(b.id), isNull, reason: 'the card went away');
      // A phone asks to cancel the other one.
      link.remoteCancel([a.id], 'kphone');
      await Future<void>.delayed(const Duration(milliseconds: 20));
      expect(link.reports[a.id]!.state, LiveUploadState.cancelled);
      expect(c.libraryUploads, isEmpty);
    });

    test('another device: the card, its ticks, offline, done', () async {
      final link = MemoryLiveUploadLink(deviceId: 'kphone');
      final live = LiveUploads(link: link);
      await live.start();
      var lists = 0;
      live.addListener(() => lists++);
      link.remoteRecord(_remote());
      await Future<void>.delayed(Duration.zero);
      final card = live.byId('u1')!;
      expect(card.local, isFalse);
      expect(card.info.value.deviceName, 'Desktop');
      final listsAfterCard = lists;
      var repaints = 0;
      card.progress.addListener(() => repaints++);
      for (var i = 1; i <= 10; i++) {
        link.remoteTick(_tick(i * 100 * 1000 * 1000));
        await Future<void>.delayed(Duration.zero);
      }
      expect(repaints, 10, reason: 'each tick repaints the card');
      expect(lists, listsAfterCard, reason: 'never the list');
      expect(card.progress.value.done, 1000 * 1000 * 1000);
      link.remoteTick(_tick(1 * _gb, state: LiveUploadState.offline));
      await Future<void>.delayed(Duration.zero);
      expect(card.progress.value.state, LiveUploadState.offline);
      link.remoteRecord(_remote(phase: LiveUploadPhase.done));
      await Future<void>.delayed(Duration.zero);
      expect(card.progress.value.state, LiveUploadState.done);
      // A cancelled one goes away everywhere.
      link.remoteRecord(_remote(id: 'u2'));
      await Future<void>.delayed(Duration.zero);
      expect(live.byId('u2'), isNotNull);
      link.remoteRecord(_remote(id: 'u2', phase: LiveUploadPhase.cancelled));
      await Future<void>.delayed(Duration.zero);
      expect(live.byId('u2'), isNull);
    });

    test('the copying devices are asked for 10 Hz only while a card is '
        'on screen', () {
      final link = MemoryLiveUploadLink();
      final live = LiveUploads(link: link)..addWatcher();
      expect(link.watching, isTrue);
      live
        ..addWatcher()
        ..removeWatcher();
      expect(link.watching, isTrue);
      live.removeWatcher();
      expect(link.watching, isFalse);
    });

    test('status lines', () {
      final ru = lookupAppLocalizations(const Locale('ru'));
      const locale = Locale('ru');
      final u = LiveUpload(
        id: 'u1',
        deviceId: 'pc',
        local: false,
        info: LiveUploadInfo.fromJson(_remote().info)!,
      );
      final (status, detail) = liveUploadStatus(
        ru,
        locale,
        u,
        _tick(1200 * 1000 * 1000).progress,
      );
      expect(status, ru.liveUploadFrom('Desktop', '41'));
      expect(detail, contains(ru.liveUploadEtaSeconds(20)));
      expect(liveUploadEta(ru, const Duration(minutes: 4)), 'ещё 4 мин');
      expect(
        liveUploadStatus(
          ru,
          locale,
          u,
          _tick(5, state: LiveUploadState.offline).progress,
        ).$1,
        'Пауза: Desktop не в сети',
      );
    });
  });

  group('the grey card', () {
    late MemoryLiveUploadLink link;

    Future<AppServices> open(
      WidgetTester t, {
      required FormFactor formFactor,
      required Size size,
      required String lang,
      Section section = Section.movies,
    }) async {
      setSurface(t, size);
      link = MemoryLiveUploadLink(deviceId: 'kphone');
      final s = await demoServices(liveUploadLink: link);
      s.locale.value = Locale(lang);
      link
        ..remoteRecord(_remote())
        ..remoteTick(_tick(1200 * 1000 * 1000));
      await t.pumpWidget(testApp(s, formFactor: formFactor, section: section));
      await settle(t);
      return s;
    }

    const forms = {
      'phone': (FormFactor.phone, Size(412, 900)),
      'desktop': (FormFactor.desktop, Size(1280, 800)),
      'tv': (FormFactor.tv, Size(1920, 1080)),
    };

    for (final MapEntry(key: form, value: (factor, size)) in forms.entries) {
      for (final lang in const ['ru', 'en', 'uk']) {
        testWidgets('$form, $lang: grey, live, cannot be opened, offline', (
          t,
        ) async {
          final l = lookupAppLocalizations(Locale(lang));
          await open(t, formFactor: factor, size: size, lang: lang);
          expect(_key('live-upload-u1'), findsOneWidget);
          expect(
            find.text(l.liveUploadFrom('Desktop', '41')),
            findsOneWidget,
          );
          expect(find.textContaining(l.liveUploadEtaSeconds(20)), findsOneWidget);
          expect(find.text('Dune: Part Two'), findsWidgets);
          // Grey and see-through while it is copied.
          final grey = t.widget<AnimatedOpacity>(
            find
                .descendant(
                  of: _key('live-upload-u1'),
                  matching: find.byType(AnimatedOpacity),
                )
                .first,
          );
          expect(grey.opacity, lessThan(.5));
          // Somebody watches: the copying device goes to 10 Hz.
          expect(link.watching, isTrue);
          await capture(t, '$form-$lang-copying');

          // The bar moves.
          link.remoteTick(_tick(2000 * 1000 * 1000));
          await settle(t, 4);
          expect(find.text(l.liveUploadFrom('Desktop', '68')), findsOneWidget);

          // It cannot be opened yet.
          await t.tap(
            find
                .descendant(
                  of: _key('live-upload-u1'),
                  matching: find.byType(AspectRatio),
                )
                .first,
          );
          await settle(t, 3);
          expect(find.text(l.liveUploadUnavailable), findsOneWidget);
          for (final m in t.stateList<ScaffoldMessengerState>(
            find.byType(ScaffoldMessenger),
          )) {
            m.removeCurrentSnackBar();
          }
          await settle(t, 3);

          // The device that copies dropped off the network.
          link.remoteTick(
            _tick(2000 * 1000 * 1000, state: LiveUploadState.offline),
          );
          await settle(t, 4);
          expect(find.text(l.liveUploadOffline('Desktop')), findsOneWidget);
          await capture(t, '$form-$lang-offline');

          // Done: no longer grey; it gives way to the item once this
          // library has it.
          link.remoteRecord(_remote(phase: LiveUploadPhase.done));
          await settle(t, 6);
          expect(find.text(l.liveUploadAdding), findsOneWidget);
          final normal = t.widget<AnimatedOpacity>(
            find
                .descendant(
                  of: _key('live-upload-u1'),
                  matching: find.byType(AnimatedOpacity),
                )
                .first,
          );
          expect(normal.opacity, 1);
          await capture(t, '$form-$lang-done');
        });
      }
    }

    testWidgets('an episode shows on the TV shows page', (t) async {
      final ru = lookupAppLocalizations(const Locale('ru'));
      setSurface(t, const Size(1280, 800));
      link = MemoryLiveUploadLink(deviceId: 'kphone');
      final s = await demoServices(liveUploadLink: link);
      s.locale.value = const Locale('ru');
      link.remoteRecord(_remote(id: 's1', kind: LiveUploadKind.series));
      await t.pumpWidget(testApp(s, section: Section.shows));
      await settle(t);
      expect(_key('live-upload-s1'), findsOneWidget);
      expect(find.text(ru.liveUploadEpisode(2, 3)), findsOneWidget);
      await capture(t, 'desktop-ru-episode');
    });

    testWidgets('an administrator cancels another device\'s copy', (t) async {
      final ru = lookupAppLocalizations(const Locale('ru'));
      await open(
        t,
        formFactor: FormFactor.desktop,
        size: const Size(1280, 800),
        lang: 'ru',
      );
      await t.longPress(
        find
            .descendant(
              of: _key('live-upload-u1'),
              matching: find.byType(AspectRatio),
            )
            .first,
      );
      await settle(t, 4);
      expect(find.text(ru.liveUploadCancel), findsOneWidget);
      await t.tap(_key('live-upload-cancel-u1'));
      await settle(t, 4);
      expect(link.cancelled, [('pc', 'u1')]);
    });
  });

  group('the guess right after a drop', () {
    testWidgets('desktop: «Это „…“?», nothing to press, «Не то» picks', (
      t,
    ) async {
      final ru = lookupAppLocalizations(const Locale('ru'));
      setSurface(t, const Size(1280, 800));
      final files = FakeSourceFiles(
        now: () => DateTime(2026, 11, 26, 12),
        sizeOf: (_) => 2900 * 1000 * 1000,
      );
      final link = MemoryLiveUploadLink(
        deviceId: 'kpc',
        candidates: const [
          LiveUploadCandidate(title: 'Дюна: Часть вторая', year: 2024),
          LiveUploadCandidate(title: 'Дюна', year: 2021),
        ],
      );
      final s = await demoServices(
        personal: FakePersonalMedia.demo(),
        sourceManager: (lib) => FakeSourceManager(lib, numericIds: true),
        files: files,
        preferences: MemoryPreferences(),
        liveUploadLink: link,
      );
      s.locale.value = const Locale('ru');
      final made = await s.createSource(
        const SourceDraft(
          type: SourceType.smb,
          name: 'Домашний NAS',
          address: 'smb://nas.local/media',
        ),
      );
      files.addFolder(int.parse(made.id), 'Фильмы');
      await t.pumpWidget(testApp(s, section: Section.movies));
      await settle(t);
      libraryLocalFs = _Fs();
      final hold = Completer<void>();
      files.holdUploads = hold;
      final zone = t.state<LibraryDropZoneState>(find.byType(LibraryDropZone));
      unawaited(zone.drop(['/home/u/Видео/Dune.Part.Two.2024.2160p.mkv']));
      await settle(t, 6);
      // The guess, before a byte is copied; no dialog.
      expect(_key('live-upload-guess'), findsOneWidget);
      expect(
        find.text(ru.liveUploadGuessYear('Dune Part Two', '2024')),
        findsOneWidget,
      );
      expect(find.byType(AlertDialog), findsNothing);
      final card = s.liveUploads.uploads.single;
      expect(_key('live-upload-${card.id}'), findsOneWidget);
      await capture(t, 'desktop-ru-guess');

      await t.tap(_key('live-upload-not-this'));
      await settle(t, 6);
      expect(_key('live-upload-picker'), findsOneWidget);
      await capture(t, 'desktop-ru-not-this');
      await t.enterText(_key('live-upload-picker-title'), 'Дюна');
      await t.tap(_key('live-upload-picker-search'));
      await settle(t, 4);
      await t.tap(_key('live-upload-candidate-0'));
      await settle(t, 4);
      expect(card.info.value.guess.title, 'Дюна: Часть вторая');
      expect(card.info.value.guess.picked, isTrue);
      expect(
        (await link.records()).single.phase,
        LiveUploadPhase.recognized,
      );
      expect(find.text('Дюна: Часть вторая'), findsWidgets);
      hold.complete();
      files.holdUploads = null;
      await settle(t, 30);
      expect((await link.records()).single.phase, LiveUploadPhase.done);
    });

    testWidgets('nothing pressed: the guess is taken by itself', (t) async {
      setSurface(t, const Size(412, 900));
      final files = FakeSourceFiles(sizeOf: (_) => 1000);
      final link = MemoryLiveUploadLink(deviceId: 'kphone');
      final s = await demoServices(
        sourceManager: (lib) => FakeSourceManager(lib, numericIds: true),
        files: files,
        preferences: MemoryPreferences(),
        liveUploadLink: link,
      );
      final made = await s.createSource(
        const SourceDraft(
          type: SourceType.smb,
          name: 'NAS',
          address: 'smb://nas.local/media',
        ),
      );
      files.addFolder(int.parse(made.id), 'Movies');
      await t.pumpWidget(testApp(s, formFactor: FormFactor.phone));
      await settle(t);
      libraryLocalFs = _Fs();
      final context = t.element(find.byType(LibraryDropZone));
      unawaited(
        addToLibrary(context, [
          const UploadRequest('/sdcard/Download/Oppenheimer.2023.mkv'),
        ]),
      );
      await settle(t, 6);
      expect(_key('live-upload-guess'), findsOneWidget);
      // Eight seconds later it is gone, and nothing asked anything.
      await settle(t, 100);
      expect(_key('live-upload-guess'), findsNothing);
      expect(find.byType(AlertDialog), findsNothing);
      expect((await link.records()).single.phase, LiveUploadPhase.done);
    });
  });
}
