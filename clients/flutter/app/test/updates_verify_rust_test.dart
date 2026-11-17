// SPDX-License-Identifier: MPL-2.0
//
// PKG-018 on the real Rust core (pa-update through pa_core): a download
// that does not match the release's SHA256SUMS — corrupt, truncated or
// replaced — is refused by the verifier the app uses, end to end through
// the controller. The SHA256SUMS here are written by `sha256sum`, as in
// release-app.yml.

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/features/updates/app_version.dart';
import 'package:kinolune/features/updates/installers.dart';
import 'package:kinolune/features/updates/releases.dart';
import 'package:kinolune/features/updates/update_controller.dart';
import 'package:kinolune/features/updates/update_platform.dart';
import 'package:kinolune/features/updates/update_setup.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/services/preferences.dart';

import 'support/fake_release_host.dart';
import 'support/native_library.dart';

class _Installer extends UpdateInstaller {
  final installed = <String>[];

  @override
  Future<InstallOutcome> install(File verified) async {
    installed.add(verified.path);
    return InstallOutcome.installed;
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  // The binding answers every HttpClient request with 400; these tests talk
  // to a real local server (FakeReleaseHost).
  setUpAll(() => HttpOverrides.global = null);

  late Directory dir;
  const asset = 'kinolune-0.1.0-alpha.7-android-x86_64.apk';
  final apk = List<int>.generate(2 << 20, (i) => (i * 7) % 256);

  setUpAll(() async {
    await RustLibrary.ensureInitialized(
      externalLibrary: openBuiltCoreLibrary(),
    );
  });

  setUp(() => dir = Directory.systemTemp.createTempSync('pkg018-verify'));
  tearDown(() => dir.deleteSync(recursive: true));

  Future<List<int>> sha256sums(Map<String, List<int>> files) async {
    final d = Directory('${dir.path}/release')..createSync();
    for (final e in files.entries) {
      File('${d.path}/${e.key}').writeAsBytesSync(e.value);
    }
    final r = await Process.run('sha256sum', [
      '--',
      ...files.keys,
    ], workingDirectory: d.path);
    expect(r.exitCode, 0, reason: '${r.stderr}');
    return utf8.encode('${r.stdout}');
  }

  test('the core verifier passes the real file and refuses others', () async {
    final sums = await sha256sums({
      asset: apk,
      'other.AppImage': [1, 2, 3],
    });
    final verify = coreArtifactVerifier(requireSignature: false);
    final file = File('${dir.path}/$asset')..writeAsBytesSync(apk);
    await verify(file: file, assetName: asset, sums: sums, signature: null);

    // One flipped byte, a truncated file, an empty file.
    for (final bad in [
      [...apk]..[1 << 20] ^= 1,
      apk.sublist(0, apk.length - 1),
      <int>[],
    ]) {
      file.writeAsBytesSync(bad);
      await expectLater(
        verify(file: file, assetName: asset, sums: sums, signature: null),
        throwsA(
          isA<UpdateVerifyFailure>().having(
            (e) => e.signature,
            'signature',
            isFalse,
          ),
        ),
      );
    }
    // Not in SHA256SUMS at all.
    file.writeAsBytesSync(apk);
    await expectLater(
      verify(file: file, assetName: 'evil.apk', sums: sums, signature: null),
      throwsA(isA<UpdateVerifyFailure>()),
    );
    // A release key that does not parse fails closed, as a signature error.
    await expectLater(
      coreArtifactVerifier(releaseKey: () => 'RWnotakey')(
        file: file,
        assetName: asset,
        sums: sums,
        signature: null,
      ),
      throwsA(
        isA<UpdateVerifyFailure>().having((e) => e.signature, 'sig', isTrue),
      ),
    );
  });

  test('PKG-022: the app refuses anything the release key did not sign', () async {
    final sums = await sha256sums({asset: apk});
    final file = File('${dir.path}/$asset')..writeAsBytesSync(apk);
    // No key at all: refused before the checksum is even read.
    await expectLater(
      coreArtifactVerifier(releaseKey: () => null)(
        file: file,
        assetName: asset,
        sums: sums,
        signature: null,
      ),
      throwsA(
        isA<UpdateVerifyFailure>().having((e) => e.signature, 'sig', isTrue),
      ),
    );
    // The project's release key (built-in remote config) and no signature.
    const projectKey = 'RWT/vdmL0r0jFQ27GAP+2yG3QI6P1rU7avFHro4lra/nRJoZMcVHcc59';
    for (final signature in [null, 'untrusted comment: x\nRWQAAAA=\n']) {
      await expectLater(
        coreArtifactVerifier(releaseKey: () => projectKey)(
          file: file,
          assetName: asset,
          sums: sums,
          signature: signature,
        ),
        throwsA(
          isA<UpdateVerifyFailure>().having((e) => e.signature, 'sig', isTrue),
        ),
      );
    }
  });

  test('a tampered release asset is never installed', () async {
    final host = await FakeReleaseHost.start();
    addTearDown(host.close);
    final sums = await sha256sums({asset: apk});
    host.releases.add(
      FakeRelease('0.1.0-alpha.7', assets: {asset: apk, 'SHA256SUMS': sums}),
    );
    final installer = _Installer();
    final c = UpdateController(
      current: AppVersion.tryParse('0.1.0-alpha.6')!,
      target: const UpdateTarget(UpdateKind.androidApk, abis: ['x86_64']),
      source: GitHubReleaseSource(api: host.api),
      verify: coreArtifactVerifier(requireSignature: false),
      cacheDir: Directory('${dir.path}/updates'),
      preferences: MemoryPreferences(),
      installer: installer,
    );
    await c.check();
    expect(c.release?.version.text, '0.1.0-alpha.7');

    // Same size, one byte changed on the way.
    host.overrides[asset] = [...apk]..[12345] ^= 0x40;
    await c.update();
    expect(c.problem, UpdateProblem.checksum);
    expect(installer.installed, isEmpty);
    expect(Directory('${dir.path}/updates').listSync(), isEmpty);

    // The genuine file goes through.
    host.overrides.clear();
    await c.update();
    expect(c.phase, UpdatePhase.installed);
    expect(installer.installed, hasLength(1));
  });
}
