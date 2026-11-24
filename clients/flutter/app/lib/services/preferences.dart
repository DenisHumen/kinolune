// SPDX-License-Identifier: MPL-2.0

import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

/// Small non-secret UI preferences (interface language…). Secrets go to
/// [SecureStore](secure_store.dart), library data to pa_library.
abstract class Preferences {
  Future<String?> read(String key);
  Future<void> write(String key, String? value);

  /// BCP 47 tag of the chosen UI language; absent = follow the system.
  static const locale = 'ui_locale';

  /// 'false' keeps playing here when another device takes the title over
  /// (SYNC-003); absent = pause.
  static const takeoverPause = 'sync_takeover_pause';

  /// 'true' shows the Twitch section in the navigation (UI-002).
  static const showTwitch = 'nav_show_twitch';

  /// 'true' stops animated emotes in the Twitch chat (TWI-012).
  static const twitchChatPauseAnimations = 'twitch_chat_pause_animations';

  /// Chat filters and highlights of a profile (TWI-024), JSON of
  /// `ChatSettings`; [profile] is the profile's `syncId` (`default` for the first one).
  static String twitchChatSettings(String profile) =>
      'twitch_chat_settings_$profile';

  /// The music queue of a profile (MUS-003): JSON of
  /// `MusicPlayerController.snapshot`; [profile] is the profile's `syncId`
  /// (`default` without profiles).
  static String musicQueue(String profile) => 'music_queue_$profile';

  /// Smart playlists of a profile (MUS-003): JSON list of
  /// `SmartPlaylist` rules.
  static String musicSmartPlaylists(String profile) =>
      'music_smart_playlists_$profile';

  /// 'true' blurs overviews and stills of unseen episodes (UI-010, Hide
  /// Spoilers); absent = off.
  static const hideSpoilers = 'library_hide_spoilers';

  /// 'true' shows the advanced sections of Settings (UI-041); absent =
  /// only the basic ones.
  static const settingsAdvanced = 'settings_advanced';

  /// Recent search queries (UI-011), a JSON list, newest first.
  static const searchHistory = 'search_history';

  /// 'list' shows a library view (`movies`, `shows`) as a list (UI-008).
  static String libraryLayout(String view) => 'library_layout_$view';

  /// 'true' makes the file browser (UI-006) show file names instead of
  /// recognized titles; absent = titles.
  static const browserShowFileNames = 'browser_show_file_names';

  /// 'false' makes SFTP sources refuse RSA host and user keys (NET-006,
  /// RUSTSEC-2023-0071); absent = accepted.
  static const sshRsaKeys = 'sources_ssh_rsa_keys';

  /// 'false' keeps lyrics lookups on the device: no LRCLIB (MUS-006);
  /// absent = LRCLIB is asked for lyrics the files do not have.
  static const lyricsOnline = 'music_lyrics_online';

  /// 'false' turns off continuous playback: no Up Next after an episode
  /// (UI-016); absent = on.
  static const continuousPlayback = 'playback_continuous';

  /// Seconds the player's skip buttons, arrow keys and double tap jump
  /// (UI-013); absent = 10.
  static const skipSeconds = 'playback_skip_seconds';

  /// What the player does in intros and credits (PLAY-011), a `SkipMode`
  /// name; absent = button.
  static const skipMode = 'playback_skip_mode';

  /// 'false' stops generating seek previews for videos without them
  /// (PLAY-009); absent = on.
  static const trickplayGenerate = 'playback_trickplay_generate';

  /// What the player may do with the TV (PLAY-AND-005): JSON of
  /// `DisplayMatchSettings` (`frame_rate`, `resolution`, `dynamic_range`);
  /// absent = defaults.
  static const displayMatch = 'playback_display_match';

  /// 'false' stops the daily update of the anime databases from GitHub
  /// (META-030); absent = on.
  static const animeDataUpdates = 'metadata_anime_data_updates';

  /// 'true' once the user agreed that frames of unrecognised anime go to
  /// trace.moe (META-031); absent = off.
  static const animeFrameSearch = 'metadata_anime_frame_search';

  /// Episode order of shows the user has not set one for (UI-016),
  /// an `EpisodeOrder` name; absent = TMDB.
  static const defaultEpisodeOrder = 'shows_default_episode_order';

  /// 'true' once the user picked the episode order of [show] on its page,
  /// so [defaultEpisodeOrder] no longer applies to it.
  static String showEpisodeOrderSet(String show) => 'show_order_set_$show';

  /// Metadata language (`en-US`, `ru-RU`); absent = the UI language.
  static const metadataLanguage = 'metadata_language';

  /// 'false' turns off gapless music playback (MUS-002); absent = on.
  static const musicGapless = 'music_gapless';

  /// ReplayGain mode of music (`off`, `track`, `album`, `auto`); absent =
  /// `auto`.
  static const musicReplayGain = 'music_replay_gain';

  /// Size limit of the image cache in bytes (LIB-013); absent = default.
  static const imageCacheMaxBytes = 'image_cache_max_bytes';

  /// Subtitle style (PLAY-007), JSON of pa_player `SubtitleSettings`.
  static const subtitleSettings = 'subtitle_settings';

  /// 'true' once the first-run setup (UI-004) was finished or skipped.
  static const onboardingDone = 'onboarding_done';

  /// 'true' once the notice "passwords are in an encrypted file, the OS
  /// keychain does not work" was dismissed (CORE-007).
  static const secretFallbackNoticeShown = 'secret_fallback_notice_shown';

  /// How the user watches (UI-004), an `AppMode` name; absent = not asked.
  static const appMode = 'app_mode';

