// SPDX-License-Identifier: MPL-2.0
import 'dart:ui' show ImageFilter;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

import '../l10n/pa_player_strings.dart';
import '../play_request.dart';
import '../ui/focus_button.dart';
import '../ui/tokens.dart';
import 'artwork.dart';
import 'lyrics.dart';
import 'music_player_controller.dart';

/// Where the lyrics of the current track are (MUS-006).
enum LyricsStatus { idle, loading, found, notFound }

/// The lyrics of the track Now Playing shows: looked up once per track
/// with the app's [LyricsLoader]; a lookup that finishes after the track
/// changed is dropped.
class LyricsSession extends ChangeNotifier {
  LyricsSession(this._loader);

  final LyricsLoader _loader;
  PlayRequest? _track;
  int _generation = 0;
  bool _disposed = false;

  LyricsStatus status = LyricsStatus.idle;
  Lyrics? lyrics;

  static bool _same(PlayRequest a, PlayRequest b) =>
      identical(a, b) || (a.uri == b.uri && a.itemId == b.itemId);

  /// Looks up the lyrics of [track] unless they are already there.
  void show(PlayRequest? track) {
    if (track == null) {
      _track = null;
      _generation++;
      _set(LyricsStatus.idle, null);
      return;
    }
    final last = _track;
    if (last != null && _same(last, track)) return;
    _track = track;
    final generation = ++_generation;
    _set(LyricsStatus.loading, null);
    _loader(track).then(
      (found) {
        if (generation != _generation) return;
        _set(
          found == null ? LyricsStatus.notFound : LyricsStatus.found,
          found,
        );
      },
      onError: (Object e) {
        debugPrint('Lyrics of ${track.title} not loaded: $e');
        if (generation == _generation) _set(LyricsStatus.notFound, null);
      },
    );
  }

  void _set(LyricsStatus s, Lyrics? l) {
    if (_disposed) return;
    status = s;
    lyrics = l;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}

/// The lyrics panel of Now Playing: «Ищем текст…», «Текст не найден», or
/// the lyrics with a button that opens them on the whole screen.
class NowPlayingLyrics extends StatelessWidget {
  const NowPlayingLyrics({
    super.key,
    required this.controller,
    required this.session,
    this.onFullscreen,
  });

  final MusicPlayerController controller;
  final LyricsSession session;

  /// Opens [LyricsFullscreen]; no button when null.
  final VoidCallback? onFullscreen;

  @override
  Widget build(BuildContext context) {
    final l = PaPlayerStrings.of(context);
    return ListenableBuilder(
      listenable: session,
      builder: (context, _) {
        final lyrics = session.lyrics;
        final body = switch (session.status) {
          LyricsStatus.found when lyrics != null => LyricsView(
            key: ValueKey(lyrics),
            controller: controller,
            lyrics: lyrics,
          ),
          LyricsStatus.notFound => _Message(
            key: const ValueKey('np-lyrics-not-found'),
            icon: Icons.lyrics_outlined,
            text: l.lyricsNotFound,
          ),
          LyricsStatus.idle => const SizedBox.shrink(),
          _ => _Message(
            key: const ValueKey('np-lyrics-loading'),
            text: l.lyricsLoading,
            busy: true,
          ),
        };
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                Expanded(
                  child: Text(
                    l.lyrics,
                    style: const TextStyle(
                      color: PaTokens.text,
                      fontSize: 18,
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                ),
                if (lyrics?.source == LyricsSource.lrclib)
                  Padding(
                    padding: const EdgeInsets.only(right: 8),
                    child: Text(
                      l.lyricsFromLrclib,
                      style: const TextStyle(
                        color: PaTokens.textSecondary,
                        fontSize: 12,
                      ),
                    ),
                  ),
                if (onFullscreen != null && lyrics != null)
                  FocusIconButton(
                    key: const ValueKey('np-lyrics-fullscreen'),
                    icon: Icons.open_in_full_rounded,
                    tooltip: l.lyricsFullscreen,
                    size: 40,
                    onPressed: onFullscreen,
                  ),
              ],
            ),
            const SizedBox(height: 8),
            Expanded(child: body),
          ],
        );
      },
    );
  }
}

