// SPDX-License-Identifier: MPL-2.0
//
// PKG-022: the rules of the update schedule (the owner's updater rules),
// quiet background checks, «Обновлять автоматически / только сообщать»,
// the file of each platform and architecture (Rosetta included), the
// Windows installer and the macOS bundle swap, and that a tampered or
// corrupt download is refused on every platform. Without the core: a fake
// verifier compares bytes (the real one: updates_verify_rust_test.dart).

import 'dart:convert';
import 'dart:ffi' show Abi;
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/features/updates/app_version.dart';
import 'package:kinolune/features/updates/installers.dart';
import 'package:kinolune/features/updates/releases.dart';
import 'package:kinolune/features/updates/update_controller.dart';
import 'package:kinolune/features/updates/update_platform.dart';
import 'package:kinolune/features/updates/update_rules.dart';
import 'package:kinolune/services/preferences.dart';

import 'support/fake_release_host.dart';

AppVersion v(String s) => AppVersion.tryParse(s)!;

List<int> bytes(String s) => utf8.encode(s);

/// Accepts a file only with exactly the expected bytes.
ArtifactVerifier fakeVerifier(Map<String, List<int>> expected) =>
    ({
      required File file,
      required String assetName,
      required List<int> sums,
      required String? signature,
    }) async {
      final want = expected[assetName];
      if (want == null) throw UpdateVerifyFailure('$assetName not listed');
      if (signature == null) {
        throw const UpdateVerifyFailure('unsigned', signature: true);
      }
      final got = file.readAsBytesSync();
      if (got.length != want.length ||
          [for (var i = 0; i < got.length; i++) got[i] == want[i]]
              .contains(false)) {
        throw UpdateVerifyFailure('checksum mismatch for $assetName');
      }
    };

class RecordingInstaller extends UpdateInstaller {
  RecordingInstaller([
    this.outcome = InstallOutcome.restartNeeded,
    this.asksUser = false,
  ]);

  final InstallOutcome outcome;

  /// Like Android's: the system asks before it installs.
  @override
  final bool asksUser;
  final installed = <String>[];
  var restarts = 0;

  @override
  Future<InstallOutcome> install(File verified) async {
    installed.add(verified.path);
    return outcome;
  }

  @override
  Future<void> restart() async => restarts++;
}

