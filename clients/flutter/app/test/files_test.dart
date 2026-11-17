// SPDX-License-Identifier: MPL-2.0
//
// UI-035: the file view «как в Nextcloud» — upload (several files, queue,
// progress, resume after a restart), new folder, rename, «Переместить в…»
// and drag and drop on desktop, copy, the trash with «Отменить», restore
// and «Очистить», the name conflict dialog («Оставить оба» by default),
// uploads into «Личное» landing in the open personal library, the
// personal storage chosen by itself and in Settings, the trash purge
// throttle and the share intake. With KINOLUNE_SCREENSHOTS_DIR set the
// frames are written (in Russian):
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-035 \
//     flutter test test/files_test.dart

import 'dart:async';
import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_personal_media.dart';
import 'package:kinolune/fakes/fake_source_files.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/files/files_controller.dart';
import 'package:kinolune/files/files_screen.dart';
import 'package:kinolune/files/share_intake.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_library/pa_library.dart' show FilesError, NameConflict;

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

Finder _key(String k) => find.byKey(ValueKey(k));

Future<void> _tapKey(WidgetTester t, String key, [int frames = 6]) async {
  final f = _key(key);
  await t.ensureVisible(f);
  await t.pump();
  await t.tap(f);
  await settle(t, frames);
}

/// Sources get numeric ids, as the core's.
class _Manager extends FakeSourceManager {
  _Manager(super.library) : super(numericIds: true);
}

class _Env {
  _Env(this.s, this.files, this.personal);
  final AppServices s;
  final FakeSourceFiles files;
  final FakePersonalMedia personal;
  late int nas;
}

Future<_Env> _open(
  WidgetTester t, {
  Size size = const Size(1280, 900),
  FormFactor formFactor = FormFactor.desktop,
  Section section = Section.settings,
  Set<int> readOnly = const {},
  Preferences? preferences,
}) async {
  setSurface(t, size);
  final personal = FakePersonalMedia.demo();
  final files = FakeSourceFiles(
    readOnly: readOnly,
    now: () => DateTime(2026, 11, 17, 12),
    sizeOf: (p) => 48 * 1000 * 1000,
  );
  // As the core's refresh: files in a personal folder join its library.
  files.onRefresh = (source, touched) {
    for (final d in touched) {
      unawaited(
        files.list(source, d.dir).then((entries) {
          for (final e in entries) {
            if (!e.isFolder) {
              personal.addFile(source, e.relPath, sizeBytes: e.sizeBytes);
            }
          }
        }),
      );
    }
  };
  final s = await demoServices(
    personal: personal,
    sourceManager: _Manager.new,
    files: files,
    preferences: preferences ?? MemoryPreferences(),
  );
  s.locale.value = const Locale('ru');
  final env = _Env(s, files, personal);
  final nas = await s.createSource(
    const SourceDraft(
      type: SourceType.smb,
      name: 'Домашний NAS',
      address: 'smb://nas.local/media',
    ),
  );
  env.nas = int.parse(nas.id);
  files
    ..addFolder(env.nas, 'Фильмы')
    ..addFolder(env.nas, 'Отпуск 2025')
    ..addFile(env.nas, 'Отпуск 2025/море.mp4', 820 * 1000 * 1000)
    ..addFile(env.nas, 'clip.mp4', 120 * 1000 * 1000)
    ..addFile(env.nas, 'Заметки.txt', 2048);
  await t.pumpWidget(testApp(s, formFactor: formFactor, section: section));
  await settle(t);
  return env;
}

Future<void> _openFiles(WidgetTester t, _Env env, {String dir = ''}) async {
  final context = t.element(find.byType(Scaffold).first);
  unawaited(
    openFiles(context, sourceId: env.nas, sourceName: 'Домашний NAS', dir: dir),
  );
  await settle(t, 8);
  expect(_key('files-page'), findsOneWidget);
}

Future<void> _menu(WidgetTester t, String path, String action) async {
  await _tapKey(t, 'files-menu-$path', 4);
  await _tapKey(t, 'files-action-$action', 6);
}

