// SPDX-License-Identifier: MPL-2.0
//
// TWI-024: chat filters and input on a scripted feed (no core, no
// network; all names are made up): autocomplete of @names and emotes,
// the input's states (signed out, no write permission, room modes, slow
// mode, Twitch's drop reasons), mention highlighting, blocking from a
// line, the settings page and their storage per profile.

import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/features/twitch/chat/chat_autocomplete.dart';
import 'package:kinolune/features/twitch/chat/chat_controller.dart';
import 'package:kinolune/features/twitch/chat/chat_input.dart';
import 'package:kinolune/features/twitch/chat/chat_model.dart';
import 'package:kinolune/features/twitch/chat/chat_panel.dart';
import 'package:kinolune/features/twitch/chat/chat_settings.dart';
import 'package:kinolune/features/twitch/chat/chat_settings_page.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/theme/kinolune_theme.dart';
import 'package:material_ui/material_ui.dart';

final en = lookupAppLocalizations(const Locale('en'));
final ru = lookupAppLocalizations(const Locale('ru'));

var _seq = 0;

ChatLine msg(String text, {String user = 'lunafox', bool mention = false}) =>
    ChatLine(
      id: 'm${_seq++}',
      userId: 'u-$user',
      login: user,
      displayName: user,
      fragments: [ChatFragment.text(text)],
      mention: mention,
    );

const emotes = [
  ChatEmote(code: 'otterWave', key: 'twitch:emote:e1', channel: true),
  ChatEmote(code: 'otterHug', key: 'twitch:emote:e2', channel: true),
  ChatEmote(code: 'FakeKappa', key: 'twitch:emote:e3'),
  ChatEmote(code: 'MoonLaugh', key: 'twitch:emote:e4'),
];

Widget app(Widget child, {Locale locale = const Locale('en')}) =>
    FormFactorScope(
      formFactor: FormFactor.phone,
      child: MaterialApp(
        locale: locale,
        supportedLocales: AppLocalizations.supportedLocales,
        localizationsDelegates: kLocalizationsDelegates,
        theme: buildKinoluneTheme(FormFactor.phone),
        home: Scaffold(body: child),
      ),
    );

ManualChatFeed signedIn({bool canSend = true}) => ManualChatFeed(
  channel: 'river_stream',
  canSend: canSend,
  ownLogin: 'moon_watcher',
  emotes: [...emotes],
);

Future<ChatController> pumpInput(
  WidgetTester tester,
  ManualChatFeed feed, {
  VoidCallback? onSignIn,
  VoidCallback? onAllowSending,
  Locale locale = const Locale('en'),
}) async {
  final c = ChatController(feed);
  addTearDown(c.dispose);
  await tester.pumpWidget(
    app(
      Align(
        alignment: Alignment.bottomCenter,
        child: TwitchChatInput(
          controller: c,
          onSignIn: onSignIn,
          onAllowSending: onAllowSending,
        ),
      ),
      locale: locale,
    ),
  );
  return c;
}

final input = find.byKey(const ValueKey('twitch-chat-input'));
final sendButton = find.byKey(const ValueKey('twitch-chat-send'));
final notice = find.byKey(const ValueKey('twitch-chat-input-notice'));

