// SPDX-License-Identifier: MPL-2.0
//
// A Plex client for widget tests and screenshots (NET-012): a sign-in that
// is "entered at plex.tv/link" after a few polls, a Plex Home with a
// protected user, and one or two servers.

import 'package:kinolune/sources/plex_link.dart';
import 'package:kinolune/sources/source_manager.dart';

/// A Plex client that records the calls.
class FakePlexLink implements PlexLink {
  FakePlexLink({
    this.pollsUntilLinked = 1,
    this.home = true,
    this.twoServers = true,
    this.expireCode = false,
  });

  final calls = <String>[];
  int pollsUntilLinked;
  final bool home;
  final bool twoServers;

  /// The first code expires instead of being entered.
  bool expireCode;
  int _polls = 0;
  int _handles = 0;

  static const owner = PlexUser(
    uuid: 'owner',
    name: 'Anna',
    admin: true,
    protected: true,
  );
  static const kid = PlexUser(
    uuid: 'kid',
    name: 'Kids',
    protected: true,
    restricted: true,
  );

  List<PlexServer> _servers(String? hint) => [
    PlexServer(
      id: 'living',
      name: 'Living Room',
      version: '1.41.3',
      suggested: hint?.contains('192.168.1.41') ?? false,
    ),
    if (twoServers)
      const PlexServer(
        id: 'friend',
        name: "Max's Plex",
        version: '1.40.5',
        owned: false,
        online: false,
      ),
  ];

  String? _hint;

  PlexSignInState _state({String user = 'owner'}) => PlexSignInState(
    linked: true,
    account: 'Anna',
    users: home ? const [owner, kid] : const [],
    currentUser: user,
    servers: _servers(_hint),
  );

  @override
  Future<PlexPin> start({required String deviceName, String? hint}) async {
    _hint = hint;
    calls.add('start${hint == null ? '' : ' $hint'}');
    _polls = 0;
    return PlexPin(
      handle: BigInt.from(++_handles),
      code: _handles == 1 ? 'K7Q2' : 'M3XP',
      linkUrl: 'https://plex.tv/link',
    );
  }

  @override
  Future<PlexSignInState> poll(BigInt handle) async {
    calls.add('poll $handle');
    if (expireCode) {
      expireCode = false;
      throw const SourceException(
        SourceErrorCode.pathNotFound,
        'server: not found: not found',
      );
    }
    if (++_polls < pollsUntilLinked) {
      return const PlexSignInState(linked: false);
    }
    return _state();
  }

  @override
  Future<PlexSignInState> switchUser(
    BigInt handle,
    String uuid, {
    String? pin,
  }) async {
    calls.add('switch $uuid ${pin ?? '-'}');
    if (uuid == 'kid' && pin != '1234') {
      throw const SourceException(
        SourceErrorCode.permissionDenied,
        'server: permission denied: permission denied',
      );
    }
    return _state(user: uuid);
  }

  @override
  Future<PlexAdded> finish(
    BigInt handle,
    String serverId, {
    String? name,
  }) async {
    calls.add('finish $serverId');
    return PlexAdded(
      source: ManagedSource(
        id: '77',
        type: SourceType.plex,
        name: name ?? (serverId == 'living' ? 'Living Room' : "Max's Plex"),
        address: 'https://192-168-1-41.abc.plex.direct:32400',
        username: 'Anna',
        hasPassword: true,
      ),
      connection: PlexConnection.local,
    );
  }

  @override
  void cancel(BigInt handle) => calls.add('cancel $handle');
}
