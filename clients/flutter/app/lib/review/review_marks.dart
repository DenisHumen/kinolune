// SPDX-License-Identifier: MPL-2.0
//
// «?» on the tiles of items the review inbox has (META-034, shown with the
// watch marks of UI-037). One batch read of the inbox per change, shared by
// every tile: a tile only looks its id up in a set.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:material_ui/material_ui.dart';

import '../integration/library.dart' show ReviewInbox;
import '../services/app_services.dart';
import '../widgets/watched_art.dart' show ReviewMark;

/// Item ids in the review inbox, read once and again after each change.
class ReviewItemIds extends ValueNotifier<Set<String>> {
  ReviewItemIds._(this.inbox) : super(const {}) {
    _sub = inbox.changes.listen((_) => unawaited(_load()));
    unawaited(_load());
  }

  final ReviewInbox inbox;
  StreamSubscription<void>? _sub;

  /// Upper bound of one read: far more than an inbox ever holds.
  static const limit = 5000;

  static final _byInbox = Expando<ReviewItemIds>('review item ids');

  /// The shared ids of [inbox]; they live as long as the inbox.
  static ReviewItemIds of(ReviewInbox inbox) =>
      _byInbox[inbox] ??= ReviewItemIds._(inbox);

  /// The ids of the app's inbox, or null without one.
  static ReviewItemIds? maybeOf(BuildContext context) {
    final inbox = context
        .getInheritedWidgetOfExactType<Services>()
        ?.notifier
        ?.reviewInbox;
    return inbox == null ? null : of(inbox);
  }

  int _generation = 0;

  Future<void> _load() async {
    final g = ++_generation;
    try {
      final entries = await inbox.entries(limit: limit);
      if (g != _generation) return;
      value = {for (final e in entries) e.itemId};
    } catch (e) {
      debugPrint('Review inbox not read: $e');
    }
  }

  @override
  void dispose() {
    unawaited(_sub?.cancel());
    super.dispose();
  }
}

/// «?» at the top left of a poster in a [Stack], when the review inbox has
/// [itemId]; nothing otherwise (and without an inbox).
class ReviewTileMark extends StatelessWidget {
  const ReviewTileMark({super.key, required this.itemId});

  final String itemId;

  @override
  Widget build(BuildContext context) {
    final ids = ReviewItemIds.maybeOf(context);
    if (ids == null) return const SizedBox.shrink();
    return ValueListenableBuilder<Set<String>>(
      valueListenable: ids,
      builder: (context, ids, _) => ids.contains(itemId)
          ? Positioned(
              left: 6,
              top: 6,
              child: IgnorePointer(
                child: ReviewMark(key: ValueKey('badge-review-$itemId')),
              ),
            )
          : const SizedBox.shrink(),
    );
  }
}
