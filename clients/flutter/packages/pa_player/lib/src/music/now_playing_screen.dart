// SPDX-License-Identifier: MPL-2.0
import 'dart:ui' show ImageFilter;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../engine/player_engine.dart';
import '../l10n/pa_player_strings.dart';
import '../system/audio_output.dart';
import '../ui/focus_button.dart';
import '../ui/scrubber.dart';
import '../ui/tokens.dart';
import 'artwork.dart';
import 'lyrics.dart';
import 'lyrics_view.dart';
import 'music_player_controller.dart';
import 'queue_panel.dart';

/// Fullscreen "Now Playing": artwork over its blurred copy, title/artist,
/// the stream's format line («FLAC · 24/96 · bit-perfect», MUS-005),
/// scrubber with times, transport (shuffle, previous, play/pause, next,
/// repeat), the system output picker where the platform has one
/// ([SystemAudioOutput]) and the queue with its history
/// ([NowPlayingQueue], MUS-003).
/// Wide layouts (desktop/TV) put the artwork left and the queue right; on a
/// phone the queue follows below and the queue button scrolls to it.
///
/// With [lyrics] the top bar gets a lyrics button (also the L key, MUS-006):
/// the lyrics take the queue's place on a wide screen and the artwork's on
/// a phone, in time with the music, and open on the whole screen
/// ([LyricsFullscreen]).
class NowPlayingScreen extends StatefulWidget {
  const NowPlayingScreen({
    super.key,
    required this.controller,
    this.onSaveQueue,
    this.audioOutput,
    this.lyrics,
    this.showLyrics = false,
  });

  final MusicPlayerController controller;

  /// "Save as playlist" of the queue; hidden when null.
  final SaveQueueCallback? onSaveQueue;

  /// The output picker; default [SystemAudioOutput.instance]. The button
  /// is hidden where it is not [SystemAudioOutput.supported].
  final SystemAudioOutput? audioOutput;

  /// Looks up the lyrics of a track (MUS-006); no lyrics button when null.
  final LyricsLoader? lyrics;

  /// Opens with the lyrics shown.
  final bool showLyrics;

  @override
  State<NowPlayingScreen> createState() => _NowPlayingScreenState();
}

class _NowPlayingScreenState extends State<NowPlayingScreen> {
  LyricsSession? _session;
  late bool _showLyrics = widget.showLyrics && widget.lyrics != null;

  MusicPlayerController get controller => widget.controller;

  @override
  void initState() {
    super.initState();
    final loader = widget.lyrics;
    if (loader != null) _session = LyricsSession(loader);
    controller.addListener(_onTrack);
    _onTrack();
  }

  @override
  void dispose() {
    controller.removeListener(_onTrack);
    _session?.dispose();
    super.dispose();
  }

  /// Looks up the lyrics of the current track while they are shown.
  void _onTrack() {
    if (_showLyrics) _session?.show(controller.current);
  }

  void _toggleLyrics() {
    if (_session == null) return;
    setState(() => _showLyrics = !_showLyrics);
    _onTrack();
  }

  void _openFullscreen() {
    final session = _session;
    if (session == null) return;
    LyricsFullscreen.open(context, controller: controller, session: session);
  }

