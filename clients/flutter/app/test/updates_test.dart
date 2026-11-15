// SPDX-License-Identifier: MPL-2.0
//
// PKG-018: in-app updates without the core — versions, the releases API,
// asset choice, the AppImage replacement and the controller's flow against
// a local stand-in for GitHub. The checksum itself is the core's
// (core/crates/pa-update, test/updates_verify_rust_test.dart); here a fake
// verifier compares bytes.

import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/features/updates/app_version.dart';
import 'package:kinolune/features/updates/installers.dart';
import 'package:kinolune/features/updates/releases.dart';
import 'package:kinolune/features/updates/update_controller.dart';
import 'package:kinolune/features/updates/update_platform.dart';
import 'package:kinolune/services/preferences.dart';

import 'support/fake_release_host.dart';

AppVersion v(String s) => AppVersion.tryParse(s)!;

const sums = 'SHA256SUMS';

List<int> bytes(String s) => utf8.encode(s);

/// "Checksums" of the fake verifier: what each asset must contain.
ArtifactVerifier fakeVerifier(Map<String, List<int>> expected) =>
    ({
      required File file,
      required String assetName,
      required List<int> sums,
      required String? signature,
    }) async {
      final want = expected[assetName];
      if (want == null) throw UpdateVerifyFailure('$assetName not listed');
      if (!sameBytes(file.readAsBytesSync(), want)) {
        throw UpdateVerifyFailure('checksum mismatch for $assetName');
      }
    };

bool sameBytes(List<int> a, List<int> b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) return false;
  }
  return true;
}

class FakeInstaller extends UpdateInstaller {
  FakeInstaller(this.outcome);

  InstallOutcome outcome;
  final installed = <List<int>>[];

