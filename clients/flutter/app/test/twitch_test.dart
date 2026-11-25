// SPDX-License-Identifier: MPL-2.0
//
// The Streams section (TWI-013): local favourites without a Twitch login
// (DEC-006), their order and filters, resolving a name once signed in,
// the playback decision (TWI-004), sub-only VODs, the Continue hook of
// Home and Device Code Flow sign-in. Fake Helix with made-up channels.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_twitch.dart';
import 'package:kinolune/integration/library.dart' show Progress;
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/home_feed.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/twitch/twitch_backend.dart';
import 'package:kinolune/twitch/twitch_models.dart';
import 'package:kinolune/twitch/twitch_service.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

const _desktop = Size(1280, 800);

TwitchFavoriteEntry _entry(
  String login, {
  bool pinned = false,
  int? viewers,
  int addedDay = 1,
  int? lastLiveDay,
}) => TwitchFavoriteEntry(
  favorite: TwitchFavorite(
    login: login,
    addedAt: DateTime(2026, 11, addedDay),
    pinned: pinned,
  ),
  channel: TwitchChannel(
    login: login,
    displayName: login,
    userId: '${login.length}',
    lastLiveAt: lastLiveDay == null ? null : DateTime(2026, 11, lastLiveDay),
  ),
  live: viewers == null
      ? null
      : TwitchStream(
          userId: '1',
          login: login,
          displayName: login,
          title: 't',
          viewers: viewers,
        ),
);

List<String> _logins(List<TwitchFavoriteEntry> e) => [
  for (final x in e) x.channel.login,
];

Future<void> _tap(WidgetTester t, Finder f) async {
  await t.ensureVisible(f.first);
  await settle(t, 4);
  await t.tap(f.first);
  await settle(t, 10);
}

