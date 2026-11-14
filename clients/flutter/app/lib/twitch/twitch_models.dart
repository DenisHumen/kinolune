// SPDX-License-Identifier: MPL-2.0
//
// Data of the "Streams" section (Twitch, TWI-013): channels, live streams,
// VODs with chapters, clips, schedules and categories, as the screens use
// them. Filled by a [TwitchBackend] (lib/twitch/twitch_backend.dart): the
// Rust core (pa-frb) or the fake Helix of the demo and the tests.

import '../integration/library.dart' show Progress;

enum TwitchBroadcasterType { partner, affiliate, normal }

TwitchBroadcasterType parseBroadcasterType(String s) => switch (s) {
  'partner' => TwitchBroadcasterType.partner,
  'affiliate' => TwitchBroadcasterType.affiliate,
  _ => TwitchBroadcasterType.normal,
};

enum TwitchVideoType { archive, highlight, upload }

TwitchVideoType parseVideoType(String s) => switch (s) {
  'highlight' => TwitchVideoType.highlight,
  'upload' => TwitchVideoType.upload,
  _ => TwitchVideoType.archive,
};

/// Fills a Twitch image template (`{width}x{height}` or
/// `%{width}x%{height}`) with a size; null for an empty template.
String? twitchImage(String? template, int width, int height) {
  if (template == null || template.isEmpty) return null;
  return template
      .replaceAll('%{width}', '$width')
      .replaceAll('%{height}', '$height')
      .replaceAll('{width}', '$width')
      .replaceAll('{height}', '$height');
}

/// A Twitch login: 3–25 letters, digits or `_`. Lower-cased.
String? normalizeLogin(String input) {
  var s = input.trim();
  // A pasted channel link: twitch.tv/<login>.
  final link = RegExp(r'twitch\.tv/([A-Za-z0-9_]+)').firstMatch(s);
  if (link != null) s = link.group(1)!;
  if (s.startsWith('@')) s = s.substring(1);
  if (!RegExp(r'^[A-Za-z0-9_]{3,25}$').hasMatch(s)) return null;
  return s.toLowerCase();
}

/// A channel (profile).
class TwitchChannel {
  const TwitchChannel({
    required this.login,
    required this.displayName,
    this.userId,
    this.avatarUrl,
    this.description,
    this.bannerUrl,
    this.type = TwitchBroadcasterType.normal,
    this.lastLiveAt,
  });

  /// Lower-case login: `twitch.tv/<login>`.
  final String login;
  final String displayName;

  /// Null for a favourite added by name that was not resolved yet.
  final String? userId;
  final String? avatarUrl;
  final String? description;
  final String? bannerUrl;
  final TwitchBroadcasterType type;
  final DateTime? lastLiveAt;

  String get url => 'https://www.twitch.tv/$login'; // l10n-ignore

  TwitchChannel copyWith({DateTime? lastLiveAt}) => TwitchChannel(
    login: login,
    displayName: displayName,
    userId: userId,
    avatarUrl: avatarUrl,
    description: description,
    bannerUrl: bannerUrl,
    type: type,
    lastLiveAt: lastLiveAt ?? this.lastLiveAt,
  );
}

/// A live stream.
class TwitchStream {
  const TwitchStream({
    required this.userId,
    required this.login,
    required this.displayName,
    required this.title,
    required this.viewers,
    this.gameId,
    this.gameName,
    this.startedAt,
    this.thumbnailUrl,
  });

  final String userId;
  final String login;
  final String displayName;
  final String title;
  final int viewers;
  final String? gameId;
  final String? gameName;
  final DateTime? startedAt;

  /// Template with `{width}`/`{height}`.
  final String? thumbnailUrl;
}

/// A chapter of a VOD: the category on air from [start] to [end].
class TwitchChapter {
  const TwitchChapter({
    required this.gameId,
    required this.start,
    this.end,
    this.gameName,
    this.boxArtUrl,
    this.approximate = false,
  });

  final String gameId;
  final String? gameName;
  final String? boxArtUrl;
  final Duration start;

  /// Null: until the end of the VOD.
  final Duration? end;

  /// The channel's current category, not a timeline: shown with "≈".
  final bool approximate;
}

/// A VOD: past broadcast (archive), highlight or upload.
class TwitchVideo {
  const TwitchVideo({
    required this.id,
    required this.userId,
    required this.login,
    required this.displayName,
    required this.title,
    required this.publishedAt,
    required this.duration,
    this.type = TwitchVideoType.archive,
    this.thumbnailUrl,
    this.viewCount = 0,
    this.subOnly = false,
    this.expiresAt,
    this.progress,
    this.chapters = const [],
    this.savedFiles = 0,
  });

  final String id;
  final String userId;
  final String login;
  final String displayName;
  final String title;
  final DateTime publishedAt;
  final Duration duration;
  final TwitchVideoType type;

  /// Template with `%{width}`/`%{height}`.
  final String? thumbnailUrl;
  final int viewCount;

  /// Subscribers only: shown with a lock and "Watch on Twitch".
  final bool subOnly;

  /// When Twitch deletes it (archives: 7/14/60 days).
  final DateTime? expiresAt;
  final Progress? progress;
  final List<TwitchChapter> chapters;

  /// Downloads and recordings (TWI-017+).
  final int savedFiles;

