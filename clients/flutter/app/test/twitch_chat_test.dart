// SPDX-License-Identifier: MPL-2.0
//
// TWI-012: the Twitch chat list on a scripted feed (no core, no network):
// ring buffer, moderation, "N new messages", shared emote images, pause of
// animations, badges, name contrast, TV density (≥ 28 px at 1080p).

import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/features/twitch/chat/chat_controller.dart';
import 'package:kinolune/features/twitch/chat/chat_model.dart';
import 'package:kinolune/features/twitch/chat/chat_panel.dart';
import 'package:kinolune/features/twitch/chat/chat_view.dart';
import 'package:kinolune/features/twitch/chat/name_color.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/theme/kinolune_theme.dart';
import 'package:material_ui/material_ui.dart';

final en = lookupAppLocalizations(const Locale('en'));
final ru = lookupAppLocalizations(const Locale('ru'));

var _seq = 0;

ChatLine msg(
  String text, {
  String user = 'lunafox',
  String? color,
  List<ChatBadge> badges = const [],
  List<ChatFragment>? fragments,
  String? id,
}) => ChatLine(
  id: id ?? 'm${_seq++}',
  userId: 'u-$user',
  login: user,
  displayName: user,
  color: color,
  badges: badges,
  fragments: fragments ?? [ChatFragment.text(text)],
);

Widget host(
  ChatController c, {
  FormFactor formFactor = FormFactor.phone,
  bool pause = false,
  Locale locale = const Locale('en'),
}) => FormFactorScope(
  formFactor: formFactor,
  child: MaterialApp(
    locale: locale,
    supportedLocales: AppLocalizations.supportedLocales,
    localizationsDelegates: kLocalizationsDelegates,
    theme: buildKinoluneTheme(formFactor),
    home: Scaffold(
      body: TwitchChatView(controller: c, pauseAnimations: pause),
    ),
  ),
);

/// Text of every rich text on screen, newest line last.
List<String> shownTexts(WidgetTester tester) => tester
    .widgetList<RichText>(find.byType(RichText))
    .map((r) => r.text.toPlainText())
    .toList();

