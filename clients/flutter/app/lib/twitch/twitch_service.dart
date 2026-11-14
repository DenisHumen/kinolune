// SPDX-License-Identifier: MPL-2.0
//
// The "Streams" section (Twitch, TWI-013) above a [TwitchBackend]:
// favourites kept on this device without a Twitch login (DEC-006, ADR-0011
// §5), their order and filters, the rows of the section's Home (research 06
// §6.2), the channel page (§6.3), search, the Continue hook of the app's
// Home (UI-007) and Device Code Flow sign-in.
//
// Without a login Helix is out of reach (it needs a token, and the project
// has no client id yet): favourites added by channel name are kept by
// login and resolved to a Twitch user id when Helix becomes available.
// Live status, new VODs and schedules then come from Helix; before that
// the section shows what the library cache has.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:material_ui/material_ui.dart';

import '../integration/library.dart' show Progress;
import '../screens/home_feed.dart' show ContinueEntry, ContinueKind;
import '../services/preferences.dart';
import 'twitch_backend.dart';
import 'twitch_models.dart';

/// Order of the favourites (pinned ones always first).
enum TwitchFavoriteSort { liveFirst, viewers, name, recentlyAdded, lastLive }

/// Which favourites to show.
enum TwitchFavoriteFilter { all, live, offline }

/// A favourite with what is known about it now.
class TwitchFavoriteEntry {
  const TwitchFavoriteEntry({
    required this.favorite,
    required this.channel,
    this.live,
  });

  final TwitchFavorite favorite;
  final TwitchChannel channel;

  /// The live stream; null when offline or unknown (no login).
  final TwitchStream? live;

  bool get isLive => live != null;
}

/// [entries] filtered and sorted; pinned favourites first in every order.
List<TwitchFavoriteEntry> sortFavorites(
  Iterable<TwitchFavoriteEntry> entries,
  TwitchFavoriteSort sort, [
  TwitchFavoriteFilter filter = TwitchFavoriteFilter.all,
]) {
  final list = [
    for (final e in entries)
      if (switch (filter) {
        TwitchFavoriteFilter.all => true,
        TwitchFavoriteFilter.live => e.isLive,
        TwitchFavoriteFilter.offline => !e.isLive,
      })
        e,
  ];
  int byName(TwitchFavoriteEntry a, TwitchFavoriteEntry b) => a
      .channel
      .displayName
      .toLowerCase()
      .compareTo(b.channel.displayName.toLowerCase());
  int byLive(TwitchFavoriteEntry a, TwitchFavoriteEntry b) =>
      (b.isLive ? 1 : 0) - (a.isLive ? 1 : 0);
  int byViewers(TwitchFavoriteEntry a, TwitchFavoriteEntry b) =>
      (b.live?.viewers ?? -1).compareTo(a.live?.viewers ?? -1);
  int byLastLive(TwitchFavoriteEntry a, TwitchFavoriteEntry b) {
    // Live now counts as "last live: now".
    final x = a.isLive ? DateTime(9999) : a.channel.lastLiveAt;
    final y = b.isLive ? DateTime(9999) : b.channel.lastLiveAt;
    if (x == null && y == null) return 0;
    if (x == null) return 1;
    if (y == null) return -1;
    return y.compareTo(x);
  }

  list.sort((a, b) {
    final pin = (b.favorite.pinned ? 1 : 0) - (a.favorite.pinned ? 1 : 0);
    if (pin != 0) return pin;
    final c = switch (sort) {
      TwitchFavoriteSort.liveFirst => _chain([byLive(a, b), byViewers(a, b)]),
      TwitchFavoriteSort.viewers => byViewers(a, b),
      TwitchFavoriteSort.name => 0,
      TwitchFavoriteSort.recentlyAdded => b.favorite.addedAt.compareTo(
        a.favorite.addedAt,
      ),
      TwitchFavoriteSort.lastLive => byLastLive(a, b),
    };
    return c != 0 ? c : byName(a, b);
  });
  return list;
}

int _chain(List<int> c) => c.firstWhere((x) => x != 0, orElse: () => 0);

