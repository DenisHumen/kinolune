// SPDX-License-Identifier: MPL-2.0
//
// TWI-039: the "Sent" bubble after a chat message. The timeline's numbers
// (scale 1.4 at 300 ms, 1.0 at 650 ms, gone at 3000 ms; drift ×1.5 on a TV;
// Reduce Motion), the burst (10 sparks + 6 confetti, never past 120 dp),
// the stack of three, the "!" variant of a dropped message, tap-to-hide,
// no focus taken, and the bubble showing only after Twitch's answer.
// Golden frames at 0, 150, 300, 450, 650, 1500, 2600 and 3000 ms on the
// fake clock for a phone, a computer and a TV, plus Reduce Motion and the
// "!" variant. Goldens use the default test font like
// test/home_golden_test.dart. Refresh deliberately:
//   flutter test --update-goldens test/twitch_sent_bubble_test.dart

import 'dart:async';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/features/twitch/chat/chat_model.dart';
import 'package:kinolune/features/twitch/chat/chat_panel.dart';
import 'package:kinolune/features/twitch/chat/sent_bubble.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/theme/kinolune_theme.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart' show setSurface;

const _shotKey = ValueKey('sent-bubble-shot');

const _frames = [0, 150, 300, 450, 650, 1500, 2600, 3000];

/// Stage of a golden: the top-right corner of the window.
const _stages = <(String, FormFactor, Size)>[
  ('phone', FormFactor.phone, Size(390, 260)),
  ('desktop', FormFactor.desktop, Size(560, 280)),
  ('tv', FormFactor.tv, Size(760, 380)),
];

Widget _avatar() => const DecoratedBox(
  decoration: BoxDecoration(shape: BoxShape.circle, color: Color(0xFF3E7BFA)),
);

SentBubbleData _sent(String text, {int seed = 7}) => SentBubbleData(
  kind: SentBubbleKind.sent,
  pieces: [SentPiece.text(text)],
  avatar: _avatar(),
  seed: seed,
);

SentBubbleData _dropped(String reason) => SentBubbleData(
  kind: SentBubbleKind.dropped,
  reason: reason,
  avatar: _avatar(),
);

Widget _harness(
  Widget child, {
  FormFactor ff = FormFactor.desktop,
  Locale locale = const Locale('en'),
  bool reduceMotion = false,
}) => FormFactorScope(
  formFactor: ff,
  child: MaterialApp(
    debugShowCheckedModeBanner: false,
    locale: locale,
    supportedLocales: AppLocalizations.supportedLocales,
    localizationsDelegates: kLocalizationsDelegates,
    theme: buildKinoluneTheme(ff),
    builder: (context, child) => MediaQuery(
      data: MediaQuery.of(context).copyWith(disableAnimations: reduceMotion),
      child: child!,
    ),
    home: Scaffold(body: child),
  ),
);

/// A "video" under the corner, and the layer of [bubbles].
Widget _stage(
  SentBubbles bubbles,
  FormFactor ff,
  Size size, {
  bool reduceMotion = false,
}) => Align(
  alignment: Alignment.topLeft,
  child: RepaintBoundary(
    key: _shotKey,
    child: SizedBox.fromSize(
      size: size,
      child: Stack(
        children: [
          const Positioned.fill(
            child: DecoratedBox(
              decoration: BoxDecoration(
                gradient: LinearGradient(
                  begin: Alignment.topLeft,
                  end: Alignment.bottomRight,
                  colors: [Color(0xFF1B2433), Color(0xFF0B0D12)],
                ),
              ),
            ),
          ),
          Positioned.fill(
            child: SentBubbleLayer(
              bubbles: bubbles,
              formFactor: ff,
              reduceMotion: reduceMotion,
            ),
          ),
        ],
      ),
    ),
  ),
);

Future<SentBubbles> _pumpStage(
  WidgetTester t, {
  FormFactor ff = FormFactor.desktop,
  Size size = const Size(560, 280),
  bool reduceMotion = false,
}) async {
  setSurface(t, Size(size.width + 40, size.height + 40));
  final b = SentBubbles();
  addTearDown(b.dispose);
  await t.pumpWidget(
    _harness(_stage(b, ff, size, reduceMotion: reduceMotion), ff: ff),
  );
  return b;
}

