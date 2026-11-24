// SPDX-License-Identifier: MPL-2.0
//
// Cloud storages (NET-017) for the app: MEGA with e-mail and password (and
// the 2FA code when the account has one) or a public link; Google Drive,
// OneDrive and Dropbox with a browser sign-in (OAuth with PKCE, no
// password in the app). The core keeps only the MEGA session or the
// refresh token. A cloud source is otherwise a source like the others: the
// core scans it into the library and streams its files through the proxy.

import 'dart:async';

import '../integration/cloud.dart';
import 'core_source_manager.dart';
import 'source_manager.dart';

/// A cloud the app can add.
class CloudProvider {
  const CloudProvider({
    required this.type,
    required this.name,
    required this.oauth,
    required this.configured,
  });

  final SourceType type;
  final String name;

  /// Signs in in the browser; else with e-mail and password (MEGA).
  final bool oauth;

  /// This build has a client id; else the user enters their own.
  final bool configured;
}

/// A browser sign-in waiting for the user.
class CloudAuth {
  const CloudAuth({required this.handle, required this.url});

  final BigInt handle;

  /// The page to open in the browser.
  final String url;
}

/// MEGA asked for the two-factor code (`mfa_required`).
class CloudMfaRequired implements Exception {
  const CloudMfaRequired();
}

/// The browser sign-in was declined or cancelled.
class CloudSignInCancelled implements Exception {
  const CloudSignInCancelled();
}

/// `kinolune://oauth?…` links the app was opened with (Android, a client
/// registered with the app link); the sign-in form waiting for one takes
/// it.
class CloudRedirects {
  CloudRedirects._();

  static final _controller = StreamController<String>.broadcast();

  static Stream<String> get stream => _controller.stream;

  static void deliver(String link) => _controller.add(link);
}

/// The cloud side of the app.
abstract class CloudLink {
  /// The clouds in the order to list them.
  List<CloudProvider> providers();

  /// Starts a browser sign-in; [clientId] (and [clientSecret]): the user's
  /// own client.
  Future<CloudAuth> start(
    SourceType type, {
    String? clientId,
    String? clientSecret,
    bool appRedirect = false,
  });

  /// Waits for the browser to come back; returns the account.
  Future<String> wait(BigInt handle, {Duration timeout});

  /// Signs in with the app link the browser came back to.
  Future<String> complete(BigInt handle, String redirect);

  /// Adds the signed-in cloud as a source.
  Future<ManagedSource> finish(BigInt handle, {String? name, String? folder});

  void cancel(BigInt handle);

  /// Signs in to MEGA and adds the account; throws [CloudMfaRequired]
  /// when the 2FA code is needed.
  Future<ManagedSource> addMega({
    required String email,
    required String password,
    String? mfa,
    String? name,
    String? folder,
  });

  /// Adds a public MEGA link (with the key after `#`).
  Future<ManagedSource> addMegaLink(String link, {String? name});
}

/// [CloudLink] on the Rust core (`CoreCloud`).
class CoreCloudLink implements CloudLink {
  CoreCloudLink(this._core);

  final CoreCloud _core;

  static Future<CoreCloudLink> open({
    required CoreLibrary library,
    required String dataDir,
  }) async =>
      CoreCloudLink(await CoreCloud.open(library_: library, dataDir: dataDir));

  static Future<T> _guard<T>(Future<T> Function() call) async {
    try {
      return await call();
    } on SourcesError catch (e) {
      if (e.code == 'mfa_required') throw const CloudMfaRequired();
      if (e.code == 'cancelled') throw const CloudSignInCancelled();
      throw SourceException.fromCore(e.code, e.message);
    }
  }

  @override
  List<CloudProvider> providers() => [
    for (final p in _core.providers())
      if (SourceType.fromCoreKind(p.id) case final t?)
        CloudProvider(
          type: t,
          name: p.name,
          oauth: p.oauth,
          configured: p.configured,
        ),
  ];

  @override
  Future<CloudAuth> start(
    SourceType type, {
    String? clientId,
    String? clientSecret,
    bool appRedirect = false,
  }) => _guard(() async {
    final a = await _core.start(
      cloud: type.coreKind,
      clientId: clientId,
      clientSecret: clientSecret,
      appRedirect: appRedirect,
    );
    return CloudAuth(handle: a.handle, url: a.url);
  });

  @override
  Future<String> wait(
    BigInt handle, {
    Duration timeout = const Duration(minutes: 5),
  }) =>
      _guard(() => _core.wait(handle: handle, timeoutSecs: timeout.inSeconds));

  @override
  Future<String> complete(BigInt handle, String redirect) =>
      _guard(() => _core.complete(handle: handle, redirect: redirect));

  @override
  Future<ManagedSource> finish(BigInt handle, {String? name, String? folder}) =>
      _guard(() async {
        final s = await _core.finish(
          handle: handle,
          name: name,
          folder: folder,
        );
        return CoreSourceManager.sourceOf(s);
      });

  @override
  void cancel(BigInt handle) => _core.cancel(handle: handle);

  @override
  Future<ManagedSource> addMega({
    required String email,
    required String password,
    String? mfa,
    String? name,
    String? folder,
  }) => _guard(() async {
    final s = await _core.addMega(
      email: email,
      password: password,
      mfa: mfa,
      name: name,
      folder: folder,
    );
    return CoreSourceManager.sourceOf(s);
  });

  @override
  Future<ManagedSource> addMegaLink(String link, {String? name}) =>
      _guard(() async {
        final s = await _core.addMegaLink(link: link, name: name);
        return CoreSourceManager.sourceOf(s);
      });
}