/// A category on the section's Home: from live streams and VOD chapters.
class TwitchCategoryEntry {
  const TwitchCategoryEntry({required this.category, required this.count});

  final TwitchCategory category;

  /// Streams and VODs of the favourites in it.
  final int count;
}

/// A scheduled broadcast of a channel.
class TwitchUpcoming {
  const TwitchUpcoming({required this.channel, required this.segment});

  final TwitchChannel channel;
  final TwitchScheduleSegment segment;
}

/// Rows of the section's Home (research 06 §6.2).
class TwitchHome {
  const TwitchHome({
    required this.favorites,
    this.live = const [],
    this.continueVods = const [],
    this.newVods = const [],
    this.soon = const [],
    this.categories = const [],
    this.liveKnown = false,
  });

  final List<TwitchFavoriteEntry> favorites;

  /// Live now: favourites and follows, pinned first, then by viewers.
  final List<TwitchStream> live;
  final List<TwitchVideo> continueVods;

  /// New VODs of the favourites, newest first.
  final List<TwitchVideo> newVods;
  final List<TwitchUpcoming> soon;
  final List<TwitchCategoryEntry> categories;

  /// Live status comes from Helix: false without a login.
  final bool liveKnown;
}

/// The channel page (research 06 §6.3).
class TwitchChannelPage {
  const TwitchChannelPage({
    required this.channel,
    this.live,
    this.vods = const [],
    this.highlights = const [],
    this.clips = const [],
    this.schedule = const [],
    this.online = false,
  });

  final TwitchChannel channel;
  final TwitchStream? live;
  final List<TwitchVideo> vods;
  final List<TwitchVideo> highlights;
  final List<TwitchClip> clips;
  final List<TwitchScheduleSegment> schedule;

  /// From Helix; false: only the library cache.
  final bool online;
}

class TwitchSearchResults {
  const TwitchSearchResults({
    this.channels = const [],
    this.categories = const [],
    this.addByName,
    this.online = false,
  });

  final List<TwitchChannel> channels;
  final List<TwitchCategory> categories;

  /// The query is a valid channel name not among [channels]: offer
  /// "Add channel `name`" (works without a login).
  final String? addByName;
  final bool online;
}

/// What plays a Twitch stream or VOD: [TwitchService.playRoute].
enum TwitchPlayRoute {
  /// Our player through GQL/usher (full builds, signed config, TWI-004).
  native,

  /// The official embedded player (store builds, flag off; TWI-005).
  embed,

  /// The Twitch app or site (no player available).
  openOnTwitch,
}

/// What to play.
class TwitchPlayTarget {
  const TwitchPlayTarget.live(this.login)
    : videoId = null,
      startAt = null,
      title = null,
      chapters = const [];
  const TwitchPlayTarget.vod(
    String this.videoId, {
    this.login,
    this.startAt,
    this.title,
    this.chapters = const [],
  });

  final String? login;
  final String? videoId;
  final Duration? startAt;

  /// VOD title, when the caller knows it (the player's title line).
  final String? title;

  /// VOD chapters (TWI-008), when the caller knows them; the native player
  /// shows them on the scrubber (TWI-014).
  final List<TwitchChapter> chapters;

  bool get isLive => videoId == null;

  /// `twitch.tv` page: the deeplink of "Open on Twitch".
  String get url => isLive
      ? 'https://www.twitch.tv/$login' // l10n-ignore
      : 'https://www.twitch.tv/videos/$videoId'; // l10n-ignore
}

/// A player of the Twitch section: TWI-014 registers the native one, TWI-005
/// the official embed.
typedef TwitchPlayerHook = Future<void> Function(
  BuildContext context,
  TwitchPlayTarget target,
);

/// Chooses the player (TWI-004, ADR-0039/0041): store builds never play
/// natively; full builds do when the signed remote config allows it and a
/// native player is there. Otherwise the official embed, else "Open on
/// Twitch".
TwitchPlayRoute decideTwitchRoute({
  required bool fullBuild,
  required bool nativeAllowedByConfig,
  required bool hasNativePlayer,
  required bool hasEmbedPlayer,
}) {
  if (fullBuild && nativeAllowedByConfig && hasNativePlayer) {
    return TwitchPlayRoute.native;
  }
  return hasEmbedPlayer ? TwitchPlayRoute.embed : TwitchPlayRoute.openOnTwitch;
}

