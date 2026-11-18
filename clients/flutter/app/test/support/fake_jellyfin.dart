// SPDX-License-Identifier: MPL-2.0
//
// A Jellyfin and Emby client for widget tests and screenshots (NET-014,
// NET-015).

import 'package:kinolune/sources/jellyfin_link.dart';
import 'package:kinolune/sources/source_manager.dart';

/// A Jellyfin client that records the calls.
class FakeJellyfinLink implements JellyfinLink {
  final calls = <String>[];
  int pollsUntilApproved = 2;
  bool quickConnectOff = false;

  ManagedSource _added(String name, [SourceType type = SourceType.jellyfin]) =>
      ManagedSource(
        id: '42',
        type: type,
        name: name,
        address: 'http://jf.test:8096',
        username: 'anna',
        hasPassword: true,
      );

  @override
  Future<JellyfinServerInfo> probe(
    String address, {
    SourceType server = SourceType.jellyfin,
  }) async => const JellyfinServerInfo(name: 'Home', version: '10.10.7');

  @override
  Future<ManagedSource> signIn({
    SourceType server = SourceType.jellyfin,
    required List<String> addresses,
    required String username,
    required String password,
    required String deviceName,
    String? name,
  }) async {
    final kind = server == SourceType.jellyfin ? '' : '${server.name} ';
    calls.add(
      'signIn $kind${addresses.join(' ')} $username ${password.length}',
    );
    if (password != 'secret') {
      throw const SourceException(
        SourceErrorCode.authFailed,
        'jellyfin: authentication failed',
      );
    }
    return _added(name ?? 'Home', server);
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

  /// Items on a server (NET-013); [playback] answers for them.
  final serverItems = <String>{};

  @override
  Future<JellyfinPlay?> playback(
    String itemId, {
    int startMs = 0,
    bool forceTranscode = false,
    int? maxBitrate,
  }) async {
    if (!serverItems.contains(itemId)) return null;
    calls.add(
      'playback $itemId $startMs${forceTranscode ? ' transcode' : ''}'
      '${maxBitrate == null ? '' : ' $maxBitrate'}',
    );
    return forceTranscode
        ? const JellyfinPlay(
            method: 'transcode',
            url: 'http://pms.test/video/:/transcode/universal/start.m3u8',
          )
        : const JellyfinPlay(method: 'direct_play');
  }

  @override
  Future<bool> isServerItem(String itemId) async =>
      serverItems.contains(itemId);

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
