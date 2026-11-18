// SPDX-License-Identifier: MPL-2.0
//
// The stats contract for Dart engines (PLAY-010): the same case as
// `pa_playback::contract::stats` in the core, which the Rust mock, Media3
// (Kotlin) and the Apple engines (Swift) run. Every engine fills the one
// model.

import 'playback_stats.dart';
import 'player_engine.dart';

/// Checks a [PlayerEngine] that has a video file with sound open and
/// playing: with the overlay on it reports [PlaybackStats] with both
/// codecs, the model follows [PlaybackStats.check], survives the JSON of
/// the core unchanged, and its counters only go up. [letTimePass] waits
/// for the engine's next reading (a second of playback for libmpv).
/// Returns the broken rules; empty when the engine passes.
Future<List<String>> statsContractProblems(
  PlayerEngine engine,
  Future<void> Function() letTimePass,
) async {
  if (engine is! StatsEngine) {
    return ['the engine does not measure stats (StatsEngine)'];
  }
  final stats = engine as StatsEngine;
  final problems = <String>[];
  void model(PlaybackStats s) {
    final broken = s.check();
    if (broken != null) problems.add('$broken: $s');
    if (PlaybackStats.fromJson(s.toJson()) != s) {
      problems.add('the JSON of the core loses fields: $s');
    }
  }

  await stats.setStatsActive(true);
  try {
    await letTimePass();
    final first = engine.state.value.playbackStats;
    if (first == null) return ['no PlaybackStats while the overlay is on'];
    if (first.videoCodec == null) problems.add('no video codec: $first');
    if (first.audioCodec == null) problems.add('no audio codec: $first');
    model(first);
    await letTimePass();
    final later = engine.state.value.playbackStats;
    if (later == null) return [...problems, 'PlaybackStats went away'];
    model(later);
    final progress = first.checkProgress(later);
    if (progress != null) problems.add('$progress: $first then $later');
  } finally {
    await stats.setStatsActive(false);
  }
  return problems;
}