class _Message extends StatelessWidget {
  const _Message({super.key, required this.text, this.icon, this.busy = false});
  final String text;
  final IconData? icon;
  final bool busy;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (busy)
            const SizedBox.square(
              dimension: 28,
              child: CircularProgressIndicator(strokeWidth: 2.5),
            )
          else if (icon != null)
            Icon(icon, color: PaTokens.textSecondary, size: 40),
          const SizedBox(height: 12),
          Text(
            text,
            textAlign: TextAlign.center,
            style: const TextStyle(color: PaTokens.textSecondary, fontSize: 16),
          ),
        ],
      ),
    );
  }
}

/// Lyrics in time with the music (MUS-006): the line being sung is
/// highlighted and kept in view; a tap (or Enter on a focused line, with
/// the remote) seeks to that line. Scrolling by hand or moving the focus
/// pauses the following for a few seconds. Plain lyrics just scroll.
///
/// The playhead is taken from [controller] and carried forward with the
/// frame clock between its updates, so the highlight changes within a
/// frame of the line's time rather than at the engine's next report.
class LyricsView extends StatefulWidget {
  const LyricsView({
    super.key,
    required this.controller,
    required this.lyrics,
    this.large = false,
  });

  final MusicPlayerController controller;
  final Lyrics lyrics;

  /// Bigger text, for [LyricsFullscreen].
  final bool large;

  /// How long a manual scroll or focus move holds the auto-scroll.
  static const holdAfterUser = Duration(seconds: 4);

  @override
  State<LyricsView> createState() => _LyricsViewState();
}

