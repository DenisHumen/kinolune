// SPDX-License-Identifier: MPL-2.0
//
// The "Sent" bubble (TWI-039, the owner's request of 2026-11-25): after
// Helix answers `is_sent=true` (TWI-024) a small bubble with a check mark,
// the user's avatar (TWI-032) and the message (two lines at most, emotes as
// images, TWI-023) pops up in the top-right corner over the video and the
// chat, plays one 3-second animation and goes away:
//
//   0–300 ms     appears: scale 0.6 → 1.4, opacity 0 → 1, easeOutCubic,
//                scaled about its top-right corner;
//   300 ms       a burst of 16 particles from its centre (10 sparks,
//                6 confetti), each living 450–650 ms, at most 120 dp away;
//   300–650 ms   settles: scale 1.4 → 1.0, easeOutBack (dips to 0.97);
//   650–2600 ms  drifts by (−28, +20) dp (×1.5 on a TV), easeInOutSine;
//   2400–3000 ms fades out: opacity 1 → 0, scale 1.0 → 0.96, easeIn.
//
// Up to three bubbles stack, the newest on top; the older ones slide down
// in 200 ms and a fourth removes the oldest. A message Twitch dropped
// (`is_sent=false`) gets the same bubble without particles, a "!" and the
// reason, for 5 s. Reduce Motion (UI-021): no scale, particles or drift —
// 150 ms in, 2.5 s shown, 300 ms out. One AnimationController per bubble;
// the particles are one CustomPainter. The bubbles never take focus or
// input: a tap hides one at once, everything else goes through.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/semantics.dart';
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../../../l10n/l10n.dart';
import '../../../platform/form_factor.dart';
import '../../../theme/kinolune_theme.dart';
import 'chat_model.dart';

/// Bubbles on screen at most.
const kSentBubbleMax = 3;

/// Particles of one burst.
const kSentSparks = 10;
const kSentConfetti = 6;

/// Particles fly at most this far from the bubble's centre (dp).
const kSentParticleReach = 120.0;

/// Distance from the window's edge: 16 dp, the TV safe area 48 dp.
double sentBubbleInset(FormFactor f) => f == FormFactor.tv ? 48 : 16;

/// Width of a bubble at most.
double sentBubbleMaxWidth(FormFactor f) => f == FormFactor.tv ? 420 : 320;

enum SentBubbleKind {
  /// Twitch posted the message (`is_sent=true`).
  sent,

  /// Twitch dropped it (`is_sent=false`): "!" and the reason.
  dropped,
}

// ------------------------------------------------------------------ timing

/// easeOutBack with a smaller overshoot: from 1.4 to 1.0 it dips to 0.97
/// (Flutter's [Curves.easeOutBack] would go to 0.96).
class _BackOut extends Curve {
  const _BackOut();

  /// For `4c³ / 27(c + 1)² = 0.075`: 7.5 % of 0.4 is 0.03.
  static const _c1 = 1.4474;
  static const _c3 = _c1 + 1;

  @override
  double transformInternal(double t) {
    final x = t - 1;
    return 1 + _c3 * x * x * x + _c1 * x * x;
  }
}

/// Where the bubble is at a moment of its life: a pure function of the
/// time, so tests check numbers without pumping frames.
@immutable
class SentBubbleTimeline {
  const SentBubbleTimeline._({
    required this.totalMs,
    required this.reduced,
    required this.particles,
    required this.drift,
  });

  /// The timeline of a bubble of [kind] on [formFactor].
  factory SentBubbleTimeline.of(
    SentBubbleKind kind, {
    FormFactor formFactor = FormFactor.desktop,
    bool reduceMotion = false,
  }) {
    final dropped = kind == SentBubbleKind.dropped;
    final k = formFactor == FormFactor.tv ? 1.5 : 1.0;
    return SentBubbleTimeline._(
      totalMs: dropped ? 5000 : (reduceMotion ? 2950 : 3000),
      reduced: reduceMotion,
      particles: !dropped && !reduceMotion,
      drift: reduceMotion ? Offset.zero : Offset(-28 * k, 20 * k),
    );
  }

