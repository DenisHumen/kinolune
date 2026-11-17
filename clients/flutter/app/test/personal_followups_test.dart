// SPDX-License-Identifier: MPL-2.0
//
// UI-033: an open personal library keeps where its videos were left — its
// own «Продолжить» row, resume in the player, «Убрать из «Продолжить»».
// UI-034 (ADR-0099): no content type in the SMB wizard and the source
// forms; «Перенести в Личное» / «Вернуть в медиатеку» for a folder or the
// whole source in its file browser (marked before the first scan), the
// mark in the sources list, and adding a folder by browsing. With
// KINOLUNE_SCREENSHOTS_DIR set the frames are written (in Russian):
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-033-034 \
//     flutter test test/personal_followups_test.dart

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_personal_media.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/add_source_dialog.dart'
    show showEditSourceDialog;
import 'package:kinolune/screens/file_browser_screen.dart' show openFileBrowser;
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));
final ru = lookupAppLocalizations(const Locale('ru'));

Finder _key(String k) => find.byKey(ValueKey(k));

Future<void> _tapKey(WidgetTester t, String key) async {
  final f = _key(key);
  await t.ensureVisible(f);
  await t.pump();
  await t.tap(f);
  await settle(t, 6);
}

Future<void> _typePin(WidgetTester t, String pin) async {
  for (final c in pin.split('')) {
    await t.sendKeyEvent(
      LogicalKeyboardKey.findKeyByKeyId(0x30 + int.parse(c))!,
    );
  }
  await t.sendKeyEvent(LogicalKeyboardKey.enter);
  await settle(t, 4);
}

/// A source manager with numeric ids (as the core's) and a folder tree to
/// browse.
class _Manager extends FakeSourceManager {
  _Manager(super.library) : super(numericIds: true);

  static const tree = {
    '': ['Дача', 'Отпуск'],
    'Отпуск': ['2024', '2025'],
    'Отпуск/2025': <String>[],
    'Дача': <String>[],
  };

  @override
  Future<List<SourceFolderEntry>> listFolder(String id, String dir) async => [
    for (final name in tree[dir] ?? const <String>[])
      SourceFolderEntry(
        name: name,
        relPath: dir.isEmpty ? name : '$dir/$name',
        isFolder: true,
      ),
    SourceFolderEntry(
      name: 'clip.mp4',
      relPath: dir.isEmpty ? 'clip.mp4' : '$dir/clip.mp4',
      isFolder: false,
    ),
  ];
}

/// Records how many content type calls were made when each scan started.
class _ScanSpy extends FakeLibrary {
  _ScanSpy(this.personal) : super.demo(scanStepDelay: Duration.zero);

  final FakePersonalMedia personal;
  final contentCallsAtScan = <int>[];

  @override
  Stream<ScanProgress> scan({String? sourceId}) {
    contentCallsAtScan.add(personal.contentCalls.length);
    return super.scan(sourceId: sourceId);
  }
}

Future<(AppServices, FakePersonalMedia, _ScanSpy)> _open(
  WidgetTester t, {
  Size size = const Size(1280, 900),
  FormFactor formFactor = FormFactor.desktop,
  Section section = Section.settings,
  Locale? locale,
  double dpr = 1,
}) async {
  setSurface(t, size, dpr: dpr);
  final personal = FakePersonalMedia.demo();
  final library = _ScanSpy(personal);
  final s = await demoServices(
    personal: personal,
    library: library,
    sourceManager: _Manager.new,
  );
  if (locale != null) s.locale.value = locale;
  await t.pumpWidget(testApp(s, formFactor: formFactor, section: section));
  await settle(t);
  return (s, personal, library);
}

Future<ManagedSource> _addLocal(AppServices s) => s.createSource(
  const SourceDraft(
    type: SourceType.local,
    name: 'Видео с телефона',
    address: '/media/phone',
  ),
);

Future<void> _openFamily(WidgetTester t) async {
  await t.tap(_key('personal-library-1'));
  await settle(t, 4);
  await _typePin(t, '2468');
  expect(_key('personal-library-page-1'), findsOneWidget);
}

Future<void> _openSources(WidgetTester t, AppLocalizations l) async {
  await t.ensureVisible(find.text(l.sourcesManage));
  await t.tap(find.text(l.sourcesManage));
  await settle(t, 8);
}