class TwitchService extends ChangeNotifier {
  TwitchService({
    required this.backend,
    required this.preferences,
    DateTime Function()? clock,
    this.fullBuild = false,
    this.nativeAllowedByConfig = false,
  }) : clock = clock ?? DateTime.now;

  final TwitchBackend backend;
  final Preferences preferences;
  final DateTime Function() clock;

  /// Native Twitch playback compiled in (`AppFlavor.twitchNative`).
  final bool fullBuild;

  /// The signed remote config allows native playback (`twitch.gql_playback`,
  /// `player = native`, no kill-switch).
  final bool nativeAllowedByConfig;

  /// Players registered by TWI-014 (native) and TWI-005 (embed).
  TwitchPlayerHook? nativePlayer;
  TwitchPlayerHook? embedPlayer;

  TwitchPlayRoute get playRoute => decideTwitchRoute(
    fullBuild: fullBuild,
    nativeAllowedByConfig: nativeAllowedByConfig,
    hasNativePlayer: nativePlayer != null,
    hasEmbedPlayer: embedPlayer != null,
  );

  /// A player saved VOD progress (TWI-005): Continue rows read it again.
  void progressChanged() => notifyListeners();

  static const newVodDays = 14;
  static const soonDays = 7;
  static const _maxChannels = 24;

  List<TwitchFavorite> _favorites = const [];
  final Map<String, TwitchChannel> _channels = {};
  final Map<String, TwitchStream> _live = {};
  TwitchFavoriteSort _sort = TwitchFavoriteSort.liveFirst;
  TwitchFavoriteFilter _filter = TwitchFavoriteFilter.all;
  bool _liveKnown = false;

  List<TwitchFavorite> get favorites => _favorites;
  TwitchFavoriteSort get sort => _sort;
  TwitchFavoriteFilter get filter => _filter;
  TwitchAccount? get account => backend.account;
  bool get canSignIn => backend.canSignIn;

  /// Live status is known (Helix reachable).
  bool get liveKnown => _liveKnown;

  Future<void> init() async {
    final raw = await preferences.read(Preferences.twitchFavorites);
    if (raw != null) {
      try {
        _favorites = [
          for (final j in jsonDecode(raw) as List) ?TwitchFavorite.fromJson(j),
        ];
      } catch (_) {
        _favorites = const [];
      }
    }
    final sortPref = await preferences.read(Preferences.twitchFavoritesSort);
    final filterPref = await preferences.read(
      Preferences.twitchFavoritesFilter,
    );
    _sort = TwitchFavoriteSort.values.firstWhere(
      (s) => s.name == sortPref,
      orElse: () => TwitchFavoriteSort.liveFirst,
    );
    _filter = TwitchFavoriteFilter.values.firstWhere(
      (f) => f.name == filterPref,
      orElse: () => TwitchFavoriteFilter.all,
    );
    notifyListeners();
  }

  Future<void> _save() => preferences.write(
    Preferences.twitchFavorites,
    _favorites.isEmpty
        ? null
        : jsonEncode([for (final f in _favorites) f.toJson()]),
  );

  bool isFavorite(String login) => _favorites.any((f) => f.login == login);

  /// The channel known for [login]: from Helix, the cache or the favourite.
  TwitchChannel channelOf(String login) {
    final known = _channels[login];
    if (known != null) return known;
    final f = _favorites.where((f) => f.login == login).firstOrNull;
    return TwitchChannel(
      login: login,
      displayName: f?.displayName ?? login,
      userId: f?.userId,
      avatarUrl: f?.avatarUrl,
    );
  }

  TwitchStream? liveOf(TwitchChannel c) =>
      c.userId == null ? null : _live[c.userId];