final _t0 = DateTime.utc(2026, 11, 17, 12);

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() => HttpOverrides.global = null);

  group('schedule rules', () {
    test('first check at once, then every 6 hours', () {
      expect(checkDue(_t0, null, null), isTrue);
      final answered = _t0;
      expect(
        checkDue(_t0.add(const Duration(hours: 5, minutes: 59)), answered, _t0),
        isFalse,
      );
      expect(
        checkDue(_t0.add(const Duration(hours: 6)), answered, _t0),
        isTrue,
      );
    });

    test('30 minutes after a check that got no answer', () {
      final tried = _t0;
      expect(
        checkDue(_t0.add(const Duration(minutes: 29)), null, tried),
        isFalse,
      );
      expect(
        checkDue(_t0.add(const Duration(minutes: 30)), null, tried),
        isTrue,
      );
      // An old answer does not hold the retry back.
      final old = _t0.subtract(const Duration(days: 1));
      expect(
        checkDue(_t0.add(const Duration(minutes: 31)), old, tried),
        isTrue,
      );
    });

    test('a clock moved back counts as long ago', () {
      final future = _t0.add(const Duration(days: 2));
      expect(checkDue(_t0, future, future), isTrue);
    });

    test('timings are the owner\'s', () {
      expect(UpdateSchedule.launchDelay, const Duration(seconds: 5));
      expect(UpdateSchedule.every, const Duration(hours: 6));
      expect(UpdateSchedule.retry, const Duration(minutes: 30));
      expect(UpdateSchedule.resumeDelay, const Duration(seconds: 60));
    });

    test('a long gap between ticks is a sleep', () {
      expect(sleptBetween(_t0, _t0.add(UpdateSchedule.tick)), isFalse);
      expect(sleptBetween(_t0, _t0.add(const Duration(hours: 3))), isTrue);
    });

    test('PKG-024: silent downloads wait for playback and metered networks', () {
      expect(mayDownloadQuietly(playing: false, metered: false), isTrue);
      expect(mayDownloadQuietly(playing: true, metered: false), isFalse);
      expect(mayDownloadQuietly(playing: false, metered: true), isFalse);
    });

    test('PKG-024: restart only when idle, in front, silent and not a TV', () {
      bool may(
        Duration quiet, {
        bool playing = false,
        bool foreground = true,
        bool tv = false,
      }) => mayApplyWhenIdle(
        now: _t0.add(quiet),
        lastInput: _t0,
        playing: playing,
        foreground: foreground,
        tv: tv,
      );
      expect(UpdateSchedule.idleApply, const Duration(minutes: 10));
      expect(may(const Duration(minutes: 9, seconds: 59)), isFalse);
      expect(may(const Duration(minutes: 10)), isTrue);
      expect(may(const Duration(hours: 3), playing: true), isFalse);
      expect(may(const Duration(hours: 3), foreground: false), isFalse);
      expect(may(const Duration(hours: 3), tv: true), isFalse);
      // The clock moved back: counts as just now.
      expect(may(const Duration(hours: -3)), isFalse);
    });

    test('never over a download or a pending restart', () {
      expect(mayCheckInBackground(UpdateActivity.idle), isTrue);
      expect(mayCheckInBackground(UpdateActivity.checking), isFalse);
      expect(mayCheckInBackground(UpdateActivity.downloading), isFalse);
      expect(mayCheckInBackground(UpdateActivity.readyToRestart), isFalse);
    });
  });

  group('the file of each platform', () {
    const ver = '0.1.0-alpha.12';
    final names = [
      'kinolune-$ver-windows-x64-setup.exe',
      'kinolune-$ver-windows-x64.zip',
      'kinolune-$ver-macos-arm64.dmg',
      'kinolune-$ver-macos-x86_64.dmg',
      'kinolune-$ver-x86_64.AppImage',
      'kinolune-$ver-linux-x86_64.tar.gz',
      'kinolune-$ver-linux-amd64.deb',
      'kinolune-$ver-linux-x86_64.rpm',
      'SHA256SUMS',
    ];
    final release = AppRelease(
      tag: 'app-v$ver',
      version: v(ver),
      prerelease: true,
      notes: '',
      page: Uri.parse('https://example.invalid/page'),
      assets: [
        for (final n in names)
          ReleaseAsset(name: n, url: Uri.parse('https://example.invalid/$n')),
      ],
    );

    Future<UpdateTarget> detect(
      String os,
      String exe, {
      Abi abi = Abi.linuxX64,
      bool rosetta = false,
      Set<String> files = const {},
      Map<String, String> env = const {},
      Map<String, String> read = const {},
    }) => UpdateTarget.detect(
      operatingSystem: os,
      executable: exe,
      environment: env,
      abi: abi,
      rosetta: () => rosetta,
      fileExists: files.contains,
      readFile: (p) => read[p],
    );

    test(
      'Windows: installed copy gets the installer, the zip only notifies',
      () async {
        final installed = await detect(
          'windows',
          r'C:\Users\u\AppData\Local\Programs\Kinolune\Kinolune.exe',
          abi: Abi.windowsX64,
          files: {r'C:\Users\u\AppData\Local\Programs\Kinolune\unins000.exe'},
        );
        expect(installed.kind, UpdateKind.windowsInstaller);
        expect(
          installed.appPath,
          r'C:\Users\u\AppData\Local\Programs\Kinolune',
        );
        expect(installed.assetIn(release)?.name, names[0]);
        expect(installed.selfInstalls, isTrue);
        expect(installed.downloadsInBackground, isTrue);

        final portable = await detect(
          'windows',
          r'D:\kinolune-0.1.0-alpha.11-windows-x64\Kinolune.exe',
          abi: Abi.windowsX64,
        );
        expect(portable.kind, UpdateKind.windowsPortable);
        expect(portable.selfInstalls, isFalse);
        expect(portable.downloadLink(release).pathSegments.last, names[1]);
      },
    );

    test(
      'macOS: the DMG of its own architecture; Rosetta gets arm64',
      () async {
        const exe = '/Applications/Kinolune.app/Contents/MacOS/Kinolune';
        final arm = await detect('macos', exe, abi: Abi.macosArm64);
        expect(arm.kind, UpdateKind.macApp);
        expect(arm.appPath, '/Applications/Kinolune.app');
        expect(arm.assetIn(release)?.name, 'kinolune-$ver-macos-arm64.dmg');

        final intel = await detect('macos', exe, abi: Abi.macosX64);
        expect(intel.assetIn(release)?.name, 'kinolune-$ver-macos-x86_64.dmg');

        final rosetta = await detect(
          'macos',
          exe,
          abi: Abi.macosX64,
          rosetta: true,
        );
        expect(rosetta.assetIn(release)?.name, 'kinolune-$ver-macos-arm64.dmg');
        expect(macDmgArch(arm64: false, translated: true), 'arm64');
        expect(macDmgArch(arm64: false, translated: false), 'x86_64');

        // Not from a bundle (flutter run): the release page.
        final loose = await detect(
          'macos',
          '/tmp/build/Kinolune',
          abi: Abi.macosArm64,
        );
        expect(loose.kind, UpdateKind.releasePage);
      },
    );

    test('Linux: AppImage updates itself; packages only notify', () async {
      const exe = '/usr/lib/kinolune/kinolune';
      final appImage = await detect(
        'linux',
        exe,
        env: {'APPIMAGE': '/home/u/Kinolune.AppImage'},
      );
      expect(appImage.kind, UpdateKind.appImage);
      expect(appImage.assetIn(release)?.name, 'kinolune-$ver-x86_64.AppImage');

      final deb = await detect(
        'linux',
        exe,
        read: {'/usr/lib/kinolune/package-type': 'deb\n'},
      );
      expect(deb.kind, UpdateKind.linuxPackage);
      expect(deb.selfInstalls, isFalse);
      expect(deb.assetIn(release)?.name, 'kinolune-$ver-linux-amd64.deb');

      final rpm = await detect(
        'linux',
        exe,
        read: {'/usr/lib/kinolune/package-type': 'rpm'},
      );
      expect(rpm.assetIn(release)?.name, 'kinolune-$ver-linux-x86_64.rpm');

      final flatpak = await detect('linux', exe, env: {'FLATPAK_ID': 'x'});
      expect(flatpak.kind, UpdateKind.linuxPackage);
      expect(flatpak.assetIn(release), isNull);
      expect(flatpak.downloadLink(release), release.page);

      final arm = await detect('linux', exe, abi: Abi.linuxArm64);
      expect(arm.kind, UpdateKind.linuxArchive);
      expect(arm.arch, 'aarch64');
    });
  });

  group('controller', () {
    late FakeReleaseHost host;
    late Directory cache;
    late MemoryPreferences prefs;
    late DateTime now;
    late List<String> logs;

    const setup = 'kinolune-0.1.0-alpha.12-windows-x64-setup.exe';
    const dmg = 'kinolune-0.1.0-alpha.12-macos-arm64.dmg';
    const appImage = 'kinolune-0.1.0-alpha.12-x86_64.AppImage';
    const apk = 'kinolune-0.1.0-alpha.12-android-x86_64.apk';
    final good = {
      setup: bytes('the new setup.exe'),
      dmg: bytes('the new dmg'),
      appImage: bytes('the new AppImage'),
      apk: bytes('the new apk'),
    };

    setUp(() async {
      host = await FakeReleaseHost.start();
      cache = Directory.systemTemp.createTempSync('pkg022-cache');
      prefs = MemoryPreferences();
      now = _t0;
      logs = [];
      host.releases.add(
        FakeRelease(
          '0.1.0-alpha.12',
          notes: 'Windows installer\n\n## Downloads\n| x |',
          assets: {
            ...good,
            'SHA256SUMS': bytes('sums'),
            'SHA256SUMS.minisig': bytes('sig'),
          },
        ),
      );
    });

    tearDown(() async {
      await host.close();
      if (cache.existsSync()) cache.deleteSync(recursive: true);
    });

    UpdateController controller(
      UpdateTarget target, {
      UpdateInstaller? installer,
      ValueListenable<bool>? playback,
      bool tv = false,
      Future<bool> Function()? metered,
      String current = '0.1.0-alpha.11',
    }) => UpdateController(
      current: v(current),
      target: target,
      source: GitHubReleaseSource(api: host.api),
      verify: fakeVerifier(good),
      cacheDir: Directory('${cache.path}/updates'),
      preferences: prefs,
      installer: installer,
      clock: () => now,
      log: (level, message) => logs.add('$level $message'),
      playback: playback,
      tv: tv,
      meteredNetwork: metered,
    );

    const windows = UpdateTarget(
      UpdateKind.windowsInstaller,
      appPath: r'C:\Kinolune',
    );

    test('auto: a background check downloads, verifies and stages', () async {
      final launched = <List<String>>[];
      var quit = 0;
      final installer = WindowsSetupInstaller(
        launch: (exe, args) async => launched.add([exe, ...args]),
        quit: () => quit++,
      );
      final c = controller(windows, installer: installer);
      await c.start();
      expect(c.mode, UpdateMode.auto);
      await c.tick();
      expect(c.phase, UpdatePhase.restartNeeded);
      expect(installer.staged?.readAsBytesSync(), good[setup]);
      expect(host.requests, greaterThan(1), reason: 'downloaded at once');

      // A scheduled check never takes «Перезапустить» away.
      final before = host.requests;
      now = now.add(const Duration(days: 1));
      await c.backgroundCheck();
      expect(host.requests, before);
      expect(c.phase, UpdatePhase.restartNeeded);

      await c.restart();
      expect(launched.single.skip(1), [
        '/VERYSILENT',
        '/SUPPRESSMSGBOXES',
        '/NORESTART',
        '/UPDATE=1',
        '/RELAUNCH=1',
      ]);
      expect(launched.single.first, endsWith(setup));
      expect(quit, 1);
      // Every status change is in the log.
      expect(
        logs.where((l) => l.startsWith('info Update status:')),
        containsAllInOrder([
          contains('available'),
          contains('downloading'),
          contains('verifying'),
          contains('installing'),
          contains('restartNeeded'),
        ]),
      );
    });

    test(
      'Windows: quitting with a staged installer installs, no relaunch',
      () async {
        final launched = <List<String>>[];
        final installer = WindowsSetupInstaller(
          launch: (exe, args) async => launched.add(args),
          quit: () => fail('must not quit by itself'),
        );
        final c = controller(windows, installer: installer);
        await c.start();
        await c.tick();
        await c.applyOnExit();
        expect(launched.single, isNot(contains('/RELAUNCH=1')));
        expect(launched.single, contains('/VERYSILENT'));
      },
    );

    test(
      'notify only: the release is offered, nothing is downloaded',
      () async {
        final installer = RecordingInstaller();
        final c = controller(windows, installer: installer);
        await c.start();
        await c.setMode(UpdateMode.notify);
        expect(await prefs.read(UpdatePrefs.mode), 'notify');
        await c.tick();
        expect(c.phase, UpdatePhase.available);
        expect(host.requests, 1);
        expect(installer.installed, isEmpty);
        // The next start remembers it.
        final next = controller(windows);
        await next.start();
        expect(next.mode, UpdateMode.notify);
      },
    );

    group('PKG-024 silent updates', () {
      const mac = UpdateTarget(
        UpdateKind.macApp,
        appPath: '/Applications/Kinolune.app',
        arch: 'arm64',
      );
      const linux = UpdateTarget(
        UpdateKind.appImage,
        appImagePath: '/x/K.AppImage',
      );
      const android = UpdateTarget(UpdateKind.androidApk, abis: ['x86_64']);

      for (final (name, target) in [
        ('Windows', windows),
        ('macOS', mac),
        ('Linux AppImage', linux),
      ]) {
        test('$name: no question, restart once idle, no tap at all', () async {
          final installer = RecordingInstaller();
          final c = controller(target, installer: installer);
          await c.start();
          await c.tick();
          expect(c.phase, UpdatePhase.restartNeeded);
          expect(c.restartsQuietly, isTrue);
          expect(c.bannerVisible, isFalse, reason: 'no «Обновить?»');
          // The note for the next start is written before the swap.
          expect(
            jsonDecode((await prefs.read(UpdatePrefs.whatsNew))!),
            {'version': '0.1.0-alpha.12', 'notes': 'Windows installer'},
          );
          now = now.add(const Duration(minutes: 9));
          expect(await c.applyIfIdle(), isFalse, reason: 'not idle yet');
          now = now.add(const Duration(minutes: 1));
          expect(await c.applyIfIdle(), isTrue);
          expect(installer.restarts, 1);
          expect(logs, contains(startsWith('info Idle with nothing playing')));
        });
      }

      test('a key press or a click holds the restart back', () async {
        final installer = RecordingInstaller();
        final c = controller(windows, installer: installer);
        await c.start();
        await c.tick();
        now = now.add(const Duration(minutes: 9));
        c.userActive();
        now = now.add(const Duration(minutes: 9));
        expect(await c.applyIfIdle(), isFalse);
        now = now.add(const Duration(minutes: 1));
        expect(await c.applyIfIdle(), isTrue);
      });

      test('a window behind other windows is not brought up', () async {
        final installer = RecordingInstaller();
        final c = controller(windows, installer: installer);
        await c.start();
        await c.tick();
        c.foreground = false;
        now = now.add(const Duration(hours: 2));
        expect(await c.applyIfIdle(), isFalse);
        c.foreground = true;
        expect(await c.applyIfIdle(), isFalse, reason: 'back: activity');
        now = now.add(UpdateSchedule.idleApply);
        expect(await c.applyIfIdle(), isTrue);
      });

      test('never during playback: no download, no restart, no strip', () async {
        final playing = ValueNotifier(true);
        final installer = RecordingInstaller();
        final c = controller(windows, installer: installer, playback: playing);
        await c.start();
        await c.tick();
        expect(c.phase, UpdatePhase.available);
        expect(host.requests, 1, reason: 'checked, not downloaded');
        expect(c.bannerVisible, isFalse);
        expect(logs, contains(contains('waits for the playback to end')));

        playing.value = false;
        for (var i = 0; i < 50 && c.phase != UpdatePhase.restartNeeded; i++) {
          await Future<void>.delayed(const Duration(milliseconds: 20));
        }
        expect(c.phase, UpdatePhase.restartNeeded);

        playing.value = true;
        now = now.add(const Duration(hours: 5));
        expect(await c.applyIfIdle(), isFalse);
        expect(installer.restarts, 0);
        // The credits end: ten more quiet minutes.
        playing.value = false;
        now = now.add(const Duration(minutes: 5));
        expect(await c.applyIfIdle(), isFalse);
        now = now.add(const Duration(minutes: 5));
        expect(await c.applyIfIdle(), isTrue);
      });

      test('a TV never restarts by itself', () async {
        final installer = RecordingInstaller();
        final c = controller(windows, installer: installer, tv: true);
        await c.start();
        await c.tick();
        expect(c.phase, UpdatePhase.restartNeeded);
        now = now.add(const Duration(days: 1));
        expect(await c.applyIfIdle(), isFalse);
        expect(installer.restarts, 0);
      });

      test('«Только сообщать» asks as before and never restarts', () async {
        final installer = RecordingInstaller();
        final c = controller(windows, installer: installer);
        await c.start();
        await c.setMode(UpdateMode.notify);
        await c.tick();
        expect(c.bannerVisible, isTrue);
        await c.update();
        expect(c.phase, UpdatePhase.restartNeeded);
        expect(c.bannerVisible, isTrue, reason: '«Перезапустить»');
        now = now.add(const Duration(days: 1));
        expect(await c.applyIfIdle(), isFalse);
      });

      test('«Что нового» after the update, once', () async {
        await prefs.write(
          UpdatePrefs.whatsNew,
          jsonEncode({'version': '0.1.0-alpha.12', 'notes': 'Faster'}),
        );
        final c = controller(windows, current: '0.1.0-alpha.12');
        await c.start();
        expect(c.whatsNew?.version.text, '0.1.0-alpha.12');
        expect(c.whatsNew?.notes, 'Faster');
        expect(c.whatsNewVisible, isTrue);
        expect(logs, contains('info Updated to 0.1.0-alpha.12'));
        await c.dismissWhatsNew();
        expect(c.whatsNew, isNull);
        expect(await prefs.read(UpdatePrefs.whatsNew), isNull);

        // A note of an older version goes; one of a staged newer one stays.
        await prefs.write(
          UpdatePrefs.whatsNew,
          jsonEncode({'version': '0.1.0-alpha.10', 'notes': ''}),
        );
        await controller(windows).start();
        expect(await prefs.read(UpdatePrefs.whatsNew), isNull);
        await prefs.write(
          UpdatePrefs.whatsNew,
          jsonEncode({'version': '0.1.0-alpha.12', 'notes': ''}),
        );
        final older = controller(windows);
        await older.start();
        expect(older.whatsNew, isNull);
        expect(await prefs.read(UpdatePrefs.whatsNew), isNotNull);
      });

      test('the note hides under the player', () async {
        await prefs.write(
          UpdatePrefs.whatsNew,
          jsonEncode({'version': '0.1.0-alpha.12', 'notes': ''}),
        );
        final playing = ValueNotifier(true);
        final c = controller(
          windows,
          current: '0.1.0-alpha.12',
          playback: playing,
        );
        await c.start();
        expect(c.whatsNewVisible, isFalse);
        playing.value = false;
        expect(c.whatsNewVisible, isTrue);
      });

      test('Android: downloaded silently, «Установить» when nothing plays, '
          'checked again before the system dialog', () async {
        final playing = ValueNotifier(false);
        final installer = RecordingInstaller(InstallOutcome.installed, true);
        final c = controller(android, installer: installer, playback: playing);
        await c.start();
        await c.tick();
        expect(c.phase, UpdatePhase.readyToInstall);
        expect(installer.installed, isEmpty, reason: 'no dialog by itself');
        expect(c.bannerVisible, isTrue);
        playing.value = true;
        expect(c.bannerVisible, isFalse, reason: 'never over the player');
        playing.value = false;
        await c.update();
        expect(c.phase, UpdatePhase.installed);
        expect(installer.installed.single, endsWith(apk));
        expect(await prefs.read(UpdatePrefs.whatsNew), contains('alpha.12'));
      });

      test('Android: a staged APK changed on disk is refused', () async {
        final installer = RecordingInstaller(InstallOutcome.installed, true);
        final c = controller(android, installer: installer);
        await c.start();
        await c.tick();
        expect(c.phase, UpdatePhase.readyToInstall);
        final staged = File('${cache.path}/updates/$apk');
        staged.writeAsBytesSync(bytes('swapped while waiting'));
        await c.update();
        expect(c.problem, UpdateProblem.checksum);
        expect(installer.installed, isEmpty);
        expect(staged.existsSync(), isFalse);
      });

      test('Android: no silent download over mobile data', () async {
        final installer = RecordingInstaller(InstallOutcome.installed, true);
        final c = controller(
          android,
          installer: installer,
          metered: () async => true,
        );
        await c.start();
        await c.tick();
        expect(c.phase, UpdatePhase.available);
        expect(host.requests, 1);
        expect(c.bannerVisible, isTrue, reason: 'offered instead');
      });
    });

    test('quiet: a failed background check only goes to the log', () async {
      host.failWith = 500;
      final c = controller(windows, installer: RecordingInstaller());
      await c.start();
      await c.tick();
      expect(c.phase, UpdatePhase.idle);
      expect(c.problem, isNull);
      expect(logs.last, startsWith('warn Scheduled update check failed'));
      // Retried 30 minutes later, not at the next tick.
      final tries = host.requests;
      now = now.add(UpdateSchedule.tick);
      await c.tick();
      expect(host.requests, tries);
      now = now.add(UpdateSchedule.tick);
      await c.tick();
      now = now.add(UpdateSchedule.tick);
      await c.tick();
      expect(host.requests, tries + 1);
      // Asked for: the error is shown.
      await c.check();
      expect(c.problem, UpdateProblem.network);
    });

    final platforms = <String, (UpdateTarget, String)>{
      'Windows': (windows, setup),
      'macOS': (
        const UpdateTarget(
          UpdateKind.macApp,
          appPath: '/Applications/Kinolune.app',
          arch: 'arm64',
        ),
        dmg,
      ),
      'Linux AppImage': (
        const UpdateTarget(UpdateKind.appImage, appImagePath: '/x/K.AppImage'),
        appImage,
      ),
      'Android': (
        const UpdateTarget(UpdateKind.androidApk, abis: ['x86_64']),
        apk,
      ),
    };
    for (final MapEntry(key: name, value: (target, asset))
        in platforms.entries) {
      test('$name: a tampered file is refused and deleted', () async {
        host.overrides[asset] = [...good[asset]!]..[2] ^= 1;
        final installer = RecordingInstaller(InstallOutcome.installed);
        final c = controller(target, installer: installer);
        await c.check();
        await c.update();
        expect(c.problem, UpdateProblem.checksum);
        expect(installer.installed, isEmpty);
        final left = Directory('${cache.path}/updates');
        expect(
          left.existsSync() ? left.listSync().whereType<File>() : const [],
          isEmpty,
        );
      });

      test('$name: a truncated file is refused', () async {
        host.overrides[asset] = good[asset]!.sublist(0, 3);
        final installer = RecordingInstaller(InstallOutcome.installed);
        final c = controller(target, installer: installer);
        await c.check();
        await c.update();
        expect(
          c.problem,
          isIn([UpdateProblem.checksum, UpdateProblem.network]),
        );
        expect(installer.installed, isEmpty);
      });

      test('$name: an unsigned release is refused', () async {
        host.releases.single.assets.remove('SHA256SUMS.minisig');
        final installer = RecordingInstaller(InstallOutcome.installed);
        final c = controller(target, installer: installer);
        await c.check();
        await c.update();
        expect(c.problem, UpdateProblem.signature);
        expect(installer.installed, isEmpty);
      });
    }

    test('auto: a tampered background download stays quiet and is not '
        'fetched again', () async {
      host.overrides[setup] = bytes('the evil setup.exe');
      final installer = RecordingInstaller();
      final c = controller(windows, installer: installer);
      await c.start();
      await c.tick();
      expect(c.phase, UpdatePhase.available, reason: 'the offer stays');
      expect(c.problem, isNull);
      expect(installer.installed, isEmpty);
      expect(logs, contains(startsWith('warn Update rejected')));
      final tries = host.requests;
      now = now.add(const Duration(hours: 7));
      await c.backgroundCheck();
      expect(host.requests, tries + 1, reason: 'checked, not downloaded');
      // «Скачать» tries again and says what went wrong.
      await c.update();
      expect(c.problem, UpdateProblem.checksum);
    });
  });

  group('macOS bundle swap', () {
    late Directory dir;
    late String app;
    late List<List<String>> commands;
    setUp(() {
      dir = Directory.systemTemp.createTempSync('pkg022-mac');
      app = '${dir.path}/Kinolune.app';
      Directory('$app/Contents/MacOS').createSync(recursive: true);
      File('$app/Contents/MacOS/Kinolune').writeAsStringSync('old');
      commands = [];
    });
    tearDown(() {
      Process.runSync('chmod', ['-R', 'u+w', dir.path]);
      dir.deleteSync(recursive: true);
    });

    /// hdiutil, ditto, xattr and open as the installer calls them.
    Future<ProcessResult> fakeCommands(String exe, List<String> args) async {
      commands.add([exe, ...args]);
      final name = exe.split('/').last;
      if (name == 'hdiutil' && args.first == 'attach') {
        final mount = args[args.indexOf('-mountpoint') + 1];
        Directory('$mount/Kinolune.app/Contents/MacOS')
            .createSync(recursive: true);
        File('$mount/Kinolune.app/Contents/MacOS/Kinolune')
            .writeAsStringSync(File(args.last).readAsStringSync());
      } else if (name == 'hdiutil' && args.first == 'detach') {
        Directory(args.last)
            .listSync()
            .forEach((e) => e.deleteSync(recursive: true));
      } else if (name == 'ditto') {
        final r = Process.runSync('cp', ['-R', args[0], args[1]]);
        return ProcessResult(0, r.exitCode, '', r.stderr);
      }
      return ProcessResult(0, 0, '', '');
    }

    test('swaps in the verified copy, quarantine off that copy only', () async {
      if (Platform.isWindows) return;
      final image = File('${dir.path}/update.dmg')..writeAsStringSync('new');
      final launched = <List<String>>[];
      var quit = 0;
      final installer = MacAppInstaller(
        app,
        run: fakeCommands,
        launch: (exe, args) async => launched.add([exe, ...args]),
        quit: () => quit++,
        environment: const {},
      );
      expect(await installer.install(image), InstallOutcome.restartNeeded);
      expect(File('$app/Contents/MacOS/Kinolune').readAsStringSync(), 'new');
      final xattr = commands.singleWhere((c) => c.first.endsWith('xattr'));
      expect(xattr, [
        '/usr/bin/xattr',
        '-dr',
        'com.apple.quarantine',
        installer.stagingPath,
      ]);
      expect(commands.where((c) => c.contains('detach')), hasLength(1));
      // The old bundle waits next to it until the next start.
      expect(
        File('${installer.oldPath}/Contents/MacOS/Kinolune').readAsStringSync(),
        'old',
      );
      installer.cleanUp();
      expect(Directory(installer.oldPath).existsSync(), isFalse);
      expect(Directory(app).existsSync(), isTrue);

      await installer.restart();
      expect(launched.single.take(2), ['/bin/sh', '-c']);
      expect(launched.single[2], contains('open -n'));
      expect(launched.single.last, app);
      expect(quit, 1);
    });

    test('an image without the app is refused; the app stays', () async {
      if (Platform.isWindows) return;
      final image = File('${dir.path}/update.dmg')..writeAsStringSync('new');
      final installer = MacAppInstaller(
        app,
        run: (exe, args) async {
          commands.add([exe, ...args]);
          return ProcessResult(0, 0, '', '');
        },
        environment: const {},
      );
      expect(await installer.install(image), InstallOutcome.failed);
      expect(installer.lastError, contains('Kinolune.app'));
      expect(File('$app/Contents/MacOS/Kinolune').readAsStringSync(), 'old');
      expect(commands.where((c) => c.contains('detach')), hasLength(1));
    });

    test('a read-only folder: the verified DMG is opened instead', () async {
      if (Platform.isWindows) return;
      final image = File('${Directory.systemTemp.path}/pkg022-$pid.dmg')
        ..writeAsStringSync('new');
      addTearDown(image.deleteSync);
      Process.runSync('chmod', ['a-w', dir.path]);
      if (canWriteFolder(dir.path)) return; // Running as root.
      final installer = MacAppInstaller(
        app,
        run: fakeCommands,
        environment: const {},
      );
      expect(await installer.install(image), InstallOutcome.openedDmg);
      expect(commands.single, ['/usr/bin/open', image.path]);
      expect(File('$app/Contents/MacOS/Kinolune').readAsStringSync(), 'old');
    });

    test('swapPaths exchanges two folders', () {
      if (Platform.isWindows) return;
      final a = Directory('${dir.path}/a')..createSync();
      File('${a.path}/f').writeAsStringSync('a');
      final b = Directory('${dir.path}/b')..createSync();
      File('${b.path}/f').writeAsStringSync('b');
      expect(swapPaths(a.path, b.path), isTrue);
      expect(File('${a.path}/f').readAsStringSync(), 'b');
      expect(File('${b.path}/f').readAsStringSync(), 'a');
    });
  });
}
