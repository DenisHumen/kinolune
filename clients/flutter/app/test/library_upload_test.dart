// SPDX-License-Identifier: MPL-2.0
//
// LIB-027: «Закинуть файл прямо через приложение». Sorting a drop (movie,
// episode, music, Blu-ray/DVD folder, ISO, photos), the kind folders of a
// source (the ones there, else «Movies» / «TV Shows» / «Music»), keep-both
// on a taken name, the refresh that recognizes only the new files, resume
// after the network dropped, no space; on desktop the drop onto the window
// (hint, the source asked once and remembered, progress on «Загрузки»);
// on Android a share of several files with a fake intent; «Добавить в
// медиатеку» on «Загрузки». With KINOLUNE_SCREENSHOTS_DIR set the frames
// are written (in Russian):
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/lib-027 \
//     flutter test test/library_upload_test.dart

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_personal_media.dart';
import 'package:kinolune/fakes/fake_source_files.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/files/files_controller.dart';
import 'package:kinolune/files/library_upload.dart';
import 'package:kinolune/files/share_intake.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/downloads_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_library/pa_library.dart' show FilesError, TouchedDir;

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

Finder _key(String k) => find.byKey(ValueKey(k));

/// Local folders: path → its files (relative, `/`); anything else is a
/// file.
class _Fs implements LocalFs {
  _Fs([this.dirs = const {}]);

  final Map<String, List<String>> dirs;

  @override
  bool isDirectory(String path) => dirs.containsKey(path);

  @override
  List<String> filesUnder(String dir) => dirs[dir] ?? const [];

  @override
  String join(String dir, String rel) => '$dir/$rel';
}

const _bluRay = '/home/u/Discs/Наш отпуск';
const _bluRayFiles = [
  'BDMV/index.bdmv',
  'BDMV/MovieObject.bdmv',
  'BDMV/STREAM/00000.m2ts',
  'BDMV/PLAYLIST/00000.mpls',
  'CERTIFICATE/id.bdmv',
  '.DS_Store',
];

/// The library's folders on the fake NAS (source 7).
FakeSourceFiles _nas({int size = 1000}) =>
    FakeSourceFiles(now: () => DateTime(2026, 11, 24, 12), sizeOf: (_) => size)
      ..addFolder(7, 'Фильмы')
      ..addFolder(7, 'Сериалы')
      ..addFile(7, 'Фильмы/Дюна (2021).mkv', 1000);

FilesController _controller(
  FakeSourceFiles files, {
  List<Duration> retry = const [Duration(milliseconds: 1)],
}) => FilesController(
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
  retryDelays: retry,
);

/// Waits until no upload runs, waits, or waits for its automatic retry.
Future<void> _drain(FilesController c) async {
  bool busy() =>
      c.pendingUploads > 0 || c.libraryUploads.any((j) => j.retryAt != null);
  for (var i = 0; i < 200 && busy(); i++) {
    await Future<void>.delayed(const Duration(milliseconds: 2));
  }
}

