// SPDX-License-Identifier: MPL-2.0
//
// TWI-027: «К игре» / «Пропустить болтовню» in the native Twitch player —
// where the game starts after Just Chatting, the button and when it is
// hidden, the chapter strip with game covers, ↑/↓ between chapters on a TV,
// a playthrough part opened from its game's category. Fixture chapters,
// fake usher, fake proxy, fake engine; no network.
//
// With KINOLUNE_SCREENSHOTS_DIR set the frames are written there:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/twi-027 \
//     flutter test test/twitch_skip_chatting_test.dart

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/twitch/twitch_play.dart';
import 'package:kinolune/screens/twitch/twitch_player_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/twitch/twitch_models.dart';
import 'package:kinolune/twitch/twitch_native.dart';
import 'package:kinolune/widgets/artwork.dart';
import 'package:kinolune/twitch/twitch_service.dart';
import 'package:kinolune/twitch/twitch_skip_chatting.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

const _desktop = Size(1440, 900);
const _phone = Size(844, 390);
const _tv = Size(1920, 1080);

/// The fixture of the task: «Just Chatting 0:00–0:42, игра 0:42–4:10».
const _talkThenGame = [
  TwitchChapter(
    gameId: '509658',
    gameName: 'Just Chatting',
    start: Duration.zero,
    end: Duration(minutes: 42),
  ),
  TwitchChapter(
    gameId: '501',
    gameName: 'Starfall Tactics',
    start: Duration(minutes: 42),
    end: Duration(hours: 4, minutes: 10),
  ),
];

/// A game from the start, then talk.
const _gameThenTalk = [
  TwitchChapter(
    gameId: '501',
    gameName: 'Starfall Tactics',
    start: Duration.zero,
    end: Duration(minutes: 120),
  ),
  TwitchChapter(
    gameId: '509658',
    gameName: 'Just Chatting',
    start: Duration(minutes: 120),
  ),
];

class _Session implements TwitchLiveSession {
  _Session(this.variant);

  String variant;

  @override
  String get playlistUrl => 'http://127.0.0.1:9/token/vod.m3u8';

  @override
  LiveTelemetry telemetry() => throw UnsupportedError('VOD');

  @override
  void setMode(LiveLatencyMode m) {}

  @override
  void setVariant(String url) => variant = url;

  @override
  void close() {}
}

class _Source implements TwitchNativeSource {
  final resolved = <TwitchPlayTarget>[];

  @override
  Future<TwitchNativeStream> resolve(TwitchPlayTarget target) async {
    resolved.add(target);
    return TwitchNativeStream([
      TwitchQuality(
        id: 'chunked',
        label: '1080p60 (source)',
        url: 'https://usher.test/vod/${target.videoId}/chunked.m3u8',
        isSource: true,
        height: 1080,
        frameRate: 60,
      ),
    ]);
  }

  @override
  TwitchLiveSession startLive(String url, LiveLatencyMode mode) =>
      throw UnsupportedError('live');

  @override
  TwitchLiveSession openVod(String url) => _Session(url);
}

class _Harness {
  _Harness(this.services);

  final AppServices services;
  final source = _Source();
  final engines = <FakePlayerEngine>[];

  FakePlayerEngine get engine => engines.last;

  void register() {
    services.twitch.nativePlayer = (context, target) => openTwitchNative(
      context,
      target,
      source: source,
      engineFactory: (_) {
        final e = FakePlayerEngine(
          loadedDuration: const Duration(hours: 4, minutes: 10),
        );
        engines.add(e);
        return e;
      },
    );
  }
}

Future<_Harness> _harness({bool russian = false}) async {
  final services = await twitchServices(twitchNative: true);
  if (russian) await services.setLocale(const Locale('ru'));
  return _Harness(services)..register();
}

Future<void> _pumpApp(
  WidgetTester t,
  _Harness h, {
  FormFactor formFactor = FormFactor.desktop,
}) async {
  await t.pumpWidget(
    testApp(h.services, section: Section.twitch, formFactor: formFactor),
  );
  await settle(t);
}

Future<void> _play(
  WidgetTester t,
  _Harness h,
  List<TwitchChapter> chapters, {
  FormFactor formFactor = FormFactor.desktop,
}) async {
  await _pumpApp(t, h, formFactor: formFactor);
  final context = t.element(find.byType(Scaffold).first);
  // ignore: unawaited_futures
  h.services.twitch.nativePlayer!(
    context,
    TwitchPlayTarget.vod(
      '9101',
      login: 'pixelheron',
      title: 'Campaign day 11: the long siege',
      chapters: chapters,
    ),
  );
  await settle(t, 14);
}

PlayerScreenState _player(WidgetTester t) =>
    t.state<PlayerScreenState>(find.byType(PlayerScreen));

Future<void> _showOsd(WidgetTester t) async {
  _player(t).osd.show();
  await settle(t, 3);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 4);
}

const _skip = ValueKey('twitch-skip-chatting');
const _strip = ValueKey('twitch-chapter-strip');