  /// Whole life, ms.
  final double totalMs;

  /// Reduce Motion: only the opacity changes.
  final bool reduced;

  /// The burst at [burstMs].
  final bool particles;

  /// Where the drift ends.
  final Offset drift;

  static const appearMs = 300.0;
  static const burstMs = 300.0;
  static const settleEndMs = 650.0;
  static const reducedInMs = 150.0;
  static const reducedOutMs = 300.0;

  /// Older bubbles slide down while a new one comes in.
  static const stackMs = 200.0;

  Duration get duration => Duration(milliseconds: totalMs.round());

  double get _driftEndMs => totalMs - 400;
  double get _fadeStartMs => totalMs - 600;

  static double _u(double ms, double from, double to) =>
      ((ms - from) / (to - from)).clamp(0.0, 1.0);

  double scaleAt(double ms) {
    if (reduced) return 1;
    if (ms <= appearMs) {
      return 0.6 + 0.8 * Curves.easeOutCubic.transform(_u(ms, 0, appearMs));
    }
    if (ms < _fadeStartMs) {
      return 1.4 -
          0.4 * const _BackOut().transform(_u(ms, appearMs, settleEndMs));
    }
    return 1 - 0.04 * Curves.easeIn.transform(_u(ms, _fadeStartMs, totalMs));
  }

  double opacityAt(double ms) {
    if (ms >= totalMs) return 0;
    if (reduced) {
      if (ms < reducedInMs) return _u(ms, 0, reducedInMs);
      final out = totalMs - reducedOutMs;
      return 1 - Curves.easeIn.transform(_u(ms, out, totalMs));
    }
    if (ms <= appearMs) {
      return Curves.easeOutCubic.transform(_u(ms, 0, appearMs));
    }
    return 1 - Curves.easeIn.transform(_u(ms, _fadeStartMs, totalMs));
  }

  Offset driftAt(double ms) =>
      drift * Curves.easeInOutSine.transform(_u(ms, settleEndMs, _driftEndMs));

  /// Share of its height the bubble takes in the stack (older ones slide
  /// down as it grows); at once under Reduce Motion.
  double stackAt(double ms) =>
      reduced ? 1 : Curves.easeOutCubic.transform(_u(ms, 0, stackMs));
}

// --------------------------------------------------------------- particles

/// One spark (a four-pointed star) or confetti (a 2×5 dp strip).
@immutable
class SentParticle {
  const SentParticle({
    required this.spark,
    required this.direction,
    required this.speed,
    required this.size,
    required this.lifeMs,
    required this.color,
    this.spin = 0,
    this.turn = 0,
  });

  final bool spark;

  /// Unit vector of the flight.
  final Offset direction;

  /// dp/s at the start.
  final double speed;

  /// Spark: diameter 3–5 dp; confetti: 2 (the strip is 2×5).
  final double size;
  final double lifeMs;
  final Color color;

  /// Confetti: radians per second (360–720°/s, either way).
  final double spin;

  /// Confetti: angle at the start.
  final double turn;

  /// Slows the flight down (1/s).
  static const drag = 3.5;

  /// dp/s² down.
  static const gravity = 300.0;

  /// Offset from the centre [ms] after the burst, at most
  /// [kSentParticleReach].
  Offset offsetAt(double ms) {
    final t = ms / 1000;
    final flown = speed / drag * (1 - math.exp(-drag * t));
    final o = direction * flown + Offset(0, gravity * t * t / 2);
    final d = o.distance;
    return d > kSentParticleReach ? o * (kSentParticleReach / d) : o;
  }

  /// Fades out over its life (easeIn).
  double opacityAt(double ms) =>
      ms < 0 || ms >= lifeMs ? 0 : 1 - Curves.easeIn.transform(ms / lifeMs);
}