void main() {
  tearDown(() {
    libraryLocalFs = const IoLocalFs();
    pickLibrary = systemPickLibrary;
  });

  group('sorting', () {
    test('files by name', () {
      expect(kindOfFile('Dune.2021.2160p.mkv'), DropKind.movie);
      expect(kindOfFile('Severance.S02E05.1080p.WEB.mkv'), DropKind.series);
      expect(kindOfFile('[Group] Frieren - 05 [1080p].mkv'), DropKind.series);
      expect(kindOfFile('Show 1x02.mp4'), DropKind.series);
      // A resolution is not an episode.
      expect(kindOfFile('Movie 1920x1080.mkv'), DropKind.movie);
      expect(kindOfFile('01 - Song.flac'), DropKind.music);
      expect(kindOfFile('Disc.iso'), DropKind.disc);
      expect(kindOfFile('VID_20240101_120000.mp4'), DropKind.personal);
      expect(kindOfFile('PXL_20250312_101010.mp4'), DropKind.personal);
      expect(kindOfFile('IMG_0001.HEIC'), DropKind.personal);
      expect(kindOfFile('report.pdf'), DropKind.unsupported);
    });

    test('the show of an episode', () {
      expect(showNameOf('Severance.S02E05.1080p.WEB.mkv'), 'Severance');
      expect(showNameOf('The_Last_of_Us_S01E01.mkv'), 'The Last of Us');
      expect(showNameOf('[Group] Frieren - 05 [1080p].mkv'), 'Frieren');
      expect(showNameOf('S01E01.mkv'), '');
    });

    test('folders: disc, the disc structure itself, show, album', () {
      final fs = _Fs({
        _bluRay: _bluRayFiles,
        '/x/Старый фильм/VIDEO_TS': ['VIDEO_TS.IFO', 'VTS_01_1.VOB'],
        '/x/Severance Season 2': ['e1.mkv', 'e2.mkv'],
        '/x/Album': ['01.flac', 'cover.jpg'],
        '/x/Docs': ['a.pdf'],
      });
      final disc = sortDropped(fs, const UploadRequest(_bluRay));
      expect(disc.kind, DropKind.disc);
      expect(disc.folderName, 'Наш отпуск');
      expect(disc.files, isNot(contains('.DS_Store')));
      final vts = sortDropped(
        fs,
        const UploadRequest('/x/Старый фильм/VIDEO_TS'),
      );
      expect(vts.kind, DropKind.disc);
      expect(vts.folderName, 'Старый фильм');
      expect(vts.files.first, 'VIDEO_TS/VIDEO_TS.IFO');
      expect(
        sortDropped(fs, const UploadRequest('/x/Severance Season 2')).kind,
        DropKind.series,
      );
      expect(
        sortDropped(fs, const UploadRequest('/x/Album')).kind,
        DropKind.music,
      );
      expect(
        sortDropped(fs, const UploadRequest('/x/Docs')).kind,
        DropKind.unsupported,
      );
    });
  });

  group('into the library (fake NAS)', () {
    test('each kind lands in its folder and is recognized there', () async {
      final files = _nas();
      final fs = _Fs({
        _bluRay: _bluRayFiles,
        '/x/Старый фильм/VIDEO_TS': ['VIDEO_TS.IFO', 'VTS_01_1.VOB'],
      });
      final plan = await LibraryPlanner(files, 7, fs: fs).plan(const [
        UploadRequest('/phone/Dune.2021.mkv'),
        UploadRequest('/phone/Dune.2021.ru.srt'),
        UploadRequest('/phone/Severance.S02E05.mkv'),
        UploadRequest('/phone/01 - Song.flac'),
        UploadRequest('/phone/Home.iso'),
        UploadRequest(_bluRay),
        UploadRequest('/x/Старый фильм/VIDEO_TS'),
        UploadRequest('/phone/IMG_0001.jpg'),
        UploadRequest('/phone/notes.pdf'),
      ]);
      String dirOf(String name) =>
          plan.uploads.firstWhere((u) => u.request.name == name).dir;
      // The folders already there ...
      expect(dirOf('Dune.2021.mkv'), 'Фильмы');
      expect(dirOf('Dune.2021.ru.srt'), 'Фильмы');
      expect(dirOf('Severance.S02E05.mkv'), 'Сериалы/Severance');
      expect(dirOf('Home.iso'), 'Фильмы');
      // ... and a new one for music.
      expect(dirOf('01 - Song.flac'), 'Music');
      expect(dirOf('00000.m2ts'), 'Фильмы/Наш отпуск/BDMV/STREAM');
      expect(dirOf('id.bdmv'), 'Фильмы/Наш отпуск/CERTIFICATE');
      expect(dirOf('VTS_01_1.VOB'), 'Фильмы/Старый фильм/VIDEO_TS');
      expect(
        plan.uploads
            .firstWhere((u) => u.request.name == 'VTS_01_1.VOB')
            .request
            .localPath,
        '/x/Старый фильм/VIDEO_TS/VTS_01_1.VOB',
      );
      expect(plan.personal.single.name, 'IMG_0001.jpg');
      expect(plan.skipped, ['notes.pdf']);

      final c = _controller(files);
      final jobs = await enqueuePlan(c, 7, plan);
      expect(jobs.every((j) => j.toLibrary), isTrue);
      await _drain(c);
      expect(files.exists(7, 'Фильмы/Dune.2021.mkv'), isTrue);
      expect(files.exists(7, 'Сериалы/Severance/Severance.S02E05.mkv'), isTrue);
      expect(files.exists(7, 'Music/01 - Song.flac'), isTrue);
      expect(files.exists(7, 'Фильмы/Наш отпуск/BDMV/index.bdmv'), isTrue);
      expect(
        files.exists(7, 'Фильмы/Старый фильм/VIDEO_TS/VIDEO_TS.IFO'),
        true,
      );
      // Only the folders that got new files are re-listed (and recognized).
      final touched = {
        for (final r in files.refreshes)
          for (final t in r.$2) t,
      };
      expect(touched, contains(const TouchedDir('Сериалы/Severance')));
      expect(touched, isNot(contains(const TouchedDir(''))));
    });

    test('a taken name keeps both', () async {
      final files = _nas()..addFolder(7, 'Фильмы/Наш отпуск');
      final plan =
          await LibraryPlanner(files, 7, fs: _Fs({_bluRay: _bluRayFiles})).plan(
            const [UploadRequest(_bluRay), UploadRequest('/p/Дюна (2021).mkv')],
          );
      final c = _controller(files);
      await enqueuePlan(c, 7, plan);
      await _drain(c);
      expect(files.exists(7, 'Фильмы/Наш отпуск (2)/BDMV/index.bdmv'), isTrue);
      expect(files.exists(7, 'Фильмы/Дюна (2021) (2).mkv'), isTrue);
      expect(files.sizeAt(7, 'Фильмы/Дюна (2021).mkv'), 1000);
    });

    test('a dropped network resumes by itself from the .part', () async {
      final files = _nas(size: 4000)..dropNextUploadAfter = 2000;
      final c = _controller(files);
      final plan = await LibraryPlanner(
        files,
        7,
        fs: _Fs(),
      ).plan(const [UploadRequest('/p/Big.Movie.2020.mkv')]);
      await enqueuePlan(c, 7, plan);
      // The first try loses the connection half way ...
      // Wait by wall time, not a fixed number of event-queue turns: on a
      // slow CI runner 50 turns were not always enough for the first try.
      final deadline = DateTime.now().add(const Duration(seconds: 10));
      while (files.sizeAt(7, 'Фильмы/Big.Movie.2020.mkv.part') != 2000 &&
          DateTime.now().isBefore(deadline)) {
        await pumpEventQueue();
        await Future<void>.delayed(const Duration(milliseconds: 5));
      }
      expect(files.sizeAt(7, 'Фильмы/Big.Movie.2020.mkv.part'), 2000);
      // ... and goes on by itself from there.
      await _drain(c);
      final job = c.libraryUploads.single;
      expect(job.state, UploadState.done);
      expect(files.resumes['Фильмы/Big.Movie.2020.mkv'], 2000);
      expect(files.exists(7, 'Фильмы/Big.Movie.2020.mkv'), isTrue);
      expect(files.exists(7, 'Фильмы/Big.Movie.2020.mkv.part'), isFalse);
      expect(files.exists(7, 'Фильмы/Big.Movie.2020 (2).mkv'), isFalse);
    });

    test('after the last automatic retry it waits for «Повторить»', () async {
      final files = _nas()..failNextUpload = FilesError.unavailable;
      final c = _controller(files, retry: const []);
      final plan = await LibraryPlanner(
        files,
        7,
        fs: _Fs(),
      ).plan(const [UploadRequest('/p/A.Movie.mkv')]);
      await enqueuePlan(c, 7, plan);
      await _drain(c);
      final job = c.libraryUploads.single;
      expect(job.state, UploadState.failed);
      expect(job.retryAt, isNull);
      c.retry(job);
      await _drain(c);
      expect(job.state, UploadState.done);
    });

    test('no space stops the batch for that source', () async {
      final files = _nas()..freeBytes = 1500;
      final c = _controller(files);
      final plan = await LibraryPlanner(files, 7, fs: _Fs()).plan(const [
        UploadRequest('/p/One.mkv'),
        UploadRequest('/p/Two.mkv'),
        UploadRequest('/p/Three.mkv'),
      ]);
      await enqueuePlan(c, 7, plan);
      await _drain(c);
      final states = [for (final j in c.libraryUploads) j.state];
      expect(states, [
        UploadState.done,
        UploadState.failed,
        UploadState.failed,
      ]);
      expect(
        c.libraryUploads.skip(1).every((j) => j.error == FilesError.full),
        isTrue,
      );
      // Only the file that tried; the third never went.
      expect(files.uploads, hasLength(1));
      files.freeBytes = 1 << 30;
      for (final j in c.libraryUploads.skip(1)) {
        c.retry(j);
      }
      await _drain(c);
      expect(files.exists(7, 'Фильмы/Three.mkv'), isTrue);
    });
  });

  group('in the app', () {
    late FakeSourceFiles files;
    late AppServices s;
    late int nas;

    Future<void> open(
      WidgetTester t, {
      FormFactor formFactor = FormFactor.desktop,
      Size size = const Size(1280, 800),
      bool second = false,
    }) async {
      setSurface(t, size);
      files = FakeSourceFiles(
        now: () => DateTime(2026, 11, 24, 12),
        sizeOf: (_) => 1200 * 1000 * 1000,
      );
      s = await demoServices(
        personal: FakePersonalMedia.demo(),
        sourceManager: (lib) => FakeSourceManager(lib, numericIds: true),
        files: files,
        preferences: MemoryPreferences(),
      );
      s.locale.value = const Locale('ru');
      final made = await s.createSource(
        const SourceDraft(
          type: SourceType.smb,
          name: 'Домашний NAS',
          address: 'smb://nas.local/media',
        ),
      );
      nas = int.parse(made.id);
      files
        ..addFolder(nas, 'Фильмы')
        ..addFolder(nas, 'Сериалы');
      if (second) {
        await s.createSource(
          const SourceDraft(
            type: SourceType.webdav,
            name: 'Облачная папка',
            address: 'https://dav.example/media',
          ),
        );
      }
      await t.pumpWidget(testApp(s, formFactor: formFactor));
      await settle(t);
    }

    LibraryDropZoneState zone(WidgetTester t) =>
        t.state<LibraryDropZoneState>(find.byType(LibraryDropZone));

    testWidgets('desktop: a drop asks the source once and shows progress', (
      t,
    ) async {
      await open(t, second: true);
      libraryLocalFs = _Fs({_bluRay: _bluRayFiles});
      zone(t).hover(true);
      await settle(t, 2);
      expect(_key('library-drop-hint'), findsOneWidget);
      expect(find.text(ru.libUploadDropHint), findsOneWidget);
      await capture(t, 'desktop-drop-hint');

      final hold = Completer<void>();
      files.holdUploads = hold;
      unawaited(zone(t).drop(['/home/u/Видео/Severance.S02E05.mkv', _bluRay]));
      await settle(t, 4);
      expect(_key('library-drop-hint'), findsNothing);
      // Two writable sources: asked once.
      expect(_key('library-upload-source-dialog'), findsOneWidget);
      await capture(t, 'desktop-choose-source');
      await t.tap(_key('library-upload-source-$nas'));
      await settle(t, 6);
      expect(await s.preferences.read(FilesPrefs.librarySource), '$nas');

      // Progress on «Загрузки».
      final context = t.element(find.byType(LibraryDropZone));
      unawaited(
        Navigator.of(context).push(
          MaterialPageRoute<void>(builder: (_) => const DownloadsScreen()),
        ),
      );
      await settle(t, 6);
      expect(_key('library-uploads'), findsOneWidget);
      expect(find.text(ru.libUploadSectionTitle), findsOneWidget);
      expect(find.byType(LinearProgressIndicator), findsWidgets);
      expect(_key('library-upload-Severance.S02E05.mkv'), findsOneWidget);
      for (final m in t.stateList<ScaffoldMessengerState>(
        find.byType(ScaffoldMessenger),
      )) {
        m.removeCurrentSnackBar();
      }
      await t.pump();
      await capture(t, 'desktop-downloads-uploading');
      hold.complete();
      files.holdUploads = null;
      await settle(t, 30);
      expect(files.exists(nas, 'Сериалы/Severance/Severance.S02E05.mkv'), true);
      expect(
        files.exists(nas, 'Фильмы/Наш отпуск/BDMV/STREAM/00000.m2ts'),
        true,
      );
      expect(find.text(ru.libUploadDone('Сериалы/Severance')), findsOneWidget);
      await capture(t, 'desktop-downloads-done');

      // The next drop does not ask again.
      Navigator.of(context).pop();
      await settle(t, 4);
      unawaited(zone(t).drop(['/home/u/Видео/Dune.2021.mkv']));
      await settle(t, 20);
      expect(_key('library-upload-source-dialog'), findsNothing);
      expect(files.exists(nas, 'Фильмы/Dune.2021.mkv'), isTrue);
    });

    testWidgets('desktop: a dropped network says so and resumes', (t) async {
      await open(t);
      libraryLocalFs = _Fs();
      files.dropNextUploadAfter = 600 * 1000 * 1000;
      unawaited(zone(t).drop(['/home/u/Видео/Big.Movie.2020.mkv']));
      await settle(t, 6);
      final context = t.element(find.byType(LibraryDropZone));
      unawaited(
        Navigator.of(context).push(
          MaterialPageRoute<void>(builder: (_) => const DownloadsScreen()),
        ),
      );
      await settle(t, 4);
      expect(find.text(ru.libUploadWillResume), findsOneWidget);
      await capture(t, 'desktop-downloads-resume');
      // The first automatic retry comes after 10 s.
      await t.pump(const Duration(seconds: 11));
      await settle(t, 10);
      expect(files.resumes['Фильмы/Big.Movie.2020.mkv'], 600 * 1000 * 1000);
      expect(files.exists(nas, 'Фильмы/Big.Movie.2020.mkv'), isTrue);
      expect(find.text(ru.libUploadDone('Фильмы')), findsOneWidget);
    });

    testWidgets('Android: a share of several files goes where each belongs', (
      t,
    ) async {
      await open(t, formFactor: FormFactor.phone, size: const Size(412, 900));
      libraryLocalFs = _Fs();
      const channel = MethodChannel('test/library-share');
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(channel, (call) async {
        if (call.method == 'take') return <String>[];
        return null;
      });
      addTearDown(() => messenger.setMockMethodCallHandler(channel, null));
      final shell = t.state<AppShellState>(find.byType(AppShell));
      final intake = ShareIntake(
        channel: channel,
        onShared: shell.handleShared,
      );
      addTearDown(intake.dispose);
      await intake.start();
      // The system hands over a share of four files (ACTION_SEND_MULTIPLE),
      // copied into the app's cache.
      await messenger.handlePlatformMessage(
        channel.name,
        const StandardMethodCodec().encodeMethodCall(
          const MethodCall('shared', [
            '/cache/shared/1/Severance.S02E06.mkv',
            '/cache/shared/1/Dune.2021.mkv',
            '/cache/shared/1/02 - Song.mp3',
            '/cache/shared/1/IMG_0042.jpg',
          ]),
        ),
        (_) {},
      );
      await settle(t, 30);
      expect(files.exists(nas, 'Сериалы/Severance/Severance.S02E06.mkv'), true);
      expect(files.exists(nas, 'Фильмы/Dune.2021.mkv'), isTrue);
      expect(files.exists(nas, 'Music/02 - Song.mp3'), isTrue);
      // The photo went to «Личное», not to the library.
      final photo = files.uploads.where((u) => u.$4 == 'IMG_0042.jpg').single;
      expect(photo.$3, isNot(anyOf('Фильмы', 'Сериалы', 'Music')));
      expect(
        s.filesControl!.libraryUploads.map((j) => j.name),
        isNot(contains('IMG_0042.jpg')),
      );
      // Only one writable source: nothing was asked.
      expect(_key('library-upload-source-dialog'), findsNothing);
      final libraryJobs = s.filesControl!.libraryUploads;
      expect(libraryJobs, hasLength(3));
      expect(libraryJobs.every((j) => j.ownedCopy), isTrue);
      final context = t.element(find.byType(LibraryDropZone));
      unawaited(
        Navigator.of(context).push(
          MaterialPageRoute<void>(builder: (_) => const DownloadsScreen()),
        ),
      );
      await settle(t, 6);
      expect(_key('library-uploads'), findsOneWidget);
      await capture(t, 'phone-downloads-shared');
    });

    testWidgets('«Добавить в медиатеку» on «Загрузки» picks a folder', (
      t,
    ) async {
      await open(t, formFactor: FormFactor.phone, size: const Size(412, 900));
      libraryLocalFs = _Fs({_bluRay: _bluRayFiles});
      var asked = <bool>[];
      pickLibrary = ({required bool folder}) async {
        asked = [...asked, folder];
        return folder ? [const UploadRequest(_bluRay)] : const [];
      };
      final context = t.element(find.byType(LibraryDropZone));
      unawaited(
        Navigator.of(context).push(
          MaterialPageRoute<void>(builder: (_) => const DownloadsScreen()),
        ),
      );
      await settle(t, 6);
      await t.tap(_key('library-upload-add'));
      await settle(t, 4);
      expect(_key('library-upload-files'), findsOneWidget);
      await capture(t, 'phone-add-menu');
      await t.tap(_key('library-upload-folder'));
      await settle(t, 30);
      expect(asked, [true]);
      expect(files.exists(nas, 'Фильмы/Наш отпуск/BDMV/index.bdmv'), isTrue);
    });
  });
}
