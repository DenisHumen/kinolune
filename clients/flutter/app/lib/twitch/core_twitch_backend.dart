// SPDX-License-Identifier: MPL-2.0
//
// [TwitchBackend] on the Rust core: pa-twitch (Helix, Device Code Flow) and
// the Twitch model of the library through pa-frb (`CoreTwitch`).

import 'package:flutter/foundation.dart' show debugPrint;
import 'package:pa_core/pa_core.dart' as core;

import '../features/twitch/chat/chat_model.dart' show ChatFeed;
import '../features/twitch/chat/core_chat_feed.dart';

import '../integration/library.dart' show Progress;
import 'core_live_source.dart';
import 'live_notifications.dart';
import 'twitch_backend.dart';
import 'twitch_models.dart';

class CoreTwitchBackend implements TwitchBackend {
  CoreTwitchBackend(this._core);

  /// Opens the Twitch part of the library database at [dbPath]. Without
  /// [clientId] (no registered Twitch application yet) the section runs on
  /// local data only.
  static Future<CoreTwitchBackend> open({
    required String dbPath,
    required String dataDir,
    String? clientId,
  }) async => CoreTwitchBackend(
    await core.CoreTwitch.open(
      dbPath: dbPath,
      dataDir: dataDir,
      clientId: clientId,
    ),
  );

  final core.CoreTwitch _core;

  /// The signed-in chat of [login] (TWI-024): reads through EventSub and
  /// sends when the sign-in allows it. Null when not signed in or the
  /// channel cannot be resolved; the caller falls back to the anonymous
  /// chat.
  Future<ChatFeed?> signedInChat(
    String login,
    core.CoreChatOptions options,
  ) async {
    if (_core.account() == null) return null;
    try {
      final users = await _core.users(ids: const [], logins: [login]);
      if (users.isEmpty) return null;
      return CoreChatFeed.signedIn(
        _core,
        channel: login,
        broadcasterId: users.first.userId,
        options: options,
      );
    } on core.TwitchApiError catch (e) {
      debugPrint('Twitch chat: signed-in chat of $login: ${e.message}');
      return null;
    } on core.CoreChatError catch (e) {
      debugPrint('Twitch chat: signed-in chat of $login: ${e.message}');
      return null;
    }
  }

  Future<T> _call<T>(Future<T> Function() f) async {
    try {
      return await f();
    } on core.TwitchApiError catch (e) {
      throw TwitchUnavailable(switch (e.kind) {
        core.TwitchErrorKind.notConfigured =>
          TwitchUnavailableReason.notConfigured,
        core.TwitchErrorKind.signedOut => TwitchUnavailableReason.signedOut,
        core.TwitchErrorKind.reauthRequired ||
        core.TwitchErrorKind.loginDenied ||
        core.TwitchErrorKind.loginExpired => TwitchUnavailableReason.signIn,
        core.TwitchErrorKind.network ||
        core.TwitchErrorKind.rateLimited => TwitchUnavailableReason.network,
        _ => TwitchUnavailableReason.other,
      }, e.message);
    }
  }

  @override
  bool get canSignIn => _core.canSignIn();

  @override
  TwitchAccount? get account {
    final a = _core.account();
    return a == null ? null : TwitchAccount(userId: a.userId, login: a.login);
  }

  @override
  Future<TwitchDeviceLogin> startLogin({bool chat = false}) =>
      _call(() async {
    final d = chat ? await _core.startChatLogin() : await _core.startLogin();
    return TwitchDeviceLogin(
      userCode: d.userCode,
      verificationUri: d.verificationUri,
      expiresIn: Duration(seconds: d.expiresInSecs),
      interval: Duration(seconds: d.intervalSecs),
    );
  });

  @override
  Future<TwitchAccount?> pollLogin() => _call(() async {
    final a = await _core.pollLogin();
    return a == null ? null : TwitchAccount(userId: a.userId, login: a.login);
  });

  @override
  Future<void> signOut() => _call(_core.signOut);

