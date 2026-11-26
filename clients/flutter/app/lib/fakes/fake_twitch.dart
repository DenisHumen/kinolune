// SPDX-License-Identifier: MPL-2.0
//
// A fake Helix for the demo mode and the tests of the "Streams" section
// (TWI-013). Every channel, game and title here is made up: the project
// has no Twitch client id, and tests never touch the network. Like Helix it
// answers only when "signed in"; signed out it keeps a local cache of the
// channels it has served, as the library does.

import '../integration/library.dart' show Progress;
import '../twitch/twitch_backend.dart';
import '../twitch/twitch_models.dart';
import '../twitch/twitch_service.dart';
import '../widgets/artwork.dart' show kDemoArtScheme;

class FakeTwitchBackend implements TwitchBackend {
  FakeTwitchBackend({
    required this.now,
    this.signedIn = true,
    this.canSignIn = true,
    List<TwitchChannel>? channels,
    List<TwitchStream>? live,
    List<TwitchVideo>? videos,
    List<TwitchClip>? clips,
    Map<String, List<TwitchScheduleSegment>>? schedules,
    List<TwitchCategory>? categories,
    List<String>? follows,
    List<TwitchProgressEntry>? progress,
    List<TwitchCategory>? top,
    Map<String, List<TwitchStream>>? categoryLive,
  }) : top = top ?? categories ?? const [],
       categoryLive = categoryLive ?? const {},
       channels = channels ?? const [],
       live = live ?? const [],
       allVideos = videos ?? const [],
       allClips = clips ?? const [],
       schedules = schedules ?? const {},
       categories = categories ?? const [],
       follows = follows ?? const [],
       progress = progress ?? const [];