class _LyricsViewState extends State<LyricsView>
    with SingleTickerProviderStateMixin {
  late final Ticker _ticker = createTicker(_tick);
  final _scroll = ScrollController();
  late List<GlobalKey> _keys = _newKeys();

  /// Frame clock of the ticker, and the playhead at its last report.
  Duration _clock = Duration.zero;
  Duration _reported = Duration.zero;
  Duration _reportedAt = Duration.zero;
  final _wall = Stopwatch()..start();

  int _current = -1;
  Duration? _userAt;

  List<GlobalKey> _newKeys() => [
    for (var i = 0; i < widget.lyrics.lines.length; i++) GlobalKey(),
  ];

  MusicPlayerController get _c => widget.controller;

  /// Wall time while the ticker is stopped, frame time while it runs.
  Duration get _now => _ticker.isActive ? _clock : _wall.elapsed;

  @override
  void initState() {
    super.initState();
    _c.addListener(_onController);
    _reported = _c.position;
    _current = lyricLineAt(widget.lyrics.lines, _reported);
    _onController();
    if (_current >= 0) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _follow(jump: true));
    }
  }

  @override
  void didUpdateWidget(LyricsView old) {
    super.didUpdateWidget(old);
    if (old.controller != widget.controller) {
      old.controller.removeListener(_onController);
      widget.controller.addListener(_onController);
    }
    if (old.lyrics != widget.lyrics) {
      _keys = _newKeys();
      _current = -1;
      _update();
    }
  }

  @override
  void dispose() {
    _c.removeListener(_onController);
    _ticker.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _onController() {
    _reported = _c.position;
    _reportedAt = _now;
    final run = _c.playing && widget.lyrics.synced;
    if (run && !_ticker.isActive) {
      _clock = Duration.zero;
      _reportedAt = Duration.zero;
      _ticker.start();
    } else if (!run && _ticker.isActive) {
      _ticker.stop();
      _reportedAt = _wall.elapsed;
    }
    _update();
  }

  void _tick(Duration elapsed) {
    _clock = elapsed;
    _update();
  }

  /// The playhead now: the last report plus the time since, while playing
  /// (at most a second, in case the engine stalls without saying so).
  Duration get _position {
    if (!_c.playing) return _reported;
    var since = _now - _reportedAt;
    if (since < Duration.zero) since = Duration.zero;
    if (since > const Duration(seconds: 1)) since = const Duration(seconds: 1);
    return _reported + since;
  }

  void _update() {
    if (!mounted || !widget.lyrics.synced) return;
    final at = lyricLineAt(widget.lyrics.lines, _position);
    if (at == _current) return;
    setState(() => _current = at);
    WidgetsBinding.instance.addPostFrameCallback((_) => _follow());
  }

  bool get _held {
    final at = _userAt;
    return at != null && _wall.elapsed - at < LyricsView.holdAfterUser;
  }

  /// Scrolls the current line to about a third from the top of this view
  /// (only this view: the page around it stays where it is).
  void _follow({bool jump = false}) {
    if (!mounted || _held || !_scroll.hasClients) return;
    final i = _current < 0 ? 0 : _current;
    if (i >= _keys.length) return;
    final ro = _keys[i].currentContext?.findRenderObject();
    if (ro == null || !ro.attached) return;
    final viewport = RenderAbstractViewport.maybeOf(ro);
    if (viewport == null) return;
    final pos = _scroll.position;
    final target = viewport
        .getOffsetToReveal(ro, 0.35)
        .offset
        .clamp(pos.minScrollExtent, pos.maxScrollExtent);
    if ((target - pos.pixels).abs() < 1) return;
    if (jump) {
      _scroll.jumpTo(target);
    } else {
      _scroll.animateTo(
        target,
        duration: const Duration(milliseconds: 450),
        curve: Curves.easeOutCubic,
      );
    }
  }

  void _seek(int i) {
    _userAt = null;
    final t = widget.lyrics.lines[i].time;
    _c.seek(t);
    _reported = t;
    _reportedAt = _now;
    _update();
  }

  bool _onScroll(UserScrollNotification n) {
    if (n.direction != ScrollDirection.idle) _userAt = _wall.elapsed;
    return false;
  }

  @override
  Widget build(BuildContext context) {
    final lyrics = widget.lyrics;
    final size = widget.large ? 34.0 : 22.0;
    if (!lyrics.synced) {
      return _PlainLyrics(
        text: lyrics.plainText ?? '',
        fontSize: widget.large ? 26 : 18,
      );
    }
    return NotificationListener<UserScrollNotification>(
      onNotification: _onScroll,
      child: ShaderMask(
        // Lines fade out at the top and bottom edges.
        shaderCallback: (r) => const LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [
            Color(0x00FFFFFF),
            Color(0xFFFFFFFF),
            Color(0xFFFFFFFF),
            Color(0x00FFFFFF),
          ],
          stops: [0, 0.08, 0.88, 1],
        ).createShader(r),
        blendMode: BlendMode.dstIn,
        child: FocusTraversalGroup(
          child: SingleChildScrollView(
            key: const ValueKey('lyrics-scroll'),
            controller: _scroll,
            padding: EdgeInsets.symmetric(vertical: widget.large ? 160 : 48),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                for (var i = 0; i < lyrics.lines.length; i++)
                  _Line(
                    key: _keys[i],
                    index: i,
                    text: lyrics.lines[i].text,
                    state: i == _current
                        ? _LineState.current
                        : i < _current
                        ? _LineState.past
                        : _LineState.future,
                    fontSize: size,
                    onTap: () => _seek(i),
                    onFocus: () => _userAt = _wall.elapsed,
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

enum _LineState { past, current, future }

class _Line extends StatelessWidget {
  const _Line({
    super.key,
    required this.index,
    required this.text,
    required this.state,
    required this.fontSize,
    required this.onTap,
    required this.onFocus,
  });

  final int index;
  final String text;
  final _LineState state;
  final double fontSize;
  final VoidCallback onTap;
  final VoidCallback onFocus;

  @override
  Widget build(BuildContext context) {
    final current = state == _LineState.current;
    final color = switch (state) {
      _LineState.current => PaTokens.text,
      _LineState.past => PaTokens.text.withValues(alpha: 0.35),
      _LineState.future => PaTokens.text.withValues(alpha: 0.55),
    };
    return Semantics(
      selected: current,
      button: true,
      child: InkWell(
        key: ValueKey('lyric-line-$index'),
        borderRadius: BorderRadius.circular(10),
        focusColor: const Color(0x33FFFFFF),
        hoverColor: const Color(0x14FFFFFF),
        onTap: onTap,
        onFocusChange: (f) {
          if (f) onFocus();
        },
        child: Padding(
          padding: EdgeInsets.symmetric(
            horizontal: 12,
            vertical: fontSize * 0.35,
          ),
          child: AnimatedDefaultTextStyle(
            duration: const Duration(milliseconds: 250),
            curve: Curves.easeOut,
            style: TextStyle(
              color: color,
              fontSize: fontSize,
              height: 1.25,
              fontWeight: current ? FontWeight.w800 : FontWeight.w700,
            ),
            child: Text(
              text.isEmpty ? '♪' : text,
              key: current ? const ValueKey('lyric-current') : null,
            ),
          ),
        ),
      ),
    );
  }
}

/// Plain lyrics: a scrolling text; arrow and page keys scroll it on TV and
/// desktop.
class _PlainLyrics extends StatefulWidget {
  const _PlainLyrics({required this.text, required this.fontSize});
  final String text;
  final double fontSize;

  @override
  State<_PlainLyrics> createState() => _PlainLyricsState();
}

class _PlainLyricsState extends State<_PlainLyrics> {
  final _scroll = ScrollController();

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  KeyEventResult _onKey(FocusNode _, KeyEvent e) {
    if (e is! KeyDownEvent && e is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    if (!_scroll.hasClients) return KeyEventResult.ignored;
    final pos = _scroll.position;
    final step = switch (e.logicalKey) {
      LogicalKeyboardKey.arrowDown => 80.0,
      LogicalKeyboardKey.arrowUp => -80.0,
      LogicalKeyboardKey.pageDown => pos.viewportDimension * 0.8,
      LogicalKeyboardKey.pageUp => -pos.viewportDimension * 0.8,
      _ => 0.0,
    };
    if (step == 0) return KeyEventResult.ignored;
    final to = (pos.pixels + step).clamp(
      pos.minScrollExtent,
      pos.maxScrollExtent,
    );
    // At an end the key moves the focus on instead.
    if (to == pos.pixels) return KeyEventResult.ignored;
    _scroll.animateTo(
      to,
      duration: const Duration(milliseconds: 200),
      curve: Curves.easeOut,
    );
    return KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    return Focus(
      onKeyEvent: _onKey,
      child: Builder(
        builder: (context) {
          final focused = Focus.of(context).hasFocus;
          return DecoratedBox(
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(10),
              border: Border.all(
                color: focused ? const Color(0x66FFFFFF) : Colors.transparent,
              ),
            ),
            child: SingleChildScrollView(
              key: const ValueKey('lyrics-plain'),
              controller: _scroll,
              padding: const EdgeInsets.all(12),
              child: Text(
                widget.text,
                style: TextStyle(
                  color: PaTokens.text.withValues(alpha: 0.9),
                  fontSize: widget.fontSize,
                  height: 1.5,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}

/// Lyrics on the whole screen (MUS-006): big lines over the blurred cover,
/// the track and a play/pause button. Back, Escape or the close button
/// return to Now Playing.
class LyricsFullscreen extends StatefulWidget {
  const LyricsFullscreen({
    super.key,
    required this.controller,
    required this.session,
  });

  final MusicPlayerController controller;
  final LyricsSession session;

  /// Pushes the fullscreen lyrics over the current route.
  static Future<void> open(
    BuildContext context, {
    required MusicPlayerController controller,
    required LyricsSession session,
  }) => Navigator.of(context).push(
    PageRouteBuilder<void>(
      opaque: true,
      transitionDuration: const Duration(milliseconds: 250),
      pageBuilder: (_, _, _) =>
          LyricsFullscreen(controller: controller, session: session),
      transitionsBuilder: (_, a, _, child) =>
          FadeTransition(opacity: a, child: child),
    ),
  );

  @override
  State<LyricsFullscreen> createState() => _LyricsFullscreenState();
}

class _LyricsFullscreenState extends State<LyricsFullscreen> {
  @override
  void initState() {
    super.initState();
    widget.controller.addListener(_onTrack);
  }

  @override
  void dispose() {
    widget.controller.removeListener(_onTrack);
    super.dispose();
  }

  void _onTrack() => widget.session.show(widget.controller.current);

  KeyEventResult _onKey(FocusNode _, KeyEvent e) {
    if (e is! KeyDownEvent) return KeyEventResult.ignored;
    final k = e.logicalKey;
    if (k == LogicalKeyboardKey.escape) {
      Navigator.of(context).maybePop();
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.space ||
        k == LogicalKeyboardKey.mediaPlayPause) {
      widget.controller.togglePlay();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  @override
  Widget build(BuildContext context) {
    final l = PaPlayerStrings.of(context);
    final c = widget.controller;
    return Theme(
      data: ThemeData.dark(useMaterial3: true),
      child: Focus(
        onKeyEvent: _onKey,
        child: Scaffold(
          key: const ValueKey('lyrics-fullscreen'),
          backgroundColor: PaTokens.bg,
          body: ListenableBuilder(
            listenable: c,
            builder: (context, _) {
              final t = c.current;
              final sub = [t?.title, t?.artist].whereType<String>().join(' — ');
              return Stack(
                fit: StackFit.expand,
                children: [
                  if (t != null)
                    ImageFiltered(
                      imageFilter: ImageFilter.blur(sigmaX: 80, sigmaY: 80),
                      child: Opacity(
                        opacity: 0.35,
                        child: Artwork(request: t, radius: 0),
                      ),
                    ),
                  const ColoredBox(color: Color(0x99000000)),
                  SafeArea(
                    child: Padding(
                      padding: const EdgeInsets.fromLTRB(16, 8, 16, 16),
                      child: Column(
                        children: [
                          Row(
                            children: [
                              FocusIconButton(
                                key: const ValueKey('lyrics-close'),
                                icon: Icons.close_fullscreen_rounded,
                                tooltip: l.close,
                                onPressed: () =>
                                    Navigator.of(context).maybePop(),
                              ),
                              const SizedBox(width: 8),
                              Expanded(
                                child: Text(
                                  sub,
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  style: const TextStyle(
                                    color: PaTokens.textSecondary,
                                    fontSize: 15,
                                    fontWeight: FontWeight.w600,
                                  ),
                                ),
                              ),
                              FocusIconButton(
                                icon: c.playing
                                    ? Icons.pause_rounded
                                    : Icons.play_arrow_rounded,
                                tooltip: c.playing ? l.pause : l.play,
                                autofocus: true,
                                onPressed: c.togglePlay,
                              ),
                            ],
                          ),
                          Expanded(
                            child: Center(
                              child: ConstrainedBox(
                                constraints: const BoxConstraints(
                                  maxWidth: 1000,
                                ),
                                child: ListenableBuilder(
                                  listenable: widget.session,
                                  builder: (context, _) {
                                    final lyrics = widget.session.lyrics;
                                    if (lyrics == null) {
                                      return _Message(
                                        text:
                                            widget.session.status ==
                                                LyricsStatus.loading
                                            ? l.lyricsLoading
                                            : l.lyricsNotFound,
                                        busy:
                                            widget.session.status ==
                                            LyricsStatus.loading,
                                        icon: Icons.lyrics_outlined,
                                      );
                                    }
                                    return LyricsView(
                                      key: ValueKey(lyrics),
                                      controller: c,
                                      lyrics: lyrics,
                                      large: true,
                                    );
                                  },
                                ),
                              ),
                            ),
                          ),
                        ],
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
