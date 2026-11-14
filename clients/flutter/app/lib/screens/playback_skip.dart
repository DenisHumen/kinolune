// SPDX-License-Identifier: MPL-2.0

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../integration/player.dart';

/// Intro, recap and credits segments of [item] for the player (PLAY-011),
/// from the library. For an episode with [analyze], its season is queued
/// for analysis in the background, so the next episodes (and this one
/// next time) have them. Previews of the next episode have no player
/// counterpart and are left out.
Future<List<SkipSegment>> skipSegmentsOf(
  Library library,
  LibraryItem item, {
  bool analyze = true,
}) async {
  if (library is! LibrarySkipSegments) return const [];
  final skip = library as LibrarySkipSegments;
  if (analyze && item.kind == ItemKind.episode) {
    unawaited(
      skip.analyzeSkipSegments(item).catchError((Object e) {
        debugPrint('Skip analysis of ${item.id} not queued: $e');
      }),
    );
  }
  try {
    return playerSegments(await skip.skipSegments(item));
  } catch (e) {
    debugPrint('Skip segments of ${item.id} not read: $e');
    return const [];
  }
}

/// The library's segments as the player's.
List<SkipSegment> playerSegments(List<MediaSkipSegment> segments) => [
  for (final s in segments)
    if (s.endMs > s.startMs)
      if (_kind(s.kind) case final kind?)
        SkipSegment(
          kind: kind,
          start: Duration(milliseconds: s.startMs),
          end: Duration(milliseconds: s.endMs),
        ),
];

SkipKind? _kind(MediaSkipKind kind) => switch (kind) {
  MediaSkipKind.intro => SkipKind.intro,
  MediaSkipKind.recap => SkipKind.recap,
  MediaSkipKind.credits => SkipKind.credits,
  MediaSkipKind.preview => null,
};