/// The 16 particles of one burst, from [random] (seeded in tests).
List<SentParticle> sentParticles(math.Random random, {Color? accent}) {
  final a = accent ?? KColors.accent;
  final confetti = [a, KColors.live, KColors.statusWarning];
  double between(double lo, double hi) => lo + random.nextDouble() * (hi - lo);
  Offset dir() {
    final angle = random.nextDouble() * math.pi * 2;
    return Offset(math.cos(angle), math.sin(angle));
  }

  return [
    for (var i = 0; i < kSentSparks; i++)
      SentParticle(
        spark: true,
        direction: dir(),
        speed: between(140, 260),
        size: between(3, 5),
        lifeMs: between(450, 650),
        color: i.isEven ? a : const Color(0xFFFFFFFF),
      ),
    for (var i = 0; i < kSentConfetti; i++)
      SentParticle(
        spark: false,
        direction: dir(),
        speed: between(140, 260),
        size: 2,
        lifeMs: between(450, 650),
        color: confetti[i % confetti.length],
        spin: between(2 * math.pi, 4 * math.pi) * (random.nextBool() ? 1 : -1),
        turn: random.nextDouble() * math.pi,
      ),
  ];
}

/// Draws a burst from the centre of its box; [progress] is the bubble's
/// controller (0..1 over [totalMs]).
class SentParticlesPainter extends CustomPainter {
  SentParticlesPainter({
    required this.particles,
    required this.progress,
    required this.timeline,
  }) : super(repaint: progress);

  final List<SentParticle> particles;
  final Animation<double> progress;

  /// The bubble's timeline: the burst follows its scaled centre.
  final SentBubbleTimeline timeline;

  /// A four-pointed star of diameter 1 around the origin.
  static final Path _star = () {
    final p = Path();
    for (var i = 0; i < 8; i++) {
      final r = i.isEven ? .5 : .2;
      final a = i * math.pi / 4 - math.pi / 2;
      final x = r * math.cos(a), y = r * math.sin(a);
      if (i == 0) {
        p.moveTo(x, y);
      } else {
        p.lineTo(x, y);
      }
    }
    return p..close();
  }();

  final _paint = Paint()..isAntiAlias = true;

  @override
  void paint(Canvas canvas, Size size) {
    final now = progress.value * timeline.totalMs;
    final ms = now - SentBubbleTimeline.burstMs;
    if (ms < 0 || ms > 700) return;
    // The card is scaled about its top-right corner.
    final k = timeline.scaleAt(now);
    final centre = Offset(size.width * (1 - k / 2), size.height * k / 2);
    for (final p in particles) {
      final o = p.opacityAt(ms);
      if (o <= 0) continue;
      _paint.color = p.color.withValues(alpha: p.color.a * o);
      final at = centre + p.offsetAt(ms);
      canvas.save();
      canvas.translate(at.dx, at.dy);
      if (p.spark) {
        canvas.scale(p.size);
        canvas.drawPath(_star, _paint);
      } else {
        canvas.rotate(p.turn + p.spin * ms / 1000);
        canvas.drawRect(
          Rect.fromCenter(center: Offset.zero, width: p.size, height: 5),
          _paint,
        );
      }
      canvas.restore();
    }
  }

  @override
  bool shouldRepaint(SentParticlesPainter old) =>
      old.particles != particles || old.timeline != timeline;
}

// ------------------------------------------------------------------ content

/// A piece of the message: text, or an emote's image.
@immutable
class SentPiece {
  const SentPiece.text(this.text) : image = null;
  const SentPiece.emote(this.text, ImageProvider this.image);

  final String text;
  final ImageProvider? image;
}

