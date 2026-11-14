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

  /// 'true' blurs overviews and stills of unseen episodes (UI-010, Hide
  /// Spoilers); absent = off.
  static const hideSpoilers = 'library_hide_spoilers';

  /// Recent search queries (UI-011), a JSON list, newest first.
  static const searchHistory = 'search_history';

  /// 'list' shows a library view (`movies`, `shows`) as a list (UI-008).
  static String libraryLayout(String view) => 'library_layout_$view';

  /// 'false' turns off continuous playback: no Up Next after an episode
  /// (UI-016); absent = on.
  static const continuousPlayback = 'playback_continuous';

  /// 'false' stops generating seek previews for videos without them
  /// (PLAY-009); absent = on.
  static const trickplayGenerate = 'playback_trickplay_generate';

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

  /// How the user watches (UI-004), an `AppMode` name; absent = not asked.
  static const appMode = 'app_mode';

  /// The local profile of the first-run setup (UI-004), JSON of
  /// `LocalProfile`; replaced by the core's profiles with LIB-012.
  static const localProfile = 'local_profile';

  /// Subtitle track and delay remembered for a series (PLAY-007), JSON of
  /// pa_player `SubtitleMemory`; [series] is the show id.
  static String subtitleSeries(String series) => 'subtitle_series_$series';
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
