// SPDX-License-Identifier: MPL-2.0
import 'package:media_kit_video/media_kit_video.dart' as mkv;

/// Makes the app's window fullscreen or not, as the video player's F key
/// does (media_kit_video: Win32 on Windows, GTK on Linux, the system UI
/// on phones). Other screens with video (the Twitch embed, TWI-029) use it
/// so they need no window code of their own.
Future<void> setWindowFullscreen(bool on) => on
    ? mkv.defaultEnterNativeFullscreen()
    : mkv.defaultExitNativeFullscreen();