/// [text] cut into words and emotes: a word that is the code of one of
/// [emotes] whose image [imageOf] has becomes the image.
List<SentPiece> sentPieces(
  String text,
  List<ChatEmote> emotes,
  ImageProvider? Function(String key) imageOf,
) {
  if (emotes.isEmpty) return [SentPiece.text(text)];
  final byCode = {for (final e in emotes) e.code: e.key};
  final out = <SentPiece>[];
  final buf = StringBuffer();
  for (final m in RegExp(r'(\s+)|(\S+)').allMatches(text)) {
    final word = m.group(2);
    final key = word == null ? null : byCode[word];
    final image = key == null ? null : imageOf(key);
    if (image == null) {
      buf.write(m.group(0));
      continue;
    }
    if (buf.isNotEmpty) {
      out.add(SentPiece.text(buf.toString()));
      buf.clear();
    }
    out.add(SentPiece.emote(word!, image));
  }
  if (buf.isNotEmpty) out.add(SentPiece.text(buf.toString()));
  return out;
}

/// One bubble to show.
class SentBubbleData {
  SentBubbleData({
    required this.kind,
    this.pieces = const [],
    this.reason,
    this.avatar,
    this.seed,
  }) : id = _ids++;

  static var _ids = 0;

  final int id;
  final SentBubbleKind kind;

  /// The message (sent).
  final List<SentPiece> pieces;

  /// Why Twitch dropped it, in the user's language (dropped).
  final String? reason;

  /// The user's own avatar (TWI-032); null: none.
  final Widget? avatar;

  /// Seed of the burst; null: a new one each time.
  final int? seed;

  String get plainText => pieces.map((p) => p.text).join();
}

// -------------------------------------------------------------------- stack

/// The bubbles on screen, newest first; at most [kSentBubbleMax].
class SentBubbles extends ChangeNotifier {
  final _items = <SentBubbleData>[];

  List<SentBubbleData> get items => List.unmodifiable(_items);
  bool get isEmpty => _items.isEmpty;

  void add(SentBubbleData b) {
    _items.insert(0, b);
    if (_items.length > kSentBubbleMax) {
      _items.removeRange(kSentBubbleMax, _items.length);
    }
    notifyListeners();
  }

  void remove(int id) {
    final before = _items.length;
    _items.removeWhere((b) => b.id == id);
    if (_items.length != before) notifyListeners();
  }

  void clear() {
    if (_items.isEmpty) return;
    _items.clear();
    notifyListeners();
  }
}

/// The corner with the bubbles of [bubbles], over the whole window. Takes
/// no focus; only a bubble itself takes a tap (and hides).
class SentBubbleLayer extends StatelessWidget {
  const SentBubbleLayer({
    super.key,
    required this.bubbles,
    required this.formFactor,
    required this.reduceMotion,
  });

  final SentBubbles bubbles;
  final FormFactor formFactor;
  final bool reduceMotion;