  /// `twitch:vod:<id>`: the progress key, the same online and downloaded.
  String get key => 'twitch:vod:$id'; // l10n-ignore
  String get url => 'https://www.twitch.tv/videos/$id'; // l10n-ignore

  /// Whole days until Twitch deletes it, at least 0; null if it stays.
  int? daysLeft(DateTime now) {
    final e = expiresAt;
    if (e == null) return null;
    final left = e.difference(now);
    if (left.isNegative) return 0;
    return (left.inHours / 24).ceil();
  }

  TwitchVideo copyWith({
    List<TwitchChapter>? chapters,
    Progress? progress,
    bool? subOnly,
  }) => TwitchVideo(
    id: id,
    userId: userId,
    login: login,
    displayName: displayName,
    title: title,
    publishedAt: publishedAt,
    duration: duration,
    type: type,
    thumbnailUrl: thumbnailUrl,
    viewCount: viewCount,
    subOnly: subOnly ?? this.subOnly,
    expiresAt: expiresAt,
    progress: progress ?? this.progress,
    chapters: chapters ?? this.chapters,
    savedFiles: savedFiles,
  );
}

/// When Twitch deletes an archive: 7 days for regular channels, 14 for
/// affiliates, 60 for partners (research 06 §1.5); highlights and uploads
/// stay. The same rule as `pa_library::twitch::vod_expires_at_ms`.
DateTime? vodExpiresAt(
  TwitchVideoType type,
  TwitchBroadcasterType channel,
  DateTime publishedAt,
) {
  if (type != TwitchVideoType.archive) return null;
  final days = switch (channel) {
    TwitchBroadcasterType.partner => 60,
    TwitchBroadcasterType.affiliate => 14,
    TwitchBroadcasterType.normal => 7,
  };
  return publishedAt.add(Duration(days: days));
}

class TwitchClip {
  const TwitchClip({
    required this.slug,
    required this.title,
    required this.viewCount,
    required this.duration,
    this.createdAt,
    this.thumbnailUrl,
    this.creatorName,
  });

  final String slug;
  final String title;
  final int viewCount;
  final Duration duration;
  final DateTime? createdAt;
  final String? thumbnailUrl;
  final String? creatorName;

  String get url => 'https://clips.twitch.tv/$slug'; // l10n-ignore
}

class TwitchScheduleSegment {
  const TwitchScheduleSegment({
    required this.id,
    required this.title,
    required this.start,
    this.end,
    this.categoryName,
    this.canceled = false,
  });

  final String id;
  final String title;
  final DateTime start;
  final DateTime? end;
  final String? categoryName;
  final bool canceled;
}

class TwitchCategory {
  const TwitchCategory({required this.id, required this.name, this.boxArtUrl});

  final String id;
  final String name;

  /// Template with `{width}`/`{height}`.
  final String? boxArtUrl;

  String get url =>
      'https://www.twitch.tv/directory/category/$id'; // l10n-ignore
}

/// The signed-in Twitch account (Device Code Flow).
class TwitchAccount {
  const TwitchAccount({required this.userId, required this.login});

  final String userId;
  final String login;
}

/// A pending Device Code Flow sign-in.
class TwitchDeviceLogin {
  const TwitchDeviceLogin({
    required this.userCode,
    required this.verificationUri,
    required this.expiresIn,
    required this.interval,
  });

  final String userCode;
  final String verificationUri;
  final Duration expiresIn;
  final Duration interval;
}

/// A favourite channel, kept on this device without a Twitch login
/// (DEC-006). Added by name it has no [userId] until Helix resolves it.
class TwitchFavorite {
  const TwitchFavorite({
    required this.login,
    required this.addedAt,
    this.userId,
    this.displayName,
    this.avatarUrl,
    this.pinned = false,
    this.notify = true,
  });

  final String login;
  final String? userId;
  final String? displayName;
  final String? avatarUrl;
  final DateTime addedAt;

  /// Pinned favourites come first in every order.
  final bool pinned;

  /// "X is live" notifications for this channel (TWI-009); on by default.
  final bool notify;

  TwitchFavorite copyWith({
    String? userId,
    String? displayName,
    String? avatarUrl,
    bool? pinned,
    bool? notify,
  }) => TwitchFavorite(
    login: login,
    addedAt: addedAt,
    userId: userId ?? this.userId,
    displayName: displayName ?? this.displayName,
    avatarUrl: avatarUrl ?? this.avatarUrl,
    pinned: pinned ?? this.pinned,
    notify: notify ?? this.notify,
  );

  Map<String, Object?> toJson() => {
    'login': login,
    if (userId != null) 'userId': userId,
    if (displayName != null) 'displayName': displayName,
    if (avatarUrl != null) 'avatarUrl': avatarUrl,
    'addedAt': addedAt.millisecondsSinceEpoch,
    if (pinned) 'pinned': true,
    if (!notify) 'notify': false,
  };

  static TwitchFavorite? fromJson(Object? json) {
    if (json is! Map) return null;
    final login = json['login'];
    final added = json['addedAt'];
    if (login is! String || normalizeLogin(login) != login) return null;
    return TwitchFavorite(
      login: login,
      userId: json['userId'] as String?,
      displayName: json['displayName'] as String?,
      avatarUrl: json['avatarUrl'] as String?,
      addedAt: DateTime.fromMillisecondsSinceEpoch(added is int ? added : 0),
      pinned: json['pinned'] == true,
      notify: json['notify'] != false,
    );
  }
}