void main() {
  group('ChatController', () {
    test('keeps only the newest lines (ring buffer)', () async {
      final feed = ManualChatFeed();
      final c = ChatController(feed, capacity: 3);
      c.apply([for (var i = 0; i < 5; i++) LineAdded(msg('n$i', id: 'id$i'))]);
      expect(c.lines.map((l) => l.id), ['id2', 'id3', 'id4']);
      // A delete of a line that fell out is ignored.
      c.apply([const LineDeleted('id0'), const LineDeleted('id3')]);
      expect(c[1].fate, ChatLineFate.deleted);
      c.dispose();
    });

    test('timeouts, bans and clears', () {
      final c = ChatController(ManualChatFeed());
      c.apply([
        LineAdded(msg('a', user: 'pixel_owl')),
        LineAdded(msg('b', user: 'lunafox')),
        LineAdded(msg('c', user: 'pixel_owl')),
        const UserCleared(userId: 'u-pixel_owl', login: 'pixel_owl', seconds: 600),
      ]);
      expect(c.lines.where((l) => l.fate == ChatLineFate.timedOut).length, 2);
      expect(c.lines.last.system, isA<UserTimedOutText>());
      c.apply([const UserCleared(userId: 'u-lunafox', login: 'lunafox', banned: true)]);
      expect(c[1].fate, ChatLineFate.banned);
      c.apply([const ChatCleared()]);
      expect(c.length, 1);
      expect(c[0].system, isA<ChatClearedText>());
      c.dispose();
    });

    test('paused: new lines wait, resume shows them in order', () {
      final c = ChatController(ManualChatFeed(), capacity: 100);
      c.apply([LineAdded(msg('first'))]);
      c.pause();
      c.apply([LineAdded(msg('x')), LineAdded(msg('y'))]);
      expect(c.length, 1);
      expect(c.unseen, 2);
      c.resume();
      expect(c.unseen, 0);
      expect(c.lines.map((l) => l.fragments.first.text), ['first', 'x', 'y']);
      c.dispose();
    });

    test('TV thinning shows at most N lines a second and counts the rest', () {
      var now = DateTime(2026, 1, 1);
      final c = ChatController(
        ManualChatFeed(),
        maxLinesPerSecond: 8,
        clock: () => now,
      );
      c.apply([for (var i = 0; i < 30; i++) LineAdded(msg('$i'))]);
      expect(c.length, 8);
      now = now.add(const Duration(seconds: 1));
      c.apply([LineAdded(msg('next'))]);
      expect(c.length, 9);
      expect(c.skipped, 22);
      c.dispose();
    });
  });

  group('name colours', () {
    const bg = Color(0xFF16161C);
    test('dark colours are lifted to WCAG AA contrast, keeping the hue', () {
      for (final hex in ['#0000FF', '#000000', '#1E1E7A', '#8A2BE2', '#B22222']) {
        final c = NameColors(bg).of(hex, 'x');
        expect(contrastRatio(c, bg), greaterThanOrEqualTo(kNameMinContrast), reason: hex);
      }
      final blue = HSLColor.fromColor(NameColors(bg).of('#0000FF', 'x'));
      expect(blue.hue, closeTo(240, 1));
    });

    test('readable colours stay as they are', () {
      expect(NameColors(bg).of('#00FF7F', 'x'), const Color(0xFF00FF7F));
    });

    test('users without a colour get a stable default', () {
      final a = NameColors(bg).of(null, 'northwind');
      expect(NameColors(bg).of(null, 'northwind'), a);
      expect(contrastRatio(a, bg), greaterThanOrEqualTo(kNameMinContrast));
    });
  });

  group('TwitchChatView', () {
    testWidgets('names, badges, text and moderation', (tester) async {
      final c = ChatController(ManualChatFeed());
      await tester.pumpWidget(host(c));
      expect(find.text(en.twitchChatConnecting), findsOneWidget);
      c.apply([
        const ConnectionChanged(ChatConnection.connected),
        LineAdded(
          msg(
            'hello chat',
            user: 'pixel_owl',
            color: '#000000',
            badges: const [
              ChatBadge(setId: 'moderator'),
              ChatBadge(setId: 'unknown_set'),
            ],
            id: 'keep',
          ),
        ),
        LineAdded(msg('rude words', id: 'bad')),
      ]);
      await tester.pump();
      final texts = shownTexts(tester);
      expect(texts.any((t) => t.contains('pixel_owl: hello chat')), isTrue);
      // Anonymous chat: the moderator badge is a drawn mark.
      expect(find.byIcon(Icons.shield_rounded), findsOneWidget);

      // The black name is readable on the dark chat.
      final rich = tester
          .widgetList<RichText>(find.byType(RichText))
          .firstWhere((r) => r.text.toPlainText().contains('hello chat'));
      Color? nameColor;
      rich.text.visitChildren((span) {
        if (span is TextSpan && span.text == 'pixel_owl') {
          nameColor = span.style?.color;
        }
        return true;
      });
      expect(
        contrastRatio(nameColor!, const Color(0xFF16161C)),
        greaterThanOrEqualTo(kNameMinContrast),
      );

      c.apply([const LineDeleted('bad')]);
      await tester.pump();
      expect(
        shownTexts(tester).any((t) => t.contains(en.twitchChatMessageDeleted)),
        isTrue,
      );
      expect(shownTexts(tester).any((t) => t.contains('rude words')), isFalse);

      c.apply([const UserCleared(userId: 'u-pixel_owl', login: 'pixel_owl', seconds: 600)]);
      await tester.pump();
      expect(find.text(en.twitchChatTimedOut('pixel_owl', 600)), findsOneWidget);
      c.apply([const ChatCleared()]);
      await tester.pump();
      expect(find.text(en.twitchChatCleared), findsOneWidget);
      expect(shownTexts(tester).any((t) => t.contains('hello chat')), isFalse);
      c.dispose();
    });

    testWidgets('emotes: code until the image is ready, then one shared image', (
      tester,
    ) async {
      final c = ChatController(ManualChatFeed());
      await tester.pumpWidget(host(c));
      const hype = ChatFragment.emote('lunaHype', 'twitch:emote:demo_2');
      c.apply([
        LineAdded(msg('', fragments: const [ChatFragment.text('go '), hype])),
        LineAdded(msg('', fragments: const [hype, ChatFragment.text(' '), hype])),
      ]);
      await tester.pump();
      expect(shownTexts(tester).any((t) => t.contains('go lunaHype')), isTrue);
      expect(find.byType(Image), findsNothing);

      c.apply([
        const ImageReady(
          ChatImage(key: 'twitch:emote:demo_2', path: '/nonexistent/hype.gif', animated: true),
        ),
      ]);
      await tester.pump();
      final images = tester.widgetList<Image>(find.byType(Image)).toList();
      expect(images, hasLength(3));
      // Same provider instance for every copy: one decode, shared frames.
      expect(images.every((i) => identical(i.image, images.first.image)), isTrue);
      c.dispose();
    });

    testWidgets('pausing animations turns tickers off for emotes', (tester) async {
      final c = ChatController(ManualChatFeed());
      c.apply([
        LineAdded(msg('', fragments: const [ChatFragment.emote('owlSpin', 'k')])),
        const ImageReady(ChatImage(key: 'k', path: '/nonexistent/spin.gif', animated: true)),
      ]);
      await tester.pumpWidget(host(c, pause: true));
      final ctx = tester.element(find.byType(Image));
      expect(TickerMode.valuesOf(ctx).enabled, isFalse);
      await tester.pumpWidget(host(c));
      expect(TickerMode.valuesOf(tester.element(find.byType(Image))).enabled, isTrue);
      c.dispose();
    });

    testWidgets('scrolled up: list holds still, "N new messages" returns', (
      tester,
    ) async {
      final c = ChatController(ManualChatFeed());
      c.apply([for (var i = 0; i < 80; i++) LineAdded(msg('line $i'))]);
      await tester.pumpWidget(host(c));
      expect(shownTexts(tester).any((t) => t.contains('line 79')), isTrue);

      await tester.drag(find.byType(ListView), const Offset(0, 400));
      await tester.pump();
      expect(c.following, isFalse);
      c.apply([for (var i = 80; i < 85; i++) LineAdded(msg('line $i'))]);
      await tester.pump();
      expect(find.text(en.twitchChatNewMessages(5)), findsOneWidget);
      expect(shownTexts(tester).any((t) => t.contains('line 84')), isFalse);

      await tester.tap(find.text(en.twitchChatNewMessages(5)));
      await tester.pump();
      expect(c.following, isTrue);
      expect(find.text(en.twitchChatNewMessages(5)), findsNothing);
      expect(shownTexts(tester).any((t) => t.contains('line 84')), isTrue);
      c.dispose();
    });

    testWidgets('Russian plural of new messages', (tester) async {
      final c = ChatController(ManualChatFeed());
      c.apply([for (var i = 0; i < 80; i++) LineAdded(msg('line $i'))]);
      await tester.pumpWidget(host(c, locale: const Locale('ru')));
      await tester.drag(find.byType(ListView), const Offset(0, 400));
      await tester.pump();
      c.apply([for (var i = 0; i < 3; i++) LineAdded(msg('x'))]);
      await tester.pump();
      expect(find.text('3 новых сообщения'), findsOneWidget);
      expect(ru.twitchChatNewMessages(5), '5 новых сообщений');
      c.dispose();
    });

    testWidgets('TV density: chat text is at least 28 px on a 1080p screen', (
      tester,
    ) async {
      // Android TV: 1920×1080 physical at 2× = 960×540 logical.
      tester.view.physicalSize = const Size(1920, 1080);
      tester.view.devicePixelRatio = 2;
      addTearDown(tester.view.reset);
      final c = ChatController(
        ManualChatFeed(),
        capacity: chatCapacityFor(FormFactor.tv),
        maxLinesPerSecond: chatRateLimitFor(FormFactor.tv),
      );
      c.apply([LineAdded(msg('readable from the sofa'))]);
      await tester.pumpWidget(host(c, formFactor: FormFactor.tv));
      final para = tester.renderObject<RenderParagraph>(
        find.byWidgetPredicate(
          (w) => w is RichText && w.text.toPlainText().contains('sofa'),
        ),
      );
      final fontSize = para.text.style!.fontSize!;
      final physical = fontSize * tester.view.devicePixelRatio;
      expect(physical, greaterThanOrEqualTo(28));
      // TV thins a busy chat.
      expect(c.maxLinesPerSecond, isNotNull);
      c.dispose();
    });
  });
}