  @override
  Widget build(BuildContext context) {
    final inset = sentBubbleInset(formFactor);
    final pad = MediaQuery.maybeViewPaddingOf(context) ?? EdgeInsets.zero;
    return ExcludeFocus(
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          Positioned(
            top: inset + pad.top,
            right: inset + pad.right,
            child: ConstrainedBox(
              constraints: BoxConstraints(
                maxWidth: sentBubbleMaxWidth(formFactor),
              ),
              child: ListenableBuilder(
                listenable: bubbles,
                builder: (context, _) => Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.end,
                  children: [
                    for (final b in bubbles.items)
                      SentBubble(
                        key: ValueKey('twitch-sent-bubble-${b.id}'),
                        data: b,
                        formFactor: formFactor,
                        reduceMotion: reduceMotion,
                        onDone: () => bubbles.remove(b.id),
                      ),
                  ],
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// One bubble: plays its timeline once, then [onDone]; a tap calls
/// [onDone] at once.
class SentBubble extends StatefulWidget {
  const SentBubble({
    super.key,
    required this.data,
    required this.onDone,
    this.formFactor = FormFactor.desktop,
    this.reduceMotion = false,
  });

  final SentBubbleData data;
  final VoidCallback onDone;
  final FormFactor formFactor;
  final bool reduceMotion;

  @override
  State<SentBubble> createState() => _SentBubbleState();
}

class _SentBubbleState extends State<SentBubble>
    with SingleTickerProviderStateMixin {
  late final SentBubbleTimeline _tl = SentBubbleTimeline.of(
    widget.data.kind,
    formFactor: widget.formFactor,
    reduceMotion: widget.reduceMotion,
  );
  late final AnimationController _c = AnimationController(
    vsync: this,
    duration: _tl.duration,
  );
  late final List<SentParticle> _particles = _tl.particles
      ? sentParticles(math.Random(widget.data.seed))
      : const [];

  var _done = false;

  @override
  void initState() {
    super.initState();
    // Gone at the last moment of the timeline (the controller itself
    // reports "completed" one frame later).
    _c.addListener(() {
      if (!_done && _c.value >= 1) {
        _done = true;
        widget.onDone();
      }
    });
    _c.forward();
  }

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final d = widget.data;
    final dropped = d.kind == SentBubbleKind.dropped;
    final label = dropped
        ? l.twitchChatNotSentBubble(d.reason ?? '')
        : l.twitchChatSentBubble;
    final card = Semantics(
      container: true,
      label: label,
      onTapHint: l.twitchChatBubbleHide,
      onTap: widget.onDone,
      excludeSemantics: true,
      child: GestureDetector(
        key: const ValueKey('twitch-sent-bubble'),
        behavior: HitTestBehavior.opaque,
        excludeFromSemantics: true,
        onTap: widget.onDone,
        child: RepaintBoundary(
          child: _SentCard(data: d, tv: widget.formFactor == FormFactor.tv),
        ),
      ),
    );
    final total = _tl.totalMs;
    return AnimatedBuilder(
      animation: _c,
      child: card,
      builder: (context, card) {
        final ms = _c.value * total;
        Widget body = Opacity(
          opacity: _tl.opacityAt(ms),
          child: Transform.scale(
            scale: _tl.scaleAt(ms),
            alignment: Alignment.topRight,
            child: card,
          ),
        );
        if (_particles.isNotEmpty) {
          body = Stack(
            clipBehavior: Clip.none,
            children: [
              body,
              Positioned.fill(
                child: IgnorePointer(
                  child: CustomPaint(
                    painter: SentParticlesPainter(
                      particles: _particles,
                      progress: _c,
                      timeline: _tl,
                    ),
                  ),
                ),
              ),
            ],
          );
        }
        return Align(
          alignment: Alignment.topRight,
          widthFactor: 1,
          heightFactor: _tl.stackAt(ms),
          child: Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: Transform.translate(offset: _tl.driftAt(ms), child: body),
          ),
        );
      },
    );
  }
}

class _SentCard extends StatelessWidget {
  const _SentCard({required this.data, required this.tv});

  final SentBubbleData data;
  final bool tv;

  @override
  Widget build(BuildContext context) {
    final dropped = data.kind == SentBubbleKind.dropped;
    final font = tv ? 20.0 : 14.0;
    final mark = tv ? 30.0 : 22.0;
    final emote = font * 1.6;
    final style = TextStyle(
      fontFamily: kFontFamily,
      fontFamilyFallback: kFontFallback,
      fontSize: font,
      height: 1.3,
      color: KColors.textPrimary,
    );
    final avatar = data.avatar;
    final text = dropped
        ? Text(
            data.reason ?? '',
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: style,
          )
        : Text.rich(
            TextSpan(
              children: [
                for (final p in data.pieces)
                  if (p.image case final image?)
                    WidgetSpan(
                      alignment: PlaceholderAlignment.middle,
                      child: Image(
                        image: image,
                        height: emote,
                        gaplessPlayback: true,
                        filterQuality: FilterQuality.medium,
                        semanticLabel: p.text,
                      ),
                    )
                  else
                    TextSpan(text: p.text),
              ],
            ),
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: style,
          );
    return DecoratedBox(
      decoration: BoxDecoration(
        color: KColors.bgElevated2.withValues(alpha: .96),
        borderRadius: BorderRadius.circular(tv ? 18 : 14),
        border: Border.all(
          color: dropped
              ? KColors.statusProblem.withValues(alpha: .7)
              : KColors.accent.withValues(alpha: .55),
        ),
        boxShadow: const [
          BoxShadow(
            color: Color(0x66000000),
            blurRadius: 12,
            offset: Offset(0, 4),
          ),
        ],
      ),
      child: Padding(
        padding: EdgeInsets.symmetric(
          horizontal: tv ? 16 : 12,
          vertical: tv ? 12 : 8,
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: mark,
              height: mark,
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                color: dropped ? KColors.statusProblem : KColors.accent,
              ),
              child: Icon(
                dropped ? Icons.priority_high_rounded : Icons.check_rounded,
                key: ValueKey(
                  dropped ? 'twitch-sent-bang' : 'twitch-sent-check',
                ),
                size: mark * .7,
                color: const Color(0xFFFFFFFF),
              ),
            ),
            if (avatar != null) ...[
              SizedBox(width: tv ? 10 : 8),
              SizedBox.square(dimension: mark + 2, child: avatar),
            ],
            SizedBox(width: tv ? 12 : 10),
            Flexible(child: text),
          ],
        ),
      ),
    );
  }
}

// ----------------------------------------------------------------- overlay

/// Shows the bubbles of one chat in the window's top overlay (over the
/// video and the chat). The layer goes into the overlay while there are
/// bubbles and out when the last one is gone.
class SentBubbleOverlay {
  SentBubbleOverlay() {
    bubbles.addListener(_changed);
  }

