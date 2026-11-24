// SPDX-License-Identifier: MPL-2.0
//
// TWI-023: 7TV, BetterTTV and FFZ emotes in the chat. Zero-width emotes
// sit on top of the emote before them; the switch in the chat settings
// (on by default) is kept per profile. With KINOLUNE_SCREENSHOTS_DIR set
// the chat on phone, desktop and TV is written to
// docs/screenshots/progress/twi-023:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/twi-023 \
//     flutter test test/twitch_third_party_emotes_test.dart
//
// The emote images are drawn here (no network, no provider artwork).

import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/features/twitch/chat/chat_controller.dart';
import 'package:kinolune/features/twitch/chat/chat_model.dart';
import 'package:kinolune/features/twitch/chat/chat_settings.dart';
import 'package:kinolune/features/twitch/chat/chat_settings_page.dart';
import 'package:kinolune/features/twitch/chat/chat_view.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/theme/kinolune_theme.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

/// `(key, code, colour)` of the made-up emotes; `hat` is zero-width.
const _emotes = <(String, String, Color?)>[
  ('twitch:emote:25', 'Kappa', Color(0xFF9E9E9E)),
  ('7tv:emote:cat', 'catJAM', Color(0xFFFFB74D)),
  ('7tv:emote:nod', 'pepeNod', Color(0xFF66BB6A)),
  ('bttv:emote:owl', 'owlStare', Color(0xFF64B5F6)),
  ('ffz:emote:tea', 'teaTime', Color(0xFFBA68C8)),
  ('7tv:emote:hat', 'partyHat', null),
];

/// A round face, or a party hat on a transparent square.
Future<List<int>> _png(Color? face) async {
  const size = 96.0;
  final rec = ui.PictureRecorder();
  final c = Canvas(rec);
  if (face != null) {
    c.drawCircle(
      const Offset(size / 2, size / 2),
      size / 2 - 6,
      Paint()..color = face,
    );
    final dark = Paint()..color = const Color(0xFF1A1A1A);
    c.drawCircle(const Offset(size * .36, size * .42), size * .07, dark);
    c.drawCircle(const Offset(size * .64, size * .42), size * .07, dark);
    c.drawArc(
      Rect.fromCircle(center: const Offset(size / 2, size * .55), radius: 20),
      .3,
      2.5,
      false,
      dark
        ..style = PaintingStyle.stroke
        ..strokeWidth = 5,
    );
  } else {
    final hat = Path()
      ..moveTo(size / 2, 0)
      ..lineTo(size * .72, size * .4)
      ..lineTo(size * .28, size * .4)
      ..close();
    c.drawPath(hat, Paint()..color = const Color(0xFFE91916));
    c.drawCircle(
      const Offset(size / 2, 4),
      6,
      Paint()..color = const Color(0xFFFFD700),
    );
  }
  final image = await rec.endRecording().toImage(size.toInt(), size.toInt());
  final data = await image.toByteData(format: ui.ImageByteFormat.png);
  image.dispose();
  return data!.buffer.asUint8List();
}

ChatFragment _e(String code, {bool overlay = false}) {
  final key = _emotes.firstWhere((e) => e.$2 == code).$1;
  return ChatFragment.emote(code, key, overlay: overlay);
}

const _t = ChatFragment.text;

var _seq = 0;
ChatLine _line(String user, String? color, List<ChatFragment> fragments) =>
    ChatLine(
      id: 'm${_seq++}',
      userId: 'u-$user',
      login: user.toLowerCase(),
      displayName: user,
      color: color,
      fragments: fragments,
    );

List<ChatLine> _chat() => [
  _line('LunaFox', '#FF7F50', [_t('hi chat '), _e('Kappa')]),
  _line('pixel_owl', '#1E1E7A', [
    _t('this song '),
    _e('catJAM'),
    _t(' '),
    _e('catJAM'),
  ]),
  _line('TeaKettle42', '#2E8B57', [
    _t('7TV '),
    _e('pepeNod'),
    _t(', BTTV '),
    _e('owlStare'),
    _t(', FFZ '),
    _e('teaTime'),
  ]),
  _line('Kettu', '#FF69B4', [
    _t('zero-width: '),
    _e('catJAM'),
    _e('partyHat', overlay: true),
    _t(' and '),
    _e('owlStare'),
    _e('partyHat', overlay: true),
  ]),
  _line('Ветерок', '#00FF7F', [_t('чат, '), _e('pepeNod'), _t(' круто')]),
  _line('Northwind', null, [
    _t('GG '),
    _e('teaTime'),
    _e('partyHat', overlay: true),
  ]),
];

Widget _host(ChatController c, FormFactor ff, {double? width}) =>
    RepaintBoundary(
      key: screenshotKey,
      child: FormFactorScope(
        formFactor: ff,
        child: MaterialApp(
          debugShowCheckedModeBanner: false,
          locale: const Locale('ru'),
          supportedLocales: AppLocalizations.supportedLocales,
          localizationsDelegates: kLocalizationsDelegates,
          theme: buildKinoluneTheme(ff),
          home: Scaffold(
            backgroundColor: const Color(0xFF0B0B0F),
            body: Align(
              alignment: Alignment.centerRight,
              child: SizedBox(
                width: width,
                child: TwitchChatView(controller: c),
              ),
            ),
          ),
        ),
      ),
    );

