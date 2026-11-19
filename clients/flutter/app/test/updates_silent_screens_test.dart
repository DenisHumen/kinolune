// SPDX-License-Identifier: MPL-2.0
//
// PKG-024: silent updates on screen. A desktop asks nothing (About says
// the update waits for the next start or an idle moment), the short
// «Что нового» after the update, nothing over the player, and Android's
// «Установить» once the APK is downloaded and verified. With
// KINOLUNE_SCREENSHOTS_DIR set the frames go to
// docs/screenshots/progress/pkg-024:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/pkg-024 \
//     flutter test --dart-define=KINOLUNE_FLAVOR=full test/updates_silent_screens_test.dart

import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
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
    'Тихие обновления.\n\n'
    '- Новая версия качается в фоне и ставится сама\n'
    '- Во время просмотра — никогда\n\n'
    '## Downloads\n\n| Platform | File |\n';

const _setup = 'kinolune-0.1.0-alpha.12-windows-x64-setup.exe';
const _apk = 'kinolune-0.1.0-alpha.12-android-arm64-v8a.apk';

class _Installer extends UpdateInstaller {
  _Installer({this.android = false});

  final bool android;
  var installs = 0;

  @override
  bool get asksUser => android;

  @override
  Future<InstallOutcome> install(File verified) async {
    installs++;
    return android ? InstallOutcome.installed : InstallOutcome.restartNeeded;
  }
}

void main() {
  setUpAll(loadAppFonts);
  setUpAll(() => HttpOverrides.global = null);

  late FakeReleaseHost host;
  late Directory cache;
  late MemoryPreferences prefs;

  setUp(() async {
    host = await FakeReleaseHost.start();
    cache = Directory.systemTemp.createTempSync('pkg024-shots');
    prefs = MemoryPreferences();
    host.releases.add(
      FakeRelease(
        '0.1.0-alpha.12',
        notes: _notes,
        assets: {
          _setup: utf8.encode('setup'),
          _apk: utf8.encode('apk'),
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

  UpdateController controller(
    _Installer installer, {
    String current = '0.1.0-alpha.11',
    UpdateTarget target = const UpdateTarget(
      UpdateKind.windowsInstaller,
      appPath: r'C:\Users\u\AppData\Local\Programs\Kinolune',
    ),
    ValueListenable<bool>? playback,
  }) => UpdateController(
    current: AppVersion.tryParse(current)!,
    target: target,
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
    playback: playback,
  );

  group('silent', () {
    testWidgets('desktop: no strip, About says when it goes in', (t) async {
      setSurface(t, const Size(1280, 900));
      final c = controller(_Installer());
      await t.runAsync(c.start);
      await t.runAsync(c.tick);
      expect(c.phase, UpdatePhase.restartNeeded);
      final s = await demoServices(updates: c);
      s.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(s, formFactor: FormFactor.desktop, section: Section.settings),
      );
      await settle(t);
      expect(find.byKey(const ValueKey('update-banner')), findsNothing);
      final about = find.text(ru.settingsAboutApp(kAppName));
      await t.ensureVisible(about);
      await t.tap(about);
      await settle(t, 8);
      await t.ensureVisible(find.byKey(const ValueKey('updates-status')));
      await settle(t, 4);
      expect(
        t.widget<Text>(find.byKey(const ValueKey('updates-status'))).data,
        ru.updatesQuietReady('0.1.0-alpha.12'),
      );
      await capture(t, '01-about-silent-ready');
    });

    testWidgets('«Что нового» after the update, hidden under the player', (
      t,
    ) async {
      setSurface(t, const Size(1280, 800));
      await t.runAsync(
        () => prefs.write(
          UpdatePrefs.whatsNew,
          jsonEncode({
            'version': '0.1.0-alpha.12',
            'notes': 'Тихие обновления.\n\n'
                '- Новая версия качается в фоне и ставится сама\n'
                '- Во время просмотра — никогда',
          }),
        ),
      );
      final playing = ValueNotifier(false);
      final c = controller(
        _Installer(),
        current: '0.1.0-alpha.12',
        playback: playing,
      );
      await t.runAsync(c.start);
      final s = await demoServices(updates: c);
      s.locale.value = const Locale('ru');
      await t.pumpWidget(testApp(s));
      await settle(t);
      expect(find.byKey(const ValueKey('update-whats-new')), findsOneWidget);
      expect(
        find.text(ru.updatesUpdatedTo(kAppName, '0.1.0-alpha.12')),
        findsOneWidget,
      );
      await t.tap(find.byKey(const ValueKey('update-whats-new-notes')));
      await settle(t, 4);
      expect(find.textContaining('Во время просмотра'), findsOneWidget);
      await capture(t, '02-whats-new');

      playing.value = true;
      await settle(t, 2);
      expect(find.byKey(const ValueKey('update-whats-new')), findsNothing);
      playing.value = false;
      await settle(t, 2);
      await t.tap(find.byKey(const ValueKey('update-whats-new-close')));
      await settle(t, 4);
      expect(find.byKey(const ValueKey('update-whats-new')), findsNothing);
      expect(c.whatsNew, isNull);
    });

    testWidgets('Android: «Установить» once downloaded, not over the player', (
      t,
    ) async {
      setSurface(t, const Size(390, 844), dpr: 2);
      final installer = _Installer(android: true);
      final playing = ValueNotifier(false);
      final c = controller(
        installer,
        target: const UpdateTarget(UpdateKind.androidApk, abis: ['arm64-v8a']),
        playback: playing,
      );
      await t.runAsync(c.start);
      await t.runAsync(c.tick);
      expect(c.phase, UpdatePhase.readyToInstall);
      expect(installer.installs, 0);
      final s = await demoServices(updates: c);
      s.locale.value = const Locale('ru');
      await t.pumpWidget(testApp(s, formFactor: FormFactor.phone));
      await settle(t);
      expect(
        t.widget<Text>(find.byKey(const ValueKey('update-banner-text'))).data,
        ru.updatesApkReady('0.1.0-alpha.12'),
      );
      expect(
        find.byKey(const ValueKey('update-banner-install')),
        findsOneWidget,
      );
      await capture(t, '03-android-ready-phone');

      playing.value = true;
      await settle(t, 2);
      expect(find.byKey(const ValueKey('update-banner')), findsNothing);
      playing.value = false;
      await settle(t, 2);
      await t.runAsync(c.update);
      await settle(t, 2);
      expect(installer.installs, 1);
    });
  }, skip: !AppFlavor.selfUpdate);
}