  final bubbles = SentBubbles();
  OverlayEntry? _entry;
  FormFactor _formFactor = FormFactor.desktop;
  bool _reduceMotion = false;

  /// Shows [data] above [context]'s window; announces it to the screen
  /// reader and, on a phone, gives a light tap.
  void show(BuildContext context, SentBubbleData data) {
    final overlay = Overlay.maybeOf(context, rootOverlay: true);
    if (overlay == null) return;
    final f = FormFactorScope.of(context);
    final reduce = MediaQuery.maybeDisableAnimationsOf(context) ?? false;
    if (_entry != null && (f != _formFactor || reduce != _reduceMotion)) {
      _entry!.markNeedsBuild();
    }
    _formFactor = f;
    _reduceMotion = reduce;
    if (_entry == null) {
      final entry = _entry = OverlayEntry(
        builder: (_) => SentBubbleLayer(
          bubbles: bubbles,
          formFactor: _formFactor,
          reduceMotion: _reduceMotion,
        ),
      );
      overlay.insert(entry);
    }
    bubbles.add(data);
    final l = context.l10n;
    final text = data.kind == SentBubbleKind.dropped
        ? l.twitchChatNotSentBubble(data.reason ?? '')
        : l.twitchChatSentBubble;
    unawaited(
      SemanticsService.sendAnnouncement(
        View.of(context),
        text,
        Directionality.maybeOf(context) ?? TextDirection.ltr,
      ),
    );
    if (f == FormFactor.phone && data.kind == SentBubbleKind.sent) {
      unawaited(HapticFeedback.lightImpact());
    }
  }

  void _changed() {
    if (bubbles.isEmpty) _removeEntry();
  }

  void _removeEntry() {
    final e = _entry;
    _entry = null;
    if (e == null) return;
    if (e.mounted) e.remove();
    e.dispose();
  }

  void dispose() {
    bubbles.removeListener(_changed);
    _removeEntry();
    bubbles.dispose();
  }
}