  /// Adds a favourite; works without a Twitch login. A channel added by
  /// name ([TwitchChannel.userId] null) is resolved by [refresh].
  Future<void> addFavorite(TwitchChannel channel) async {
    if (isFavorite(channel.login)) return;
    _channels[channel.login] = channel;
    _favorites = [
      ..._favorites,
      TwitchFavorite(
        login: channel.login,
        userId: channel.userId,
        displayName: channel.displayName,
        avatarUrl: channel.avatarUrl,
        addedAt: clock(),
      ),
    ];
    await _save();
    notifyListeners();
    await _syncFavorite(channel, true);
  }

  /// Adds a favourite by channel name (`login`, `@login` or a
  /// `twitch.tv/login` link). Returns false for an invalid name.
  Future<bool> addFavoriteByName(String input) async {
    final login = normalizeLogin(input);
    if (login == null) return false;
    await addFavorite(channelOf(login));
    unawaited(refresh());
    return true;
  }

  Future<void> removeFavorite(String login) async {
    final channel = channelOf(login);
    _favorites = [
      for (final f in _favorites)
        if (f.login != login) f,
    ];
    await _save();
    notifyListeners();
    await _syncFavorite(channel, false);
  }

  Future<void> toggleFavorite(TwitchChannel channel) =>
      isFavorite(channel.login)
      ? removeFavorite(channel.login)
      : addFavorite(channel);

  Future<void> setPinned(String login, bool pinned) async {
    _favorites = [
      for (final f in _favorites)
        f.login == login ? f.copyWith(pinned: pinned) : f,
    ];
    await _save();
    notifyListeners();
  }

  /// Whether the "X is live" bell of favourite [login] is on (TWI-009).
  bool notifyOf(String login) =>
      _favorites.where((f) => f.login == login).firstOrNull?.notify ?? false;

  /// Turns the "X is live" bell of favourite [login] on or off; local to
  /// this device.
  Future<void> setNotify(String login, bool notify) async {
    if (!isFavorite(login) || notifyOf(login) == notify) return;
    _favorites = [
      for (final f in _favorites)
        f.login == login ? f.copyWith(notify: notify) : f,
    ];
    await _save();
    notifyListeners();
    try {
      await backend.syncNotify(channelOf(login), notify);
    } catch (e) {
      debugPrint('Twitch bell not mirrored: $e');
    }
  }

  Future<void> setSort(TwitchFavoriteSort sort) async {
    _sort = sort;
    await preferences.write(Preferences.twitchFavoritesSort, sort.name);
    notifyListeners();
  }

  Future<void> setFilter(TwitchFavoriteFilter filter) async {
    _filter = filter;
    await preferences.write(Preferences.twitchFavoritesFilter, filter.name);
    notifyListeners();
  }

  Future<void> _syncFavorite(TwitchChannel c, bool on) async {
    try {
      await backend.syncFavorite(c, on);
    } catch (e) {
      debugPrint('Twitch favourite not synced: $e');
    }
  }

  /// Favourites with their channel and live stream, in the chosen order.
  List<TwitchFavoriteEntry> favoriteEntries({
    TwitchFavoriteSort? sort,
    TwitchFavoriteFilter? filter,
  }) {
    final entries = [
      for (final f in _favorites)
        TwitchFavoriteEntry(
          favorite: f,
          channel: channelOf(f.login),
          live: liveOf(channelOf(f.login)),
        ),
    ];
    return sortFavorites(entries, sort ?? _sort, filter ?? _filter);
  }

  /// Resolves favourites added by name, refreshes profiles and live status
  /// from Helix. Without a login only cached profiles are used.
  Future<void> refresh() async {
    if (account == null) {
      await _fromCache();
      _liveKnown = false;
      notifyListeners();
      return;
    }
    try {
      final logins = [for (final f in _favorites) f.login];
      if (logins.isNotEmpty) {
        final users = await backend.users(logins: logins);
        _remember(users);
        var changed = false;
        _favorites = [
          for (final f in _favorites)
            if (_channels[f.login] case final c? when c.userId != null)
              _resolved(f, c, () => changed = true)
            else
              f,
        ];
        if (changed) await _save();
      }
      final ids = [for (final f in _favorites) ?_channels[f.login]?.userId];
      final streams = await backend.liveStreams(ids);
      _live
        ..clear()
        ..addEntries(streams.map((s) => MapEntry(s.userId, s)));
      final now = clock();
      for (final s in streams) {
        final c = _channels[s.login];
        if (c != null) _channels[s.login] = c.copyWith(lastLiveAt: now);
      }
      _liveKnown = true;
    } on TwitchUnavailable catch (e) {
      debugPrint('Twitch refresh: $e');
      _liveKnown = false;
    }
    notifyListeners();
  }