void main() {
  group('models', () {
    test('channel names: login, @login and links', () {
      expect(normalizeLogin('PixelHeron'), 'pixelheron');
      expect(normalizeLogin(' @luna_forge '), 'luna_forge');
      expect(normalizeLogin('https://www.twitch.tv/NovaMarsh'), 'novamarsh');
      expect(normalizeLogin('ab'), isNull);
      expect(normalizeLogin('two words'), isNull);
    });

    test('VOD retention: 7, 14 and 60 days for archives only', () {
      final p = DateTime(2026, 11, 1);
      expect(
        vodExpiresAt(TwitchVideoType.archive, TwitchBroadcasterType.normal, p),
        DateTime(2026, 11, 8),
      );
      expect(
        vodExpiresAt(
          TwitchVideoType.archive,
          TwitchBroadcasterType.affiliate,
          p,
        ),
        DateTime(2026, 11, 15),
      );
      expect(
        vodExpiresAt(TwitchVideoType.archive, TwitchBroadcasterType.partner, p),
        DateTime(2026, 12, 31),
      );
      expect(
        vodExpiresAt(
          TwitchVideoType.highlight,
          TwitchBroadcasterType.normal,
          p,
        ),
        isNull,
      );
    });

    test('image templates', () {
      expect(
        twitchImage('https://x/%{width}x%{height}.jpg', 640, 360),
        'https://x/640x360.jpg',
      );
      expect(
        twitchImage('https://x/{width}x{height}.jpg', 3, 4),
        'https://x/3x4.jpg',
      );
      expect(twitchImage('', 1, 1), isNull);
    });
  });

  group('sorting and filtering favourites', () {
    final entries = [
      _entry('delta', addedDay: 4, lastLiveDay: 10),
      _entry('alpha', viewers: 50, addedDay: 2),
      _entry('charlie', viewers: 900, addedDay: 1),
      _entry('bravo', pinned: true, addedDay: 3, lastLiveDay: 12),
      _entry('echo', addedDay: 5),
    ];

    test('pinned first in every order', () {
      for (final s in TwitchFavoriteSort.values) {
        expect(sortFavorites(entries, s).first.channel.login, 'bravo');
      }
    });

    test('orders', () {
      expect(_logins(sortFavorites(entries, TwitchFavoriteSort.liveFirst)), [
        'bravo',
        'charlie',
        'alpha',
        'delta',
        'echo',
      ]);
      expect(_logins(sortFavorites(entries, TwitchFavoriteSort.name)), [
        'bravo',
        'alpha',
        'charlie',
        'delta',
        'echo',
      ]);
      expect(
        _logins(sortFavorites(entries, TwitchFavoriteSort.recentlyAdded)),
        ['bravo', 'echo', 'delta', 'alpha', 'charlie'],
      );
      expect(_logins(sortFavorites(entries, TwitchFavoriteSort.lastLive)), [
        'bravo',
        'alpha',
        'charlie',
        'delta',
        'echo',
      ]);
      expect(_logins(sortFavorites(entries, TwitchFavoriteSort.viewers)), [
        'bravo',
        'charlie',
        'alpha',
        'delta',
        'echo',
      ]);
    });

    test('filters', () {
      expect(
        _logins(
          sortFavorites(
            entries,
            TwitchFavoriteSort.name,
            TwitchFavoriteFilter.live,
          ),
        ),
        ['alpha', 'charlie'],
      );
      expect(
        _logins(
          sortFavorites(
            entries,
            TwitchFavoriteSort.name,
            TwitchFavoriteFilter.offline,
          ),
        ),
        ['bravo', 'delta', 'echo'],
      );
    });
  });

  group('route for every flag combination (TWI-030)', () {
    // Spelled out per row: store/full flavor, native allowed by the signed
    // config, kill-switch hit/miss, sub-only VOD, embed available.
    TwitchPlayRoute expected({
      required bool full,
      required bool allowed,
      required bool killed,
      required bool subOnly,
      required bool embed,
    }) {
      if (subOnly) return TwitchPlayRoute.openOnTwitch;
      if (full && allowed && !killed) return TwitchPlayRoute.native;
      return embed ? TwitchPlayRoute.embed : TwitchPlayRoute.openOnTwitch;
    }

    for (final full in [false, true]) {
      for (final allowed in [false, true]) {
        for (final killed in [false, true]) {
          for (final subOnly in [false, true]) {
            for (final embed in [false, true]) {
              final want = expected(
                full: full,
                allowed: allowed,
                killed: killed,
                subOnly: subOnly,
                embed: embed,
              );
              final name = [
                full ? 'full' : 'store',
                allowed ? 'native on' : 'native off',
                killed ? 'kill-switch hit' : 'kill-switch miss',
                if (subOnly) 'sub-only',
                if (!embed) 'no embed',
              ].join(', ');
              test('$name: ${want.name}', () {
                expect(
                  decideTwitchRoute(
                    fullBuild: full,
                    nativeAllowedByConfig: allowed,
                    killSwitchHit: killed,
                    hasNativePlayer: true,
                    hasEmbedPlayer: embed,
                    subOnly: subOnly,
                  ),
                  want,
                );
              });
            }
          }
        }
      }
    }

    test('no native player registered: never native', () {
      expect(
        decideTwitchRoute(
          fullBuild: true,
          nativeAllowedByConfig: true,
          hasNativePlayer: false,
          hasEmbedPlayer: true,
        ),
        TwitchPlayRoute.embed,
      );
    });

    test('the service passes the kill-switch and sub-only on', () {
      Future<void> noop(BuildContext _, TwitchPlayTarget _) async {}
      TwitchService service({required bool killed}) =>
          TwitchService(
              backend: const LocalTwitchBackend(),
              preferences: MemoryPreferences(),
              fullBuild: true,
              nativeAllowedByConfig: true,
              nativeKilledByConfig: killed,
            )
            ..nativePlayer = noop
            ..embedPlayer = noop;
      expect(service(killed: false).playRoute, TwitchPlayRoute.native);
      expect(service(killed: true).playRoute, TwitchPlayRoute.embed);
      expect(
        service(killed: false).routeFor(subOnly: true),
        TwitchPlayRoute.openOnTwitch,
      );
    });
  });

  group('playback decision (TWI-004)', () {
    test('store builds never play natively', () {
      expect(
        decideTwitchRoute(
          fullBuild: false,
          nativeAllowedByConfig: true,
          hasNativePlayer: true,
          hasEmbedPlayer: false,
        ),
        TwitchPlayRoute.openOnTwitch,
      );
      expect(
        decideTwitchRoute(
          fullBuild: false,
          nativeAllowedByConfig: true,
          hasNativePlayer: true,
          hasEmbedPlayer: true,
        ),
        TwitchPlayRoute.embed,
      );
    });

    test('full builds: native only while the config allows it', () {
      expect(
        decideTwitchRoute(
          fullBuild: true,
          nativeAllowedByConfig: true,
          hasNativePlayer: true,
          hasEmbedPlayer: true,
        ),
        TwitchPlayRoute.native,
      );
      expect(
        decideTwitchRoute(
          fullBuild: true,
          nativeAllowedByConfig: false,
          hasNativePlayer: true,
          hasEmbedPlayer: true,
        ),
        TwitchPlayRoute.embed,
      );
      expect(
        decideTwitchRoute(
          fullBuild: true,
          nativeAllowedByConfig: false,
          hasNativePlayer: true,
          hasEmbedPlayer: false,
        ),
        TwitchPlayRoute.openOnTwitch,
      );
    });
  });

  group('without a Twitch login', () {
    testWidgets('local favourites: add by name, kept across restarts', (
      t,
    ) async {
      setSurface(t, _desktop);
      final prefs = MemoryPreferences();
      // No client id: Helix and sign-in are out of reach.
      final helix = FakeTwitchBackend.demo(
        now: demoNow,
        signedIn: false,
        canSignIn: false,
      );
      final services = await demoServices(twitch: helix, preferences: prefs);
      await services.setShowTwitch(true);
      await t.pumpWidget(testApp(services, section: Section.twitch));
      await settle(t);
      expect(
        find.byKey(const ValueKey('twitch-favorites-empty')),
        findsOneWidget,
      );
      expect(find.byKey(const ValueKey('twitch-sign-in-hint')), findsOneWidget);
      expect(find.byKey(const ValueKey('twitch-sign-in-button')), findsNothing);

      await _tap(t, find.byKey(const ValueKey('twitch-tab-search')));
      await t.enterText(
        find.byKey(const ValueKey('twitch-search-field')),
        'PixelHeron',
      );
      await settle(t, 10);
      await _tap(t, find.byKey(const ValueKey('twitch-add-by-name')));
      expect(services.twitch.isFavorite('pixelheron'), isTrue);
      expect(prefs.values[Preferences.twitchFavorites], contains('pixelheron'));
      // Searching again finds the favourite locally.
      expect(
        find.byKey(const ValueKey('twitch-result-pixelheron')),
        findsOneWidget,
      );
      expect(helix.calls, isNot(contains('users')), reason: 'no Helix');

      await _tap(t, find.byKey(const ValueKey('twitch-tab-home')));
      expect(
        find.byKey(const ValueKey('twitch-favorite-pixelheron')),
        findsOneWidget,
      );
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);

      // A new start with the same preferences keeps the favourite.
      final again = await demoServices(
        twitch: FakeTwitchBackend.demo(
          now: demoNow,
          signedIn: false,
          canSignIn: false,
        ),
        preferences: prefs,
      );
      expect(again.twitch.favorites.map((f) => f.login), ['pixelheron']);
      expect(again.twitch.favorites.single.userId, isNull);
    });

    testWidgets('remove from favourites on the channel page', (t) async {
      setSurface(t, _desktop);
      final services = await demoServices(twitch: const LocalTwitchBackend());
      await services.setShowTwitch(true);
      await services.twitch.addFavoriteByName('twitch.tv/QuietKettle');
      await t.pumpWidget(testApp(services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-favorite-quietkettle')));
      expect(
        find.byKey(const ValueKey('twitch-channel-page-quietkettle')),
        findsOneWidget,
      );
      expect(find.byKey(const ValueKey('twitch-attribution')), findsWidgets);
      await _tap(t, find.byKey(const ValueKey('twitch-channel-favorite')));
      expect(services.twitch.favorites, isEmpty);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    test('a favourite added by name is resolved after sign-in', () async {
      final helix = FakeTwitchBackend.demo(now: demoNow, signedIn: false);
      final twitch = TwitchService(
        backend: helix,
        preferences: MemoryPreferences(),
        clock: () => demoNow,
      );
      await twitch.init();
      expect(await twitch.addFavoriteByName('lunaforge'), isTrue);
      await twitch.refresh();
      expect(twitch.favorites.single.userId, isNull);
      expect(twitch.liveKnown, isFalse);

      await twitch.startLogin();
      expect(await twitch.pollLogin(), isNotNull);
      await twitch.refresh();
      final f = twitch.favorites.single;
      expect(f.userId, '7002');
      expect(f.displayName, 'LunaForge');
      expect(twitch.liveKnown, isTrue);
      expect(twitch.favoriteEntries().single.isLive, isTrue);
      // Mirrored into the library once resolved (synced between devices).
      await pumpEventQueue();
      expect(helix.synced['7002'], isTrue);
    });
  });

  group('signed in', () {
    testWidgets(
      'Home rows: live, favourites, continue, new, soon, categories',
      (t) async {
        setSurface(t, _desktop);
        final services = await twitchServices();
        await t.pumpWidget(testApp(services, section: Section.twitch));
        await settle(t);
        for (final key in [
          'twitch-row-live',
          'twitch-row-favorites',
          'twitch-row-continue',
          'twitch-row-new',
        ]) {
          expect(find.byKey(ValueKey(key)), findsOneWidget, reason: key);
        }
        // The pinned favourite comes first, live ones carry the ring.
        expect(
          services.twitch.favoriteEntries().first.channel.login,
          'quietkettle',
        );
        // An affiliate archive two days before deletion is marked.
        expect(find.textContaining('Deleted in'), findsWidgets);
        expect(find.byKey(const ValueKey('twitch-sign-in-hint')), findsNothing);
        await t.ensureVisible(find.byKey(const ValueKey('twitch-row-soon')));
        await settle(t, 2);
        await t.ensureVisible(
          find.byKey(const ValueKey('twitch-category-503')),
        );
        await settle(t, 2);
        await _tap(t, find.byKey(const ValueKey('twitch-category-503')));
        // VODs found by their chapters, not by the current category.
        expect(
          find.byKey(const ValueKey('twitch-category-page-503')),
          findsOneWidget,
        );
        // TWI-034: below the category's live streams.
        await t.scrollUntilVisible(
          find.byKey(const ValueKey('twitch-category-vod-9101')),
          400,
          scrollable: find
              .descendant(
                of: find.byKey(const ValueKey('twitch-category-page-503')),
                matching: find.byType(Scrollable),
              )
              .first,
        );
        await settle(t, 2);
        expect(
          find.byKey(const ValueKey('twitch-category-vod-9101')),
          findsOneWidget,
        );
        await t.pumpWidget(const SizedBox());
        await settle(t, 2);
      },
    );

    testWidgets('sort menu changes the order and is saved', (t) async {
      setSurface(t, _desktop);
      final services = await twitchServices();
      await t.pumpWidget(testApp(services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-tab-channels')));
      await _tap(t, find.byKey(const ValueKey('twitch-sort')));
      await _tap(t, find.byKey(const ValueKey('twitch-sort-name')));
      expect(services.twitch.sort, TwitchFavoriteSort.name);
      expect(
        await services.preferences.read(Preferences.twitchFavoritesSort),
        'name',
      );
      final y = {
        for (final l in ['bytelantern', 'lunaforge', 'novamarsh', 'pixelheron'])
          l: t.getTopLeft(find.byKey(ValueKey('twitch-channel-$l'))).dy,
      };
      expect(y['bytelantern']! < y['lunaforge']!, isTrue);
      expect(y['novamarsh']! < y['pixelheron']!, isTrue);
      // Filter: live only hides the offline QuietKettle.
      await _tap(t, find.byKey(const ValueKey('twitch-filter-live')));
      expect(
        find.byKey(const ValueKey('twitch-channel-quietkettle')),
        findsNothing,
      );
      expect(
        find.byKey(const ValueKey('twitch-channel-pixelheron')),
        findsOneWidget,
      );
      // Pinning moves a channel to the top.
      await _tap(t, find.byKey(const ValueKey('twitch-filter-all')));
      await _tap(t, find.byKey(const ValueKey('twitch-pin-pixelheron')));
      expect(
        services.twitch.favoriteEntries().map((e) => e.channel.login).take(2),
        ['pixelheron', 'quietkettle'],
      );
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('sub-only VOD: lock and "Watch on Twitch" link', (t) async {
      setSurface(t, _desktop);
      final services = await twitchServices();
      await t.pumpWidget(testApp(services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-favorite-novamarsh')));
      expect(find.byIcon(Icons.lock_outline), findsOneWidget);
      await _tap(t, find.byKey(const ValueKey('twitch-vod-9104')));
      expect(find.byKey(const ValueKey('twitch-open-dialog')), findsOneWidget);
      expect(find.text('https://www.twitch.tv/videos/9104'), findsOneWidget);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('store build: "Watch live" offers Open on Twitch', (t) async {
      setSurface(t, _desktop);
      final services = await twitchServices();
      expect(services.twitch.playRoute, TwitchPlayRoute.openOnTwitch);
      await t.pumpWidget(testApp(services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-favorite-pixelheron')));
      await _tap(t, find.byKey(const ValueKey('twitch-watch-live')));
      expect(find.text('https://www.twitch.tv/pixelheron'), findsOneWidget);
      // A registered player (TWI-014 / TWI-005) takes over.
      await t.tap(find.text('Close'));
      await settle(t, 4);
      TwitchPlayTarget? played;
      services.twitch.embedPlayer = (_, target) async => played = target;
      await _tap(t, find.byKey(const ValueKey('twitch-watch-live')));
      expect(played?.login, 'pixelheron');
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('Following lists the follows with live status', (t) async {
      setSurface(t, _desktop);
      final services = await twitchServices();
      await t.pumpWidget(testApp(services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-tab-channels')));
      await _tap(t, find.byKey(const ValueKey('twitch-channels-following')));
      expect(
        find.byKey(const ValueKey('twitch-channel-embertide')),
        findsOneWidget,
      );
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('Device Code Flow sign-in dialog', (t) async {
      setSurface(t, _desktop);
      final helix = FakeTwitchBackend.demo(now: demoNow, signedIn: false);
      final services = await demoServices(twitch: helix);
      await services.setShowTwitch(true);
      await t.pumpWidget(
        testApp(
          services,
          section: Section.twitch,
          formFactor: FormFactor.desktop,
        ),
      );
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-sign-in-button')));
      expect(find.byKey(const ValueKey('twitch-user-code')), findsOneWidget);
      expect(find.text('KTWQ-PLMN'), findsOneWidget);
      await t.pump(const Duration(seconds: 6));
      await settle(t, 6);
      expect(find.byKey(const ValueKey('twitch-sign-in')), findsNothing);
      expect(services.twitch.account?.login, 'demo_viewer');
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  });

  group('Continue (Home hook)', () {
    test('started VODs, nothing while the section is hidden', () async {
      final services = await demoServices();
      final enabled = ValueNotifier(false);
      final rows = <List<ContinueEntry>>[];
      final sub = services.twitch
          .continueEntries(enabled: enabled)
          .listen(rows.add);
      await pumpEventQueue();
      expect(rows.last, isEmpty);
      enabled.value = true;
      await pumpEventQueue();
      expect(rows.last.map((e) => e.id), [
        'twitch:vod:9101',
        'twitch:vod:9105',
      ]);
      expect(rows.last.first.kind, ContinueKind.twitchVod);
      expect(rows.last.first.title, 'Campaign day 11: the long siege');
      await sub.cancel();
    });

    test('a VOD whose metadata expired keeps its progress', () async {
      final helix = FakeTwitchBackend(
        now: demoNow,
        progress: [
          TwitchProgressEntry(
            videoId: '1',
            progress: Progress(
              itemId: 'twitch:vod:1',
              positionMs: 1000,
              durationMs: 10000,
              updatedAt: demoNow,
            ),
          ),
        ],
      );
      final services = await demoServices(twitch: helix);
      final rows = await services.twitch.continueEntries().first;
      expect(rows.single.id, 'twitch:vod:1');
      expect(rows.single.progress.positionMs, 1000);
    });
  });
}
