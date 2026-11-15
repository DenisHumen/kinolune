// SPDX-License-Identifier: MPL-2.0
//
// A Jellyfin client for widget tests and screenshots (NET-014).

import 'package:kinolune/sources/jellyfin_link.dart';
import 'package:kinolune/sources/source_manager.dart';

/// A Jellyfin client that records the calls.
class FakeJellyfinLink implements JellyfinLink {
  final calls = <String>[];
  int pollsUntilApproved = 2;
  bool quickConnectOff = false;

  ManagedSource _added(String name) => ManagedSource(
    id: '42',
    type: SourceType.jellyfin,
    name: name,
    address: 'http://jf.test:8096',
    username: 'anna',
    hasPassword: true,
  );

  @override
  Future<JellyfinServerInfo> probe(String address) async =>
      const JellyfinServerInfo(name: 'Home', version: '10.10.7');

  @override
  Future<ManagedSource> signIn({
    required List<String> addresses,
    required String username,
    required String password,
    required String deviceName,
    String? name,
  }) async {
    calls.add('signIn ${addresses.join(' ')} $username ${password.length}');
    if (password != 'secret') {
      throw const SourceException(
        SourceErrorCode.authFailed,
        'jellyfin: authentication failed',
      );
    }
    return _added(name ?? 'Home');
  }

  @override
  Future<QuickConnectTicket> quickConnectStart({
    required List<String> addresses,
    required String deviceName,
  }) async {
    calls.add('qcStart ${addresses.join(' ')}');
    if (quickConnectOff) {
      throw const SourceException(SourceErrorCode.authFailed, 'off');
    }
    return QuickConnectTicket(
      handle: BigInt.one,
      code: '123456',
      serverName: 'Home',
    );
  }

  @override
  Future<bool> quickConnectPoll(BigInt handle) async {
    calls.add('qcPoll');
    return --pollsUntilApproved <= 0;
  }

  @override
  Future<ManagedSource> quickConnectFinish(
    BigInt handle, {
    String? name,
  }) async {
    calls.add('qcFinish');
    return _added(name ?? 'Home');
  }

  @override
  void quickConnectCancel(BigInt handle) => calls.add('qcCancel');

  @override
  Future<JellyfinPlay?> playback(
    String itemId, {
    int startMs = 0,
    bool forceTranscode = false,
  }) async => null;

  @override
  Future<void> report(
    String itemId,
    int positionMs, {
    bool paused = false,
  }) async => calls.add('report $itemId $positionMs');

  @override
  Future<void> stopped(String itemId, int positionMs) async =>
      calls.add('stopped $itemId $positionMs');

  @override
  Future<void> setFavorite(String itemId, bool favorite) async =>
      calls.add('favorite $itemId $favorite');

  @override
  Future<void> setWatched(String itemId, bool watched) async =>
      calls.add('watched $itemId $watched');

  @override
  Future<void> removeFromResume(String itemId) async =>
      calls.add('unresume $itemId');

  @override
  Future<int> importUserData(String sourceId) async {
    calls.add('import $sourceId');
    return 0;
  }
}