  /// Six made-up channels: four live, VODs with chapters (one sub-only,
  /// one about to expire), clips, schedules and two VODs in progress.
  factory FakeTwitchBackend.demo({
    required DateTime now,
    bool signedIn = true,
    bool canSignIn = true,
  }) {
    DateTime ago(int hours) => now.subtract(Duration(hours: hours));
    TwitchChannel ch(
      String id,
      String login,
      String name,
      TwitchBroadcasterType type,
      String about,
    ) => TwitchChannel(
      userId: id,
      login: login,
      displayName: name,
      type: type,
      description: about,
      lastLiveAt: ago(30),
      avatarUrl: kDemoArtScheme,
      bannerUrl: kDemoArtScheme,
    );
    final channels = [
      ch(
        '7001',
        'pixelheron',
        'PixelHeron',
        TwitchBroadcasterType.partner,
        'Cozy strategy nights and long talks.',
      ),
      ch(
        '7002',
        'lunaforge',
        'LunaForge',
        TwitchBroadcasterType.affiliate,
        'Speedruns of games that do not exist.',
      ),
      ch(
        '7003',
        'quietkettle',
        'QuietKettle',
        TwitchBroadcasterType.normal,
        'Tea, pixel art and lo-fi.',
      ),
      ch(
        '7004',
        'novamarsh',
        'NovaMarsh',
        TwitchBroadcasterType.partner,
        'Racing sims every weekend.',
      ),
      ch(
        '7005',
        'bytelantern',
        'ByteLantern',
        TwitchBroadcasterType.affiliate,
        'Building tiny gardens.',
      ),
      ch(
        '7006',
        'embertide',
        'EmberTide',
        TwitchBroadcasterType.normal,
        'Late-night co-op.',
      ),
    ];
    const games = [
      TwitchCategory(id: '501', name: 'Starfall Tactics'),
      TwitchCategory(id: '502', name: 'Pocket Garden'),
      TwitchCategory(id: '503', name: 'Talk & Tea'),
      TwitchCategory(id: '504', name: 'Neon Drift'),
      TwitchCategory(id: '505', name: 'Hollow Lanterns'),
    ];
    TwitchStream stream(
      TwitchChannel c,
      String title,
      TwitchCategory g,
      int viewers,
      int hours, {
      String language = 'en',
    }) => TwitchStream(
      userId: c.userId!,
      login: c.login,
      displayName: c.displayName,
      title: title,
      viewers: viewers,
      gameId: g.id,
      gameName: g.name,
      startedAt: ago(hours),
      thumbnailUrl: kDemoArtScheme,
      language: language,
    );
    final live = [
      stream(channels[0], 'Grand campaign, day 12', games[0], 18400, 3),
      stream(
        channels[1],
        'Any% attempts until it works',
        games[3],
        5200,
        1,
        language: 'uk',
      ),
      stream(channels[3], 'Endurance race, 6 hours', games[3], 9100, 2),
      stream(
        channels[4],
        'Spring garden build',
        games[1],
        870,
        4,
        language: 'uk',
      ),
    ];
    // TWI-034: the browse grid. The five demo games first, then made-up
    // categories enough for more than one page.
    final top = [
      for (final g in games)
        TwitchCategory(id: g.id, name: g.name, boxArtUrl: kDemoArtScheme),
      for (var i = 6; i <= 52; i++)
        TwitchCategory(
          id: '${500 + i}',
          name: 'Made-up category $i',
          boxArtUrl: kDemoArtScheme,
        ),
    ];
    const languages = ['uk', 'en', 'ru', 'de', 'uk', 'en', 'es', 'pl'];
    final byCategory = <String, List<TwitchStream>>{};
    for (final (i, g) in top.indexed) {
      final extra = i < 5 ? 14 : 3;
      byCategory[g.id] = [
        for (final s in live)
          if (s.gameId == g.id) s,
        for (var k = 0; k < extra; k++)
          TwitchStream(
            userId: '8${i.toString().padLeft(2, '0')}$k',
            login: 'madeup_${i}_$k',
            displayName: 'MadeUp_${i}_$k',
            title: 'Made-up stream $k of ${g.name}',
            viewers: 4000 ~/ (k + 1) + 7 * (extra - k),
            gameId: g.id,
            gameName: g.name,
            startedAt: ago(1 + k % 5),
            thumbnailUrl: kDemoArtScheme,
            language: languages[(i + k) % languages.length],
          ),
      ];
    }
    TwitchChapter chap(
      TwitchCategory g,
      int startMin,
      int? endMin, {
      bool approx = false,
    }) => TwitchChapter(
      gameId: g.id,
      gameName: g.name,
      start: Duration(minutes: startMin),
      end: endMin == null ? null : Duration(minutes: endMin),
      approximate: approx,
    );
    TwitchVideo vod(
      String id,
      TwitchChannel c,
      String title,
      int hoursAgo,
      int minutes, {
      List<TwitchChapter> chapters = const [],
      TwitchVideoType type = TwitchVideoType.archive,
      bool subOnly = false,
    }) {
      final published = ago(hoursAgo);
      return TwitchVideo(
        id: id,
        userId: c.userId!,
        login: c.login,
        displayName: c.displayName,
        title: title,
        publishedAt: published,
        duration: Duration(minutes: minutes),
        type: type,
        viewCount: 1000 + int.parse(id) % 977,
        thumbnailUrl: kDemoArtScheme,
        subOnly: subOnly,
        expiresAt: vodExpiresAt(type, c.type, published),
        chapters: chapters,
      );
    }

    final videos = [
      vod(
        '9101',
        channels[0],
        'Campaign day 11: the long siege',
        26,
        245,
        chapters: [
          chap(games[2], 0, 40),
          chap(games[0], 40, 200),
          chap(games[2], 200, null),
        ],
      ),
      vod(
        '9102',
        channels[2],
        'Pixel art and tea, episode 40',
        150,
        130,
        chapters: [chap(games[2], 0, null, approx: true)],
      ),
      vod(
        '9103',
        channels[1],
        'New personal best?',
        30,
        95,
        chapters: [chap(games[3], 0, 70), chap(games[4], 70, null)],
      ),
      vod(
        '9104',
        channels[3],
        'Night race at the harbour',
        50,
        180,
        chapters: [chap(games[3], 0, null)],
        subOnly: true,
      ),
      vod(
        '9105',
        channels[5],
        'Co-op until sunrise',
        70,
        210,
        chapters: [chap(games[4], 0, 120), chap(games[1], 120, null)],
      ),
      vod(
        '9106',
        channels[0],
        'Best of the siege',
        20,
        12,
        type: TwitchVideoType.highlight,
      ),
      vod(
        '9107',
        channels[4],
        'Garden tour',
        100,
        75,
        chapters: [chap(games[1], 0, null)],
      ),
    ];
    final clips = [
      TwitchClip(
        slug: 'calmheronsiegewin',
        title: 'The wall finally falls',
        viewCount: 48210,
        duration: const Duration(seconds: 28),
        createdAt: ago(22),
        thumbnailUrl: kDemoArtScheme,
        creatorName: 'teacup_fan',
      ),
      TwitchClip(
        slug: 'heronlaughsforever',
        title: 'Laughing for a full minute',
        viewCount: 12044,
        duration: const Duration(seconds: 60),
        createdAt: ago(80),
        thumbnailUrl: kDemoArtScheme,
        creatorName: 'moss_bit',
      ),
    ];
    TwitchScheduleSegment seg(String id, String title, int inHours, String g) =>
        TwitchScheduleSegment(
          id: id,
          title: title,
          start: now.add(Duration(hours: inHours)),
          end: now.add(Duration(hours: inHours + 3)),
          categoryName: g,
        );
    final schedules = {
      '7002': [seg('s1', 'Practice session', 20, 'Neon Drift')],
      '7003': [seg('s2', 'Sunday tea stream', 30, 'Talk & Tea')],
      '7006': [seg('s3', 'Co-op night', 52, 'Hollow Lanterns')],
      '7001': [seg('s4', 'Campaign day 13', 26, 'Starfall Tactics')],
    };
    final progress = [
      TwitchProgressEntry(
        videoId: '9101',
        progress: Progress(
          itemId: 'twitch:vod:9101',
          positionMs: const Duration(minutes: 95).inMilliseconds,
          durationMs: const Duration(minutes: 245).inMilliseconds,
          updatedAt: ago(5),
        ),
      ),
      TwitchProgressEntry(
        videoId: '9105',
        progress: Progress(
          itemId: 'twitch:vod:9105',
          positionMs: const Duration(minutes: 30).inMilliseconds,
          durationMs: const Duration(minutes: 210).inMilliseconds,
          updatedAt: ago(40),
        ),
      ),
    ];
    return FakeTwitchBackend(
      now: now,
      signedIn: signedIn,
      canSignIn: canSignIn,
      channels: channels,
      live: live,
      videos: videos,
      clips: clips,
      schedules: schedules,
      categories: games,
      top: top,
      categoryLive: byCategory,
      follows: const ['7001', '7002', '7003', '7004', '7005', '7006'],
      progress: progress,
    );
  }

