// SPDX-License-Identifier: MPL-2.0
//
// Browse of the Streams section (TWI-034) over the fake Helix with made-up
// categories and channels: top categories page by page without duplicates,
// a category's live streams by viewers with the language filter, search
// with channels on air first and recent searches, signed-out, error and
// empty states, phone and TV layouts (D-pad without traps, focus back on
// the tile after the category page).

import 'package:flutter/gestures.dart' show kSecondaryButton;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_twitch.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/twitch/twitch_browse.dart';
import 'package:kinolune/screens/twitch/twitch_widgets.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/twitch/twitch_backend.dart';
import 'package:kinolune/twitch/twitch_models.dart';
import 'package:kinolune/twitch/twitch_pager.dart';
import 'package:kinolune/twitch/twitch_service.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

const _desktop = Size(1280, 800);
const _phone = Size(390, 844);
const _tv = Size(960, 540);

Future<void> _tap(WidgetTester t, Finder f) async {
  await t.ensureVisible(f.first);
  await settle(t, 4);
  await t.tap(f.first);
  await settle(t, 10);
}

Future<(AppServices, FakeTwitchBackend)> _open(
  WidgetTester t, {
  Size size = _desktop,
  FormFactor ff = FormFactor.desktop,
  bool signedIn = true,
  bool canSignIn = true,
  void Function(FakeTwitchBackend helix)? edit,
}) async {
  setSurface(t, size);
  final helix = FakeTwitchBackend.demo(
    now: demoNow,
    signedIn: signedIn,
    canSignIn: canSignIn,
  );
  edit?.call(helix);
  final services = await twitchServices(twitch: helix);
  await t.pumpWidget(
    testApp(services, formFactor: ff, section: Section.twitch),
  );
  await settle(t);
  await _tap(t, find.byKey(const ValueKey('twitch-tab-browse')));
  return (services, helix);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

/// Category tiles of Browse in grid order.
List<String> _tiles(WidgetTester t) => [
  for (final w in t.widgetList<TwitchCategoryCard>(
    find.byType(TwitchCategoryCard),
  ))
    w.category.id,
];

/// Live cards of the category page in grid order.
List<TwitchStream> _streams(WidgetTester t) => [
  for (final w in t.widgetList<TwitchLiveCard>(find.byType(TwitchLiveCard)))
    if (w.key case ValueKey<String>(:final value)
        when value.startsWith('twitch-category-stream-'))
      w.stream,
];

/// Drags the category page [pageKey] up [times] times.
Future<void> _scrollDown(WidgetTester t, String pageKey, int times) async {
  final list = find
      .descendant(
        of: find.byKey(ValueKey(pageKey)),
        matching: find.byType(Scrollable),
      )
      .first;
  for (var i = 0; i < times; i++) {
    await t.drag(list, const Offset(0, -700));
    await settle(t, 4);
  }
}

void main() {
  group('pager', () {
    test('floating pages: no duplicates, ends on the last page', () async {
      final helix = FakeTwitchBackend.demo(now: demoNow)..floatingPages = true;
      final pager = TwitchPager<TwitchCategory>(
        (c) => helix.topCategories(cursor: c, limit: 20),
        keyOf: (c) => c.id,
      );
      while (pager.hasMore) {
        await pager.loadMore();
      }
      final ids = [for (final c in pager.items) c.id];
      expect(ids.length, helix.top.length);
      expect(ids.toSet().length, ids.length);
      expect(ids, [for (final c in helix.top) c.id]);
      expect(helix.calls.where((c) => c == 'games_top').length, 3);
    });

    test('a repeated cursor ends the list; an error is kept', () async {
      var calls = 0;
      final pager = TwitchPager<String>((cursor) async {
        calls++;
        if (calls == 3) {
          throw const TwitchUnavailable(TwitchUnavailableReason.network);
        }
        return TwitchPage(['a$calls'], 'same');
      }, keyOf: (s) => s);
      await pager.loadMore();
      await pager.loadMore();
      expect(pager.items, ['a1', 'a2']);
      expect(pager.hasMore, isFalse);
      // reset: a fresh list; the failure stays for "Retry".
      await pager.reset();
      expect(pager.items, isEmpty);
      expect(pager.error, isA<TwitchUnavailable>());
      expect(pager.initial, isTrue);
    });

    test('order by viewers across pages, equal ones keep arrival', () async {
      final pages = [
        TwitchPage([_s('a', 10), _s('b', 5)], '1'),
        TwitchPage([_s('c', 50), _s('d', 5)]),
      ];
      var i = 0;
      final pager = TwitchPager<TwitchStream>(
        (_) async => pages[i++],
        keyOf: (s) => s.userId,
        compare: (a, b) => b.viewers.compareTo(a.viewers),
      );
      await pager.loadMore();
      await pager.loadMore();
      expect([for (final s in pager.items) s.userId], ['c', 'a', 'b', 'd']);
    });
  });

  group('service', () {
    test('search puts channels on air first, by viewers', () async {
      final helix = FakeTwitchBackend.demo(now: demoNow);
      final twitch = TwitchService(
        backend: helix,
        preferences: MemoryPreferences(),
        clock: () => demoNow,
      );
      await twitch.init();
      final r = await twitch.search('r');
      expect(
        [for (final c in r.channels) c.login],
        ['pixelheron', 'novamarsh', 'lunaforge', 'bytelantern', 'embertide'],
      );
      expect(twitch.liveOf(r.channels.first)!.viewers, 18400);
      expect(twitch.liveOf(r.channels.last), isNull);
      expect(r.categories.map((c) => c.name), contains('Neon Drift'));
    });

    test(
      'recent searches: newest first, no repeats, at most 8, kept',
      () async {
        final prefs = MemoryPreferences();
        final twitch = TwitchService(
          backend: FakeTwitchBackend.demo(now: demoNow),
          preferences: prefs,
        );
        await twitch.init();
        for (var i = 0; i < 10; i++) {
          await twitch.rememberSearch('query $i');
        }
        await twitch.rememberSearch('QUERY 5');
        await twitch.rememberSearch('x'); // too short
        expect(twitch.recentSearches.first, 'QUERY 5');
        expect(twitch.recentSearches.length, TwitchService.maxRecentSearches);
        expect(
          twitch.recentSearches.where((q) => q.toLowerCase() == 'query 5'),
          hasLength(1),
        );
        final again = TwitchService(
          backend: FakeTwitchBackend.demo(now: demoNow),
          preferences: prefs,
        );
        await again.init();
        expect(again.recentSearches, twitch.recentSearches);
        await again.clearRecentSearches();
        expect(again.recentSearches, isEmpty);
      },
    );

    test('browse language: UI language by default, "all" is kept', () async {
      final prefs = MemoryPreferences();
      final twitch = TwitchService(
        backend: FakeTwitchBackend.demo(now: demoNow),
        preferences: prefs,
      );
      await twitch.init();
      expect(twitch.browseLanguage('uk'), 'uk');
      await twitch.setBrowseLanguage(null, uiLanguage: 'uk');
      expect(twitch.browseLanguage('uk'), isNull);
      expect(prefs.values[Preferences.twitchBrowseLanguage], '*');
      await twitch.setBrowseLanguage('de', uiLanguage: 'uk');
      expect(twitch.browseLanguage('uk'), 'de');
      // Choosing the UI language again follows the UI language.
      await twitch.setBrowseLanguage('uk', uiLanguage: 'uk');
      expect(prefs.values.containsKey(Preferences.twitchBrowseLanguage), false);
      expect(twitch.browseLanguage('ru'), 'ru');
    });
  });

  group('desktop', () {
    testWidgets('browse → category → stream; pages without duplicates', (
      t,
    ) async {
      final (services, helix) = await _open(
        t,
        edit: (h) => h.floatingPages = true,
      );
      final played = <String?>[];
      services.twitch.embedPlayer = (_, target) async =>
          played.add(target.login);
      expect(find.byKey(const ValueKey('twitch-browse')), findsOneWidget);
      expect(_tiles(t).length, TwitchService.categoriesPageSize);
      // Scrolling loads the next page; the repeated item is dropped.
      for (var i = 0; i < 12 && _tiles(t).length < helix.top.length; i++) {
        await t.drag(find.byType(CustomScrollView), const Offset(0, -900));
        await settle(t, 4);
      }
      final ids = _tiles(t);
      expect(ids.length, helix.top.length);
      expect(ids.toSet().length, ids.length);
      expect(find.byKey(const ValueKey('twitch-list-more')), findsNothing);
      expect(find.text('#1 on Twitch'), findsOneWidget);

      // A category: its live streams, most viewers first, in the UI
      // language (English in tests).
      await t.drag(find.byType(CustomScrollView), const Offset(0, 20000));
      await settle(t, 4);
      await _tap(t, find.byKey(const ValueKey('twitch-top-category-504')));
      expect(
        find.byKey(const ValueKey('twitch-category-page-504')),
        findsOneWidget,
      );
      expect(helix.streamQueries.last, '504:en:-');
      final shown = _streams(t);
      expect(shown, isNotEmpty);
      expect(shown.every((s) => s.language == 'en'), isTrue);
      for (var i = 1; i < shown.length; i++) {
        expect(shown[i - 1].viewers, greaterThanOrEqualTo(shown[i].viewers));
      }
      expect(shown.first.login, 'novamarsh');
      // Watch the top stream.
      await _tap(
        t,
        find.byKey(const ValueKey('twitch-category-stream-novamarsh')),
      );
      expect(played, ['novamarsh']);
      await _close(t);
    });

    testWidgets('language filter: all languages in one tap, kept', (t) async {
      final (services, helix) = await _open(t);
      await _tap(t, find.byKey(const ValueKey('twitch-top-category-501')));
      // madeup_0_0 streams in Ukrainian: hidden in English.
      expect(
        find.byKey(const ValueKey('twitch-category-stream-madeup_0_0')),
        findsNothing,
      );
      await _tap(t, find.byKey(const ValueKey('twitch-language-all')));
      expect(helix.streamQueries.last, '501:*:-');
      expect(
        find.byKey(const ValueKey('twitch-category-stream-madeup_0_0')),
        findsOneWidget,
      );
      // The language shows on the cards when all are listed.
      expect(find.textContaining('Українська'), findsWidgets);
      expect(services.twitch.browseLanguage('en'), isNull);
      // Another language from the menu, with nothing on air in it.
      await _tap(t, find.byKey(const ValueKey('twitch-language-other')));
      await _tap(t, find.byKey(const ValueKey('twitch-language-pick-ko')));
      expect(helix.streamQueries.last, '501:ko:-');
      expect(
        find.byKey(const ValueKey('twitch-category-streams-empty')),
        findsOneWidget,
      );
      await _tap(t, find.byKey(const ValueKey('twitch-category-streams-all')));
      expect(_streams(t), isNotEmpty);
      await _close(t);
    });

    testWidgets('category streams page by cursor while scrolling', (t) async {
      const cat = TwitchCategory(id: '900', name: 'Made-up Big Game');
      final many = [
        for (var i = 0; i < 70; i++)
          TwitchStream(
            userId: '9${i.toString().padLeft(3, '0')}',
            login: 'big_$i',
            displayName: 'Big_$i',
            title: 'Made-up $i',
            // Not in order: the page sorts.
            viewers: (i * 37) % 101 + 1,
            gameId: '900',
            gameName: cat.name,
            language: 'en',
          ),
      ];
      final (_, helix) = await _open(
        t,
        edit: (h) {
          h.top.clear();
          h.top.add(cat);
          h.categoryLive['900'] = many;
          h.floatingPages = true;
        },
      );
      await _tap(t, find.byKey(const ValueKey('twitch-top-category-900')));
      expect(_streams(t).length, TwitchService.streamsPageSize);
      await _scrollDown(t, 'twitch-category-page-900', 30);
      final shown = _streams(t);
      expect(shown.length, 70);
      expect({for (final s in shown) s.userId}.length, 70);
      for (var i = 1; i < shown.length; i++) {
        expect(shown[i - 1].viewers, greaterThanOrEqualTo(shown[i].viewers));
      }
      expect(helix.streamQueries, ['900:en:-', '900:en:30', '900:en:60']);
      await _close(t);
    });

    testWidgets('stream menu: open the channel, add to favourites', (t) async {
      final (services, _) = await _open(t);
      await _tap(t, find.byKey(const ValueKey('twitch-top-category-501')));
      await _tap(t, find.byKey(const ValueKey('twitch-language-all')));
      final card = find.byKey(
        const ValueKey('twitch-category-stream-madeup_0_1'),
      );
      await t.ensureVisible(card);
      await settle(t, 2);
      await t.tap(card, buttons: kSecondaryButton);
      await settle(t, 6);
      await _tap(t, find.byKey(const ValueKey('twitch-stream-menu-favorite')));
      expect(services.twitch.isFavorite('madeup_0_1'), isTrue);
      await t.tap(card, buttons: kSecondaryButton);
      await settle(t, 6);
      await _tap(t, find.byKey(const ValueKey('twitch-stream-menu-channel')));
      expect(find.text('MadeUp_0_1'), findsWidgets);
      await _close(t);
    });

    testWidgets('search: channels on air first, categories, recent', (t) async {
      final (services, _) = await _open(t);
      await _tap(t, find.byKey(const ValueKey('twitch-tab-search')));
      await t.enterText(find.byKey(const ValueKey('twitch-search-field')), 'r');
      await settle(t, 10);
      final order = [
        for (final w in t.widgetList(
          find.byWidgetPredicate(
            (w) =>
                w.key is ValueKey<String> &&
                (w.key! as ValueKey<String>).value.startsWith(
                  'twitch-result-',
                ) &&
                !(w.key! as ValueKey<String>).value.startsWith(
                  'twitch-result-categor',
                ),
          ),
        ))
          (w.key! as ValueKey<String>).value.substring('twitch-result-'.length),
      ];
      expect(order.first, 'pixelheron');
      expect(order.last, 'embertide');
      expect(
        find.byKey(const ValueKey('twitch-search-watch-pixelheron')),
        findsOne,
      );
      expect(
        find.byKey(const ValueKey('twitch-search-watch-embertide')),
        findsNothing,
      );
      expect(
        find.byKey(const ValueKey('twitch-result-category-504')),
        findsOneWidget,
      );
      // One letter is too short for the recent searches.
      await t.testTextInput.receiveAction(TextInputAction.search);
      await settle(t, 6);
      expect(services.twitch.recentSearches, isEmpty);
      await t.enterText(
        find.byKey(const ValueKey('twitch-search-field')),
        'neon',
      );
      await settle(t, 10);
      await t.testTextInput.receiveAction(TextInputAction.search);
      await settle(t, 6);
      expect(services.twitch.recentSearches, ['neon']);
      await _tap(t, find.byKey(const ValueKey('twitch-result-category-504')));
      expect(
        find.byKey(const ValueKey('twitch-category-page-504')),
        findsOneWidget,
      );
      expect(services.twitch.recentSearches, ['neon']);
      await _tap(t, find.byIcon(Icons.arrow_back).last);
      await t.enterText(find.byKey(const ValueKey('twitch-search-field')), '');
      await settle(t, 6);
      expect(find.byKey(const ValueKey('twitch-search-recent')), findsOne);
      await _tap(t, find.byKey(const ValueKey('twitch-search-recent-neon')));
      expect(
        find.byKey(const ValueKey('twitch-result-category-504')),
        findsOneWidget,
      );
      await _close(t);
    });

    testWidgets('signed out: explanation and "Sign in"', (t) async {
      final (services, helix) = await _open(t, signedIn: false);
      expect(find.byKey(const ValueKey('twitch-browse-sign-in')), findsOne);
      expect(
        find.byKey(const ValueKey('twitch-browse-sign-in-button')),
        findsOne,
      );
      expect(find.byType(TwitchCategoryCard), findsNothing);
      expect(helix.calls, isNot(contains('games_top')));
      // Once signed in, the categories come without leaving the tab.
      await services.twitch.startLogin();
      await services.twitch.pollLogin();
      await settle(t, 10);
      expect(find.byKey(const ValueKey('twitch-browse-sign-in')), findsNothing);
      expect(_tiles(t), isNotEmpty);
      await _close(t);
    });

    testWidgets('no client id: says so, no sign-in button', (t) async {
      await _open(t, signedIn: false, canSignIn: false);
      expect(find.byKey(const ValueKey('twitch-browse-sign-in')), findsOne);
      expect(
        find.byKey(const ValueKey('twitch-browse-sign-in-button')),
        findsNothing,
      );
      await _close(t);
    });

    testWidgets('offline: error with Retry, then the grid', (t) async {
      final (_, helix) = await _open(
        t,
        edit: (h) => h.discoveryError = const TwitchUnavailable(
          TwitchUnavailableReason.network,
        ),
      );
      expect(find.byKey(const ValueKey('twitch-list-error')), findsOne);
      expect(find.textContaining('No connection to Twitch'), findsOne);
      helix.discoveryError = null;
      await _tap(t, find.byKey(const ValueKey('twitch-list-retry')));
      expect(find.byKey(const ValueKey('twitch-browse')), findsOne);
      expect(_tiles(t), isNotEmpty);
      await _close(t);
    });

    testWidgets('empty top list', (t) async {
      setSurface(t, _desktop);
      final helix = FakeTwitchBackend(now: demoNow, top: const []);
      final services = await twitchServices(twitch: helix);
      await t.pumpWidget(testApp(services, section: Section.twitch));
      await settle(t);
      await _tap(t, find.byKey(const ValueKey('twitch-tab-browse')));
      expect(find.byKey(const ValueKey('twitch-browse-empty')), findsOne);
      await _close(t);
    });
  });

  testWidgets('phone: browse → category without overflow', (t) async {
    final (_, helix) = await _open(t, size: _phone, ff: FormFactor.phone);
    expect(_tiles(t), isNotEmpty);
    await _tap(t, find.byKey(const ValueKey('twitch-top-category-502')));
    expect(find.byKey(const ValueKey('twitch-category-page-502')), findsOne);
    expect(_streams(t), isNotEmpty);
    expect(helix.streamQueries.last, '502:en:-');
    expect(t.takeException(), isNull);
    await _close(t);
  });

  testWidgets('TV: D-pad walks the grid to the next page, focus returns', (
    t,
  ) async {
    final (_, helix) = await _open(t, size: _tv, ff: FormFactor.tv);
    final first = find.byKey(const ValueKey('twitch-top-category-501'));
    Focus.of(
      t.element(find.descendant(of: first, matching: find.byType(AspectRatio))),
    ).requestFocus();
    await settle(t, 4);
    String? focusedTile() {
      final ctx = FocusManager.instance.primaryFocus?.context;
      if (ctx == null) return null;
      String? id;
      ctx.visitAncestorElements((e) {
        final k = e.widget.key;
        if (k is ValueKey<String> &&
            k.value.startsWith('twitch-top-category-')) {
          id = k.value.substring('twitch-top-category-'.length);
          return false;
        }
        if (k == const ValueKey('twitch-list-more')) {
          id = 'more';
          return false;
        }
        return true;
      });
      return id;
    }

    expect(focusedTile(), '501');
    await pressKey(t, LogicalKeyboardKey.arrowRight);
    expect(focusedTile(), '502');
    final seen = <String>[];
    for (var i = 0; i < 40; i++) {
      final before = focusedTile();
      await pressKey(t, LogicalKeyboardKey.arrowDown);
      final now = focusedTile();
      expect(now, isNotNull, reason: 'focus left the grid after $before');
      if (now == before) break; // the last row
      seen.add(now!);
    }
    // Down through the rows reached the second page (scrolling loads it).
    expect(
      seen.any((id) => id != 'more' && int.parse(id) > 540),
      isTrue,
      reason: '$seen',
    );
    expect(helix.calls.where((c) => c == 'games_top').length, 2);
    // OK opens the category; Back returns with the focus on the tile.
    final tile = focusedTile()!;
    await pressKey(t, LogicalKeyboardKey.select);
    await settle(t, 6);
    expect(find.byKey(ValueKey('twitch-category-page-$tile')), findsOne);
    await pressKey(t, LogicalKeyboardKey.escape);
    await settle(t, 6);
    expect(find.byKey(ValueKey('twitch-category-page-$tile')), findsNothing);
    expect(focusedTile(), tile);
    await _close(t);
  });
}

TwitchStream _s(String id, int viewers) => TwitchStream(
  userId: id,
  login: id,
  displayName: id,
  title: 't',
  viewers: viewers,
);
