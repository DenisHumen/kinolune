// SPDX-License-Identifier: MPL-2.0
//
// Plex Media Servers (NET-012, ADR-0131) for the app: the sign-in with a
// code entered at plex.tv/link — no password in the app — then the Plex
// Home user (with their PIN when protected) and the server. The core picks
// the connection that answers (local, then remote, then relay). A Plex
// source is otherwise a source like the others: the core scans its
// libraries into the library and streams its files through the proxy.

import '../integration/plex.dart';
import 'core_source_manager.dart';
import 'source_manager.dart';

/// A sign-in waiting for its code to be entered.
class PlexPin {
  const PlexPin({
    required this.handle,
    required this.code,
    required this.linkUrl,
  });

  final BigInt handle;

  /// Four characters to enter at [linkUrl].
  final String code;

  /// `https://plex.tv/link`.
  final String linkUrl;
}

/// A user of the account's Plex Home.
class PlexUser {
  const PlexUser({
    required this.uuid,
    required this.name,
    this.admin = false,
    this.protected = false,
    this.restricted = false,
  });

  final String uuid;
  final String name;

  /// The Home's owner.
  final bool admin;

  /// Needs their PIN.
  final bool protected;
  final bool restricted;
}

/// A server the user can reach.
class PlexServer {
  const PlexServer({
    required this.id,
    required this.name,
    this.version = '',
    this.owned = true,
    this.online = true,
    this.suggested = false,
  });

  final String id;
  final String name;
  final String version;
  final bool owned;
  final bool online;

  /// Preselected: found on the network, or the only one.
  final bool suggested;
}

/// Where a sign-in stands.
class PlexSignInState {
  const PlexSignInState({
    required this.linked,
    this.account = '',
    this.users = const [],
    this.currentUser,
    this.servers = const [],
  });

  /// The code has been entered.
  final bool linked;
  final String account;

  /// Home users to choose from (empty: nothing to choose).
  final List<PlexUser> users;
  final String? currentUser;
  final List<PlexServer> servers;
}

/// How an added server is reached.
enum PlexConnection { local, remote, relay }

/// A Plex server added as a source.
class PlexAdded {
  const PlexAdded({required this.source, required this.connection});

  final ManagedSource source;
  final PlexConnection connection;
}

/// The Plex side of the app.
abstract class PlexLink {
  /// Starts a sign-in; [hint] is the URL of a server found on the network,
  /// preselected once the servers are known.
  Future<PlexPin> start({required String deviceName, String? hint});

  /// Whether the code has been entered; then the users and servers.
  Future<PlexSignInState> poll(BigInt handle);

  /// Switches to Home user [uuid]; [pin] when the user is protected.
  Future<PlexSignInState> switchUser(BigInt handle, String uuid, {String? pin});

  /// Adds server [serverId] as a source over the connection that answers.
  Future<PlexAdded> finish(BigInt handle, String serverId, {String? name});

  void cancel(BigInt handle);
}

/// [PlexLink] on the Rust core (`CorePlex`).
class CorePlexLink implements PlexLink {
  CorePlexLink(this._core);

  final CorePlex _core;

  /// Opens the link over the library [library] with the secret store whose
  /// fallback file is in [dataDir].
  static Future<CorePlexLink> open({
    required CoreLibrary library,
    required String dataDir,
  }) async =>
      CorePlexLink(await CorePlex.open(library_: library, dataDir: dataDir));

  static Future<T> _guard<T>(Future<T> Function() call) async {
    try {
      return await call();
    } on SourcesError catch (e) {
      throw SourceException.fromCore(e.code, e.message);
    }
  }

  static PlexSignInState _state(CorePlexState s) => PlexSignInState(
    linked: s.linked,
    account: s.account,
    users: [
      for (final u in s.users)
        PlexUser(
          uuid: u.uuid,
          name: u.name,
          admin: u.admin,
          protected: u.protected,
          restricted: u.restricted,
        ),
    ],
    currentUser: s.currentUser,
    servers: [
      for (final r in s.servers)
        PlexServer(
          id: r.id,
          name: r.name,
          version: r.version,
          owned: r.owned,
          online: r.online,
          suggested: r.suggested,
        ),
    ],
  );

  @override
  Future<PlexPin> start({required String deviceName, String? hint}) =>
      _guard(() async {
        final p = await _core.start(deviceName: deviceName, hint: hint);
        return PlexPin(handle: p.handle, code: p.code, linkUrl: p.linkUrl);
      });

  @override
  Future<PlexSignInState> poll(BigInt handle) =>
      _guard(() async => _state(await _core.poll(handle: handle)));

  @override
  Future<PlexSignInState> switchUser(
    BigInt handle,
    String uuid, {
    String? pin,
  }) => _guard(
    () async =>
        _state(await _core.switchUser(handle: handle, uuid: uuid, pin: pin)),
  );

  @override
  Future<PlexAdded> finish(BigInt handle, String serverId, {String? name}) =>
      _guard(() async {
        final a = await _core.finish(
          handle: handle,
          serverId: serverId,
          name: name,
        );
        return PlexAdded(
          source: CoreSourceManager.sourceOf(a.source),
          connection: PlexConnection.values.firstWhere(
            (c) => c.name == a.connection,
            orElse: () => PlexConnection.remote,
          ),
        );
      });

  @override
  void cancel(BigInt handle) => _core.cancel(handle: handle);
}