  TwitchFavorite _resolved(
    TwitchFavorite f,
    TwitchChannel c,
    void Function() changed,
  ) {
    final next = f.copyWith(
      userId: c.userId,
      displayName: c.displayName,
      avatarUrl: c.avatarUrl,
    );
    if (next.userId != f.userId ||
        next.displayName != f.displayName ||
        next.avatarUrl != f.avatarUrl) {
      changed();
      if (f.userId == null) unawaited(_syncFavorite(c, true));
    }
    return next;
  }

  void _remember(Iterable<TwitchChannel> channels) {
    for (final c in channels) {
      _channels[c.login] = c;
    }
  }

  Future<void> _fromCache() async {
    for (final f in _favorites) {
      if (_channels.containsKey(f.login)) continue;
      try {
        final hits = await backend.searchCached(f.login);
        _remember(hits.where((c) => c.login == f.login));
      } catch (_) {}
    }
  }

  // ----- Home --------------------------------------------------------------

  Future<TwitchHome> home() async {
    await refresh();
    final favorites = favoriteEntries(filter: TwitchFavoriteFilter.all);
    final online = _liveKnown;
    final live = <String, TwitchStream>{
      for (final s in favorites.map((e) => e.live).nonNulls) s.userId: s,
    };
    if (online) {
      try {
        final followed = await backend.followed();
        _remember(followed);
        final missing = [
          for (final c in followed)
            if (c.userId != null && !live.containsKey(c.userId)) c.userId!,
        ];
        for (final s in await backend.liveStreams(missing)) {
          live[s.userId] = s;
        }
      } on TwitchUnavailable catch (e) {
        debugPrint('Twitch follows: $e');
      }
    }
    final pinned = {
      for (final e in favorites)
        if (e.favorite.pinned) ?e.channel.userId,
    };
    final liveList = live.values.toList()
      ..sort((a, b) {
        final p =
            (pinned.contains(b.userId) ? 1 : 0) -
            (pinned.contains(a.userId) ? 1 : 0);
        return p != 0 ? p : b.viewers.compareTo(a.viewers);
      });

    final now = clock();
    final channels = [
      for (final e in favorites)
        if (e.channel.userId != null) e.channel,
    ].take(_maxChannels).toList();
    final vods = <TwitchVideo>[];
    final soon = <TwitchUpcoming>[];
    for (final c in channels) {
      final id = c.userId!;
      try {
        final list = online
            ? await backend.videos(id, type: TwitchVideoType.archive, limit: 5)
            : await backend.cachedVideos(id);
        vods.addAll(
          list.where(
            (v) =>
                now.difference(v.publishedAt) <
                const Duration(days: newVodDays),
          ),
        );
      } on TwitchUnavailable catch (e) {
        debugPrint('Twitch VODs of ${c.login}: $e');
      }
      if (!online) continue;
      try {
        for (final s in await backend.schedule(id)) {
          if (s.canceled || s.start.isBefore(now)) continue;
          if (s.start.difference(now) > const Duration(days: soonDays)) {
            continue;
          }
          if (live.containsKey(id)) continue;
          soon.add(TwitchUpcoming(channel: c, segment: s));
        }
      } on TwitchUnavailable catch (e) {
        debugPrint('Twitch schedule of ${c.login}: $e');
      }
    }
    vods.sort((a, b) => b.publishedAt.compareTo(a.publishedAt));
    final newVods = await _withChapters(vods.take(20).toList());
    soon.sort((a, b) => a.segment.start.compareTo(b.segment.start));

    final continueVods = await _continueVideos();
    return TwitchHome(
      favorites: favoriteEntries(),
      live: liveList,
      continueVods: continueVods,
      newVods: newVods,
      soon: soon,
      categories: _categories(liveList, newVods),
      liveKnown: online,
    );
  }