  @override
  Future<InstallOutcome> install(File verified) async {
    installed.add(verified.readAsBytesSync());
    return outcome;
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  // The binding answers every HttpClient request with 400; these tests talk
  // to a real local server (FakeReleaseHost).
  setUpAll(() => HttpOverrides.global = null);

  group('AppVersion', () {
    test('orders by semver precedence', () {
      final ordered = [
        '0.1.0-alpha.5',
        '0.1.0-alpha.6',
        '0.1.0-alpha.10',
        '0.1.0-alpha.10.1',
        '0.1.0-beta',
        '0.1.0-beta.2',
        '0.1.0-rc.1',
        '0.1.0',
        '0.1.1',
        '0.2.0',
        '1.0.0',
      ].map(v).toList();
      for (var i = 0; i + 1 < ordered.length; i++) {
        expect(ordered[i] < ordered[i + 1], isTrue, reason: '${ordered[i]}');
      }
      expect(v('app-v0.1.0-alpha.6'), v('0.1.0-alpha.6'));
      expect(v('v1.2.3+99'), v('1.2.3'));
      expect(v('0.1.0-alpha.6').isPrerelease, isTrue);
      expect(v('0.1.0').isPrerelease, isFalse);
    });

    test('rejects what is not a version', () {
      for (final bad in [
        '',
        '1',
        '1.2',
        '1.2.3.4',
        'a.b.c',
        '1.2.3-',
        '1.2.3-a..b',
      ]) {
        expect(AppVersion.tryParse(bad), isNull, reason: bad);
      }
    });
  });

  group('releases', () {
    Map<String, dynamic> entry(
      String tag, {
      bool prerelease = false,
      bool draft = false,
      String body = '',
      String? name,
    }) => {
      'tag_name': tag,
      'name': name ?? tag,
      'draft': draft,
      'prerelease': prerelease,
      'body': body,
      'html_url': 'https://github.com/x/y/releases/tag/$tag',
      'assets': [
        {
          'name': 'SHA256SUMS',
          'size': 10,
          'browser_download_url': 'https://example.invalid/SHA256SUMS',
        },
      ],
    };

    test('only published app-v releases', () {
      expect(AppRelease.fromGitHub(entry('app-v0.1.0')), isNotNull);
      expect(AppRelease.fromGitHub(entry('app-v0.1.0', draft: true)), isNull);
      expect(AppRelease.fromGitHub(entry('os-v1.0.0')), isNull);
      expect(AppRelease.fromGitHub(entry('app-vnext')), isNull);
      final r = AppRelease.fromGitHub(entry('app-v0.2.0-alpha.1'))!;
      expect(r.prerelease, isTrue, reason: 'a pre-release version');
      expect(r.asset('SHA256SUMS')?.size, 10);
    });

    test('the changelog stops at the generated downloads table', () {
      final r = AppRelease.fromGitHub(
        entry(
          'app-v0.1.0-alpha.6',
          name: 'Kinolune 0.1.0-alpha.6 (Android debug-signed)',
          body:
              '> [!WARNING]\n> **Unsigned / debug-signed Android build.**\n\n'
              'Movie and show pages\n\n- music\n\n## Downloads\n\n| Platform | File |\n'
              'Android signing: **debug**.',
        ),
      )!;
      expect(r.changelog, 'Movie and show pages\n\n- music');
      expect(r.androidDebugSigned, isTrue);
    });

    test('newest update per channel', () {
      final list = [
        for (final t in [
          'app-v0.1.0-alpha.5',
          'app-v0.1.0-alpha.7',
          'app-v0.0.9',
        ])
          AppRelease.fromGitHub(entry(t, prerelease: t.contains('-')))!,
      ];
      expect(
        newestUpdate(list, v('0.1.0-alpha.6'), UpdateChannel.beta)?.tag,
        'app-v0.1.0-alpha.7',
      );
      expect(
        newestUpdate(list, v('0.1.0-alpha.6'), UpdateChannel.stable),
        isNull,
      );
      expect(
        newestUpdate(list, v('0.1.0-alpha.7'), UpdateChannel.beta),
        isNull,
      );
    });
  });

  group('assets', () {
    final release = AppRelease(
      tag: 'app-v0.1.0-alpha.7',
      version: v('0.1.0-alpha.7'),
      prerelease: true,
      notes: '',
      page: Uri.parse('https://example.invalid'),
      assets: [
        for (final n in [
          'kinolune-0.1.0-alpha.7-android-arm64-v8a.apk',
          'kinolune-0.1.0-alpha.7-android-x86_64.apk',
          'kinolune-0.1.0-alpha.7-android-universal.apk',
          'kinolune-0.1.0-alpha.7-x86_64.AppImage',
          'kinolune-0.1.0-alpha.7-linux-x86_64.tar.gz',
        ])
          ReleaseAsset(name: n, url: Uri.parse('https://example.invalid/$n')),
      ],
    );

    test('the APK of the first supported ABI, else universal', () {
      String? pick(List<String> abis) => UpdateTarget(
        UpdateKind.androidApk,
        abis: abis,
      ).assetIn(release)?.name;
      expect(
        pick(['arm64-v8a', 'armeabi-v7a', 'armeabi']),
        'kinolune-0.1.0-alpha.7-android-arm64-v8a.apk',
      );
      expect(
        pick(['x86_64', 'arm64-v8a']),
        'kinolune-0.1.0-alpha.7-android-x86_64.apk',
      );
      expect(
        pick(['armeabi-v7a']),
        'kinolune-0.1.0-alpha.7-android-universal.apk',
      );
    });

    test('Linux: AppImage or archive; others: none', () {
      expect(
        const UpdateTarget(UpdateKind.appImage).assetIn(release)?.name,
        'kinolune-0.1.0-alpha.7-x86_64.AppImage',
      );
      expect(
        const UpdateTarget(UpdateKind.linuxArchive).assetIn(release)?.name,
        'kinolune-0.1.0-alpha.7-linux-x86_64.tar.gz',
      );
      expect(
        const UpdateTarget(UpdateKind.releasePage).assetIn(release),
        isNull,
      );
    });

    test(
      'detects AppImage, archive and Flatpak from the environment',
      () async {
        if (!Platform.isLinux) return;
        expect(
          (await UpdateTarget.detect(
            environment: {'APPIMAGE': '/a/K.AppImage'},
          )).kind,
          UpdateKind.appImage,
        );
        expect(
          (await UpdateTarget.detect(environment: {})).kind,
          UpdateKind.linuxArchive,
        );
        expect(
          (await UpdateTarget.detect(
            environment: {'FLATPAK_ID': 'app.kinolune'},
          )).kind,
          UpdateKind.managed,
        );
      },
    );
  });

  group('AppImage replacement', () {
    late Directory dir;
    setUp(() => dir = Directory.systemTemp.createTempSync('pkg018-appimage'));
    tearDown(() => dir.deleteSync(recursive: true));

    test(
      'renames the verified download over the AppImage, executable',
      () async {
        if (!Platform.isLinux) return;
        final appImage = File(
          '${dir.path}/kinolune-0.1.0-alpha.6-x86_64.AppImage',
        )..writeAsStringSync('old');
        await Process.run('chmod', ['755', appImage.path]);
        final installer = AppImageInstaller(
          appImage.path,
          relaunch: (_) async {},
        );
        final download = installer.downloadFile(dir, 'ignored')
          ..writeAsStringSync('new');
        expect(download.parent.path, dir.path, reason: 'same folder: atomic');
        expect(await installer.install(download), InstallOutcome.restartNeeded);
        expect(appImage.readAsStringSync(), 'new');
        expect(download.existsSync(), isFalse);
        expect(appImage.statSync().mode & 0x1ff, 0x1ed); // 0755
        // Nothing else is left in the folder.
        expect(dir.listSync().map((e) => e.path), [appImage.path]);
      },
    );

    test('a read-only folder is reported, the AppImage stays', () async {
      if (!Platform.isLinux) return;
      final appImage = File('${dir.path}/K.AppImage')..writeAsStringSync('old');
      expect(canReplaceAppImage(appImage.path), isTrue);
      expect(canReplaceAppImage('${dir.path}/missing.AppImage'), isFalse);
      final elsewhere = File('${dir.path}/../pkg018-elsewhere.AppImage');
      await expectLater(
        replaceAppImage(appImage, elsewhere),
        throwsA(isA<FileSystemException>()),
      );
      expect(appImage.readAsStringSync(), 'old');
    });
  });

  group('UpdateController', () {
    late FakeReleaseHost host;
    late Directory cache;
    late MemoryPreferences prefs;
    var now = DateTime.utc(2026, 11, 15, 12);

    const apk = 'kinolune-0.1.0-alpha.7-android-x86_64.apk';
    final good = bytes('the new apk');

    setUp(() async {
      host = await FakeReleaseHost.start();
      cache = Directory.systemTemp.createTempSync('pkg018-cache');
      prefs = MemoryPreferences();
      now = DateTime.utc(2026, 11, 15, 12);
      host.releases.addAll([
        FakeRelease('0.1.0-alpha.6', assets: {sums: bytes('x')}),
        FakeRelease(
          '0.1.0-alpha.7',
          notes: 'Faster scans\n\n## Downloads\n| x |',
          assets: {apk: good, sums: bytes('sums')},
        ),
      ]);
    });

    tearDown(() async {
      await host.close();
      if (cache.existsSync()) cache.deleteSync(recursive: true);
    });

    UpdateController controller({
      String version = '0.1.0-alpha.6',
      UpdateInstaller? installer,
      bool killed = false,
    }) => UpdateController(
      current: v(version),
      target: const UpdateTarget(UpdateKind.androidApk, abis: ['x86_64']),
      source: GitHubReleaseSource(api: host.api),
      verify: fakeVerifier({apk: good}),
      cacheDir: Directory('${cache.path}/updates'),
      preferences: prefs,
      installer: installer,
      installKilled: killed,
      clock: () => now,
    );

    test('the daily check runs once a day and can be turned off', () async {
      final c = controller(installer: FakeInstaller(InstallOutcome.installed));
      await c.start();
      expect(host.requests, 1);
      expect(c.phase, UpdatePhase.available);
      expect(c.release?.version.text, '0.1.0-alpha.7');
      expect(c.release?.changelog, 'Faster scans');
      expect(c.bannerVisible, isTrue);

      now = now.add(const Duration(hours: 23));
      await c.maybeAutoCheck();
      expect(host.requests, 1, reason: 'not due yet');
      now = now.add(const Duration(hours: 2));
      await c.maybeAutoCheck();
      expect(host.requests, 2);

      await c.setAutoCheck(false);
      now = now.add(const Duration(days: 3));
      await c.maybeAutoCheck();
      expect(host.requests, 2);
      expect(await prefs.read(UpdatePrefs.autoCheck), 'false');

      // A new controller (the next start) remembers both.
      final next = controller();
      await next.start();
      expect(next.autoCheck, isFalse);
      expect(host.requests, 2);
    });

    test('the request carries no identifiers', () async {
      await controller().check();
      final uri = host.seenUris.single;
      expect(uri.queryParameters, {'per_page': '30'});
      final h = host.seenHeaders.single;
      expect(h.value(HttpHeaders.userAgentHeader), 'Kinolune');
      expect(h.value(HttpHeaders.cookieHeader), isNull);
      expect(h.value(HttpHeaders.authorizationHeader), isNull);
    });

    test('channels: stable ignores pre-releases', () async {
      final c = controller();
      await c.start();
      expect(c.channel, UpdateChannel.beta, reason: 'alpha app: beta');
      await c.setChannel(UpdateChannel.stable);
      expect(c.release, isNull);
      await c.check();
      expect(c.phase, UpdatePhase.upToDate);
      expect(await prefs.read(UpdatePrefs.channel), 'stable');
      // A stable app defaults to stable.
      expect(controller(version: '0.1.0').channel, UpdateChannel.stable);
    });

    test('downloads, verifies and installs', () async {
      final installer = FakeInstaller(InstallOutcome.installed);
      final c = controller(installer: installer);
      await c.check();
      await c.update();
      expect(c.phase, UpdatePhase.installed);
      expect(installer.installed.single, good);
    });

    test(
      'a tampered download is rejected, deleted and not installed',
      () async {
        host.overrides[apk] = bytes('the evil apk');
        final installer = FakeInstaller(InstallOutcome.installed);
        final c = controller(installer: installer);
        await c.check();
        await c.update();
        expect(c.phase, UpdatePhase.failed);
        expect(c.problem, UpdateProblem.checksum);
        expect(installer.installed, isEmpty);
        expect(
          Directory('${cache.path}/updates').listSync().whereType<File>(),
          isEmpty,
          reason: 'the rejected file is gone',
        );
      },
    );

    test('a truncated download is rejected', () async {
      host.overrides[apk] = good.sublist(0, 4);
      final installer = FakeInstaller(InstallOutcome.installed);
      final c = controller(installer: installer);
      await c.check();
      await c.update();
      expect(c.problem, isIn([UpdateProblem.checksum, UpdateProblem.network]));
      expect(installer.installed, isEmpty);
    });

    test('a release signed with another key: reinstall message', () async {
      final c = controller(
        installer: FakeInstaller(InstallOutcome.signatureMismatch),
      );
      await c.check();
      await c.update();
      expect(c.problem, UpdateProblem.signatureMismatch);
      expect(c.bannerVisible, isTrue);
    });

    test('a kill-switch leaves installing to the user', () async {
      final installer = FakeInstaller(InstallOutcome.installed);
      final c = controller(installer: installer, killed: true);
      await c.check();
      expect(c.canInstall, isFalse);
      await c.update();
      expect(c.problem, UpdateProblem.killed);
      expect(installer.installed, isEmpty);
    });

    test('rate limits and errors: silent when automatic', () async {
      host.failWith = 403;
      final c = controller();
      await c.start();
      expect(c.phase, UpdatePhase.idle, reason: 'automatic: no error shown');
      await c.check();
      expect(c.problem, UpdateProblem.rateLimited);
    });

    test('Later hides the banner for that release only', () async {
      final c = controller();
      await c.check();
      await c.dismiss();
      expect(c.bannerVisible, isFalse);
      expect(c.release, isNotNull, reason: 'About still offers it');
      expect(await prefs.read(UpdatePrefs.dismissed), 'app-v0.1.0-alpha.7');
    });

    test('AppImage: replaced in place, then restart', () async {
      if (!Platform.isLinux) return;
      const name = 'kinolune-0.1.0-alpha.7-x86_64.AppImage';
      final newImage = bytes('#!/bin/sh\necho new\n');
      host.releases.last.assets[name] = newImage;
      final dir = Directory.systemTemp.createTempSync('pkg018-run');
      addTearDown(() => dir.deleteSync(recursive: true));
      final appImage = File(
        '${dir.path}/kinolune-0.1.0-alpha.6-x86_64.AppImage',
      )..writeAsStringSync('#!/bin/sh\necho old\n');
      await Process.run('chmod', ['755', appImage.path]);
      String? relaunched;
      final c = UpdateController(
        current: v('0.1.0-alpha.6'),
        target: UpdateTarget(UpdateKind.appImage, appImagePath: appImage.path),
        source: GitHubReleaseSource(api: host.api),
        verify: fakeVerifier({name: newImage}),
        cacheDir: Directory('${cache.path}/updates'),
        preferences: prefs,
        installer: AppImageInstaller(
          appImage.path,
          relaunch: (p) async => relaunched = p,
        ),
        clock: () => now,
      );
      await c.check();
      await c.update();
      expect(c.phase, UpdatePhase.restartNeeded);
      final run = await Process.run(appImage.path, []);
      expect('${run.stdout}'.trim(), 'new');
      await c.restart();
      expect(relaunched, appImage.path);

      // A tampered AppImage never replaces the running one.
      host.overrides[name] = bytes('#!/bin/sh\necho evil\n');
      appImage.writeAsStringSync('#!/bin/sh\necho old\n');
      await c.check();
      await c.update();
      expect(c.problem, UpdateProblem.checksum);
      expect('${(await Process.run(appImage.path, [])).stdout}'.trim(), 'old');
      expect(dir.listSync().map((e) => e.path), [appImage.path]);
    });
  });

  group('Android installer', () {
    const channel = MethodChannel('app.kinolune/updates');
    final calls = <String>[];
    var signature = 'same';
    var canInstall = true;
    var result = 'installed';

    setUp(() {
      calls.clear();
      signature = 'same';
      canInstall = true;
      result = 'installed';
    });

    AndroidApkInstaller installer() {
      final bridge = AndroidUpdateBridge(channel: channel);
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, (call) async {
            calls.add(call.method);
            switch (call.method) {
              case 'deviceInfo':
                return {
                  'abis': ['x86_64'],
                  'canInstall': canInstall,
                };
              case 'signature':
                return signature;
              case 'install':
                // PackageInstaller answers later, through the receiver.
                Future<void>.delayed(Duration.zero, () async {
                  await TestDefaultBinaryMessengerBinding
                      .instance
                      .defaultBinaryMessenger
                      .handlePlatformMessage(
                        channel.name,
                        channel.codec.encodeMethodCall(
                          MethodCall('installResult', {'result': result}),
                        ),
                        (_) {},
                      );
                });
                return null;
            }
            return null;
          });
      return AndroidApkInstaller(bridge);
    }

    final apk = File('${Directory.systemTemp.path}/pkg018.apk');

    test('another signing key is caught before the system dialog', () async {
      signature = 'different';
      expect(await installer().install(apk), InstallOutcome.signatureMismatch);
      expect(calls, ['signature']);
    });

    test('asks for "Install unknown apps" first', () async {
      canInstall = false;
      expect(await installer().install(apk), InstallOutcome.needsPermission);
      expect(calls, ['signature', 'deviceInfo']);
    });

    test('maps PackageInstaller results', () async {
      for (final (r, outcome) in [
        ('installed', InstallOutcome.installed),
        ('aborted', InstallOutcome.aborted),
        ('incompatible', InstallOutcome.signatureMismatch),
        ('blocked', InstallOutcome.needsPermission),
        ('failed', InstallOutcome.failed),
      ]) {
        result = r;
        expect(await installer().install(apk), outcome, reason: r);
      }
    });
  });
}
