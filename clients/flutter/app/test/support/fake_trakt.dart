// SPDX-License-Identifier: MPL-2.0
//
// A Trakt link in memory (SYNC-011).

import 'package:flutter/foundation.dart';
import 'package:kinolune/trakt/trakt_link.dart';

class FakeTrakt implements TraktLink {
  FakeTrakt({String? account}) : _account = ValueNotifier(account);

  final ValueNotifier<String?> _account;
  final calls = <String>[];
  int syncs = 0;

  @override
  ValueListenable<String?> get account => _account;

  set signedIn(String? name) => _account.value = name;

  @override
  bool get configured => true;

  @override
  bool get ownClient => false;

  @override
  Future<void> setOwnClient(String? clientId, String? clientSecret) async {}

  @override
  Future<TraktDeviceLogin> startLogin() async => const TraktDeviceLogin(
    userCode: 'ABCD1234',
    verificationUrl: 'https://trakt.tv/activate',
    qrUrl: 'https://trakt.tv/activate/ABCD1234',
    expiresIn: Duration(minutes: 10),
    interval: Duration(seconds: 5),
  );

  /// Polls before the code is confirmed.
  int pollsUntilApproved = 0;

  @override
  Future<String?> pollLogin() async {
    if (pollsUntilApproved-- > 0) return null;
    _account.value = 'tester';
    return 'tester';
  }

  @override
  void cancelLogin() {}

  @override
  Future<void> signOut() async => _account.value = null;

  @override
  Future<bool> scrobble(String itemId, String action, double progress) async {
    calls.add('$action $itemId ${progress.toStringAsFixed(1)}');
    return true;
  }

  @override
  Future<TraktSyncResult> sync() async {
    syncs++;
    return const TraktSyncResult(watchedImported: 1);
  }

  @override
  Future<bool> setWatched(String itemId, bool watched) async => true;

  @override
  Future<bool> setRating(String itemId, int? stars) async => true;

  @override
  /// Library item ids on the watchlist.
  List<String> watchlist = [];

  @override
  Future<List<String>> watchlistItems({bool refresh = false}) async =>
      _account.value == null ? const [] : watchlist;

  @override
  Future<bool> inWatchlist(String itemId) async => false;

  @override
  Future<bool> setWatchlisted(String itemId, bool listed) async => true;
}
