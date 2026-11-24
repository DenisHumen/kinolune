// SPDX-License-Identifier: MPL-2.0

/// Kinolune player (release 0.1): video [PlayerScreen] and the music player
/// ([MusicPlayerController], [NowPlayingBar], [NowPlayingScreen]) on
/// media_kit / libmpv. Call [ensurePaPlayerInitialized] in `main()`.
library;

export 'src/book/book_player_controller.dart';
export 'src/engine/adaptive_buffer.dart';
export 'src/engine/audio_processing.dart';
export 'src/engine/audio_profile.dart';
export 'src/engine/media_kit_engine.dart'
    show MediaKitEngine, ensurePaPlayerInitialized;
export 'src/engine/parts.dart'
    show partsUri, partsOf, edlScheme, partLengths, partSubtitlesUri;
export 'src/engine/playback_activity.dart';
export 'src/engine/playback_error.dart';
export 'src/engine/playback_stats.dart';
export 'src/engine/player_engine.dart';
export 'src/l10n/pa_player_strings.dart';
export 'src/live/live_controller.dart';
export 'src/live/live_options.dart';
export 'src/music/artwork.dart' show Artwork;
export 'src/music/audio_format.dart';
export 'src/music/music_player_controller.dart';
export 'src/music/music_queue.dart';
export 'src/music/now_playing_bar.dart';
export 'src/music/now_playing_screen.dart';
export 'src/music/queue_panel.dart';
export 'src/play_request.dart';
export 'src/subtitles/languages.dart';
export 'src/subtitles/subtitle_overlay.dart';
export 'src/subtitles/subtitle_policy.dart';
export 'src/subtitles/subtitle_rendering.dart';
export 'src/subtitles/subtitle_search.dart';
export 'src/subtitles/subtitle_style.dart';
export 'src/system/audio_output.dart';
export 'src/system/media_session.dart';
export 'src/system/music_session.dart';
export 'src/system/pip.dart';
export 'src/tracks/track_policy.dart';
export 'src/trickplay.dart';
export 'src/ui/min_tap_target.dart';
export 'src/ui/tokens.dart' show PaTokens, formatDuration;
export 'src/video/osd_controller.dart';
export 'src/video/osd_model.dart';
export 'src/video/screen_brightness.dart';
export 'src/video/player_screen.dart';
export 'src/video/progress_reporter.dart';
export 'src/video/track_panel_strings.dart';
export 'src/video/up_next.dart';
