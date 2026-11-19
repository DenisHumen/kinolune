// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/foundation.dart';

/// Stable codes of playback failures (CORE-009 style, UI-044): the screen
/// picks its human text by the code; mpv's own message is only a detail.
enum PlaybackErrorCode {
  /// The file or stream cannot be opened: missing, no access, the share
  /// or the server is unreachable.
  openFailed('open_failed'),

  /// The file opens but its format is not recognised (damaged or not a
  /// media file).
  unsupportedFormat('unsupported_format'),

  /// The file opens, but no audio or video stream can be decoded.
  nothingToPlay('nothing_to_play');

  const PlaybackErrorCode(this.code);

  /// The stable string code (snake_case, as `pa_runtime::error_codes`).
  final String code;
}

/// A playback failure the player screen shows: playback cannot go on.
@immutable
class PlaybackError {
  const PlaybackError(this.code, [this.detail = '']);

  final PlaybackErrorCode code;

  /// The engine's message (English, technical): for logs and reports,
  /// never the main text on screen.
  final String detail;

  @override
  bool operator ==(Object other) =>
      other is PlaybackError && other.code == code && other.detail == detail;

  @override
  int get hashCode => Object.hash(code, detail);

  @override
  String toString() => 'PlaybackError(${code.code}: $detail)';
}

/// Tells fatal mpv log messages from harmless ones for one open file
/// (BUG-003).
///
/// mpv logs at `error` level many things that do not stop playback: the
/// hardware decoder probe without a GPU ("Could not open codec", then a
/// fallback to software), a broken frame, an external subtitle that does
/// not open. Only these count as a failure:
/// * any `fatal` message (e.g. "No video or audio streams selected");
/// * before the file has loaded: the file, stream or demuxer cannot be
///   opened or recognised;
/// * every stream of the file failed to get a decoder.
class PlaybackErrorTracker {
  final Set<String> _failedDecoders = {};

  /// A new file is being opened: nothing of the previous one counts.
  void reset() => _failedDecoders.clear();

  /// Classifies one mpv log message. [loaded] is true once the file has
  /// reported its duration; [hasVideo] and [hasAudio] tell which streams
  /// the file has (to know when every decoder has failed).
  PlaybackError? onLog({
    required String prefix,
    required String level,
    required String text,
    bool loaded = false,
    bool hasVideo = false,
    bool hasAudio = false,
  }) {
    final t = text.trim();
    if (level != 'error' && level != 'fatal') return null;
    if (t.contains('No video or audio streams')) {
      return PlaybackError(PlaybackErrorCode.nothingToPlay, t);
    }
    if (level == 'fatal') {
      return PlaybackError(PlaybackErrorCode.openFailed, t);
    }
    if ((prefix == 'vd' || prefix == 'ad') &&
        t.startsWith('Failed to initialize a decoder')) {
      _failedDecoders.add(prefix);
      final videoGone = !hasVideo || _failedDecoders.contains('vd');
      final audioGone = !hasAudio || _failedDecoders.contains('ad');
      if ((hasVideo || hasAudio) && videoGone && audioGone) {
        return PlaybackError(PlaybackErrorCode.nothingToPlay, t);
      }
      return null;
    }
    // Once the file plays, an open error is about something else (an
    // external subtitle, a prefetched next item).
    if (loaded) return null;
    if (t.startsWith('Failed to recognize file format')) {
      return PlaybackError(PlaybackErrorCode.unsupportedFormat, t);
    }
    switch (prefix) {
      case 'file':
      case 'stream':
        return PlaybackError(PlaybackErrorCode.openFailed, t);
      case 'cplayer':
        if (t.startsWith('Failed to open') ||
            t.startsWith('Cannot open') ||
            t.startsWith('Error opening')) {
          return PlaybackError(PlaybackErrorCode.openFailed, t);
        }
    }
    return null;
  }
}