Future<void> _editSource(WidgetTester t, AppLocalizations l, String id) async {
  final edit = find.descendant(
    of: _key('source-card-$id'),
    matching: find.text(l.sourcesEdit),
  );
  await t.ensureVisible(edit);
  await settle(t, 2);
  await t.tap(edit);
  await settle(t, 6);
}

void main() {
  setUpAll(loadAppFonts);

  group('UI-033 continue watching inside a personal library', () {
    testWidgets('the open library has its own row and resumes', (t) async {
      final (_, personal, _) = await _open(t, section: Section.personal);
      await _openFamily(t);
      final videos = await personal.videos(1);
      final sea = videos.firstWhere((v) => v.title == 'Море 2025');
      final prom = videos.firstWhere((v) => v.title == 'Выпускной');
      final dacha = videos.firstWhere((v) => v.title == 'Дача, июль');

      // Started videos, latest first; never-played ones are not there.
      expect(_key('personal-continue'), findsOneWidget);
      expect(_key('personal-continue-${sea.fileId}'), findsOneWidget);
      expect(_key('personal-continue-${prom.fileId}'), findsOneWidget);
      expect(_key('personal-continue-${dacha.fileId}'), findsNothing);
      expect(
        t.getTopLeft(_key('personal-continue-${sea.fileId}')).dx,
        lessThan(t.getTopLeft(_key('personal-continue-${prom.fileId}')).dx),
      );
      expect(_key('personal-continue-progress-${sea.fileId}'), findsOneWidget);
      // The grid shows the same position.
      expect(_key('personal-video-progress-${sea.fileId}'), findsOneWidget);

      // Resumes where it was left; the player's reports stay here.
      await t.tap(_key('personal-continue-${sea.fileId}'));
      await settle(t, 6);
      final player = t.widget<PlayerScreen>(find.byType(PlayerScreen));
      expect(player.request.startPositionMs, 12 * 60000);
      player.onProgress(13 * 60000, 38 * 60000);
      await t.pump();
      expect(personal.progressCalls.last, (1, sea.fileId, 13 * 60000));
      Navigator.of(
        t.element(find.byType(PlayerScreen)),
        rootNavigator: true,
      ).pop();
      await settle(t, 8);
      // The fake engine reports its own position (0) when it closes: put
      // back where a real player would have left it, then reload the page.
      await personal.saveProgress(1, sea, 13 * 60000, 38 * 60000);
      await _tapKey(t, 'personal-stars-4');
      await _tapKey(t, 'personal-stars-4');
      expect(_key('personal-continue-${sea.fileId}'), findsOneWidget);

      // «Убрать из «Продолжить»».
      await _tapKey(t, 'personal-continue-more-${prom.fileId}');
      await _tapKey(t, 'personal-video-clear-progress');
      expect(_key('personal-continue-${prom.fileId}'), findsNothing);
      expect(_key('personal-continue-${sea.fileId}'), findsOneWidget);

      // A filter shows the matching videos only, without the row.
      await _tapKey(t, 'personal-stars-5');
      expect(_key('personal-continue'), findsNothing);
      await _tapKey(t, 'personal-stars-5');
      expect(_key('personal-continue'), findsOneWidget);

      // Locked: no row, and the positions cannot be read.
      await _tapKey(t, 'personal-lock-now');
      expect(_key('personal-continue'), findsNothing);
      await expectLater(
        personal.continueWatching(1),
        throwsA(isA<PersonalLocked>()),
      );
    });

    testWidgets('a video watched to the end starts from the beginning', (
      t,
    ) async {
      final (_, personal, _) = await _open(t, section: Section.personal);
      await _openFamily(t);
      final sea = (await personal.videos(1))
          .firstWhere((v) => v.title == 'Море 2025');
      await personal.saveProgress(1, sea, 37 * 60000, 38 * 60000);
      await t.tap(_key('personal-stars-4'));
      await settle(t, 4);
      expect(_key('personal-video-watched-${sea.fileId}'), findsOneWidget);
      await t.tap(_key('personal-video-${sea.fileId}'));
      await settle(t, 6);
      expect(
        t
            .widget<PlayerScreen>(find.byType(PlayerScreen))
            .request
            .startPositionMs,
        0,
      );
    });
  });

  group('UI-034 personal media is a place, not a content type', () {
    testWidgets('no content choice in the SMB wizard and the forms', (t) async {
      final (s, _, _) = await _open(t);
      await t.ensureVisible(find.text(en.addSourceTitle).first);
      await t.tap(find.text(en.addSourceTitle).first);
      await settle(t, 6);
      await t.tap(find.text(en.sourcesTypeSmb));
      await settle(t, 6);
      await t.enterText(_key('smb-server'), '192.168.1.10');
      await _tapKey(t, 'smb-next');
      expect(_key('smb-user'), findsOneWidget);
      expect(_key('source-content'), findsNothing);
      expect(find.text(en.sourceContentPersonal), findsNothing);
      await t.tap(find.text(en.commonCancel).last);
      await settle(t, 6);

      final source = await _addLocal(s);
      await settle(t, 4);
      await _openSources(t, en);
      await _editSource(t, en, source.id);
      expect(_key('source-submit'), findsOneWidget);
      expect(_key('source-content'), findsNothing);
      await t.tap(find.text(en.commonCancel).last);
      await settle(t, 4);

      // ignore: unawaited_futures
      showEditSourceDialog(
        t.element(find.byType(AppShell)),
        const ManagedSource(
          id: '7',
          type: SourceType.jellyfin,
          name: 'Jellyfin',
          address: 'http://jellyfin.local:8096',
        ),
      );
      await settle(t, 6);
      expect(_key('jellyfin-submit'), findsOneWidget);
      expect(_key('source-content'), findsNothing);
    });

    testWidgets('a folder moves to Personal before the first scan', (t) async {
      final (s, personal, library) = await _open(t);
      final source = await _addLocal(s);
      await settle(t, 4);
      final scansBefore = library.contentCallsAtScan.length;
      // ignore: unawaited_futures
      openFileBrowser(t.element(find.byType(AppShell)), source);
      await settle(t, 8);
      expect(_key('browser-entry-Отпуск'), findsOneWidget);
      await _tapKey(t, 'browser-folder-to-personal-Отпуск');
      // Two libraries: the open one («Домашнее видео»).
      await _tapKey(t, 'personal-target-2');
      final id = int.parse(source.id);
      final home = (await personal.libraries()).firstWhere((l) => l.id == 2);
      expect(home.folders, contains(PersonalFolder(id, '/Отпуск')));
      // Marked before anything scanned the source.
      expect(library.contentCallsAtScan.length, scansBefore);
      await settle(t, 4);
      expect(find.text(en.browserInPersonal), findsOneWidget);
      expect(_key('browser-folder-back-Отпуск'), findsOneWidget);
      expect(_key('browser-folder-to-personal-Дача'), findsOneWidget);

      // Back to the library: out of the personal one, and scanned.
      await _tapKey(t, 'browser-folder-back-Отпуск');
      final after = (await personal.libraries()).firstWhere((l) => l.id == 2);
      expect(after.folders, isNot(contains(PersonalFolder(id, '/Отпуск'))));
      expect(library.contentCallsAtScan.length, greaterThan(scansBefore));
      expect(_key('browser-folder-to-personal-Отпуск'), findsOneWidget);
    });

    testWidgets('a locked library is unlocked to take a folder', (t) async {
      final (s, personal, _) = await _open(t);
      await personal.deleteLibrary(2);
      final source = await _addLocal(s);
      await settle(t, 4);
      // ignore: unawaited_futures
      openFileBrowser(t.element(find.byType(AppShell)), source);
      await settle(t, 8);
      await _tapKey(t, 'browser-folder-to-personal-Дача');
      // The only library is locked: its PIN first.
      expect(_key('pin-dialog'), findsOneWidget);
      await _typePin(t, '2468');
      final family = (await personal.libraries()).single;
      expect(
        family.folders,
        contains(PersonalFolder(int.parse(source.id), '/Дача')),
      );
    });

    testWidgets('the whole source moves to Personal and is marked', (t) async {
      final (s, personal, library) = await _open(t);
      final source = await _addLocal(s);
      await settle(t, 4);
      final id = int.parse(source.id);
      // ignore: unawaited_futures
      openFileBrowser(t.element(find.byType(AppShell)), source);
      await settle(t, 8);
      await _tapKey(t, 'browser-source-to-personal');
      await _tapKey(t, 'personal-target-2');
      expect(personal.contentCalls, [(id, SourceContent.personal, 2)]);
      await settle(t, 4);
      expect(_key('browser-source-back'), findsOneWidget);
      // Every folder is in Personal already: no folder actions.
      expect(_key('browser-folder-to-personal-Дача'), findsNothing);
      Navigator.of(t.element(_key('browser-source-back'))).pop();
      await settle(t, 6);
      await _openSources(t, en);
      expect(_key('source-content-mark-${source.id}'), findsOneWidget);

      // And back: movies and shows again, scanned.
      final scans = library.contentCallsAtScan.length;
      // ignore: unawaited_futures
      openFileBrowser(t.element(find.byType(AppShell)), source);
      await settle(t, 8);
      await _tapKey(t, 'browser-source-back');
      expect(personal.contentCalls.last, (id, SourceContent.video, null));
      expect(library.contentCallsAtScan.length, greaterThan(scans));
    });

    testWidgets('a folder joins a library by browsing the source', (t) async {
      final (s, personal, _) = await _open(t, section: Section.personal);
      final source = await _addLocal(s);
      await settle(t, 4);
      await t.tap(_key('personal-library-2'));
      await settle(t, 6);
      await _tapKey(t, 'personal-menu');
      await t.tap(find.text(en.personalAddFolder).last);
      await settle(t, 6);
      // The only source opens at once: its folders, no files.
      expect(_key('personal-folder-entry-Дача'), findsOneWidget);
      expect(_key('personal-folder-entry-Отпуск'), findsOneWidget);
      expect(find.text('clip.mp4'), findsNothing);
      await _tapKey(t, 'personal-folder-entry-Отпуск');
      await _tapKey(t, 'personal-folder-entry-Отпуск/2025');
      expect(find.text(en.personalFolderNoSubfolders), findsOneWidget);
      await _tapKey(t, 'personal-folder-up');
      expect(_key('personal-folder-entry-Отпуск/2024'), findsOneWidget);
      await _tapKey(t, 'personal-folder-ok');
      final lib = (await personal.libraries()).firstWhere((l) => l.id == 2);
      expect(
        lib.folders,
        contains(PersonalFolder(int.parse(source.id), '/Отпуск')),
      );
    });
  });

  for (final (name, ff, size, dpr) in <(String, FormFactor, Size, double)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800), 1),
    ('phone', FormFactor.phone, const Size(390, 844), 2),
  ]) {
    testWidgets('screenshots on $name', (t) async {
      final (s, personal, _) = await _open(
        t,
        size: size,
        formFactor: ff,
        dpr: dpr,
        section: Section.personal,
        locale: const Locale('ru'),
      );
      final source = await _addLocal(s);
      await settle(t, 4);
      await _openFamily(t);
      await capture(t, '01-continue-row-$name');
      Navigator.of(t.element(_key('personal-library-page-1'))).pop();
      await settle(t, 6);

      await t.tap(_key('personal-library-2'));
      await settle(t, 6);
      await _tapKey(t, 'personal-menu');
      await t.tap(find.text(ru.personalAddFolder).last);
      await settle(t, 6);
      await _tapKey(t, 'personal-folder-entry-Отпуск');
      await capture(t, '02-folder-picker-$name');
      await _tapKey(t, 'personal-folder-ok');
      Navigator.of(t.element(_key('personal-library-page-2'))).pop();
      await settle(t, 6);

      // ignore: unawaited_futures
      openFileBrowser(t.element(find.byType(AppShell)), source);
      await settle(t, 8);
      await capture(t, '03-browser-folder-in-personal-$name');
      await _tapKey(t, 'browser-source-to-personal');
      await capture(t, '04-browser-choose-library-$name');
      await _tapKey(t, 'personal-target-2');
      await settle(t, 4);
      Navigator.of(t.element(_key('browser-source-back'))).pop();
      await settle(t, 6);

      final shell = t.state<AppShellState>(find.byType(AppShell));
      shell.select(Section.settings);
      await settle(t, 6);
      await _openSources(t, ru);
      await t.ensureVisible(_key('source-content-mark-${source.id}'));
      await settle(t, 2);
      await capture(t, '05-sources-mark-$name');
      expect(personal.contentCalls, isNotEmpty);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