void main() {
  setUpAll(loadAppFonts);
  tearDown(() => pickUploads = systemPickUploads);

  group('file view', () {
    testWidgets('lists folders first and uploads several files', (t) async {
      final env = await _open(t);
      await _openFiles(t, env);
      // Folders first, then files by name.
      final folder = t.getTopLeft(_key('files-entry-Фильмы')).dy;
      final file = t.getTopLeft(_key('files-entry-clip.mp4')).dy;
      expect(folder, lessThan(file));
      await capture(t, 'desktop-files');

      final hold = Completer<void>();
      env.files.holdUploads = hold;
      pickUploads = ({required bool media}) async => [
        const UploadRequest('/pick/IMG_0001.mp4'),
        const UploadRequest('/pick/IMG_0002.mp4'),
        const UploadRequest('/pick/IMG_0003.mp4'),
      ];
      await _tapKey(t, 'files-upload', 4);
      // The queue shows progress of the first file while it goes.
      expect(_key('files-uploads'), findsOneWidget);
      expect(find.text(ru.filesUploadsLeft(3)), findsOneWidget);
      expect(find.byType(LinearProgressIndicator), findsWidgets);
      await capture(t, 'desktop-uploading');
      hold.complete();
      env.files.holdUploads = null;
      await settle(t, 12);
      expect(env.files.uploads.map((u) => u.$4), [
        'IMG_0001.mp4',
        'IMG_0002.mp4',
        'IMG_0003.mp4',
      ]);
      expect(find.text(ru.filesUploadsDone), findsOneWidget);
      expect(_key('files-entry-IMG_0002.mp4'), findsOneWidget);
      // Only the folder that changed is refreshed.
      expect(env.files.refreshes.last.$2.single.dir, '');
      await _tapKey(t, 'files-uploads-clear');
      expect(_key('files-uploads'), findsNothing);
    });

    testWidgets('a failed upload says why and retries', (t) async {
      final env = await _open(t);
      await _openFiles(t, env);
      env.files.failNextUpload = FilesError.unavailable;
      pickUploads = ({required bool media}) async => [
        const UploadRequest('/pick/a.mp4'),
      ];
      await _tapKey(t, 'files-upload', 10);
      expect(
        find.text(ru.filesUploadFailed(ru.filesErrUnavailable)),
        findsOneWidget,
      );
      await _tapKey(t, 'files-upload-retry-a.mp4', 10);
      expect(env.files.exists(env.nas, 'a.mp4'), isTrue);
    });

    testWidgets('a phone picks from the gallery or from files', (t) async {
      final env = await _open(
        t,
        size: const Size(400, 860),
        formFactor: FormFactor.phone,
      );
      await _openFiles(t, env);
      bool? asked;
      pickUploads = ({required bool media}) async {
        asked = media;
        return [const UploadRequest('/pick/v.mp4', ownedCopy: true)];
      };
      await _tapKey(t, 'files-upload', 4);
      expect(_key('files-upload-gallery'), findsOneWidget);
      expect(_key('files-upload-files'), findsOneWidget);
      await capture(t, 'phone-upload-sheet');
      await _tapKey(t, 'files-upload-gallery', 12);
      expect(asked, isTrue);
      expect(env.files.exists(env.nas, 'v.mp4'), isTrue);
      await capture(t, 'phone-files');
    });

    testWidgets('creates, opens and renames a folder', (t) async {
      final env = await _open(t);
      await _openFiles(t, env);
      await _tapKey(t, 'files-new-folder', 4);
      await t.enterText(_key('files-name'), 'Дача');
      await _tapKey(t, 'files-name-ok', 8);
      expect(env.files.exists(env.nas, 'Дача'), isTrue);
      expect(_key('files-entry-Дача'), findsOneWidget);

      await _tapKey(t, 'files-entry-Дача', 6);
      expect(_key('files-empty'), findsOneWidget);
      expect(_key('files-crumb-'), findsOneWidget);
      await _tapKey(t, 'files-crumb-', 6);
      expect(_key('files-entry-Дача'), findsOneWidget);

      await _menu(t, 'Дача', 'rename');
      await t.enterText(_key('files-name'), 'Дача 2025');
      await _tapKey(t, 'files-name-ok', 8);
      expect(env.files.exists(env.nas, 'Дача 2025'), isTrue);
      expect(env.files.exists(env.nas, 'Дача'), isFalse);
    });

    testWidgets('moves with «Переместить в…» and copies', (t) async {
      final env = await _open(t);
      await _openFiles(t, env);
      await _menu(t, 'clip.mp4', 'move');
      expect(_key('files-picker'), findsOneWidget);
      // The folder being moved is never offered; here a file moves.
      await _tapKey(t, 'files-pick-Отпуск 2025', 6);
      await capture(t, 'desktop-move-to');
      await _tapKey(t, 'files-pick-here', 10);
      expect(env.files.exists(env.nas, 'Отпуск 2025/clip.mp4'), isTrue);
      expect(env.files.exists(env.nas, 'clip.mp4'), isFalse);
      expect(_key('files-entry-clip.mp4'), findsNothing);

      await _menu(t, 'Заметки.txt', 'copy');
      await _tapKey(t, 'files-pick-Фильмы', 6);
      await _tapKey(t, 'files-pick-here', 10);
      expect(env.files.exists(env.nas, 'Фильмы/Заметки.txt'), isTrue);
      expect(env.files.exists(env.nas, 'Заметки.txt'), isTrue);
    });

    testWidgets('drag and drop onto a folder moves on desktop', (t) async {
      final env = await _open(t);
      await _openFiles(t, env);
      final from = t.getCenter(_key('files-entry-clip.mp4'));
      final to = t.getCenter(_key('files-entry-Фильмы'));
      final g = await t.startGesture(from);
      await t.pump(const Duration(milliseconds: 50));
      await g.moveTo(from + const Offset(0, 20));
      await t.pump();
      await g.moveTo(to);
      await t.pump();
      await g.up();
      await settle(t, 10);
      expect(env.files.exists(env.nas, 'Фильмы/clip.mp4'), isTrue);
      expect(env.files.transfers.single.$1, isTrue);
    });

    testWidgets('deletes to the trash, undoes, restores and empties', (
      t,
    ) async {
      final env = await _open(t);
      await _openFiles(t, env);
      await _menu(t, 'clip.mp4', 'trash');
      expect(env.files.exists(env.nas, 'clip.mp4'), isFalse);
      expect(find.text(ru.filesMovedToTrash('clip.mp4')), findsOneWidget);
      await _tapKey(t, 'files-undo', 8);
      expect(env.files.exists(env.nas, 'clip.mp4'), isTrue);

      await _menu(t, 'clip.mp4', 'trash');
      await _menu(t, 'Отпуск 2025', 'trash');
      await _tapKey(t, 'files-open-trash', 8);
      expect(_key('files-trash-list'), findsOneWidget);
      expect(find.text(ru.filesTrashHint), findsOneWidget);
      expect(find.text('clip.mp4'), findsOneWidget);
      expect(find.text('Отпуск 2025'), findsOneWidget);
      await capture(t, 'desktop-trash');
      final items = await env.files.trashList(env.nas);
      final holiday = items.firstWhere((i) => i.name == 'Отпуск 2025');
      await _tapKey(t, 'files-restore-${holiday.id}', 8);
      expect(env.files.exists(env.nas, 'Отпуск 2025/море.mp4'), isTrue);

      await _tapKey(t, 'files-empty-trash', 4);
      expect(find.text(ru.filesEmptyTrashConfirm(1)), findsOneWidget);
      await _tapKey(t, 'files-confirm-ok', 8);
      expect(await env.files.trashList(env.nas), isEmpty);
      expect(_key('files-trash-empty'), findsOneWidget);
    });

    testWidgets('a read-only source has no «Файлы» in the browser', (t) async {
      final env = await _open(t, readOnly: {101});
      final ro = await env.s.createSource(
        const SourceDraft(
          type: SourceType.webdav,
          name: 'Только чтение',
          address: 'https://dav.local/',
        ),
      );
      expect(ro.id, '101');
      expect(await env.s.filesControl!.writable(101), isFalse);
      expect(await env.s.filesControl!.writable(env.nas), isTrue);
    });
  });

  group('name conflict', () {
    Future<void> uploadClip(WidgetTester t) async {
      pickUploads = ({required bool media}) async => [
        const UploadRequest('/pick/clip.mp4'),
      ];
      await _tapKey(t, 'files-upload', 6);
    }

    testWidgets('«Оставить оба» is the default', (t) async {
      final env = await _open(t);
      await _openFiles(t, env);
      await uploadClip(t);
      expect(_key('files-conflict'), findsOneWidget);
      expect(find.text(ru.filesConflictOne('clip.mp4')), findsOneWidget);
      final keep = t.widget<FilledButton>(_key('files-conflict-keep'));
      expect(keep.autofocus, isTrue);
      await capture(t, 'desktop-conflict');
      await _tapKey(t, 'files-conflict-keep', 12);
      expect(env.files.exists(env.nas, 'clip (2).mp4'), isTrue);
      expect(env.files.sizeAt(env.nas, 'clip.mp4'), 120 * 1000 * 1000);
    });

    testWidgets('«Заменить» sends the old one to the trash', (t) async {
      final env = await _open(t);
      await _openFiles(t, env);
      await uploadClip(t);
      await _tapKey(t, 'files-conflict-replace', 12);
      expect(env.files.sizeAt(env.nas, 'clip.mp4'), 48 * 1000 * 1000);
      expect(
        (await env.files.trashList(env.nas)).single.original,
        'clip.mp4',
      );
    });

    testWidgets('«Пропустить» writes nothing', (t) async {
      final env = await _open(t);
      await _openFiles(t, env);
      await uploadClip(t);
      await _tapKey(t, 'files-conflict-skip', 12);
      expect(env.files.uploads, isEmpty);
      expect(env.files.sizeAt(env.nas, 'clip.mp4'), 120 * 1000 * 1000);
    });

    testWidgets('a taken folder name keeps both', (t) async {
      final env = await _open(t);
      await _openFiles(t, env);
      await _tapKey(t, 'files-new-folder', 4);
      await t.enterText(_key('files-name'), 'Фильмы');
      await _tapKey(t, 'files-name-ok', 6);
      // A folder is never replaced.
      expect(_key('files-conflict-replace'), findsNothing);
      await _tapKey(t, 'files-conflict-keep', 8);
      expect(env.files.exists(env.nas, 'Фильмы (2)'), isTrue);
    });
  });

  group('«Личное»', () {
    testWidgets('uploads land in the open library unrecognized', (t) async {
      final env = await _open(
        t,
        size: const Size(400, 860),
        formFactor: FormFactor.phone,
        section: Section.personal,
      );
      // «Домашнее видео» (id 2) has no lock.
      await _tapKey(t, 'personal-library-2', 8);
      expect(_key('personal-library-page-2'), findsOneWidget);
      pickUploads = ({required bool media}) async => [
        const UploadRequest('/cache/VID_1.mp4', ownedCopy: true),
        const UploadRequest('/cache/VID_2.mp4', ownedCopy: true),
        const UploadRequest('/cache/VID_3.mp4', ownedCopy: true),
      ];
      await _tapKey(t, 'personal-upload', 4);
      await _tapKey(t, 'personal-upload-gallery', 20);
      // The first writable source holds Kinolune/<library>, now a folder
      // of the library.
      const dir = 'Kinolune/Домашнее видео';
      for (final n in ['VID_1.mp4', 'VID_2.mp4', 'VID_3.mp4']) {
        expect(env.files.exists(env.nas, '$dir/$n'), isTrue, reason: n);
      }
      final lib = (await env.personal.libraries()).firstWhere(
        (l) => l.id == 2,
      );
      expect(lib.folders.map((f) => (f.sourceId, f.path)), contains((env.nas, '/$dir')));
      final titles = (await env.personal.videos(2)).map((v) => v.title);
      expect(titles, containsAll(['VID_1', 'VID_2', 'VID_3']));
      await settle(t, 6);
      expect(find.text('VID_2'), findsOneWidget);
      await capture(t, 'phone-personal-uploaded');

      // «Файлы» opens that folder; its root is the library.
      await _tapKey(t, 'personal-files', 8);
      expect(_key('files-page'), findsOneWidget);
      expect(_key('files-entry-$dir/VID_1.mp4'), findsOneWidget);
      expect(find.text('Домашнее видео'), findsWidgets);
      await capture(t, 'phone-personal-files');
    });

    testWidgets('the first upload makes «Личное» when there is none', (
      t,
    ) async {
      final env = await _open(t);
      for (final l in await env.personal.libraries()) {
        if (l.locked) await env.personal.unlock(l.id, '2468');
        await env.personal.deleteLibrary(l.id);
      }
      final c = env.s.filesControl!;
      final lib = await c.defaultLibrary(ru.filesPersonalName);
      expect(lib?.name, 'Личное');
      final target = await c.uploadFolderOf(lib!);
      expect(target, (env.nas, 'Kinolune/Личное'));
    });
  });

  group('personal storage', () {
    FilesController controller({
      required FakeSourceFiles files,
      required List<ManagedSource> sources,
      Preferences? prefs,
      Future<int> Function()? device,
    }) => FilesController(
      files: files,
      preferences: prefs ?? MemoryPreferences(),
      sources: () => sources,
      createDeviceSource: device,
      fileSize: (_) async => 1000,
      deleteLocal: (_) async {},
    );

    ManagedSource src(int id, SourceType type) => ManagedSource(
      id: '$id',
      type: type,
      name: 'S$id',
      address: 'x',
    );

    test('the first writable source, else the device folder', () async {
      final files = FakeSourceFiles(readOnly: {1});
      final sources = [
        src(1, SourceType.smb),
        src(2, SourceType.jellyfin),
        src(3, SourceType.webdav),
      ];
      var c = controller(files: files, sources: sources);
      expect(await c.storagePlace(), const StoragePlace(3, 'Kinolune'));

      var made = 0;
      final none = FakeSourceFiles(readOnly: {1, 3});
      final withDevice = [...sources];
      c = controller(
        files: none,
        sources: withDevice,
        device: () async {
          made++;
          withDevice.add(src(9, SourceType.local));
          return 9;
        },
      );
      expect(
        await c.storagePlace(),
        const StoragePlace(9, '', onDevice: true),
      );
      expect(made, 1);
    });

    test('Settings can pin a source or this device', () async {
      final files = FakeSourceFiles();
      final sources = [src(1, SourceType.smb), src(2, SourceType.sftp)];
      final prefs = MemoryPreferences();
      final c = controller(
        files: files,
        sources: sources,
        prefs: prefs,
        device: () async => 7,
      );
      await c.setStorageChoice(const SourceStorage(2));
      expect(await c.storagePlace(), const StoragePlace(2, 'Kinolune'));
      expect(await prefs.read(FilesPrefs.storage), 'source:2');
      await c.setStorageChoice(const DeviceStorage());
      expect(
        await c.storagePlace(),
        const StoragePlace(7, '', onDevice: true),
      );
      await c.setStorageChoice(const AutoStorage());
      expect(await prefs.read(FilesPrefs.storage), isNull);
      expect(await c.storagePlace(), const StoragePlace(1, 'Kinolune'));
    });

    test('the queue survives a restart and resumes', () async {
      final prefs = MemoryPreferences();
      final files = FakeSourceFiles()..holdUploads = Completer<void>();
      final sources = [src(1, SourceType.smb)];
      final first = controller(files: files, sources: sources, prefs: prefs);
      await first.enqueue(
        const [UploadRequest('/a.mp4'), UploadRequest('/b.mp4')],
        sourceId: 1,
        dir: 'In',
        conflict: NameConflict.keepBoth,
      );
      await pumpEventQueue();
      final saved = jsonDecode((await prefs.read(FilesPrefs.queue))!) as List;
      expect(saved, hasLength(2));
      expect((saved.first as Map)['started'], isTrue);

      // The app is gone; a new one starts with the same preferences.
      final again = FakeSourceFiles();
      final second = controller(files: again, sources: sources, prefs: prefs);
      await second.start();
      for (var i = 0; i < 20 && second.pendingUploads > 0; i++) {
        await pumpEventQueue();
      }
      expect(again.exists(1, 'In/a.mp4'), isTrue);
      expect(again.exists(1, 'In/b.mp4'), isTrue);
      expect(await prefs.read(FilesPrefs.queue), isNull);
    });

    test('a file finished before the restart is not uploaded twice', () async {
      final prefs = MemoryPreferences();
      await prefs.write(
        FilesPrefs.queue,
        jsonEncode([
          {
            'id': 'up-1',
            'path': '/a.mp4',
            'source': 1,
            'dir': '',
            'name': 'a.mp4',
            'size': 1000,
            'started': true,
          },
        ]),
      );
      final files = FakeSourceFiles()..addFile(1, 'a.mp4', 1000);
      final c = controller(
        files: files,
        sources: [src(1, SourceType.smb)],
        prefs: prefs,
      );
      await c.start();
      for (var i = 0; i < 20 && c.pendingUploads > 0; i++) {
        await pumpEventQueue();
      }
      expect(files.uploads, isEmpty);
      expect(files.exists(1, 'a (2).mp4'), isFalse);
      expect(c.uploads.single.state, UploadState.done);
    });

    test('the trash is purged once a day per writable source', () async {
      var now = DateTime(2026, 11, 17, 9);
      final prefs = MemoryPreferences();
      final files = FakeSourceFiles(readOnly: {2});
      final c = FilesController(
        files: files,
        preferences: prefs,
        sources: () => [
          src(1, SourceType.smb),
          src(2, SourceType.webdav),
          src(3, SourceType.jellyfin),
        ],
        clock: () => now,
      );
      await c.purgeTrashes();
      expect(files.purges, [1, 2]);
      now = now.add(const Duration(hours: 5));
      await c.purgeTrashes();
      expect(files.purges, [1, 2]);
      now = now.add(const Duration(days: 1));
      await c.purgeTrashes();
      expect(files.purges, [1, 2, 1, 2]);
    });
  });

  group('settings', () {
    testWidgets('«Личное хранилище» shows and changes the place', (t) async {
      final env = await _open(t, size: const Size(1280, 1600));
      final tile = _key('settings-personal-storage');
      await t.ensureVisible(tile);
      await settle(t, 6);
      expect(find.text(ru.filesStorageNow('Домашний NAS')), findsOneWidget);
      await capture(t, 'desktop-settings-storage');
      await _tapKey(t, 'settings-personal-storage', 8);
      expect(_key('files-storage-dialog'), findsOneWidget);
      expect(_key('files-storage-source:${env.nas}'), findsOneWidget);
      await _tapKey(t, 'files-storage-source:${env.nas}', 8);
      expect(
        await env.s.preferences.read(FilesPrefs.storage),
        'source:${env.nas}',
      );
    });
  });

  test('a share hands its files over once Dart listens', () async {
    TestWidgetsFlutterBinding.ensureInitialized();
    const channel = MethodChannel('test/share');
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
          if (call.method == 'take') return ['/cache/shared/1/a.mp4'];
          return null;
        });
    final got = <List<String>>[];
    final intake = ShareIntake(
      channel: channel,
      onShared: (p) async => got.add(p),
    );
    await intake.start();
    expect(got.single, ['/cache/shared/1/a.mp4']);
    intake.dispose();
  });

  test('a library of the personal storage keeps its folder', () async {
    final files = FakeSourceFiles();
    final personal = FakePersonalMedia();
    final lib = await personal.createLibrary('Семья');
    final c = FilesController(
      files: files,
      preferences: MemoryPreferences(),
      sources: () => [
        const ManagedSource(
          id: '4',
          type: SourceType.local,
          name: 'L',
          address: '/l',
        ),
      ],
      personal: personal,
    );
    expect(await c.uploadFolderOf(lib), (4, 'Kinolune/Семья'));
    final again = (await personal.libraries()).single;
    expect(await c.uploadFolderOf(again), (4, 'Kinolune/Семья'));
    expect(again.folders.single.path, '/Kinolune/Семья');
  });
}