  Future<List<TwitchVideo>> _withChapters(List<TwitchVideo> vods) async {
    final out = <TwitchVideo>[];
    for (final v in vods) {
      if (v.chapters.isNotEmpty) {
        out.add(v);
        continue;
      }
      try {
        out.add(v.copyWith(chapters: await backend.chapters(v.id)));
      } catch (_) {
        out.add(v);
      }
    }
    return out;
  }

  List<TwitchCategoryEntry> _categories(
    List<TwitchStream> live,
    List<TwitchVideo> vods,
  ) {
    final counts = <String, (TwitchCategory, int)>{};
    void add(String? id, String? name, String? art) {
      if (id == null || name == null || name.isEmpty) return;
      final (c, n) =
          counts[id] ?? (TwitchCategory(id: id, name: name, boxArtUrl: art), 0);
      counts[id] = (c, n + 1);
    }

    for (final s in live) {
      add(s.gameId, s.gameName, null);
    }
    for (final v in vods) {
      for (final id in {for (final c in v.chapters) c.gameId}) {
        final c = v.chapters.firstWhere((c) => c.gameId == id);
        add(c.gameId, c.gameName, c.boxArtUrl);
      }
    }
    final list =
        [
          for (final (c, n) in counts.values)
            TwitchCategoryEntry(category: c, count: n),
        ]..sort((a, b) {
          final c = b.count.compareTo(a.count);
          return c != 0 ? c : a.category.name.compareTo(b.category.name);
        });
    return list;
  }

  Future<List<TwitchVideo>> _continueVideos() async {
    try {
      final rows = await backend.continueVods();
      return [
        for (final r in rows)
          (r.video ??
                  TwitchVideo(
                    id: r.videoId,
                    userId: r.channel?.userId ?? '',
                    login: r.channel?.login ?? '',
                    displayName: r.channel?.displayName ?? '',
                    title: '',
                    publishedAt: r.progress.updatedAt,
                    duration: Duration(milliseconds: r.progress.durationMs),
                  ))
              .copyWith(progress: r.progress),
      ];
    } catch (e) {
      debugPrint('Twitch Continue: $e');
      return const [];
    }
  }

  /// Twitch VODs for the app's shared Continue row (UI-007 hook): emits
  /// now and after every change of the section; nothing while [enabled]
  /// (the Settings switch of the section) is off.
  Stream<List<ContinueEntry>> continueEntries({
    ValueListenable<bool>? enabled,
  }) {
    late StreamController<List<ContinueEntry>> c;
    Future<void> emit() async {
      if (enabled != null && !enabled.value) {
        if (!c.isClosed) c.add(const []);
        return;
      }
      final vods = await _continueVideos();
      if (c.isClosed) return;
      c.add([
        for (final v in vods)
          if (v.progress case final Progress p)
            ContinueEntry(
              kind: ContinueKind.twitchVod,
              id: v.key,
              title: v.title.isNotEmpty ? v.title : v.displayName,
              progress: p,
              imageUrl: twitchImage(v.thumbnailUrl, 640, 360),
              subtitle: v.displayName.isEmpty ? null : v.displayName,
            ),
      ]);
    }

    void listener() => unawaited(emit());
    c = StreamController<List<ContinueEntry>>(
      onListen: () {
        addListener(listener);
        enabled?.addListener(listener);
        unawaited(emit());
      },
      onCancel: () {
        removeListener(listener);
        enabled?.removeListener(listener);
      },
    );
    return c.stream;
  }

  // ----- Following, channel page, search, categories -----------------------

