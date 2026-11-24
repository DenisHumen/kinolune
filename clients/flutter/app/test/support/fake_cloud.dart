// SPDX-License-Identifier: MPL-2.0
//
// A made-up cloud side (NET-017) for widget tests: Google Drive signs in
// as anna@example.org after the browser "comes back"; OneDrive has no
// client id in this build; MEGA wants a 2FA code (123456).

import 'dart:async';

import 'package:kinolune/sources/cloud_link.dart';
import 'package:kinolune/sources/source_manager.dart';

class FakeCloudLink implements CloudLink {
  FakeCloudLink({this.declineBrowser = false});

  /// The browser sign-in is declined.
  final bool declineBrowser;

  final calls = <String>[];

  /// Completes [wait]; tests complete it to simulate the browser.
  final browser = Completer<void>();

  @override
  List<CloudProvider> providers() => const [
    CloudProvider(
      type: SourceType.mega,
      name: 'MEGA',
      oauth: false,
      configured: true,
    ),
    CloudProvider(
      type: SourceType.gdrive,
      name: 'Google Drive',
      oauth: true,
      configured: true,
    ),
    CloudProvider(
      type: SourceType.onedrive,
      name: 'OneDrive',
      oauth: true,
      configured: false,
    ),
    CloudProvider(
      type: SourceType.dropbox,
      name: 'Dropbox',
      oauth: true,
      configured: true,
    ),
  ];

  @override
  Future<CloudAuth> start(
    SourceType type, {
    String? clientId,
    String? clientSecret,
    bool appRedirect = false,
  }) async {
    calls.add('start ${type.name} ${clientId ?? '-'}');
    return CloudAuth(
      handle: BigInt.one,
      url: 'https://accounts.example/auth?state=s',
    );
  }

  @override
  Future<String> wait(
    BigInt handle, {
    Duration timeout = const Duration(minutes: 5),
  }) async {
    await browser.future;
    if (declineBrowser) throw const CloudSignInCancelled();
    calls.add('wait');
    return 'anna@example.org';
  }

  @override
  Future<String> complete(BigInt handle, String redirect) async {
    calls.add('complete');
    return 'anna@example.org';
  }

  @override
  Future<ManagedSource> finish(
    BigInt handle, {
    String? name,
    String? folder,
  }) async {
    calls.add('finish ${folder ?? '-'}');
    return ManagedSource(
      id: '88',
      type: SourceType.gdrive,
      name: name ?? 'Google Drive · anna@example.org',
      address: 'gdrive:/${folder ?? ''}',
      username: 'anna@example.org',
      hasPassword: true,
    );
  }

  @override
  void cancel(BigInt handle) => calls.add('cancel');

  @override
  Future<ManagedSource> addMega({
    required String email,
    required String password,
    String? mfa,
    String? name,
    String? folder,
  }) async {
    calls.add('mega $email ${mfa ?? '-'}');
    if (mfa != '123456') throw const CloudMfaRequired();
    return ManagedSource(
      id: '89',
      type: SourceType.mega,
      name: name ?? 'MEGA · $email',
      address: 'mega:/${folder ?? ''}',
      username: email,
      hasPassword: true,
    );
  }

  @override
  Future<ManagedSource> addMegaLink(String link, {String? name}) async {
    calls.add('mega-link');
    return ManagedSource(
      id: '90',
      type: SourceType.mega,
      name: name ?? 'MEGA',
      address: 'https://mega.nz/folder/AbCd1234',
      hasPassword: true,
    );
  }
}