Finder get _bubbleCards => find.byKey(const ValueKey('twitch-sent-bubble'));

/// The scale of the bubble's card now.
double _scaleOf(WidgetTester t) {
  final tr = t.widget<Transform>(
    find
        .ancestor(of: _bubbleCards.first, matching: find.byType(Transform))
        .first,
  );
  return tr.transform.getMaxScaleOnAxis();
}

Iterable<SentParticlesPainter> _painters(WidgetTester t) => t
    .widgetList<CustomPaint>(find.byType(CustomPaint))
    .map((c) => c.painter)
    .whereType<SentParticlesPainter>();

/// After adding a bubble: one frame builds it, the next is its 0 ms.
Future<void> _start(WidgetTester t) async {
  await t.pump();
  await t.pump();
}

/// Pumps from [now] to [ms] of the bubble's life.
Future<void> _to(WidgetTester t, int ms, int now) =>
    t.pump(Duration(milliseconds: ms - now));

void main() {
  group('timeline', () {
    final tl = SentBubbleTimeline.of(SentBubbleKind.sent);

    test('appear, settle, drift, fade', () {
      expect(tl.totalMs, 3000);
      expect(tl.scaleAt(0), closeTo(.6, 1e-9));
      expect(tl.opacityAt(0), 0);
      expect(tl.scaleAt(300), closeTo(1.4, 1e-9));
      expect(tl.opacityAt(300), closeTo(1, 1e-9));
      expect(tl.scaleAt(650), closeTo(1.0, 1e-9));
      // easeOutBack: a short dip to 0.97 and back.
      final dip = [
        for (var ms = 300; ms <= 650; ms++) tl.scaleAt(ms.toDouble()),
      ].reduce(math.min);
      expect(dip, closeTo(.97, .002));
      expect(tl.driftAt(650), Offset.zero);
      expect(tl.driftAt(2600).dx, closeTo(-28, 1e-9));
      expect(tl.driftAt(2600).dy, closeTo(20, 1e-9));
      expect(tl.opacityAt(2400), closeTo(1, 1e-9));
      expect(tl.scaleAt(3000), closeTo(.96, 1e-9));
      expect(tl.opacityAt(3000), 0);
      expect(tl.particles, isTrue);
    });

    test('the TV drifts 1.5 times as far', () {
      final tv = SentBubbleTimeline.of(
        SentBubbleKind.sent,
        formFactor: FormFactor.tv,
      );
      expect(tv.driftAt(2600).dx, closeTo(-42, 1e-9));
      expect(tv.driftAt(2600).dy, closeTo(30, 1e-9));
    });

    test('Reduce Motion: only opacity, 150 ms in, 2.5 s, 300 ms out', () {
      final r = SentBubbleTimeline.of(SentBubbleKind.sent, reduceMotion: true);
      expect(r.particles, isFalse);
      expect(r.totalMs, 2950);
      for (var ms = 0; ms <= 2950; ms += 50) {
        expect(r.scaleAt(ms.toDouble()), 1);
        expect(r.driftAt(ms.toDouble()), Offset.zero);
      }
      expect(r.opacityAt(75), closeTo(.5, 1e-9));
      expect(r.opacityAt(150), 1);
      expect(r.opacityAt(2650), 1);
      expect(r.opacityAt(2950), 0);
    });

    test('a dropped message: no burst, five seconds', () {
      final d = SentBubbleTimeline.of(SentBubbleKind.dropped);
      expect(d.particles, isFalse);
      expect(d.totalMs, 5000);
      expect(d.scaleAt(300), closeTo(1.4, 1e-9));
      expect(d.opacityAt(4400), closeTo(1, 1e-9));
      expect(d.opacityAt(5000), 0);
    });
  });

  group('burst', () {
    final ps = sentParticles(math.Random(1));

    test('10 sparks and 6 confetti in the spec ranges', () {
      expect(ps, hasLength(16));
      final sparks = ps.where((p) => p.spark).toList();
      final confetti = ps.where((p) => !p.spark).toList();
      expect(sparks, hasLength(10));
      expect(confetti, hasLength(6));
      for (final p in ps) {
        expect(p.speed, inInclusiveRange(140, 260));
        expect(p.lifeMs, inInclusiveRange(450, 650));
        expect(p.direction.distance, closeTo(1, 1e-9));
      }
      for (final p in sparks) {
        expect(p.size, inInclusiveRange(3, 5));
      }
      expect(
        sparks.map((p) => p.color).toSet(),
        containsAll([KColors.accent, const Color(0xFFFFFFFF)]),
      );
      for (final p in confetti) {
        expect(p.size, 2);
        expect(p.spin.abs(), inInclusiveRange(2 * math.pi, 4 * math.pi));
      }
      expect(confetti.map((p) => p.color).toSet(), hasLength(3));
    });

    test('never further than 120 dp; gone after its life', () {
      for (final seed in [1, 2, 3, 42]) {
        for (final p in sentParticles(math.Random(seed))) {
          for (var ms = 0; ms <= 700; ms += 10) {
            expect(
              p.offsetAt(ms.toDouble()).distance,
              lessThanOrEqualTo(kSentParticleReach + 1e-9),
            );
          }
          expect(p.opacityAt(0), 1);
          expect(p.opacityAt(p.lifeMs), 0);
        }
      }
    });

    test('the same seed gives the same burst', () {
      final a = sentParticles(math.Random(9));
      final b = sentParticles(math.Random(9));
      for (var i = 0; i < a.length; i++) {
        expect(a[i].direction, b[i].direction);
        expect(a[i].speed, b[i].speed);
      }
    });
  });

  group('content', () {
    test('emote codes become images when the image is there', () {
      const emotes = [
        ChatEmote(code: 'otterWave', key: 'e1'),
        ChatEmote(code: 'NoImage', key: 'e2'),
      ];
      final img = MemoryImage(kTransparentPng);
      final pieces = sentPieces(
        'hi otterWave and NoImage otterWave',
        emotes,
        (k) => k == 'e1' ? img : null,
      );
      expect(pieces.map((p) => (p.text, p.image != null)).toList(), [
        ('hi ', false),
        ('otterWave', true),
        (' and NoImage ', false),
        ('otterWave', true),
      ]);
      expect(
        pieces.map((p) => p.text).join(),
        'hi otterWave and NoImage otterWave',
      );
    });

    testWidgets('text: two lines at most, emotes as images', (t) async {
      final b = await _pumpStage(t);
      b.add(
        SentBubbleData(
          kind: SentBubbleKind.sent,
          pieces: [
            SentPiece.text('a very long message ' * 10),
            SentPiece.emote('otterWave', MemoryImage(kTransparentPng)),
          ],
        ),
      );
      await _start(t);
      await t.pump(const Duration(milliseconds: 700));
      final text = t.widget<Text>(
        find.descendant(of: _bubbleCards, matching: find.byType(Text)),
      );
      expect(text.maxLines, 2);
      expect(text.overflow, TextOverflow.ellipsis);
      expect(
        find.descendant(of: _bubbleCards, matching: find.byType(Image)),
        findsOneWidget,
      );
      expect(t.getSize(_bubbleCards).width, lessThanOrEqualTo(320));
      await t.pump(const Duration(seconds: 3));
    });

    testWidgets('a TV bubble is up to 420 dp wide, 48 dp from the edge', (
      t,
    ) async {
      const size = Size(760, 380);
      final b = await _pumpStage(t, ff: FormFactor.tv, size: size);
      b.add(_sent('a very long message ' * 10));
      await _start(t);
      await t.pump(const Duration(milliseconds: 700));
      final rect = t.getRect(_bubbleCards);
      expect(rect.width, lessThanOrEqualTo(420));
      expect(rect.width, greaterThan(320));
      expect(rect.top, closeTo(48, .5));
      expect(size.width - rect.right, closeTo(48, .5));
      await t.pump(const Duration(seconds: 3));
    });
  });

  group('bubble', () {
    testWidgets('scale 1.4 at 300 ms, 1.0 at 650 ms, gone at 3000 ms', (
      t,
    ) async {
      final b = await _pumpStage(t);
      b.add(_sent('GG'));
      await _start(t);
      await _to(t, 300, 0);
      expect(_scaleOf(t), closeTo(1.4, 1e-6));
      expect(_painters(t), hasLength(1));
      await _to(t, 650, 300);
      expect(_scaleOf(t), closeTo(1.0, 1e-6));
      await _to(t, 2999, 650);
      expect(_bubbleCards, findsOneWidget);
      await _to(t, 3000, 2999);
      await t.pump();
      expect(_bubbleCards, findsNothing);
      expect(b.isEmpty, isTrue);
    });

    testWidgets('Reduce Motion: no particles, no scale', (t) async {
      final b = await _pumpStage(t, reduceMotion: true);
      b.add(_sent('GG'));
      await _start(t);
      var now = 0;
      for (final ms in [75, 300, 450, 1500]) {
        await _to(t, ms, now);
        now = ms;
        expect(_painters(t), isEmpty);
        expect(_scaleOf(t), 1);
      }
      await _to(t, 2950, now);
      await t.pump();
      expect(_bubbleCards, findsNothing);
    });

    testWidgets('a dropped message: "!", no particles, five seconds', (
      t,
    ) async {
      final b = await _pumpStage(t);
      b.add(_dropped('Only subscribers can chat right now'));
      await _start(t);
      await t.pump(const Duration(milliseconds: 450));
      expect(find.byKey(const ValueKey('twitch-sent-bang')), findsOneWidget);
      expect(find.byKey(const ValueKey('twitch-sent-check')), findsNothing);
      expect(find.text('Only subscribers can chat right now'), findsOneWidget);
      expect(_painters(t), isEmpty);
      await t.pump(const Duration(milliseconds: 4500));
      expect(_bubbleCards, findsOneWidget);
      await t.pump(const Duration(milliseconds: 50));
      await t.pump();
      expect(_bubbleCards, findsNothing);
    });

    testWidgets('three in a stack, newest on top; a fourth drops the oldest', (
      t,
    ) async {
      final b = await _pumpStage(t, size: const Size(560, 420));
      final items = <SentBubbleData>[];
      for (final text in ['one', 'two', 'three']) {
        final d = _sent(text);
        items.add(d);
        b.add(d);
        await t.pump(const Duration(milliseconds: 250));
      }
      await t.pump(const Duration(milliseconds: 400));
      expect(_bubbleCards, findsNWidgets(3));
      // Layout places, before the drift moves the card.
      double top(SentBubbleData d) =>
          t.getTopLeft(find.byKey(ValueKey('twitch-sent-bubble-${d.id}'))).dy;
      final [one, two, three] = items;
      expect(top(three), lessThan(top(two)));
      expect(top(two), lessThan(top(one)));
      b.add(_sent('four'));
      await _start(t);
      expect(_bubbleCards, findsNWidgets(3));
      expect(find.text('one'), findsNothing);
      // The older ones slide down within 200 ms.
      final before = top(three);
      await t.pump(const Duration(milliseconds: 100));
      final middle = top(three);
      await t.pump(const Duration(milliseconds: 100));
      final after = top(three);
      expect(middle, greaterThan(before));
      expect(after, greaterThan(middle));
      await t.pump(const Duration(milliseconds: 100));
      expect(top(three), after);
      expect(top(two), greaterThan(top(three)));
      await t.pump(const Duration(seconds: 4));
      await t.pump();
      expect(_bubbleCards, findsNothing);
    });

    testWidgets('a tap hides it at once; elsewhere taps go through', (t) async {
      setSurface(t, const Size(800, 600));
      final b = SentBubbles();
      addTearDown(b.dispose);
      final field = FocusNode();
      addTearDown(field.dispose);
      var below = 0;
      await t.pumpWidget(
        _harness(
          Stack(
            children: [
              Positioned(
                left: 20,
                top: 20,
                child: SizedBox(
                  width: 300,
                  child: TextField(
                    key: const ValueKey('field'),
                    focusNode: field,
                  ),
                ),
              ),
              Positioned(
                left: 20,
                bottom: 20,
                child: TextButton(
                  key: const ValueKey('below'),
                  onPressed: () => below++,
                  child: const Text('below'),
                ),
              ),
              Positioned.fill(
                child: SentBubbleLayer(
                  bubbles: b,
                  formFactor: FormFactor.desktop,
                  reduceMotion: false,
                ),
              ),
            ],
          ),
        ),
      );
      field.requestFocus();
      await t.pump();
      expect(field.hasFocus, isTrue);
      b.add(_sent('hello'));
      await _start(t);
      await t.pump(const Duration(milliseconds: 700));
      // It took no focus; the input keeps it and still types.
      expect(field.hasFocus, isTrue);
      await t.enterText(find.byKey(const ValueKey('field')), 'next');
      expect(find.text('next'), findsOneWidget);
      // Nothing in the bubble's subtree can be focused.
      expect(
        find.ancestor(of: _bubbleCards, matching: find.byType(ExcludeFocus)),
        findsWidgets,
      );
      await t.tap(find.byKey(const ValueKey('below')));
      expect(below, 1);
      await t.tap(_bubbleCards);
      await t.pump();
      expect(_bubbleCards, findsNothing);
      expect(field.hasFocus, isTrue);
    });

    testWidgets('screen reader: label and tap hint, ru', (t) async {
      final handle = t.ensureSemantics();
      setSurface(t, const Size(600, 320));
      final b = SentBubbles();
      addTearDown(b.dispose);
      await t.pumpWidget(
        _harness(
          SentBubbleLayer(
            bubbles: b,
            formFactor: FormFactor.desktop,
            reduceMotion: false,
          ),
          locale: const Locale('ru'),
        ),
      );
      b.add(_sent('привет'));
      await _start(t);
      await t.pump(const Duration(milliseconds: 700));
      expect(
        t.getSemantics(_bubbleCards),
        matchesSemantics(
          label: 'Сообщение отправлено',
          hasTapAction: true,
          onTapHint: 'Скрыть',
        ),
      );
      await t.pump(const Duration(seconds: 3));
      handle.dispose();
    });
  });

  group('chat panel', () {
    Future<void> send(WidgetTester t, String text) async {
      await t.enterText(find.byKey(const ValueKey('twitch-chat-input')), text);
      await t.tap(find.byKey(const ValueKey('twitch-chat-send')));
    }

    Future<_GatedFeed> pumpPanel(WidgetTester t, {FormFactor? ff}) async {
      setSurface(t, const Size(900, 700));
      final feed = _GatedFeed();
      await t.pumpWidget(
        _harness(TwitchChatPanel(feed: feed), ff: ff ?? FormFactor.desktop),
      );
      await t.pump();
      return feed;
    }

    testWidgets('only after Twitch posted the message', (t) async {
      final feed = await pumpPanel(t);
      await send(t, 'GG otterWave');
      await t.pump(const Duration(milliseconds: 100));
      expect(feed.sent, ['GG otterWave']);
      expect(_bubbleCards, findsNothing);
      feed.answer(const ChatSent('m-1'));
      await t.pump();
      await t.pump(const Duration(milliseconds: 700));
      expect(_bubbleCards, findsOneWidget);
      expect(find.byKey(const ValueKey('twitch-sent-check')), findsOneWidget);
      expect(find.text('GG otterWave'), findsOneWidget);
      // Over the chat, outside the panel's input: the focus stays there.
      final input = find.descendant(
        of: find.byKey(const ValueKey('twitch-chat-input')),
        matching: find.byType(EditableText),
      );
      expect(t.widget<EditableText>(input).focusNode.hasFocus, isTrue);
      await t.pump(const Duration(seconds: 3));
      await t.pump();
      expect(_bubbleCards, findsNothing);
    });

    testWidgets('dropped: the "!" bubble with the reason', (t) async {
      final feed = await pumpPanel(t);
      await send(t, 'hello');
      await t.pump();
      feed.answer(
        const ChatDropped(code: 'msg_subsonly', message: 'subs only'),
      );
      await t.pump();
      await t.pump(const Duration(milliseconds: 700));
      expect(find.byKey(const ValueKey('twitch-sent-bang')), findsOneWidget);
      expect(
        find.descendant(
          of: _bubbleCards,
          matching: find.text('Only subscribers can chat right now'),
        ),
        findsOneWidget,
      );
      await t.pump(const Duration(seconds: 5));
      await t.pump();
      expect(_bubbleCards, findsNothing);
    });

    testWidgets('a failed request shows no bubble', (t) async {
      final feed = await pumpPanel(t);
      await send(t, 'hello');
      await t.pump();
      feed.answer(const ChatSendFailed(ChatSendError.network));
      await t.pump();
      await t.pump(const Duration(milliseconds: 700));
      expect(_bubbleCards, findsNothing);
    });

    testWidgets('closing the chat takes its bubbles away', (t) async {
      final feed = await pumpPanel(t);
      await send(t, 'hello');
      await t.pump();
      feed.answer(const ChatSent('m-1'));
      await t.pump();
      await t.pump(const Duration(milliseconds: 300));
      expect(_bubbleCards, findsOneWidget);
      await t.pumpWidget(_harness(const SizedBox()));
      await t.pump();
      expect(_bubbleCards, findsNothing);
    });
  });

  group('goldens', () {
    for (final (name, ff, size) in _stages) {
      testWidgets('frames on $name', (t) async {
        final b = await _pumpStage(t, ff: ff, size: size);
        b.add(_sent('GG, what a play! otterWave'));
        await _start(t);
        var now = 0;
        for (final ms in _frames) {
          await _to(t, ms, now);
          now = ms;
          await expectLater(
            find.byKey(_shotKey),
            matchesGoldenFile('goldens/twitch_sent_bubble_${name}_$ms.png'),
          );
        }
      });
    }

    testWidgets('Reduce Motion frames', (t) async {
      final b = await _pumpStage(t, reduceMotion: true);
      b.add(_sent('GG, what a play! otterWave'));
      await _start(t);
      var now = 0;
      for (final ms in [75, 300, 2800]) {
        await _to(t, ms, now);
        now = ms;
        await expectLater(
          find.byKey(_shotKey),
          matchesGoldenFile('goldens/twitch_sent_bubble_reduced_$ms.png'),
        );
      }
    });

    testWidgets('a dropped message and a stack of three', (t) async {
      final b = await _pumpStage(t, size: const Size(560, 360));
      b.add(_sent('first one'));
      await t.pump(const Duration(milliseconds: 700));
      b.add(_dropped('Only subscribers can chat right now'));
      await t.pump(const Duration(milliseconds: 700));
      b.add(_sent('and the third'));
      await t.pump(const Duration(milliseconds: 1500));
      await expectLater(
        find.byKey(_shotKey),
        matchesGoldenFile('goldens/twitch_sent_bubble_stack.png'),
      );
      await t.pump(const Duration(seconds: 5));
      await t.pump();
    });
  });
}

/// A feed whose send waits for [answer]: the bubble must wait too.
class _GatedFeed extends ManualChatFeed {
  _GatedFeed()
    : super(
        channel: 'river_stream',
        canSend: true,
        ownLogin: 'moon_watcher',
        emotes: const [ChatEmote(code: 'otterWave', key: 'twitch:emote:e1')],
      );

  Completer<ChatSendResult>? _pending;

  void answer(ChatSendResult r) => _pending!.complete(r);

  @override
  Future<ChatSendResult> send(String text, {String? replyTo}) {
    sent.add(text);
    return (_pending = Completer()).future;
  }
}

/// A 1×1 transparent PNG.
final kTransparentPng = Uint8List.fromList(const [
  0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, //
  0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
  0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00,
  0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
  0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
  0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
]);
