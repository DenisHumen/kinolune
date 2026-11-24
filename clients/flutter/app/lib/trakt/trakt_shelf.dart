// SPDX-License-Identifier: MPL-2.0
//
// The Home shelf «Trakt: хочу посмотреть» (SYNC-011): titles of the user's
// Trakt watchlist that are in the library. Empty (and so hidden) when
// signed out or when none of them is here.

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../integration/library.dart';
import 'trakt_link.dart';

/// Library items of the Trakt watchlist: now, after a sign-in or sign-out
/// and every [every] (the core fetches the list at most every ten minutes).
Stream<List<LibraryItem>> traktWatchlistShelf(
  TraktLink trakt,
  Library library, {
  Duration every = const Duration(minutes: 15),
}) {
  late StreamController<List<LibraryItem>> controller;
  Timer? timer;
  var busy = false;

  Future<void> load() async {
    if (busy || controller.isClosed) return;
    busy = true;
    try {
      final ids = trakt.account.value == null
          ? const <String>[]
          : await trakt.watchlistItems();
      final items = <LibraryItem>[];
      for (final id in ids) {
        final item = await library.item(id);
        if (item != null) items.add(item);
      }
      if (!controller.isClosed) controller.add(items);
    } catch (e) {
      debugPrint('Trakt watchlist not loaded: $e');
      if (!controller.isClosed) controller.add(const []);
    } finally {
      busy = false;
    }
  }

  void onAccount() => unawaited(load());

  controller = StreamController<List<LibraryItem>>(
    onListen: () {
      trakt.account.addListener(onAccount);
      timer = Timer.periodic(every, (_) => unawaited(load()));
      unawaited(load());
    },
    onCancel: () {
      trakt.account.removeListener(onAccount);
      timer?.cancel();
    },
  );
  return controller.stream;
}
