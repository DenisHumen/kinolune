// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../l10n/pa_player_strings.dart';
import '../ui/focus_button.dart';
import '../ui/scrubber.dart';
import '../ui/tokens.dart';
import 'artwork.dart';
import 'music_player_controller.dart';
import 'now_playing_screen.dart';

/// How the mini player looks (MUS-005, research 01 §4.11).
enum MiniPlayerStyle {
  /// Phone: a capsule floating above the tab bar.
  capsule,

  /// Desktop and tablet: a bar along the bottom with the transport, the
  /// seek bar and its times.
  bar,

  /// TV: a compact plate; the remote focuses it, OK opens Now Playing.
  tv,
}

/// The mini player at the bottom of the app (MUS-005). Hidden while the
/// queue is empty. Tapping it (OK on a remote) opens [NowPlayingScreen]
/// (or [onOpen]).
class NowPlayingBar extends StatelessWidget {
  const NowPlayingBar({
    super.key,
    required this.controller,
    this.onOpen,
    this.style = MiniPlayerStyle.capsule,
  });

  final MusicPlayerController controller;
  final VoidCallback? onOpen;
  final MiniPlayerStyle style;

  void _open(BuildContext context) {
    if (onOpen != null) {
      onOpen!();
      return;
    }
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        fullscreenDialog: true,
        builder: (_) => NowPlayingScreen(controller: controller),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: controller,
      builder: (context, _) {
        if (controller.current == null) return const SizedBox.shrink();
        return Semantics(
          container: true,
          label: PaPlayerStrings.of(context).miniPlayer,
          child: switch (style) {
            MiniPlayerStyle.capsule => _Capsule(
              controller: controller,
              onOpen: () => _open(context),
            ),
            MiniPlayerStyle.bar => _DesktopBar(
              controller: controller,
              onOpen: () => _open(context),
            ),
            MiniPlayerStyle.tv => _TvPlate(
              controller: controller,
              onOpen: () => _open(context),
            ),
          },
        );
      },
    );
  }
}

double _fraction(MusicPlayerController c) {
  final d = c.duration.inMilliseconds;
  return d <= 0 ? 0.0 : (c.position.inMilliseconds / d).clamp(0.0, 1.0);
}

class _Titles extends StatelessWidget {
  const _Titles({required this.controller, this.titleSize = 15});
  final MusicPlayerController controller;
  final double titleSize;

  @override
  Widget build(BuildContext context) {
    final t = controller.current!;
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          t.title,
          key: const ValueKey('bar-title'),
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(
            color: PaTokens.text,
            fontSize: titleSize,
            fontWeight: FontWeight.w600,
          ),
        ),
        if (t.artist != null)
          Text(
            t.artist!,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: const TextStyle(color: PaTokens.textSecondary, fontSize: 13),
          ),
      ],
    );
  }
}

/// Play/pause of the mini player.
class _PlayButton extends StatelessWidget {
  const _PlayButton({required this.controller, this.size = 44});
  final MusicPlayerController controller;
  final double size;

  @override
  Widget build(BuildContext context) {
    final l = PaPlayerStrings.of(context);
    return FocusIconButton(
      key: const ValueKey('bar-play'),
      icon: controller.playing ? Icons.pause_rounded : Icons.play_arrow_rounded,
      tooltip: controller.playing ? l.pause : l.play,
      size: size,
      onPressed: controller.togglePlay,
    );
  }
}

class _NextButton extends StatelessWidget {
  const _NextButton({required this.controller, this.size = 44});
  final MusicPlayerController controller;
  final double size;

  @override
  Widget build(BuildContext context) => FocusIconButton(
    key: const ValueKey('bar-next'),
    icon: Icons.skip_next_rounded,
    tooltip: PaPlayerStrings.of(context).next,
    size: size,
    onPressed: controller.queue.nextPosition() == null ? null : controller.next,
  );
}

/// Phone: a capsule above the tab bar with the progress along its bottom.
class _Capsule extends StatelessWidget {
  const _Capsule({required this.controller, required this.onOpen});
  final MusicPlayerController controller;
  final VoidCallback onOpen;