  KeyEventResult _onKey(FocusNode _, KeyEvent e) {
    if (e is! KeyDownEvent) return KeyEventResult.ignored;
    final k = e.logicalKey;
    if (k == LogicalKeyboardKey.space ||
        k == LogicalKeyboardKey.mediaPlayPause) {
      controller.togglePlay();
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.mediaTrackNext) {
      controller.next();
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.mediaTrackPrevious) {
      controller.previous();
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.keyL && _session != null) {
      _toggleLyrics();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  SystemAudioOutput? get _output {
    final o = widget.audioOutput ?? SystemAudioOutput.instance;
    return o.supported ? o : null;
  }

  @override
  Widget build(BuildContext context) {
    final session = _session;
    final lyrics = session == null || !_showLyrics
        ? null
        : NowPlayingLyrics(
            key: const ValueKey('np-lyrics-panel'),
            controller: controller,
            session: session,
            onFullscreen: _openFullscreen,
          );
    final toggle = _LyricsToggle(
      available: session != null,
      shown: _showLyrics,
      onToggle: _toggleLyrics,
    );
    return Theme(
      data: ThemeData.dark(useMaterial3: true),
      child: Focus(
        onKeyEvent: _onKey,
        child: Scaffold(
          backgroundColor: PaTokens.bg,
          body: ListenableBuilder(
            listenable: controller,
            builder: (context, _) {
              final t = controller.current;
              return Stack(
                fit: StackFit.expand,
                children: [
                  // Blurred artwork backdrop.
                  if (t != null)
                    ImageFiltered(
                      imageFilter: ImageFilter.blur(sigmaX: 60, sigmaY: 60),
                      child: Opacity(
                        opacity: 0.45,
                        child: Artwork(request: t, radius: 0),
                      ),
                    ),
                  const DecoratedBox(
                    decoration: BoxDecoration(
                      gradient: LinearGradient(
                        begin: Alignment.topCenter,
                        end: Alignment.bottomCenter,
                        colors: [Color(0x66000000), Color(0xEE0B0B0F)],
                      ),
                    ),
                  ),
                  SafeArea(
                    child: LayoutBuilder(
                      builder: (context, c) => c.maxWidth >= 900
                          ? _Wide(
                              controller: controller,
                              onSave: widget.onSaveQueue,
                              output: _output,
                              lyricsToggle: toggle,
                              lyrics: lyrics,
                            )
                          : _Narrow(
                              controller: controller,
                              onSave: widget.onSaveQueue,
                              output: _output,
                              lyricsToggle: toggle,
                              lyrics: lyrics,
                            ),
                    ),
                  ),
                ],
              );
            },
          ),
        ),
      ),
    );
  }
}

/// The lyrics button of the top bar (MUS-006).
class _LyricsToggle {
  const _LyricsToggle({
    required this.available,
    required this.shown,
    required this.onToggle,
  });
  final bool available;
  final bool shown;
  final VoidCallback onToggle;
}

class _Wide extends StatelessWidget {
  const _Wide({
    required this.controller,
    required this.lyricsToggle,
    this.onSave,
    this.output,
    this.lyrics,
  });
  final MusicPlayerController controller;
  final SaveQueueCallback? onSave;
  final SystemAudioOutput? output;
  final _LyricsToggle lyricsToggle;

  /// Shown in place of the queue.
  final Widget? lyrics;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(64, 24, 64, 40),
      child: Column(
        children: [
          _TopBar(output: output, lyrics: lyricsToggle),
          Expanded(
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.center,
              children: [
                Expanded(
                  flex: 5,
                  child: Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    children: [
                      Flexible(child: _PulsingArtwork(controller: controller)),
                      const SizedBox(height: 32),
                      _Info(controller: controller),
                      const SizedBox(height: 16),
                      _Transport(controller: controller),
                    ],
                  ),
                ),
                const SizedBox(width: 64),
                Expanded(
                  flex: 4,
                  child:
                      lyrics ??
                      NowPlayingQueue(controller: controller, onSave: onSave),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _Narrow extends StatefulWidget {
  const _Narrow({
    required this.controller,
    required this.lyricsToggle,
    this.onSave,
    this.output,
    this.lyrics,
  });
  final MusicPlayerController controller;
  final SaveQueueCallback? onSave;
  final SystemAudioOutput? output;
  final _LyricsToggle lyricsToggle;

  /// Shown in place of the artwork.
  final Widget? lyrics;

  @override
  State<_Narrow> createState() => _NarrowState();
}

class _NarrowState extends State<_Narrow> {
  final _queueKey = GlobalKey();

  void _showQueue() {
    final c = _queueKey.currentContext;
    if (c == null) return;
    Scrollable.ensureVisible(
      c,
      duration: const Duration(milliseconds: 350),
      curve: Curves.easeOutCubic,
    );
  }

  @override
  Widget build(BuildContext context) {
    final controller = widget.controller;
    final onSave = widget.onSave;
    return CustomScrollView(
      slivers: [
        SliverToBoxAdapter(
          child: _TopBar(
            output: widget.output,
            onQueue: _showQueue,
            lyrics: widget.lyricsToggle,
          ),
        ),
        SliverPadding(
          padding: const EdgeInsets.symmetric(horizontal: 24),
          sliver: SliverList.list(
            children: [
              const SizedBox(height: 8),
              if (widget.lyrics case final lyrics?)
                SizedBox(height: 420, child: lyrics)
              else
                Center(
                  child: ConstrainedBox(
                    constraints: const BoxConstraints(maxWidth: 420),
                    child: _PulsingArtwork(controller: controller),
                  ),
                ),
              const SizedBox(height: 28),
              _Info(controller: controller),
              const SizedBox(height: 12),
              _Transport(controller: controller),
              const SizedBox(height: 28),
              SizedBox(
                key: _queueKey,
                height: 480,
                child: NowPlayingQueue(controller: controller, onSave: onSave),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class _TopBar extends StatelessWidget {
  const _TopBar({this.output, this.onQueue, this.lyrics});

  /// The lyrics button; none when null or unavailable.
  final _LyricsToggle? lyrics;

  /// The output picker; no button when null.
  final SystemAudioOutput? output;

  /// Scrolls to the queue (phone); no button when null.
  final VoidCallback? onQueue;

  @override
  Widget build(BuildContext context) {
    final l = PaPlayerStrings.of(context);
    final o = output;
    final ly = lyrics;
    final trailing = <Widget>[
      if (ly != null && ly.available)
        FocusIconButton(
          key: const ValueKey('np-lyrics'),
          icon: ly.shown ? Icons.lyrics_rounded : Icons.lyrics_outlined,
          tooltip: l.lyrics,
          active: ly.shown,
          onPressed: ly.onToggle,
        ),
      if (onQueue != null)
        FocusIconButton(
          key: const ValueKey('np-show-queue'),
          icon: Icons.queue_music_rounded,
          tooltip: l.showQueue,
          onPressed: onQueue,
        ),
      if (o != null)
        FocusIconButton(
          key: const ValueKey('np-output'),
          icon: Icons.speaker_group_outlined,
          tooltip: l.audioOutput,
          onPressed: o.showPicker,
        ),
    ];
    // The title stays centred: as wide on the left as the buttons right.
    final side = 48.0 * (trailing.isEmpty ? 1 : trailing.length);
    return Padding(
      padding: const EdgeInsets.all(8),
      child: Row(
        children: [
          SizedBox(
            width: side,
            child: Align(
              alignment: AlignmentDirectional.centerStart,
              child: FocusIconButton(
                icon: Icons.keyboard_arrow_down_rounded,
                tooltip: l.close,
                onPressed: () => Navigator.of(context).maybePop(),
              ),
            ),
          ),
          Expanded(
            child: Text(
              l.nowPlaying,
              textAlign: TextAlign.center,
              style: const TextStyle(
                color: PaTokens.textSecondary,
                fontSize: 13,
                letterSpacing: 1.2,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
          SizedBox(
            width: side,
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: trailing,
            ),
          ),
        ],
      ),
    );
  }
}

/// Artwork that shrinks slightly while paused (Apple Music style).
class _PulsingArtwork extends StatelessWidget {
  const _PulsingArtwork({required this.controller});
  final MusicPlayerController controller;

  @override
  Widget build(BuildContext context) {
    return AnimatedScale(
      scale: controller.playing ? 1.0 : 0.86,
      duration: const Duration(milliseconds: 350),
      curve: Curves.easeOutBack,
      child: Container(
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(12),
          boxShadow: const [
            BoxShadow(
              color: Color(0x80000000),
              blurRadius: 40,
              offset: Offset(0, 16),
            ),
          ],
        ),
        child: Artwork(request: controller.current, radius: 12),
      ),
    );
  }
}

class _Info extends StatelessWidget {
  const _Info({required this.controller});
  final MusicPlayerController controller;

  @override
  Widget build(BuildContext context) {
    final t = controller.current;
    final sub = [t?.artist, t?.album].whereType<String>().join(' — ');
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          t?.title ?? PaPlayerStrings.of(context).nothingPlaying,
          key: const ValueKey('np-title'),
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: const TextStyle(
            color: PaTokens.text,
            fontSize: 24,
            fontWeight: FontWeight.w700,
          ),
        ),
        if (sub.isNotEmpty)
          Text(
            sub,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: const TextStyle(color: PaTokens.textSecondary, fontSize: 16),
          ),
        if (controller.formatLine case final format?) ...[
          const SizedBox(height: 10),
          Align(
            alignment: AlignmentDirectional.centerStart,
            child: _FormatLine(format, bitPerfect: controller.bitPerfect),
          ),
        ],
        const SizedBox(height: 16),
        Scrubber(
          position: controller.position,
          duration: controller.duration,
          onSeek: controller.seek,
        ),
      ],
    );
  }
}

class _Transport extends StatelessWidget {
  const _Transport({required this.controller});
  final MusicPlayerController controller;

  @override
  Widget build(BuildContext context) {
    final l = PaPlayerStrings.of(context);
    final repeatIcon = controller.repeat == QueueRepeat.one
        ? Icons.repeat_one_rounded
        : Icons.repeat_rounded;
    return FocusTraversalGroup(
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          FocusIconButton(
            key: const ValueKey('np-shuffle'),
            icon: Icons.shuffle_rounded,
            tooltip: controller.shuffle ? l.shuffleOn : l.shuffleOff,
            active: controller.shuffle,
            onPressed: controller.toggleShuffle,
          ),
          FocusIconButton(
            key: const ValueKey('np-prev'),
            icon: Icons.skip_previous_rounded,
            tooltip: l.previous,
            size: 56,
            onPressed: controller.previous,
          ),
          FocusIconButton(
            key: const ValueKey('np-play'),
            icon: controller.playing
                ? Icons.pause_rounded
                : Icons.play_arrow_rounded,
            tooltip: controller.playing ? l.pause : l.play,
            size: 72,
            filled: true,
            autofocus: true,
            onPressed: controller.togglePlay,
          ),
          FocusIconButton(
            key: const ValueKey('np-next'),
            icon: Icons.skip_next_rounded,
            tooltip: l.next,
            size: 56,
            onPressed: controller.queue.nextPosition() == null
                ? null
                : controller.next,
          ),
          FocusIconButton(
            key: const ValueKey('np-repeat'),
            icon: repeatIcon,
            tooltip: switch (controller.repeat) {
              QueueRepeat.off => l.repeatOff,
              QueueRepeat.all => l.repeatAll,
              QueueRepeat.one => l.repeatOne,
            },
            active: controller.repeat != QueueRepeat.off,
            onPressed: controller.cycleRepeat,
          ),
        ],
      ),
    );
  }
}

/// «FLAC · 24/96 · bit-perfect» in a quiet pill; bit-perfect is tinted.
class _FormatLine extends StatelessWidget {
  const _FormatLine(this.text, {required this.bitPerfect});
  final String text;
  final bool bitPerfect;

  @override
  Widget build(BuildContext context) {
    final color = bitPerfect ? PaTokens.accent : PaTokens.textSecondary;
    return DecoratedBox(
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(6),
        border: Border.all(color: color.withValues(alpha: 0.6)),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
        child: Text(
          text,
          key: const ValueKey('np-format'),
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(
            color: color,
            fontSize: 12,
            fontWeight: FontWeight.w600,
            letterSpacing: 0.6,
            fontFeatures: const [FontFeature.tabularFigures()],
          ),
        ),
      ),
    );
  }
}
