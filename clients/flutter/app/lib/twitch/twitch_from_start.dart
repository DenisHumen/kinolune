// SPDX-License-Identifier: MPL-2.0
//
// «Смотреть с начала эфира» (TWI-025, full builds, native player): the
// stream's own VOD (Twitch archives a stream while it is on air) played from
// 0 through the core's loopback proxy; when the playhead catches up with the
// live DVR window the player switches to live at the same moment, so no
// minute is skipped.
//
// - [currentStreamVod]: the archive of the stream on air now (Helix: the
//   stream's `started_at` and the channel's newest archives).
// - [fromStartStep]: stay on the VOD, switch into the live DVR window, or
//   straight to live.

import 'twitch_backend.dart';
import 'twitch_models.dart';

/// How far an archive's creation may be from the stream start and still be
/// its VOD (Twitch creates it within seconds of the start).
const kStreamVodSlack = Duration(minutes: 10);

/// The archive of a stream that started at [startedAt]: the one created
/// closest to the start, within [kStreamVodSlack]; null when the streamer
/// does not keep broadcasts (or it is not listed yet).
TwitchVideo? pickStreamVod(DateTime? startedAt, List<TwitchVideo> videos) {
  if (startedAt == null) return null;
  TwitchVideo? best;
  Duration? bestGap;
  for (final v in videos) {
    if (v.type != TwitchVideoType.archive) continue;
    final gap = v.publishedAt.difference(startedAt).abs();
    if (gap > kStreamVodSlack) continue;
    if (bestGap == null || gap < bestGap) {
      best = v;
      bestGap = gap;
    }
  }
  return best;
}

/// The VOD of [channel]'s stream on air now, and when the stream started;
/// null when it is offline or keeps no archive. Helix needs a signed-in
/// account ([TwitchUnavailable] otherwise).
Future<(TwitchVideo, DateTime)?> currentStreamVod(
  TwitchBackend backend,
  TwitchChannel channel,
) async {
  var userId = channel.userId;
  if (userId == null || userId.isEmpty) {
    final users = await backend.users(logins: [channel.login]);
    userId = users.firstOrNull?.userId;
  }
  if (userId == null || userId.isEmpty) return null;
  final stream = (await backend.liveStreams([userId])).firstOrNull;
  final started = stream?.startedAt;
  if (started == null) return null;
  final archives = await backend.videos(
    userId,
    type: TwitchVideoType.archive,
    limit: 5,
  );
  final vod = pickStreamVod(started, archives);
  return vod == null ? null : (vod, started);
}

/// What "watch from the start" does next.
enum FromStartStep {
  /// Keep playing the VOD.
  stay,

  /// The VOD reached the live DVR window: continue live at the same moment.
  toWindow,

  /// The VOD is (almost) at the live edge: go live.
  toLive,
}

/// The step for a VOD playhead at program date-time [vodPlayhead], given
/// the live DVR window from [windowStart] to [edge] and the live latency
/// [target]. [margin] keeps the switch a little inside the window, which
/// slides while the stream reloads.
FromStartStep fromStartStep({
  required DateTime? vodPlayhead,
  required DateTime? windowStart,
  required DateTime? edge,
  Duration target = const Duration(seconds: 3),
  Duration margin = const Duration(seconds: 10),
}) {
  if (vodPlayhead == null || windowStart == null || edge == null) {
    return FromStartStep.stay;
  }
  if (!vodPlayhead.isBefore(
    edge.subtract(target + const Duration(seconds: 2)),
  )) {
    return FromStartStep.toLive;
  }
  if (!vodPlayhead.isBefore(windowStart.add(margin))) {
    return FromStartStep.toWindow;
  }
  return FromStartStep.stay;
}