  final DateTime now;
  bool signedIn;
  @override
  final bool canSignIn;
  final List<TwitchChannel> channels;
  final List<TwitchStream> live;
  final List<TwitchVideo> allVideos;
  final List<TwitchClip> allClips;
  final Map<String, List<TwitchScheduleSegment>> schedules;
  final List<TwitchCategory> categories;

  /// User ids the user follows; tests change it (TWI-033: followed on
  /// Twitch itself).
  List<String> follows;
  final List<TwitchProgressEntry> progress;

  /// `games/top` in order (TWI-034).
  final List<TwitchCategory> top;

  /// Live streams by category id; without an entry, [live] of that game.
  final Map<String, List<TwitchStream>> categoryLive;

  /// Like real Helix lists, a page starts with the last item of the page
  /// before ("floating" lists, research 06 §1.4).
  bool floatingPages = false;

  /// Thrown by the browse calls (`games/top`, `streams?game_id`) while set.
  TwitchUnavailable? discoveryError;

  /// Streams asked for, as `<game>:<language or *>:<cursor or ->`.
  final List<String> streamQueries = [];

  /// Channels served while signed in, like the library cache.
  final Set<String> cached = {};

  /// Favourites mirrored into the library (user id → favourite).
  final Map<String, bool> synced = {};