  @override
  Future<List<TwitchChannel>> users({
    List<String> ids = const [],
    List<String> logins = const [],
  }) => _call(() async {
    final users = await _core.users(ids: ids, logins: logins);
    return users.map(_channel).toList();
  });

  @override
  Future<List<TwitchStream>> liveStreams(List<String> userIds) =>
      _call(() async {
        final streams = await _core.liveStreams(userIds: userIds);
        return [
          for (final s in streams)
            TwitchStream(
              userId: s.userId,
              login: s.userLogin,
              displayName: s.userName,
              title: s.title,
              viewers: s.viewerCount,
              gameId: s.gameId,
              gameName: s.gameName,
              startedAt: _time(s.startedAtMs),
              thumbnailUrl: s.thumbnailUrl,
            ),
        ];
      });

  @override
  Future<List<TwitchChannel>> followed() =>
      _call(() async => (await _core.followed()).map(_channel).toList());

  @override
  Future<List<TwitchVideo>> videos(
    String userId, {
    TwitchVideoType? type,
    int limit = 20,
  }) => _call(() async {
    final videos = await _core.channelVideos(
      userId: userId,
      videoType: type?.name ?? '',
      limit: limit,
    );
    return videos.map(_video).toList();
  });

  @override
  Future<List<TwitchClip>> clips(String userId, {int limit = 20}) =>
      _call(() async {
        final clips = await _core.clips(userId: userId, limit: limit);
        return [
          for (final c in clips)
            TwitchClip(
              slug: c.slug,
              title: c.title,
              viewCount: c.viewCount,
              duration: Duration(milliseconds: (c.durationSecs * 1000).round()),
              createdAt: _time(c.createdAtMs),
              thumbnailUrl: c.thumbnailUrl,
              creatorName: c.creatorName,
            ),
        ];
      });

  @override
  Future<List<TwitchScheduleSegment>> schedule(String userId) =>
      _call(() async {
        final segments = await _core.schedule(userId: userId);
        return [
          for (final s in segments)
            TwitchScheduleSegment(
              id: s.id,
              title: s.title,
              start: DateTime.fromMillisecondsSinceEpoch(s.startMs),
              end: _time(s.endMs),
              categoryName: s.categoryName,
              canceled: s.canceled,
            ),
        ];
      });

  @override
  Future<List<TwitchChannel>> searchChannels(String query) => _call(
    () async => (await _core.searchChannels(
      query: query,
      limit: 20,
    )).map(_channel).toList(),
  );

  @override
  Future<List<TwitchCategory>> searchCategories(String query) =>
      _call(() async {
        final cats = await _core.searchCategories(query: query, limit: 20);
        return [
          for (final c in cats)
            TwitchCategory(id: c.gameId, name: c.name, boxArtUrl: c.boxArtUrl),
        ];
      });

  @override
  Future<List<TwitchChannel>> searchCached(String query) => _call(() async {
    final hits = await _core.searchCached(query: query, limit: 20);
    final out = <TwitchChannel>[];
    for (final h in hits) {
      if (h.kind != 'streamer') continue;
      final u = _core.cachedUser(userId: h.twitchId);
      if (u != null) out.add(_channel(u));
    }
    return out;
  });

  @override
  Future<List<TwitchVideo>> cachedVideos(String userId) => _call(
    () async => (await _core.cachedVideos(
      userId: userId,
      limit: 50,
    )).map(_video).toList(),
  );

  @override
  Future<List<TwitchChapter>> chapters(String videoId) => _call(() async {
    final chapters = await _core.chapters(videoId: videoId);
    return [
      for (final c in chapters)
        TwitchChapter(
          gameId: c.gameId,
          gameName: c.gameName,
          boxArtUrl: c.boxArtUrl,
          start: Duration(milliseconds: c.startMs),
          end: c.endMs == null ? null : Duration(milliseconds: c.endMs!),
          approximate: c.approximate,
        ),
    ];
  });

  @override
  Future<List<TwitchVideo>> categoryVideos(String gameId) => _call(
    () async => (await _core.categoryVideos(
      gameId: gameId,
      limit: 50,
    )).map(_video).toList(),
  );