Future<ChatController> _controllerWithImages(WidgetTester t) async {
  final dir = Directory.systemTemp.createTempSync('twi023');
  addTearDown(() => dir.deleteSync(recursive: true));
  final images = <ChatUpdate>[];
  await t.runAsync(() async {
    for (final (key, _, color) in _emotes) {
      final f = File('${dir.path}/${key.replaceAll(':', '_')}.png');
      f.writeAsBytesSync(await _png(color));
      images.add(
        ImageReady(ChatImage(key: key, path: f.path, animated: false)),
      );
    }
  });
  final c = ChatController(ManualChatFeed());
  c.apply([for (final l in _chat()) LineAdded(l), ...images]);
  return c;
}

/// Pumps [w] with real time running, so the image codecs started by the
/// first frame finish (real file I/O), then draws again.
Future<void> _pumpWithImages(WidgetTester t, Widget w) async {
  await t.runAsync(() async {
    await t.pumpWidget(w);
    for (var i = 0; i < 10; i++) {
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
  });
  await t.pump();
  await t.pump();
}

void main() {
  setUpAll(loadAppFonts);

  group('settings', () {
    test('third-party emotes are on by default and remembered', () {
      expect(const ChatSettings().thirdPartyEmotes, isTrue);
      expect(ChatSettings.fromJson(const {'v': 1}).thirdPartyEmotes, isTrue);
      final off = const ChatSettings().copyWith(thirdPartyEmotes: false);
      final back = ChatSettings.fromJson(jsonDecode(jsonEncode(off.toJson())));
      expect(back.thirdPartyEmotes, isFalse);
      expect(back, off);
    });

    testWidgets('the switch in the advanced section turns them off', (t) async {
      ChatSettings? saved;
      setSurface(t, const Size(390, 844), dpr: 2);
      await t.pumpWidget(
        RepaintBoundary(
          key: screenshotKey,
          child: MaterialApp(
            debugShowCheckedModeBanner: false,
            locale: const Locale('ru'),
            supportedLocales: AppLocalizations.supportedLocales,
            localizationsDelegates: kLocalizationsDelegates,
            // The test fonts have no Material title face: Inter for the bar.
            theme: buildKinoluneTheme(FormFactor.phone).copyWith(
              appBarTheme: const AppBarTheme(
                titleTextStyle: TextStyle(
                  fontFamily: kFontFamily,
                  fontSize: 20,
                  fontWeight: FontWeight.w600,
                  color: Color(0xFFF2F2F5),
                ),
              ),
            ),
            home: TwitchChatSettingsPage(
              settings: const ChatSettings(),
              onChanged: (s) => saved = s,
            ),
          ),
        ),
      );
      final tile = find.byKey(const ValueKey('chat-third-party-emotes'));
      await t.dragUntilVisible(tile, find.byType(ListView), const Offset(0, -300));
      await t.pumpAndSettle();
      expect(find.text(ru.twitchChatAdvanced), findsOneWidget);
      expect(find.text(ru.twitchChatThirdPartyEmotes), findsOneWidget);
      await capture(t, 'phone-2-settings-advanced');
      await t.tap(tile);
      await t.pumpAndSettle();
      expect(saved?.thirdPartyEmotes, isFalse);
    });
  });

  group('chat', () {
    testWidgets('a zero-width emote is stacked on the emote before it', (
      t,
    ) async {
      setSurface(t, const Size(390, 844), dpr: 2);
      final c = await _controllerWithImages(t);
      await _pumpWithImages(t, _host(c, FormFactor.phone));
      // Three overlays: each a Stack of the base emote and the hat.
      final stacks = find.descendant(
        of: find.byType(TwitchChatView),
        matching: find.byWidgetPredicate(
          (w) =>
              w is Stack &&
              w.children.length == 2 &&
              w.children.every((x) => x is Image),
        ),
      );
      expect(stacks, findsNWidgets(3));
      // Every emote of the chat is an image: no code left as text.
      final texts = t
          .widgetList<RichText>(find.byType(RichText))
          .map((r) => r.text.toPlainText())
          .join('\n');
      for (final (_, code, _) in _emotes) {
        expect(texts.contains(code), isFalse, reason: code);
      }
      c.dispose();
    });

    testWidgets('an overlay whose image is missing stays readable', (t) async {
      final c = ChatController(ManualChatFeed());
      c.apply([
        LineAdded(
          _line('a', null, [_e('catJAM'), _e('partyHat', overlay: true)]),
        ),
      ]);
      await t.pumpWidget(_host(c, FormFactor.phone));
      final texts = t
          .widgetList<RichText>(find.byType(RichText))
          .map((r) => r.text.toPlainText())
          .join();
      expect(texts, contains('catJAM partyHat'));
      c.dispose();
    });

    for (final (name, ff, size, dpr, width)
        in <(String, FormFactor, Size, double, double?)>[
          ('phone-1-chat', FormFactor.phone, const Size(390, 844), 2.0, null),
          (
            'desktop-1-chat',
            FormFactor.desktop,
            const Size(1280, 800),
            1.0,
            380,
          ),
          // Android TV: 1920×1080 physical at 2× (as the TV density test).
          ('tv-1-chat', FormFactor.tv, const Size(960, 540), 2.0, 340),
        ]) {
      testWidgets('screenshot $name', (t) async {
        setSurface(t, size, dpr: dpr);
        final c = await _controllerWithImages(t);
        await _pumpWithImages(t, _host(c, ff, width: width));
        expect(find.byType(Image), findsWidgets);
        await capture(t, name);
        c.dispose();
      });
    }
  });
}