  /// Helix calls made, by name.
  final List<String> calls = [];

  bool _pendingLogin = false;

  /// The signed-in viewer's own channel (TWI-032): `GET /helix/users`
  /// with the user token. A test changes it to "change the avatar on
  /// Twitch".
  TwitchChannel me = const TwitchChannel(
    userId: '7999',
    login: 'demo_viewer',
    displayName: 'Demo_Viewer',
    avatarUrl: kDemoArtScheme,
  );

  /// Twitch no longer accepts the sign-in (401 after a failed refresh):
  /// every Helix call asks to sign in again.
  bool expired = false;

  void _helix(String name) {
    calls.add(name);
    if (!canSignIn) {
      throw const TwitchUnavailable(TwitchUnavailableReason.notConfigured);
    }
    if (!signedIn) {
      throw const TwitchUnavailable(TwitchUnavailableReason.signedOut);
    }
    if (expired) {
      throw const TwitchUnavailable(TwitchUnavailableReason.signIn);
    }
  }

  List<TwitchChannel> _serve(Iterable<TwitchChannel> list) {
    final out = list.toList();
    cached.addAll(out.map((c) => c.login));
    return out;
  }

  @override
  TwitchAccount? get account =>
      signedIn ? TwitchAccount(userId: me.userId!, login: me.login) : null;

  @override
  Future<TwitchDeviceLogin> startLogin({bool chat = false}) async {
    if (!canSignIn) {
      throw const TwitchUnavailable(TwitchUnavailableReason.notConfigured);
    }
    _pendingLogin = true;
    return const TwitchDeviceLogin(
      userCode: 'KTWQ-PLMN',
      verificationUri: 'https://www.twitch.tv/activate?device-code=KTWQPLMN',
      expiresIn: Duration(minutes: 30),
      interval: Duration(seconds: 5),
    );
  }

  @override
  Future<TwitchAccount?> pollLogin() async {
    if (!_pendingLogin) {
      throw const TwitchUnavailable(TwitchUnavailableReason.signIn);
    }
    _pendingLogin = false;
    signedIn = true;
    expired = false;
    return account;
  }

  @override
  Future<void> signOut() async => signedIn = false;

  @override
  Future<List<TwitchChannel>> users({
    List<String> ids = const [],
    List<String> logins = const [],
  }) async {
    _helix('users');
    return [
      if (ids.contains(me.userId) || logins.contains(me.login)) me,
      ..._serve(
        channels.where(
          (c) => ids.contains(c.userId) || logins.contains(c.login),
        ),
      ),
    ];
  }

  @override
  Future<List<TwitchStream>> liveStreams(List<String> userIds) async {
    _helix('streams');
    return [
      for (final s in live)
        if (userIds.contains(s.userId)) s,
    ];
  }

  @override
  Future<List<TwitchChannel>> followed() async {
    _helix('followed');
    return _serve(channels.where((c) => follows.contains(c.userId)));
  }

  @override
  Future<List<TwitchVideo>> videos(
    String userId, {
    TwitchVideoType? type,
    int limit = 20,
  }) async {
    _helix('videos');
    return _videosOf(userId, type).take(limit).toList();
  }

  Iterable<TwitchVideo> _videosOf(String userId, TwitchVideoType? type) => [
    for (final v in allVideos)
      if (v.userId == userId && (type == null || v.type == type))
        _withProgress(v),
  ]..sort((a, b) => b.publishedAt.compareTo(a.publishedAt));

  TwitchVideo _withProgress(TwitchVideo v) {
    final p = progress.where((p) => p.videoId == v.id).firstOrNull;
    return p == null ? v : v.copyWith(progress: p.progress);
  }

  @override
  Future<List<TwitchClip>> clips(String userId, {int limit = 20}) async {
    _helix('clips');
    return userId == '7001' ? allClips : const [];
  }

  @override
  Future<List<TwitchScheduleSegment>> schedule(String userId) async {
    _helix('schedule');
    return schedules[userId] ?? const [];
  }