  @override
  Future<List<TwitchProgressEntry>> continueVods() => _call(() async {
    final rows = await _core.continueVods(limit: 20);
    return [
      for (final r in rows)
        TwitchProgressEntry(
          videoId: r.videoId,
          progress: Progress(
            itemId: 'twitch:vod:${r.videoId}', // l10n-ignore
            positionMs: r.positionMs,
            durationMs: r.durationMs,
            updatedAt: DateTime.fromMillisecondsSinceEpoch(r.updatedAtMs),
          ),
          video: r.video == null ? null : _video(r.video!),
          channel: r.user == null ? null : _channel(r.user!),
        ),
    ];
  });

  @override
  Future<void> syncFavorite(TwitchChannel channel, bool favorite) async {
    final id = channel.userId;
    if (id == null) return;
    await _call(
      () => _core.setFavorite(
        userId: id,
        favorite: favorite,
        user: core.CoreTwitchUser(
          userId: id,
          login: channel.login,
          displayName: channel.displayName,
          broadcasterType: channel.type.name,
          description: channel.description,
          avatarUrl: channel.avatarUrl,
          offlineBannerUrl: channel.bannerUrl,
          lastLiveAtMs: channel.lastLiveAt?.millisecondsSinceEpoch,
        ),
      ),
    );
  }

  @override
  Future<void> syncNotify(TwitchChannel channel, bool notify) async {
    final id = channel.userId;
    if (id == null) return;
    await _call(() => _core.setFavoriteNotify(userId: id, notify: notify));
  }

  /// The "X is live" watcher of the core (TWI-009); null when signed out.
  CoreLiveSource? liveSource() {
    if (account == null) return null;
    try {
      // The core re-reads the active profile: none for kid profiles.
      if (!_core.liveNotificationsAllowed()) return null;
      return CoreLiveSource(_core.liveMonitor());
    } on core.TwitchApiError {
      return null;
    }
  }

  /// One background check (Android WorkManager): notifications to show.
  Future<List<LiveAlertInfo>> checkLiveOnce(
    List<LiveWatch> channels, {
    required Duration maxAge,
  }) async {
    final alerts = await _call(
      () => _core.checkLiveOnce(
        channels: [for (final c in channels) coreLiveChannel(c)],
        maxAgeSecs: maxAge.inSeconds,
      ),
    );
    return [for (final a in alerts) liveAlertOf(a)];
  }

  static DateTime? _time(int? ms) =>
      ms == null ? null : DateTime.fromMillisecondsSinceEpoch(ms);

  static TwitchChannel _channel(core.CoreTwitchUser u) => TwitchChannel(
    login: u.login,
    displayName: u.displayName,
    userId: u.userId,
    avatarUrl: u.avatarUrl,
    description: u.description,
    bannerUrl: u.offlineBannerUrl,
    type: parseBroadcasterType(u.broadcasterType),
    lastLiveAt: _time(u.lastLiveAtMs),
  );

  static TwitchVideo _video(core.CoreTwitchVideo v) => TwitchVideo(
    id: v.videoId,
    userId: v.userId,
    login: v.userLogin,
    displayName: v.userName,
    title: v.title,
    publishedAt: DateTime.fromMillisecondsSinceEpoch(v.publishedAtMs),
    duration: Duration(milliseconds: v.durationMs),
    type: parseVideoType(v.videoType),
    thumbnailUrl: v.thumbnailUrl,
    viewCount: v.viewCount,
    subOnly: v.isSubOnly ?? false,
    expiresAt: _time(v.expiresAtMs),
    savedFiles: v.savedFiles,
    progress: v.positionMs == null
        ? null
        : Progress(
            itemId: 'twitch:vod:${v.videoId}', // l10n-ignore
            positionMs: v.positionMs!,
            durationMs: v.durationMs,
            updatedAt: DateTime.fromMillisecondsSinceEpoch(
              v.progressUpdatedAtMs ?? 0,
            ),
            watched: v.watched,
          ),
  );
}