void main() {
  group('autocomplete', () {
    test('finds the word at the cursor', () {
      final q = completionAt('hi @lun', 7)!;
      expect(q.kind, CompletionKind.mention);
      expect((q.start, q.end, q.prefix), (3, 7, 'lun'));
      expect(completionAt('hi :ot', 6)!.kind, CompletionKind.emote);
      // One letter after ':' is too little (":D" is a smiley).
      expect(completionAt('hi :o', 5), isNull);
      expect(completionAt('hi ', 3), isNull);
      // A plain word completes only on Tab.
      expect(completionAt('otte', 4), isNull);
      expect(completionAt('otte', 4, tab: true)!.prefix, 'otte');
    });

    test('suggests chatters by start and emotes channel first', () {
      final names = suggestionsFor(
        completionAt('@pi', 3)!,
        chatters: ['pixel_owl', 'lunafox', 'Pine_Marten'],
        emotes: emotes,
      );
      expect(names.map((s) => s.insert), ['@pixel_owl', '@Pine_Marten']);
      final codes = suggestionsFor(
        completionAt(':otter', 6)!,
        chatters: const [],
        emotes: emotes,
      );
      expect(codes.map((s) => s.insert), ['otterWave', 'otterHug']);
      // Three letters or more also match inside the code.
      final inside = suggestionsFor(
        completionAt(':kap', 4)!,
        chatters: const [],
        emotes: emotes,
      );
      expect(inside.map((s) => s.insert), ['FakeKappa']);
    });

    test('replaces the word and adds one space', () {
      const v = TextEditingValue(
        text: 'hey @lu how',
        selection: TextSelection.collapsed(offset: 7),
      );
      final q = completionAt(v.text, 7)!;
      final next = applySuggestion(v, q, const ChatSuggestion.mention('lunafox'));
      expect(next.text, 'hey @lunafox how');
      expect(next.selection.baseOffset, 13);
      final end = applySuggestion(
        const TextEditingValue(text: ':otterW'),
        completionAt(':otterW', 7)!,
        ChatSuggestion.emote(emotes.first),
      );
      expect(end.text, 'otterWave ');
    });
  });

  group('ChatController (TWI-024)', () {
    test('recent chatters, newest first, one entry per user', () {
      final c = ChatController(ManualChatFeed());
      c.apply([
        LineAdded(msg('a', user: 'pixel_owl')),
        LineAdded(msg('b', user: 'lunafox')),
        LineAdded(msg('c', user: 'pixel_owl')),
      ]);
      expect(c.recentChatters, ['pixel_owl', 'lunafox']);
      c.dispose();
    });

    test('settings go to the feed; blocked lines leave the screen', () {
      final feed = ManualChatFeed();
      final c = ChatController(feed);
      c.apply([
        LineAdded(msg('spam', user: 'noisy_otter')),
        LineAdded(msg('hello', user: 'lunafox')),
      ]);
      final s = const ChatSettings().block('@Noisy_Otter');
      c.applySettings(s);
      expect(feed.filters, s);
      expect(c.lines.map((l) => l.login), ['lunafox']);
      expect(c.recentChatters, ['lunafox']);
      c.dispose();
    });

    test('room modes', () {
      final c = ChatController(ManualChatFeed());
      const modes = ChatRoomModes(slowSeconds: 30, subscribersOnly: true);
      c.apply([const RoomModesChanged(modes)]);
      expect(c.roomModes, modes);
      c.dispose();
    });
  });

  group('settings', () {
    test('JSON round trip and tolerant reading', () {
      const s = ChatSettings(
        blocked: ['noisy_otter'],
        words: [
          ChatWordRule('spoiler'),
          ChatWordRule(r'buy\s+followers', regex: true, hide: true),
        ],
        highlightOwnName: false,
        highlightKeywords: ['giveaway'],
        hideBots: true,
        extraBots: ['helper_robo'],
      );
      expect(ChatSettings.fromJson(jsonDecode(jsonEncode(s.toJson()))), s);
      expect(ChatSettings.fromJson('nonsense'), const ChatSettings());
      expect(
        ChatSettings.fromJson({
          'blocked': ['  @Loud_Badger ', 3, ''],
          'words': [
            {'pattern': ''},
            {'pattern': 'ok'},
          ],
        }),
        const ChatSettings(blocked: ['loud_badger'], words: [ChatWordRule('ok')]),
      );
    });

    test('stored per profile', () async {
      final prefs = MemoryPreferences();
      var profile = 1;
      final store = ChatSettingsStore(prefs, () async => profile);
      await store.ensureLoaded();
      await store.save(const ChatSettings(hideBots: true));
      profile = 2;
      expect(await store.ensureLoaded(), const ChatSettings());
      await store.save(const ChatSettings(blocked: ['noisy_otter']));
      profile = 1;
      expect((await store.ensureLoaded()).hideBots, isTrue);
      expect(
        await prefs.read(Preferences.twitchChatSettings('2')),
        contains('noisy_otter'),
      );
      store.dispose();
    });
  });

  group('input', () {
    testWidgets('signed out: no field, a sign-in button', (tester) async {
      var asked = 0;
      await pumpInput(tester, ManualChatFeed(), onSignIn: () => asked++);
      expect(input, findsNothing);
      expect(find.text(en.twitchChatSignInToChat), findsOneWidget);
      await tester.tap(find.text(en.twitchChatSignIn));
      expect(asked, 1);
    });

    testWidgets('signed in without permission: "Allow sending"', (
      tester,
    ) async {
      var asked = 0;
      await pumpInput(
        tester,
        signedIn(canSend: false),
        onAllowSending: () => asked++,
      );
      expect(input, findsNothing);
      expect(find.text(en.twitchChatSendingOff), findsOneWidget);
      await tester.tap(find.text(en.twitchChatAllowSending));
      expect(asked, 1);
    });

    testWidgets('@ suggests recent chatters; a tap completes', (tester) async {
      final feed = signedIn();
      final c = await pumpInput(tester, feed);
      c.apply([
        LineAdded(msg('hi', user: 'pixel_owl')),
        LineAdded(msg('yo', user: 'lunafox')),
      ]);
      await tester.enterText(input, 'hey @pix');
      await tester.pump();
      expect(find.byKey(const ValueKey('twitch-chat-suggestions')), findsOne);
      await tester.tap(find.text('pixel_owl'));
      await tester.pump();
      expect(tester.widget<TextField>(input).controller!.text, 'hey @pixel_owl ');
      expect(find.byKey(const ValueKey('twitch-chat-suggestions')), findsNothing);
    });

    testWidgets('emotes: ":" and two letters, and Tab on a plain word', (
      tester,
    ) async {
      final feed = signedIn();
      await pumpInput(tester, feed);
      await tester.enterText(input, ':ot');
      await tester.pump();
      expect(find.text('otterWave'), findsOneWidget);
      expect(find.text('otterHug'), findsOneWidget);
      // Arrow down picks the second; Tab accepts it.
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await tester.sendKeyEvent(LogicalKeyboardKey.tab);
      await tester.pump();
      final text = tester.widget<TextField>(input).controller!;
      expect(text.text, 'otterHug ');

      await tester.enterText(input, 'nice Moon');
      await tester.pump();
      expect(find.byKey(const ValueKey('twitch-chat-suggestions')), findsNothing);
      await tester.sendKeyEvent(LogicalKeyboardKey.tab);
      await tester.pump();
      expect(text.text, 'nice MoonLaugh ');
    });

    testWidgets('sends, then waits out slow mode', (tester) async {
      final feed = signedIn();
      final c = await pumpInput(tester, feed);
      c.apply([
        const RoomModesChanged(
          ChatRoomModes(slowSeconds: 3, subscribersOnly: true),
        ),
      ]);
      await tester.pump();
      // Room modes above the input.
      final modes = find.byKey(const ValueKey('twitch-chat-modes'));
      expect(
        find.descendant(
          of: modes,
          matching: find.textContaining(en.twitchChatModeSubsOnly),
        ),
        findsOne,
      );
      expect(find.textContaining(en.twitchChatModeSlow(3)), findsOne);

      await tester.enterText(input, '  good luck  ');
      await tester.tap(sendButton);
      await tester.pump();
      expect(feed.sent, ['good luck']);
      expect(tester.widget<TextField>(input).controller!.text, isEmpty);
      expect(tester.widget<Text>(notice).data, en.twitchChatSlowWait(3));
      expect(tester.widget<IconButton>(sendButton).onPressed, isNull);

      await tester.pump(const Duration(seconds: 1));
      expect(tester.widget<Text>(notice).data, en.twitchChatSlowWait(2));
      await tester.pump(const Duration(seconds: 2));
      expect(notice, findsNothing);
      expect(tester.widget<IconButton>(sendButton).onPressed, isNotNull);
    });

    testWidgets("Twitch's drop reasons, localised", (tester) async {
      final feed = signedIn()
        ..sendResult = const ChatDropped(
          code: 'msg_subsonly',
          message: 'This room is in subscribers only mode.',
        );
      await pumpInput(tester, feed, locale: const Locale('ru'));
      await tester.enterText(input, 'привет');
      await tester.testTextInput.receiveAction(TextInputAction.send);
      await tester.pump();
      expect(tester.widget<Text>(notice).data, ru.twitchChatDropSubsOnly);
      // Text stays so the user can edit it.
      expect(tester.widget<TextField>(input).controller!.text, 'привет');

      feed.sendResult = const ChatDropped(
        code: 'msg_automod_blocked',
        message: 'Held by AutoMod.',
      );
      await tester.tap(sendButton);
      await tester.pump();
      expect(
        tester.widget<Text>(notice).data,
        ru.twitchChatDropOther('Held by AutoMod.'),
      );

      feed.sendResult = const ChatSendFailed(ChatSendError.network);
      await tester.tap(sendButton);
      await tester.pump();
      expect(tester.widget<Text>(notice).data, ru.twitchChatSendFailed);
    });
  });

  group('panel', () {
    Future<(ChatController, List<ChatSettings>)> pumpPanel(
      WidgetTester tester,
      ManualChatFeed feed, {
      ChatSettings settings = const ChatSettings(),
      ChatRuleCheck? check,
    }) async {
      final saved = <ChatSettings>[];
      await tester.pumpWidget(
        app(
          TwitchChatPanel(
            feed: feed,
            settings: settings,
            onSettingsChanged: saved.add,
            checkRule: check,
          ),
        ),
      );
      // The panel's controller is private; reach it through the input.
      final c = tester
          .widget<TwitchChatInput>(find.byType(TwitchChatInput))
          .controller;
      return (c, saved);
    }

    testWidgets('settings reach the feed; mentions are tinted', (
      tester,
    ) async {
      final feed = signedIn();
      const s = ChatSettings(hideBots: true);
      final (c, _) = await pumpPanel(tester, feed, settings: s);
      expect(feed.filters, s);
      c.apply([
        LineAdded(msg('hey @moon_watcher', user: 'pixel_owl', mention: true)),
        LineAdded(msg('plain', user: 'lunafox')),
      ]);
      await tester.pump();
      expect(find.byKey(const ValueKey('twitch-chat-mention')), findsOne);
    });

    testWidgets('long press: block hides the user and saves', (tester) async {
      final feed = signedIn();
      final (c, saved) = await pumpPanel(tester, feed);
      c.apply([
        LineAdded(msg('buy my stuff', user: 'noisy_otter')),
        LineAdded(msg('hello', user: 'lunafox')),
      ]);
      await tester.pump();
      await tester.longPress(find.textContaining('buy my stuff'));
      await tester.pumpAndSettle();
      expect(find.text(en.twitchChatMentionUser('noisy_otter')), findsOne);
      await tester.tap(find.text(en.twitchChatBlockUser('noisy_otter')));
      await tester.pumpAndSettle();
      expect(saved.single.blocked, ['noisy_otter']);
      expect(feed.filters!.blocked, ['noisy_otter']);
      expect(find.textContaining('buy my stuff'), findsNothing);
      expect(find.textContaining('hello'), findsOne);
    });

    testWidgets('long press: mention fills the input', (tester) async {
      final feed = signedIn();
      final (c, _) = await pumpPanel(tester, feed);
      c.apply([LineAdded(msg('hello', user: 'lunafox'))]);
      await tester.pump();
      await tester.longPress(find.textContaining('hello'));
      await tester.pumpAndSettle();
      await tester.tap(find.text(en.twitchChatMentionUser('lunafox')));
      await tester.pumpAndSettle();
      expect(tester.widget<TextField>(input).controller!.text, '@lunafox ');
    });

    testWidgets('settings page: invalid regex is refused, valid ones saved', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(800, 2400);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final feed = signedIn();
      String? check(ChatWordRule r) =>
          r.regex && r.pattern.contains('(') && !r.pattern.contains(')')
          ? 'unclosed group'
          : null;
      final (_, saved) = await pumpPanel(tester, feed, check: check);
      await tester.tap(find.byKey(const ValueKey('twitch-chat-settings')));
      await tester.pumpAndSettle();
      expect(find.text(en.twitchChatSettings), findsOne);

      await tester.tap(find.byKey(const ValueKey('chat-word-regex')));
      await tester.enterText(
        find.byKey(const ValueKey('chat-word-field')),
        '(free',
      );
      await tester.tap(find.byKey(const ValueKey('chat-word-add')));
      await tester.pump();
      expect(find.text(en.twitchChatInvalidRule('unclosed group')), findsOne);
      expect(saved, isEmpty);

      await tester.enterText(
        find.byKey(const ValueKey('chat-word-field')),
        r'free\s+gift',
      );
      await tester.tap(find.byKey(const ValueKey('chat-word-hide')));
      await tester.tap(find.byKey(const ValueKey('chat-word-add')));
      await tester.pump();
      expect(saved.last.words, [
        const ChatWordRule(r'free\s+gift', regex: true, hide: true),
      ]);
      expect(feed.filters!.words, saved.last.words);

      await tester.tap(find.byKey(const ValueKey('chat-hide-bots')));
      await tester.pump();
      expect(saved.last.hideBots, isTrue);
      expect(saved.last.words, hasLength(1));
    });
  });

  test('ru strings', () {
    expect(ru.twitchChatModeSlow(5), 'Медленный режим: 5 секунд между сообщениями');
    expect(ru.twitchChatSlowWait(2), 'Медленный режим: подождите 2 секунды');
    expect(ru.twitchChatBlockUser('lunafox'), 'Заблокировать lunafox');
  });
}
