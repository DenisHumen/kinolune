// SPDX-License-Identifier: MPL-2.0
//
// PKG-018: Settings → About → Updates and the banner, in the full flavor
// (store builds show the store link only). With KINOLUNE_SCREENSHOTS_DIR set
// the frames go to docs/screenshots/progress/pkg-018:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/pkg-018 \
//     flutter test --dart-define=KINOLUNE_FLAVOR=full test/updates_screens_test.dart

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
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/fake_release_host.dart';
import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));
final en = lookupAppLocalizations(const Locale('en'));

const _notes =
    '> [!WARNING]\n> **Unsigned / debug-signed Android build.**\n\n'
    'Обновление из приложения: проверка раз в день, SHA256SUMS, установка '
    'через PackageInstaller и замена AppImage.\n\n'
    '- Медиатека: категории и теги\n'
    '- Плеер: доступность и крупный шрифт\n'
    '- Распознавание: точнее для аниме\n\n'
    '## Downloads\n\n| Platform | File |\n';

class _Installer extends UpdateInstaller {
  _Installer(this.outcome);

  final InstallOutcome outcome;

  @override
  Future<InstallOutcome> install(File verified) async => outcome;
}

void main() {
  setUpAll(loadAppFonts);
  // The binding answers every HttpClient request with 400; the controller
  // talks to a real local server (FakeReleaseHost) inside runAsync.
  setUpAll(() => HttpOverrides.global = null);

  late FakeReleaseHost host;
  late Directory cache;
  const apk = 'kinolune-0.1.0-alpha.7-android-arm64-v8a.apk';
  final good = utf8.encode('apk');

  setUp(() async {
    host = await FakeReleaseHost.start();
    cache = Directory.systemTemp.createTempSync('pkg018-shots');
    host.releases.add(
      FakeRelease(
        '0.1.0-alpha.7',
        notes: _notes,
        assets: {apk: good, 'SHA256SUMS': utf8.encode('sums')},
      ),
    );
  });

  tearDown(() async {
    await host.close();
    cache.deleteSync(recursive: true);
  });

  UpdateController controller(InstallOutcome outcome, {bool tamper = false}) {
    if (tamper) host.overrides[apk] = utf8.encode('evil');
    return UpdateController(
      current: AppVersion.tryParse('0.1.0-alpha.6')!,
      target: const UpdateTarget(UpdateKind.androidApk, abis: ['arm64-v8a']),
      source: GitHubReleaseSource(api: host.api),
      verify:
          ({
            required File file,
            required String assetName,
            required List<int> sums,
            required String? signature,
          }) async {
            if (file.readAsStringSync() != 'apk') {
              throw const UpdateVerifyFailure('checksum mismatch');
            }
          },
      cacheDir: Directory('${cache.path}/updates'),
      preferences: MemoryPreferences(),
      installer: _Installer(outcome),
      clock: () => demoNow.toUtc(),
    );
  }

  Future<void> openAbout(
    WidgetTester t,
    AppServices s,
    FormFactor ff, [
    AppLocalizations? l,
  ]) async {
    await t.pumpWidget(testApp(s, formFactor: ff, section: Section.settings));
    await settle(t);
    final about = find.text((l ?? ru).settingsAboutApp(kAppName));
    await t.ensureVisible(about);
    await t.tap(about);
    await settle(t, 8);
    await t.ensureVisible(find.byKey(const ValueKey('about-updates')));
    await settle(t, 4);
  }

  testWidgets('store builds link to the store', (t) async {
    final s = await demoServices();
    s.locale.value = const Locale('ru');
    await openAbout(t, s, FormFactor.desktop);
    expect(find.byKey(const ValueKey('about-updates')), findsOneWidget);
    expect(find.text(ru.updatesCheck), findsNothing);
  }, skip: AppFlavor.selfUpdate);

  group('full', () {
    testWidgets('About: version available with the changelog', (t) async {
      setSurface(t, const Size(1280, 860));
      final c = controller(InstallOutcome.installed);
      await t.runAsync(c.check);
      final s = await demoServices(updates: c);
      s.locale.value = const Locale('ru');
      await openAbout(t, s, FormFactor.desktop);
      expect(find.text(ru.updatesAvailable('0.1.0-alpha.7')), findsOneWidget);
      expect(find.textContaining('Медиатека: категории'), findsOneWidget);
      expect(find.textContaining('debug-signed'), findsNothing);
      expect(find.text(ru.updatesInstall), findsOneWidget);
      expect(find.text(ru.updatesAutoCheck), findsOneWidget);
      await capture(t, '01-about-available-desktop');
    });

    testWidgets('banner across the app after the daily check', (t) async {
      setSurface(t, const Size(1280, 800));
      final c = controller(InstallOutcome.installed);
      await t.runAsync(c.check);
      final s = await demoServices(updates: c);
      s.locale.value = const Locale('ru');
      await t.pumpWidget(testApp(s));
      await settle(t);
      expect(
        find.text(ru.updatesBannerAvailable(kAppName, '0.1.0-alpha.7')),
        findsOneWidget,
      );
      await capture(t, '02-banner-desktop');
      // UI-026: a new release is announced by the in-app notice card; the
      // strip across the top waits for a running update.
      expect(find.byKey(const ValueKey('update-banner')), findsNothing);
      expect(find.text(ru.updatesLater), findsOneWidget);
      await t.tap(find.byKey(const ValueKey('notice-dismiss')));
      await settle(t, 4);
      expect(find.byKey(const ValueKey('in-app-notice')), findsNothing);
      expect(find.byKey(const ValueKey('update-banner')), findsNothing);
      expect(c.bannerVisible, isFalse);
    });

    testWidgets('Android: another signing key — reinstall', (t) async {
      setSurface(t, const Size(390, 844), dpr: 2);
      final c = controller(InstallOutcome.signatureMismatch);
      await t.runAsync(c.check);
      await t.runAsync(c.update);
      expect(c.problem, UpdateProblem.signatureMismatch);
      final s = await demoServices(updates: c);
      s.locale.value = const Locale('ru');
      await openAbout(t, s, FormFactor.phone);
      expect(
        find.text(ru.updatesErrorSignatureMismatch(kAppName)),
        findsOneWidget,
      );
      expect(
        find.text(ru.updatesBannerReinstall(kAppName, '0.1.0-alpha.7')),
        findsOneWidget,
      );
      await t.ensureVisible(find.byKey(const ValueKey('updates-status')));
      await settle(t, 2);
      await capture(t, '03-signature-mismatch-phone');
    });

    testWidgets('a tampered download is refused', (t) async {
      setSurface(t, const Size(1280, 860));
      final c = controller(InstallOutcome.installed, tamper: true);
      await t.runAsync(c.check);
      await t.runAsync(c.update);
      expect(c.problem, UpdateProblem.checksum);
      final s = await demoServices(updates: c);
      s.locale.value = const Locale('en');
      await openAbout(t, s, FormFactor.desktop, en);
      expect(find.text(en.updatesErrorChecksum), findsOneWidget);
      expect(
        find.text(en.updatesBannerFailed(kAppName, '0.1.0-alpha.7')),
        findsOneWidget,
      );
      await capture(t, '04-checksum-rejected-desktop-en');
    });
  }, skip: !AppFlavor.selfUpdate);
}
