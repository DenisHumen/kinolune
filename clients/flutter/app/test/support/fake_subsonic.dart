// SPDX-License-Identifier: MPL-2.0
//
// A fake Subsonic link (NET-016) for widget tests: the password `secret`
// (or the API key `key-1`) is right, anything else is refused; every call
// is recorded.

import 'package:kinolune/sources/source_manager.dart';
import 'package:kinolune/sources/subsonic_link.dart';

class FakeSubsonicLink implements SubsonicLink {
  final List<String> calls = [];

  /// Library item ids that are on a Subsonic source.
  Set<String> subsonicItems = {};

  @override
  Future<SubsonicServerInfo> probe(
    String address, {
    String? username,
    required String secret,
  }) async {
    calls.add('probe $address ${username ?? '-'}');
    final ok = username == null ? secret == 'key-1' : secret == 'secret';
    if (!ok) {
      throw const SourceException(SourceErrorCode.authFailed, 'wrong');
    }
    return const SubsonicServerInfo(
      apiVersion: '1.16.1',
      serverType: 'navidrome',
      serverVersion: '0.58.0',
      openSubsonic: true,
    );
  }

  @override
  Future<bool> scrobble(
    String itemId, {
    required bool submission,
    DateTime? at,
  }) async {
    calls.add('scrobble $itemId $submission');
    return subsonicItems.contains(itemId);
  }
}