  /// The local profile of the first-run setup (UI-004), JSON of
  /// `LocalProfile`, for a library without profiles. On the Rust core it is
  /// moved into the default core profile (LIB-012) and removed.
  static const localProfile = 'local_profile';

  /// Profiles (UI-015): "who is watching?" at start (absent = on, `false`
  /// = off), App Lock and Search Lock (`true` = on), the wrong-PIN backoff
  /// (JSON `{failures, until}`), whether a profile hides the Streams
  /// section (`true`/`false`; absent = hidden for restricted profiles) and
  /// the search history of a profile other than the first ([syncId]).
  static const profilesPickerOnStart = 'profiles_picker_on_start';
  static const profilesAppLock = 'profiles_app_lock';
  static const profilesSearchLock = 'profiles_search_lock';
  static const profilesPinAttempts = 'profiles_pin_attempts';
  static String profileHideTwitch(String syncId) =>
      'profiles_hide_twitch_$syncId';

  /// UI-047: the profile is in the "just watch" mode (`true`; absent = off).
  static String profileJustWatch(String syncId) =>
      'profiles_just_watch_$syncId';
  static String profileSearchHistory(String syncId) =>
      syncId == 'default' ? searchHistory : '${searchHistory}_$syncId';

  /// Twitch favourites kept on this device (TWI-013): JSON list of
  /// `TwitchFavorite`; order and filter of the favourites.
  static const twitchFavorites = 'twitch_favorites';
  static const twitchFavoritesSort = 'twitch_favorites_sort';
  static const twitchFavoritesFilter = 'twitch_favorites_filter';

  /// Offline downloads (NET-019): Wi-Fi only (`false` = also over mobile
  /// data; on by default).
  static const offlineWifiOnly = 'offline_wifi_only';

  /// UI-018 (ADR-0123): `1` once the default auto-cleanup («Delete
  /// watched» after [offlineDefaultCleanupDays]) was applied; it is applied
  /// once, a later choice of the user stays.
  static const offlineCleanupDefault = 'offline_cleanup_default';

  /// Background downloads (NET-019, ADR-NNNN): job ids Battery Saver
  /// paused while the app had no visible UI (a JSON list), resumed when it
  /// ends — also after the process was restarted.
  static const offlineHeldJobs = 'offline_held_jobs';

  /// "X is live" notifications (TWI-009): `false` = off; on by default.
  static const twitchLiveNotifications = 'twitch_live_notifications';

  /// Native Twitch player (TWI-014): the quality picked last (`source`,
  /// `audio_only` or a height like `720`) and the live latency mode
  /// (`low` or `stable`).
  static const twitchQuality = 'twitch_quality';
  static const twitchLatencyMode = 'twitch_latency_mode';

  /// Width of the Twitch chat beside the video, logical pixels (TWI-016).
  static const twitchChatWidth = 'twitch_chat_width';

  /// Twitch file name templates (TWI-019), JSON `{folder, vod, live}`;
  /// the core keeps them in memory, the app applies them on start.
  static const twitchNameTemplates = 'twitch_name_templates';

  /// The ToS / personal-use warning of recordings was accepted (TWI-019).
  static const twitchRecordWarningAccepted = 'twitch_record_warning_accepted';

  /// Subtitle and audio tracks and delays remembered for a series
  /// (PLAY-007, PLAY-004), JSON of pa_player `TrackMemory`; [series] is the
  /// show id. Per profile: the default profile keeps the PLAY-007 key.
  static String subtitleSeries(String series, {String profile = 'default'}) =>
      profile == 'default'
      ? 'subtitle_series_$series'
      : 'track_series_${profile}_$series';

  /// Audio and subtitle languages, subtitle mode and SDH of a profile
  /// (PLAY-004), JSON of pa_player `TrackPolicy`.
  static String trackPolicy(String syncId) =>
      syncId == 'default' ? 'track_policy' : 'track_policy_$syncId';

  /// Sound processing of a profile (PLAY-004): downmix, LFE, Volume Boost;
  /// JSON of pa_player `AudioProcessing`.
  static String audioProcessing(String syncId) =>
      syncId == 'default' ? 'audio_processing' : 'audio_processing_$syncId';

  /// Music equalizer of a profile (MUS-015); JSON of pa_player
  /// `Equalizer`.
  static String musicEqualizer(String syncId) => 'music_equalizer_$syncId';
}

/// JSON file in the application support directory.
class FilePreferences implements Preferences {
  FilePreferences(this.path);

  final String path;
  Map<String, String>? _cache;

  Future<Map<String, String>> _load() async {
    final cached = _cache;
    if (cached != null) return cached;
    try {
      final f = File(path);
      if (await f.exists()) {
        final data = jsonDecode(await f.readAsString());
        if (data is Map) {
          return _cache = {
            for (final e in data.entries)
              if (e.value is String) '${e.key}': e.value as String,
          };
        }
      }
    } catch (e) {
      debugPrint('Preferences: cannot read $path: $e');
    }
    return _cache = {};
  }

  @override
  Future<String?> read(String key) async => (await _load())[key];

  @override
  Future<void> write(String key, String? value) async {
    final m = await _load();
    if (value == null) {
      m.remove(key);
    } else {
      m[key] = value;
    }
    try {
      final f = File(path);
      await f.parent.create(recursive: true);
      await f.writeAsString(jsonEncode(m));
    } catch (e) {
      debugPrint('Preferences: cannot write $path: $e');
    }
  }
}

class MemoryPreferences implements Preferences {
  final values = <String, String>{};

  @override
  Future<String?> read(String key) async => values[key];

  @override
  Future<void> write(String key, String? value) async {
    if (value == null) {
      values.remove(key);
    } else {
      values[key] = value;
    }
  }
}
