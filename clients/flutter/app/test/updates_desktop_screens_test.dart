// SPDX-License-Identifier: MPL-2.0
//
// PKG-022: the update strip on a desktop install (Windows installer):
// «Доступна версия X» with the changelog and «Скачать», then
// «Перезапустить»; and the setting «Обновлять автоматически / только
// сообщать» in About. With KINOLUNE_SCREENSHOTS_DIR set the frames go to
// docs/screenshots/progress/pkg-022:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/pkg-022 \
//     flutter test --dart-define=KINOLUNE_FLAVOR=full test/updates_desktop_screens_test.dart

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/branding.dart';
import 'package:kinolune/features/updates/app_version.dart';
import 'package:kinolune/features/updates/installers.dart';
import 'package:kinolune/features/updates/releases.dart';
import 'package:kinolune/features/updates/update_controller.dart';
import 'package:kinolune/features/updates/update_platform.dart';
import 'package:kinolune/flavor.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/fake_release_host.dart';
import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

const _notes =
    'Установщик для Windows и автообновление на всех десктопах.\n\n'
    '- Обновление скачивается и проверяется в фоне\n'
    '- «Перезапустить» ставит его и открывает новую версию\n'
    '- Подпись minisign обязательна\n\n'
    '## Downloads\n\n| Platform | File |\n';

const _setup = 'kinolune-0.1.0-alpha.12-windows-x64-setup.exe';

class _Installer extends UpdateInstaller {
  var restarts = 0;

  @override
  Future<InstallOutcome> install(File verified) async =>
      InstallOutcome.restartNeeded;

  @override
  Future<void> restart() async => restarts++;
}

void main() {
  setUpAll(loadAppFonts);
  setUpAll(() => HttpOverrides.global = null);

  late FakeReleaseHost host;
  late Directory cache;
  late MemoryPreferences prefs;

  setUp(() async {
    host = await FakeReleaseHost.start();
    cache = Directory.systemTemp.createTempSync('pkg022-shots');
    prefs = MemoryPreferences();
    host.releases.add(
      FakeRelease(
        '0.1.0-alpha.12',
        notes: _notes,
        assets: {
          _setup: utf8.encode('setup'),
          'SHA256SUMS': utf8.encode('sums'),
          'SHA256SUMS.minisig': utf8.encode('sig'),
        },
      ),
    );
  });

  tearDown(() async {
    await host.close();
    cache.deleteSync(recursive: true);
  });

  UpdateController controller(_Installer installer) => UpdateController(
    current: AppVersion.tryParse('0.1.0-alpha.11')!,
    target: const UpdateTarget(
      UpdateKind.windowsInstaller,
      appPath: r'C:\Users\u\AppData\Local\Programs\Kinolune',
    ),
    source: GitHubReleaseSource(api: host.api),
    verify: ({
      required File file,
      required String assetName,
      required List<int> sums,
      required String? signature,
    }) async {},
    cacheDir: Directory('${cache.path}/updates'),
    preferences: prefs,
    installer: installer,
    clock: () => demoNow.toUtc(),
  );

  group('desktop', () {
    testWidgets('the strip: available with the changelog, then restart', (
      t,
    ) async {
      setSurface(t, const Size(1280, 800));
      final installer = _Installer();
      final c = controller(installer);
      await t.runAsync(c.check);
      final s = await demoServices(updates: c);
      s.locale.value = const Locale('ru');
      await t.pumpWidget(testApp(s));
      await settle(t);
      expect(
        t.widget<Text>(find.byKey(const ValueKey('update-banner-text'))).data,
        ru.updatesAvailable('0.1.0-alpha.12'),
      );
      await t.tap(find.byKey(const ValueKey('update-banner-notes')));
      await settle(t, 4);
      expect(
        find.byKey(const ValueKey('update-banner-changelog')),
        findsOneWidget,
      );
      expect(find.textContaining('Подпись minisign'), findsOneWidget);
      expect(find.text(ru.updatesHideNotes), findsOneWidget);
      await capture(t, '01-banner-available-changelog');

      // «Скачать» runs c.update(); its HTTP needs real time, so the test
      // runs it outside the fake clock.
      expect(
        find.byKey(const ValueKey('update-banner-update')),
        findsOneWidget,
      );
      await t.runAsync(c.update);
      await settle(t, 4);
      expect(c.phase, UpdatePhase.restartNeeded);
      expect(
        t.widget<Text>(find.byKey(const ValueKey('update-banner-text'))).data,
        ru.updatesReadyToInstall('0.1.0-alpha.12'),
      );
      await t.tap(find.byKey(const ValueKey('update-banner-notes')));
      await settle(t, 4);
      await capture(t, '02-banner-restart');
      await t.tap(find.byKey(const ValueKey('update-banner-restart')));
      await settle(t, 2);
      expect(installer.restarts, 1);
    });

    testWidgets('About: «Обновлять автоматически / только сообщать»', (
      t,
    ) async {
      setSurface(t, const Size(1280, 900));
      final c = controller(_Installer());
      await t.runAsync(c.start);
      final s = await demoServices(updates: c);
      s.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(s, formFactor: FormFactor.desktop, section: Section.settings),
      );
      await settle(t);
      final about = find.text(ru.settingsAboutApp(kAppName));
      await t.ensureVisible(about);
      await t.tap(about);
      await settle(t, 8);
      final mode = find.byKey(const ValueKey('updates-mode'));
      await t.ensureVisible(mode);
      await settle(t, 4);
      expect(find.text(ru.updatesMode), findsOneWidget);
      expect(find.text(ru.updatesModeAuto), findsOneWidget);
      await capture(t, '03-about-update-mode');

      await t.tap(mode);
      await settle(t, 4);
      expect(find.text(ru.updatesModeHint), findsOneWidget);
      await capture(t, '04-update-mode-dialog');
      await t.tap(find.byKey(const ValueKey('updates-mode-1')));
      await settle(t, 4);
      expect(c.mode, UpdateMode.notify);
      expect(await t.runAsync(() => prefs.read(UpdatePrefs.mode)), 'notify');
      expect(find.text(ru.updatesModeNotify), findsOneWidget);
    });
  }, skip: !AppFlavor.selfUpdate);
}
