// SPDX-License-Identifier: MPL-2.0
//
// UI-022: `kinolune://` deep links and launch arguments — parsing, the
// single-instance hand-over on the desktop (the runner's channel
// `app.kinolune/links` is mocked) and routing to an item, a channel, Search
// and a file.

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/platform/app_links.dart';
import 'package:kinolune/screens/movie_detail_screen.dart';
import 'package:kinolune/screens/search_screen.dart';
import 'package:kinolune/screens/twitch/twitch_channel_screen.dart';
import 'package:kinolune/shell/app_shell.dart';

import 'support/test_app.dart';

const _channel = MethodChannel('app.kinolune/links');

AppShellState _shell(WidgetTester t) =>
    t.state<AppShellState>(find.byType(AppShell));

/// The runner hands over [args] as a second start does.
Future<Object?> _open(WidgetTester t, List<String> args) async {
  Object? reply;
  await t.binding.defaultBinaryMessenger.handlePlatformMessage(
    _channel.name,
    const StandardMethodCodec().encodeMethodCall(MethodCall('open', args)),
    (data) {
      if (data != null) {
        reply = const StandardMethodCodec().decodeEnvelope(data);
      }
    },
  );
  return reply;
}

void main() {
  tearDown(LaunchLinks.reset);

  group('AppLink.parse', () {
    test('item, play, channel, search and VOD links round-trip', () {
      final item = AppLink.parse(AppLink.item('show:a b/c'));
      expect((item! as ItemLink).id, 'show:a b/c');
      final play = AppLink.parse(AppLink.play('movie-1'));
      expect((play! as PlayLink).id, 'movie-1');
      final channel = AppLink.parse(AppLink.channel('some_streamer'));
      expect((channel! as ChannelLink).login, 'some_streamer');
      final search = AppLink.parse(AppLink.search('звёздные войны'));
      expect((search! as SearchLink).query, 'звёздные войны');
      final vod = AppLink.parse(AppLink.twitchVod('42', 1500));
      expect((vod! as TwitchVodLink).videoId, '42');
      expect((vod as TwitchVodLink).startMs, 1500);
    });

    test('channel logins are case-insensitive and checked', () {
      final c = AppLink.parse('kinolune://channel/Some_Streamer');
      expect((c! as ChannelLink).login, 'some_streamer');
      for (final bad in [
        'kinolune://channel/',
        'kinolune://channel/a%20b',
        'kinolune://channel/${'x' * 26}',
        'kinolune://channel/a/b',
      ]) {
        expect(AppLink.parse(bad), isNull, reason: bad);
      }
    });

    test('a trailing slash is the same link', () {
      expect((AppLink.parse('kinolune://item/m1/')! as ItemLink).id, 'm1');
    });

    test('other schemes and hosts are not links', () {
      for (final bad in [
        'projectalpha://item/1',
        'https://example.org/item/1',
        'kinolune://unknown/1',
        'kinolune://search',
        'kinolune://search/x?q=a',
      ]) {
        expect(AppLink.parse(bad), isNull, reason: bad);
      }
    });
  });

  group('AppLink.fromArgument', () {
    test('links, file URIs and absolute paths', () {
      expect(AppLink.fromArgument('kinolune://item/m1'), isA<ItemLink>());
      final uri = AppLink.fromArgument('file:///home/u/Видео/a%20b.mkv');
      expect((uri! as FileLink).path, '/home/u/Видео/a b.mkv');
      expect((uri as FileLink).name, 'a b.mkv');
      final linux = AppLink.fromArgument('/media/music/song.FLAC');
      expect((linux! as FileLink).audio, isTrue);
      final windows = AppLink.fromArgument(r'C:\Videos\film.mkv');
      expect((windows! as FileLink).name, 'film.mkv');
      expect((windows as FileLink).audio, isFalse);
    });

    test('options, relative paths and folders are ignored', () {
      for (final arg in [
        '',
        '--enable-software-rendering',
        '-v',
        'film.mkv',
        '/home/u/Videos',
        'https://example.org/a.mkv',
      ]) {
        expect(AppLink.fromArgument(arg), isNull, reason: arg);
      }
    });

    test('several arguments keep their order', () {
      final links = AppLink.fromArguments([
        '--flag',
        'kinolune://search?q=orbit',
        '/a/b.mp4',
      ]);
      expect(links.map((l) => l.runtimeType), [SearchLink, FileLink]);
    });
  });

  group('LaunchLinks (single instance hand-over)', () {
    testWidgets('first-start arguments once, then pending and new ones', (
      t,
    ) async {
      LaunchLinks.reset(initial: ['kinolune://item/first']);
      final calls = <String>[];
      t.binding.defaultBinaryMessenger.setMockMethodCallHandler(_channel, (
        call,
      ) async {
        calls.add(call.method);
        // A second start came while Dart was starting.
        if (call.method == 'take') return ['kinolune://search?q=early'];
        return null;
      });
      addTearDown(
        () => t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          _channel,
          null,
        ),
      );
      final got = <AppLink>[];
      final links = LaunchLinks(onLink: (l) async => got.add(l));
      await links.start();
      expect(calls, ['take']);
      expect((got[0] as ItemLink).id, 'first');
      expect((got[1] as SearchLink).query, 'early');

      // A second start while running: the runner sends `open`.
      final reply = await _open(t, ['--x', '/films/a.mkv']);
      expect(reply, isTrue);
      expect((got.last as FileLink).path, '/films/a.mkv');
      links.dispose();

      // A new shell (another profile) does not replay the first start.
      final again = <AppLink>[];
      final next = LaunchLinks(onLink: (l) async => again.add(l));
      await next.start();
      expect(again.whereType<ItemLink>(), isEmpty);
      next.dispose();
    });

    testWidgets('without a runner (tests, web) nothing breaks', (t) async {
      t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        _channel,
        (call) async => throw MissingPluginException(),
      );
      addTearDown(
        () => t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          _channel,
          null,
        ),
      );
      final got = <AppLink>[];
      final links = LaunchLinks(onLink: (l) async => got.add(l));
      await links.start();
      expect(got, isEmpty);
      links.dispose();
    });
  });

  group('routing', () {
    testWidgets('item link opens its page over Home', (t) async {
      setSurface(t, const Size(1280, 800));
      await t.pumpWidget(
        testApp(await demoServices(), section: Section.movies),
      );
      await settle(t);
      openAppLink(_shell(t), const ItemLink('movie-harbor-lights')).ignore();
      await settle(t);
      expect(_shell(t).section, Section.home);
      expect(find.byType(MovieDetailScreen), findsOneWidget);
      expect(find.text('Harbor Lights'), findsWidgets);
    });

    testWidgets('unknown item: nothing happens', (t) async {
      setSurface(t, const Size(1280, 800));
      await t.pumpWidget(
        testApp(await demoServices(), section: Section.movies),
      );
      await settle(t);
      openAppLink(_shell(t), const ItemLink('no-such-item')).ignore();
      await settle(t);
      expect(_shell(t).section, Section.movies);
      expect(find.byType(MovieDetailScreen), findsNothing);
    });

    testWidgets('search link from a second start opens Search', (t) async {
      setSurface(t, const Size(1280, 800));
      await t.pumpWidget(testApp(await demoServices()));
      await settle(t);
      await t.runAsync(() => _open(t, ['kinolune://search?q=orbit']));
      await settle(t);
      expect(_shell(t).section, Section.search);
      final screen = t.widget<SearchScreen>(find.byType(SearchScreen).last);
      expect(screen.initialQuery, 'orbit');
      expect(find.text('The Quiet Orbit'), findsWidgets);
    });

    testWidgets('channel link opens the channel while Streams is on', (
      t,
    ) async {
      setSurface(t, const Size(1280, 800));
      await t.pumpWidget(testApp(await twitchServices()));
      await settle(t);
      openAppLink(_shell(t), const ChannelLink('somechannel')).ignore();
      await settle(t);
      final page = t.widget<TwitchChannelScreen>(
        find.byType(TwitchChannelScreen),
      );
      expect(page.channel.login, 'somechannel');
    });

    testWidgets('channel link does nothing while Streams is hidden', (t) async {
      setSurface(t, const Size(1280, 800));
      await t.pumpWidget(testApp(await demoServices()));
      await settle(t);
      openAppLink(_shell(t), const ChannelLink('somechannel')).ignore();
      await settle(t);
      expect(find.byType(TwitchChannelScreen), findsNothing);
    });
  });
}