  /// Channels the signed-in user follows with live status.
  Future<List<TwitchFavoriteEntry>> following({
    TwitchFavoriteSort? sort,
    TwitchFavoriteFilter? filter,
  }) async {
    final followed = await backend.followed();
    _remember(followed);
    final streams = await backend.liveStreams([
      for (final c in followed) ?c.userId,
    ]);
    for (final s in streams) {
      _live[s.userId] = s;
    }
    final now = clock();
    return sortFavorites(
      [
        for (final c in followed)
          TwitchFavoriteEntry(
            favorite: TwitchFavorite(login: c.login, addedAt: now),
            channel: c,
            live: liveOf(c),
          ),
      ],
      sort ?? _sort,
      filter ?? _filter,
    );
  }

  Future<TwitchChannelPage> channelPage(TwitchChannel channel) async {
    var c = _channels[channel.login] ?? channel;
    if (account != null) {
      try {
        if (c.userId == null) {
          final users = await backend.users(logins: [c.login]);
          _remember(users);
          c = _channels[c.login] ?? c;
        }
        final id = c.userId;
        if (id != null) {
          final live = await backend.liveStreams([id]);
          if (live.isNotEmpty) _live[id] = live.first;
          final vods = await backend.videos(id, type: TwitchVideoType.archive);
          final highlights = await backend.videos(
            id,
            type: TwitchVideoType.highlight,
          );
          final clips = await backend.clips(id);
          final schedule = await backend.schedule(id);
          return TwitchChannelPage(
            channel: c,
            live: live.firstOrNull,
            vods: await _withChapters(vods),
            highlights: highlights,
            clips: clips,
            schedule: [
              for (final s in schedule)
                if (!s.canceled && s.start.isAfter(clock())) s,
            ],
            online: true,
          );
        }
      } on TwitchUnavailable catch (e) {
        debugPrint('Twitch channel ${c.login}: $e');
      }
    }
    final id = c.userId;
    final cached = id == null
        ? const <TwitchVideo>[]
        : await backend.cachedVideos(id).catchError((_) => <TwitchVideo>[]);
    return TwitchChannelPage(
      channel: c,
      vods: await _withChapters([
        for (final v in cached)
          if (v.type == TwitchVideoType.archive) v,
      ]),
      highlights: [
        for (final v in cached)
          if (v.type == TwitchVideoType.highlight) v,
      ],
    );
  }

  /// Channels and categories. Without a login: the favourites and the
  /// library cache, plus "Add channel `name`".
  Future<TwitchSearchResults> search(String query) async {
    final q = query.trim();
    if (q.isEmpty) return const TwitchSearchResults();
    final online = account != null;
    final channels = <String, TwitchChannel>{};
    final categories = <TwitchCategory>[];
    final lower = q.toLowerCase();
    for (final f in _favorites) {
      final c = channelOf(f.login);
      if (c.login.contains(lower) ||
          c.displayName.toLowerCase().contains(lower)) {
        channels[c.login] = c;
      }
    }
    try {
      final found = online
          ? await backend.searchChannels(q)
          : await backend.searchCached(q);
      _remember(found);
      for (final c in found) {
        channels.putIfAbsent(c.login, () => c);
      }
      if (online) categories.addAll(await backend.searchCategories(q));
    } on TwitchUnavailable catch (e) {
      debugPrint('Twitch search: $e');
    }
    final login = normalizeLogin(q);
    return TwitchSearchResults(
      channels: channels.values.toList(),
      categories: categories,
      addByName: login != null && !channels.containsKey(login) ? login : null,
      online: online,
    );
  }

  /// VODs of the favourites with a chapter in [category] (TWI-008).
  Future<List<TwitchVideo>> categoryVideos(TwitchCategory category) async {
    try {
      return await _withChapters(await backend.categoryVideos(category.id));
    } on TwitchUnavailable {
      return const [];
    }
  }

  // ----- sign-in -------------------------------------------------------------

  Future<TwitchDeviceLogin> startLogin() => backend.startLogin();

  /// Polls once; on success refreshes the section.
  Future<TwitchAccount?> pollLogin() async {
    final a = await backend.pollLogin();
    if (a != null) {
      notifyListeners();
      unawaited(refresh());
    }
    return a;
  }

  Future<void> signOut() async {
    await backend.signOut();
    _live.clear();
    _liveKnown = false;
    notifyListeners();
  }
}