void main() {
  group('where the game starts', () {
    test('Just Chatting 0:00–0:42, game 0:42–4:10 → 0:42', () {
      expect(
        firstGameAfterTalk(_talkThenGame)?.start,
        const Duration(minutes: 42),
      );
      // Order of the list does not matter.
      expect(
        firstGameAfterTalk(_talkThenGame.reversed.toList())?.start,
        const Duration(minutes: 42),
      );
    });

    test('no talk at the start, talk only, approximate: nothing', () {
      expect(firstGameAfterTalk(_gameThenTalk), isNull);
      expect(firstGameAfterTalk(const []), isNull);
      expect(firstGameAfterTalk([_talkThenGame.first]), isNull);
      expect(
        firstGameAfterTalk(const [
          TwitchChapter(gameId: '509658', start: Duration.zero),
          TwitchChapter(
            gameId: '26936',
            gameName: 'Music',
            start: Duration(minutes: 30),
          ),
        ]),
        isNull,
      );
      expect(
        firstGameAfterTalk(const [
          TwitchChapter(
            gameId: '509658',
            gameName: 'Just Chatting',
            start: Duration.zero,
            approximate: true,
          ),
          TwitchChapter(gameId: '501', start: Duration(minutes: 42)),
        ]),
        isNull,
      );
    });

    test('talk after talk is skipped too; names count without an id', () {
      final game = firstGameAfterTalk(const [
        TwitchChapter(
          gameId: '',
          gameName: 'Just Chatting',
          start: Duration.zero,
        ),
        TwitchChapter(
          gameId: '417752',
          gameName: 'Talk Shows & Podcasts',
          start: Duration(minutes: 20),
        ),
        TwitchChapter(
          gameId: '505',
          gameName: 'Hollow Lanterns',
          start: Duration(minutes: 55),
        ),
      ]);
      expect(game?.gameName, 'Hollow Lanterns');
      expect(game?.start, const Duration(minutes: 55));
      expect(
        isNonGameChapter(
          const TwitchChapter(
            gameId: 'x',
            gameName: "I'm Only Sleeping",
            start: Duration.zero,
          ),
        ),
        isTrue,
      );
    });

    test('a game part starts where the game does', () {
      expect(gameStartIn(_talkThenGame, '501'), const Duration(minutes: 42));
      expect(gameStartIn(_talkThenGame, '509658'), isNull);
      expect(gameStartIn(_talkThenGame, '999'), isNull);
    });
  });

  test('box art by game id when Helix gave none', () {
    expect(
      twitchImage(twitchBoxArtTemplate('509658'), 52, 72),
      'https://static-cdn.jtvnw.net/ttv-boxart/509658-52x72.jpg',
    );
    expect(twitchBoxArtTemplate(null), isNull);
    expect(twitchBoxArtTemplate('not-a-number'), isNull);
  });

  testWidgets('«Skip to the game» seeks to 0:42 and then hides', (t) async {
    setSurface(t, _desktop);
    final h = await _harness();
    await _play(t, h, _talkThenGame);
    await _showOsd(t);
    expect(find.byKey(_skip), findsOneWidget);
    expect(find.text('Skip to the game'), findsOneWidget);
    expect(
      find.byTooltip('Skip the chatting: Starfall Tactics from 42:00'),
      findsOneWidget,
    );
    await t.tap(find.byKey(_skip));
    await settle(t, 3);
    expect(h.engine.calls, contains('seek:${42 * 60000}'));
    expect(h.engine.state.value.position, const Duration(minutes: 42));
    // In the game: nothing to skip.
    expect(find.byKey(_skip), findsNothing);
    // Back in the talk: offered again.
    await h.engine.seek(const Duration(minutes: 5));
    await settle(t, 2);
    expect(find.byKey(_skip), findsOneWidget);
    await _close(t);
  });

  testWidgets('no talk at the start: no button, the strip stays', (t) async {
    setSurface(t, _desktop);
    final h = await _harness();
    await _play(t, h, _gameThenTalk);
    await _showOsd(t);
    expect(find.byKey(_skip), findsNothing);
    expect(find.byKey(_strip), findsOneWidget);
    await _close(t);
  });

  testWidgets('one chapter: no button and no strip', (t) async {
    setSurface(t, _desktop);
    final h = await _harness();
    await _play(t, h, [_gameThenTalk.first]);
    await _showOsd(t);
    expect(find.byKey(_skip), findsNothing);
    expect(find.byKey(_strip), findsNothing);
    await _close(t);
  });

  testWidgets('chapter strip: a segment per chapter, tap seeks', (t) async {
    setSurface(t, _desktop);
    final h = await _harness();
    await _play(t, h, _talkThenGame);
    await _showOsd(t);
    expect(find.byKey(_strip), findsOneWidget);
    final talk = find.byKey(const ValueKey('twitch-chapter-seg-0'));
    final game = find.byKey(const ValueKey('twitch-chapter-seg-1'));
    expect(talk, findsOneWidget);
    expect(game, findsOneWidget);
    // As long as the chapters: 42 min against 3 h 28 min.
    final wTalk = t.getSize(talk).width, wGame = t.getSize(game).width;
    expect(wTalk / wGame, closeTo(42 / 208, 0.02));
    // Names, and the category cover by game id even without box_art_url
    // (no letter placeholder).
    expect(
      find.descendant(of: game, matching: find.text('Starfall Tactics')),
      findsOneWidget,
    );
    expect(find.descendant(of: game, matching: find.text('S')), findsNothing);
    expect(find.descendant(of: game, matching: find.byType(Artwork)), findsOneWidget);
    // Above the scrubber.
    final strip = t.getRect(find.byKey(_strip));
    final scrubber = t.getRect(find.byKey(const ValueKey('osd-scrubber')));
    expect(strip.bottom, lessThanOrEqualTo(scrubber.top));
    final semantics = t.ensureSemantics();
    expect(
      find.bySemanticsLabel('Starfall Tactics from 42:00'),
      findsOneWidget,
    );
    semantics.dispose();

    await t.tap(game);
    await settle(t, 3);
    expect(h.engine.calls.last, 'seek:${42 * 60000}');
    await t.tap(talk);
    await settle(t, 3);
    expect(h.engine.calls.last, 'seek:0');
    await _close(t);
  });

  testWidgets('TV: ↓ with the OSD hidden goes to the game, ↑ back', (t) async {
    setSurface(t, _tv);
    final h = await _harness();
    await _play(t, h, _talkThenGame, formFactor: FormFactor.tv);
    final osd = _player(t).osd..hide();
    await settle(t, 2);
    await pressKey(t, LogicalKeyboardKey.arrowDown);
    expect(h.engine.calls, contains('seek:${42 * 60000}'));
    expect(osd.visible, isFalse);
    await pressKey(t, LogicalKeyboardKey.arrowUp);
    expect(h.engine.calls.last, 'seek:0');
    await _close(t);
  });

  testWidgets('desktop: ↑/↓ keep showing the OSD, PageDown is the key', (
    t,
  ) async {
    setSurface(t, _desktop);
    final h = await _harness();
    await _play(t, h, _talkThenGame);
    final osd = _player(t).osd..hide();
    await settle(t, 2);
    await pressKey(t, LogicalKeyboardKey.arrowDown);
    expect(h.engine.calls.where((c) => c.startsWith('seek')), isEmpty);
    osd.hide();
    await settle(t, 2);
    await pressKey(t, LogicalKeyboardKey.pageDown);
    expect(h.engine.calls, contains('seek:${42 * 60000}'));
    await _close(t);
  });

  testWidgets('a playthrough part from its category starts with the game', (
    t,
  ) async {
    setSurface(t, _desktop);
    final h = await _harness();
    await _pumpApp(t, h);
    final context = t.element(find.byType(Scaffold).first);
    final video = TwitchVideo(
      id: '9201',
      userId: '7001',
      login: 'pixelheron',
      displayName: 'PixelHeron',
      title: 'Starfall Tactics, part 3',
      publishedAt: DateTime(2026, 11, 18),
      duration: const Duration(hours: 4, minutes: 10),
      chapters: _talkThenGame,
    );
    // ignore: unawaited_futures
    playTwitchVideo(context, video, gameId: '501');
    await settle(t, 14);
    expect(h.source.resolved.last.startAt, const Duration(minutes: 42));
    expect(
      h.engine.calls.firstWhere((c) => c.startsWith('open')),
      startsWith('open:1:0:${42 * 60000}:'),
    );
    await _close(t);
    // From anywhere else: from the start.
    await _pumpApp(t, h);
    // ignore: unawaited_futures
    playTwitchVideo(t.element(find.byType(Scaffold).first), video);
    await settle(t, 14);
    expect(h.source.resolved.last.startAt, isNull);
    await _close(t);
  });

  testWidgets('Russian strings', (t) async {
    setSurface(t, _desktop);
    final h = await _harness(russian: true);
    await _play(t, h, _talkThenGame);
    await _showOsd(t);
    expect(find.text('К игре'), findsOneWidget);
    expect(
      find.byTooltip('Пропустить болтовню: Starfall Tactics с 42:00'),
      findsOneWidget,
    );
    final l = t.element(find.byType(TwitchPlayerScreen)).l10n;
    expect(l.twitchChapterAt('Just Chatting', '0:00'), 'Just Chatting с 0:00');
    await _close(t);
  });

  group('screenshots', () {
    setUpAll(loadAppFonts);
    for (final (name, ff, size, dpr) in <(String, FormFactor, Size, double)>[
      ('phone', FormFactor.phone, _phone, 2),
      ('desktop', FormFactor.desktop, const Size(1280, 800), 1),
      ('tv', FormFactor.tv, _tv, 1),
    ]) {
      testWidgets('on $name', (t) async {
        setSurface(t, size, dpr: dpr);
        final h = await _harness(russian: true);
        await _play(t, h, _talkThenGame, formFactor: ff);
        await h.engine.seek(const Duration(minutes: 7, seconds: 30));
        await h.engine.pause();
        await _showOsd(t);
        await capture(t, '01-skip-chatting-$name');
        await t.tap(find.byKey(_skip));
        await settle(t, 3);
        await _showOsd(t);
        await capture(t, '02-in-the-game-$name');
        await _close(t);
      });
    }
  });
}
