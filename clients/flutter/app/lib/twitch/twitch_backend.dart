// SPDX-License-Identifier: MPL-2.0
//
// Where the "Streams" section gets Twitch data from (TWI-013). Helix-shaped
// on purpose: [CoreTwitchBackend] calls pa-twitch through pa-frb, and the
// fake Helix (lib/fakes/fake_twitch.dart) serves made-up channels to the
// demo and the tests. Section logic (favourites, sorting, Home rows) lives
// in [TwitchService] above both.

import '../integration/library.dart' show Progress;
import 'twitch_models.dart';

/// Why a Twitch call could not be made.
enum TwitchUnavailableReason {
  /// No Twitch client id in this build: local data only.
  notConfigured,

  /// The call needs a Twitch sign-in (Helix answers only with a token).
  signedOut,

  /// The sign-in expired or was declined: sign in again.
  signIn,
  network,
  other,
}

class TwitchUnavailable implements Exception {
  const TwitchUnavailable(this.reason, [this.message]);

  final TwitchUnavailableReason reason;
  final String? message;

  @override
  String toString() => 'TwitchUnavailable(${reason.name}: $message)';
}

/// A started VOD for "Continue watching". [video] is null once the 24-hour
/// metadata cache dropped it; the progress itself stays.
class TwitchProgressEntry {
  const TwitchProgressEntry({
    required this.videoId,
    required this.progress,
    this.video,
    this.channel,
  });

  final String videoId;
  final Progress progress;
  final TwitchVideo? video;
  final TwitchChannel? channel;
}

abstract interface class TwitchBackend {
  /// A Twitch client id is configured: sign-in is possible.
  bool get canSignIn;

  /// The signed-in account; null without a login (local data only).
  TwitchAccount? get account;

  /// Starts a sign-in; with [chat] it also asks for permission to send
  /// chat messages (TWI-024), with [clips] to make clips (TWI-040).
  /// Permissions granted before stay.
  Future<TwitchDeviceLogin> startLogin({bool chat = false, bool clips = false});

  /// The signed-in account may make clips (`clips:edit`). No network.
  bool get canMakeClips;

  /// Makes a clip of the live stream of [broadcasterId] and waits until
  /// Twitch finished it (TWI-040). The expected failures are outcomes,
  /// not exceptions; [TwitchUnavailable] only without a client id.
  Future<TwitchClipResult> makeClip(String broadcasterId);

  /// Polls the pending sign-in once: the account when confirmed.
  Future<TwitchAccount?> pollLogin();
  Future<void> signOut();

  // Helix (needs [account]); throw [TwitchUnavailable] otherwise.

  Future<List<TwitchChannel>> users({
    List<String> ids = const [],
    List<String> logins = const [],
  });
  Future<List<TwitchStream>> liveStreams(List<String> userIds);
  Future<List<TwitchChannel>> followed();
  Future<List<TwitchVideo>> videos(
    String userId, {
    TwitchVideoType? type,
    int limit = 20,
  });
  Future<List<TwitchClip>> clips(String userId, {int limit = 20});
  Future<List<TwitchScheduleSegment>> schedule(String userId);
  Future<List<TwitchChannel>> searchChannels(String query);
  Future<List<TwitchCategory>> searchCategories(String query);

  /// The most watched categories now, one page (`games/top`, TWI-034).
  Future<TwitchPage<TwitchCategory>> topCategories({
    String? cursor,
    int limit = 40,
  });

  /// Live streams of category [gameId], most viewers first, one page
  /// (`streams?game_id`, TWI-034); [language] is ISO 639-1, null = all.
  Future<TwitchPage<TwitchStream>> categoryStreams(
    String gameId, {
    String? language,
    String? cursor,
    int limit = 30,
  });

  // Local (library cache, no network).

  /// Cached channels matching [query] (name or login).
  Future<List<TwitchChannel>> searchCached(String query);

  /// Cached VODs of a channel, newest first.
  Future<List<TwitchVideo>> cachedVideos(String userId);

  /// Stored chapters of a VOD (TWI-008).
  Future<List<TwitchChapter>> chapters(String videoId);

  /// Cached VODs with a chapter in category [gameId] (TWI-008).
  Future<List<TwitchVideo>> categoryVideos(String gameId);

  /// Started VODs, latest first.
  Future<List<TwitchProgressEntry>> continueVods();

  /// Mirrors a favourite into the library (synced between devices).
  /// Only resolved channels (with a user id).
  Future<void> syncFavorite(TwitchChannel channel, bool favorite);

  /// Mirrors a favourite's "X is live" bell into the library (local to the
  /// device, TWI-009).
  Future<void> syncNotify(TwitchChannel channel, bool notify);
}

/// No Twitch client id and no library cache (the v0 fallback library):
/// favourites by name only.
class LocalTwitchBackend implements TwitchBackend {
  const LocalTwitchBackend();

  Never _off() =>
      throw const TwitchUnavailable(TwitchUnavailableReason.notConfigured);

  @override
  bool get canSignIn => false;

  @override
  TwitchAccount? get account => null;

  @override
  Future<TwitchDeviceLogin> startLogin({
    bool chat = false,
    bool clips = false,
  }) async => _off();

  @override
  bool get canMakeClips => false;

  @override
  Future<TwitchClipResult> makeClip(String broadcasterId) async => _off();

  @override
  Future<TwitchAccount?> pollLogin() async => _off();

  @override
  Future<void> signOut() async {}

  @override
  Future<List<TwitchChannel>> users({
    List<String> ids = const [],
    List<String> logins = const [],
  }) async => _off();

  @override
  Future<List<TwitchStream>> liveStreams(List<String> userIds) async => _off();

  @override
  Future<List<TwitchChannel>> followed() async => _off();

  @override
  Future<List<TwitchVideo>> videos(
    String userId, {
    TwitchVideoType? type,
    int limit = 20,
  }) async => _off();

  @override
  Future<List<TwitchClip>> clips(String userId, {int limit = 20}) async =>
      _off();

  @override
  Future<List<TwitchScheduleSegment>> schedule(String userId) async => _off();

  @override
  Future<List<TwitchChannel>> searchChannels(String query) async => _off();

  @override
  Future<List<TwitchCategory>> searchCategories(String query) async => _off();

  @override
  Future<TwitchPage<TwitchCategory>> topCategories({
    String? cursor,
    int limit = 40,
  }) async => _off();

  @override
  Future<TwitchPage<TwitchStream>> categoryStreams(
    String gameId, {
    String? language,
    String? cursor,
    int limit = 30,
  }) async => _off();

  @override
  Future<List<TwitchChannel>> searchCached(String query) async => const [];

  @override
  Future<List<TwitchVideo>> cachedVideos(String userId) async => const [];

  @override
  Future<List<TwitchChapter>> chapters(String videoId) async => const [];

  @override
  Future<List<TwitchVideo>> categoryVideos(String gameId) async => const [];

  @override
  Future<List<TwitchProgressEntry>> continueVods() async => const [];

  @override
  Future<void> syncFavorite(TwitchChannel channel, bool favorite) async {}

  @override
  Future<void> syncNotify(TwitchChannel channel, bool notify) async {}
}