  @override
  Widget build(BuildContext context) {
    return ClipRRect(
      key: const ValueKey('mini-player-capsule'),
      borderRadius: BorderRadius.circular(32),
      child: Material(
        color: PaTokens.surface,
        child: InkWell(
          key: const ValueKey('now-playing-bar'),
          onTap: onOpen,
          child: SizedBox(
            height: 64,
            child: Stack(
              children: [
                Padding(
                  padding: const EdgeInsets.fromLTRB(8, 0, 6, 0),
                  child: Row(
                    children: [
                      Artwork(
                        request: controller.current,
                        size: 48,
                        radius: 24,
                      ),
                      const SizedBox(width: 12),
                      Expanded(child: _Titles(controller: controller)),
                      _PlayButton(controller: controller),
                      _NextButton(controller: controller),
                    ],
                  ),
                ),
                Positioned(
                  left: 24,
                  right: 24,
                  bottom: 0,
                  child: LinearProgressIndicator(
                    value: _fraction(controller),
                    minHeight: 2,
                    backgroundColor: PaTokens.track,
                    color: PaTokens.text,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// Desktop: artwork and titles, the transport over a seek bar with its
/// times, and the format of the stream.
class _DesktopBar extends StatelessWidget {
  const _DesktopBar({required this.controller, required this.onOpen});
  final MusicPlayerController controller;
  final VoidCallback onOpen;

  @override
  Widget build(BuildContext context) {
    final l = PaPlayerStrings.of(context);
    final format = controller.formatLine;
    return ClipRRect(
      key: const ValueKey('mini-player-bar'),
      borderRadius: BorderRadius.circular(14),
      child: Material(
        color: PaTokens.surface,
        child: InkWell(
          key: const ValueKey('now-playing-bar'),
          onTap: onOpen,
          child: SizedBox(
            height: 76,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12),
              child: LayoutBuilder(
                builder: (context, c) {
                  final roomy = c.maxWidth >= 720;
                  return Row(
                    children: [
                      Artwork(request: controller.current, size: 52, radius: 6),
                      const SizedBox(width: 12),
                      if (roomy)
                        SizedBox(
                          width: 220,
                          child: _Titles(controller: controller),
                        )
                      else
                        Expanded(child: _Titles(controller: controller)),
                      if (roomy) ...[
                        const SizedBox(width: 16),
                        Expanded(
                          child: Column(
                            mainAxisAlignment: MainAxisAlignment.center,
                            children: [
                              Row(
                                mainAxisAlignment: MainAxisAlignment.center,
                                children: [
                                  FocusIconButton(
                                    key: const ValueKey('bar-prev'),
                                    icon: Icons.skip_previous_rounded,
                                    tooltip: l.previous,
                                    size: 40,
                                    onPressed: controller.previous,
                                  ),
                                  _PlayButton(controller: controller),
                                  _NextButton(
                                    controller: controller,
                                    size: 40,
                                  ),
                                ],
                              ),
                              SizedBox(
                                height: 22,
                                child: Scrubber(
                                  key: const ValueKey('bar-seek'),
                                  position: controller.position,
                                  duration: controller.duration,
                                  onSeek: controller.seek,
                                  height: 4,
                                ),
                              ),
                            ],
                          ),
                        ),
                        const SizedBox(width: 16),
                        SizedBox(
                          width: 160,
                          child: format == null
                              ? null
                              : Text(
                                  format,
                                  key: const ValueKey('bar-format'),
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  textAlign: TextAlign.end,
                                  style: TextStyle(
                                    color: controller.bitPerfect
                                        ? PaTokens.accent
                                        : PaTokens.textSecondary,
                                    fontSize: 12,
                                    fontWeight: FontWeight.w600,
                                  ),
                                ),
                        ),
                      ] else ...[
                        _PlayButton(controller: controller),
                        _NextButton(controller: controller),
                      ],
                      FocusIconButton(
                        key: const ValueKey('bar-open'),
                        icon: Icons.keyboard_arrow_up_rounded,
                        tooltip: l.openNowPlaying,
                        size: 40,
                        onPressed: onOpen,
                      ),
                    ],
                  );
                },
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// TV: a compact plate. The remote focuses the plate itself (white ring,
/// slight zoom, tvOS style); OK opens Now Playing, play/pause on the
/// remote plays and pauses.
class _TvPlate extends StatefulWidget {
  const _TvPlate({required this.controller, required this.onOpen});
  final MusicPlayerController controller;
  final VoidCallback onOpen;

  @override
  State<_TvPlate> createState() => _TvPlateState();
}

class _TvPlateState extends State<_TvPlate> {
  bool _focused = false;

  KeyEventResult _onKey(FocusNode _, KeyEvent e) {
    if (e is! KeyDownEvent) return KeyEventResult.ignored;
    final k = e.logicalKey;
    if (k == LogicalKeyboardKey.mediaPlayPause ||
        k == LogicalKeyboardKey.mediaPlay ||
        k == LogicalKeyboardKey.mediaPause) {
      widget.controller.togglePlay();
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.mediaTrackNext) {
      widget.controller.next();
      return KeyEventResult.handled;
    }
    // OK on the remote (DPAD_CENTER), whatever shortcuts the app has.
    if (k == LogicalKeyboardKey.select ||
        k == LogicalKeyboardKey.enter ||
        k == LogicalKeyboardKey.numpadEnter ||
        k == LogicalKeyboardKey.gameButtonA) {
      widget.onOpen();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  @override
  Widget build(BuildContext context) {
    final c = widget.controller;
    final l = PaPlayerStrings.of(context);
    return Focus(
      onKeyEvent: _onKey,
      skipTraversal: true,
      child: FocusableActionDetector(
        key: const ValueKey('now-playing-bar'),
        onFocusChange: (f) => setState(() => _focused = f),
        mouseCursor: SystemMouseCursors.click,
        actions: {
          ActivateIntent: CallbackAction<ActivateIntent>(
            onInvoke: (_) {
              widget.onOpen();
              return null;
            },
          ),
        },
        child: Semantics(
          button: true,
          label: l.openNowPlaying,
          child: GestureDetector(
            onTap: widget.onOpen,
            child: AnimatedScale(
              scale: _focused ? 1.06 : 1.0,
              duration: const Duration(milliseconds: 150),
              child: AnimatedContainer(
                key: const ValueKey('mini-player-tv'),
                duration: const Duration(milliseconds: 150),
                height: 72,
                padding: const EdgeInsets.fromLTRB(10, 10, 16, 10),
                decoration: BoxDecoration(
                  color: _focused ? PaTokens.focusRing : PaTokens.glassStrong,
                  borderRadius: BorderRadius.circular(PaTokens.radius),
                ),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Artwork(request: c.current, size: 52, radius: 8),
                    const SizedBox(width: 12),
                    Flexible(
                      child: _TvTitles(controller: c, focused: _focused),
                    ),
                    const SizedBox(width: 12),
                    Icon(
                      c.playing
                          ? Icons.equalizer_rounded
                          : Icons.pause_rounded,
                      key: const ValueKey('tv-state'),
                      color: _focused ? PaTokens.bg : PaTokens.textSecondary,
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _TvTitles extends StatelessWidget {
  const _TvTitles({required this.controller, required this.focused});
  final MusicPlayerController controller;
  final bool focused;

  @override
  Widget build(BuildContext context) {
    final t = controller.current!;
    final fg = focused ? PaTokens.bg : PaTokens.text;
    final fg2 = focused ? PaTokens.bg : PaTokens.textSecondary;
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          t.title,
          key: const ValueKey('bar-title'),
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: TextStyle(
            color: fg,
            fontSize: 16,
            fontWeight: FontWeight.w600,
          ),
        ),
        if (t.artist != null)
          Text(
            t.artist!,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(color: fg2, fontSize: 13),
          ),
        const SizedBox(height: 6),
        SizedBox(
          width: 160,
          child: LinearProgressIndicator(
            value: _fraction(controller),
            minHeight: 2,
            backgroundColor: focused ? const Color(0x33000000) : PaTokens.track,
            color: fg,
          ),
        ),
      ],
    );
  }
}