  @override
  Future<List<TwitchChannel>> searchChannels(String query) async {
    _helix('search_channels');
    final q = query.toLowerCase();
    return _serve(
      channels.where(
        (c) => c.login.contains(q) || c.displayName.toLowerCase().contains(q),
      ),
    );
  }

  @override
  Future<List<TwitchCategory>> searchCategories(String query) async {
    _helix('search_categories');
    final q = query.toLowerCase();
    return [
      for (final c in categories)
        if (c.name.toLowerCase().contains(q)) c,
    ];
  }

  /// Offset cursors; with [floatingPages] a page repeats the last item of
  /// the page before.
  TwitchPage<T> _page<T>(List<T> all, String? cursor, int limit) {
    final offset = int.tryParse(cursor ?? '') ?? 0;
    final from = (floatingPages && offset > 0 ? offset - 1 : offset).clamp(
      0,
      all.length,
    );
    final end = (offset + limit).clamp(0, all.length);
    return TwitchPage(all.sublist(from, end), end < all.length ? '$end' : null);
  }

  @override
  Future<TwitchPage<TwitchCategory>> topCategories({
    String? cursor,
    int limit = 40,
  }) async {
    _helix('games_top');
    final error = discoveryError;
    if (error != null) throw error;
    return _page(top, cursor, limit);
  }

  @override
  Future<TwitchPage<TwitchStream>> categoryStreams(
    String gameId, {
    String? language,
    String? cursor,
    int limit = 30,
  }) async {
    _helix('category_streams');
    streamQueries.add('$gameId:${language ?? '*'}:${cursor ?? '-'}');
    final error = discoveryError;
    if (error != null) throw error;
    final all = [
      for (final s
          in categoryLive[gameId] ??
              [
                for (final s in live)
                  if (s.gameId == gameId) s,
              ])
        if (language == null || s.language == language) s,
    ]..sort((a, b) => b.viewers.compareTo(a.viewers));
    return _page(all, cursor, limit);
  }

  @override
  Future<List<TwitchChannel>> searchCached(String query) async {
    final q = query.toLowerCase();
    return [
      for (final c in channels)
        if (cached.contains(c.login) &&
            (c.login.contains(q) || c.displayName.toLowerCase().contains(q)))
          c,
    ];
  }

  @override
  Future<List<TwitchVideo>> cachedVideos(String userId) async {
    final c = channels.where((c) => c.userId == userId).firstOrNull;
    if (c == null || !cached.contains(c.login)) return const [];
    return _videosOf(userId, null).toList();
  }

  @override
  Future<List<TwitchChapter>> chapters(String videoId) async =>
      allVideos.where((v) => v.id == videoId).firstOrNull?.chapters ?? const [];

  @override
  Future<List<TwitchVideo>> categoryVideos(String gameId) async => [
    for (final v in allVideos)
      if (v.chapters.any((c) => c.gameId == gameId)) _withProgress(v),
  ];

  @override
  Future<List<TwitchProgressEntry>> continueVods() async => [
    for (final p in progress)
      TwitchProgressEntry(
        videoId: p.videoId,
        progress: p.progress,
        video: allVideos.where((v) => v.id == p.videoId).firstOrNull,
      ),
  ];

  @override
  Future<void> syncFavorite(TwitchChannel channel, bool favorite) async {
    final id = channel.userId;
    if (id != null) synced[id] = favorite;
  }

  /// Bells mirrored by [syncNotify], by user id.
  final Map<String, bool> syncedNotify = {};

  @override
  Future<void> syncNotify(TwitchChannel channel, bool notify) async {
    final id = channel.userId;
    if (id != null) syncedNotify[id] = notify;
  }
}

/// Demo favourites: five channels, one pinned (demo mode, tests).
Future<void> seedDemoTwitchFavorites(TwitchService twitch) async {
  final backend = twitch.backend;
  if (twitch.favorites.isNotEmpty || backend is! FakeTwitchBackend) return;
  for (final c in backend.channels.take(5)) {
    await twitch.addFavorite(c);
  }
  await twitch.setPinned('quietkettle', true);
}
