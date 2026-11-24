// SPDX-License-Identifier: MPL-2.0
//
// One search for everything (UI-045): groups from every source, «Все»,
// locked personal media never in the results, recent queries per profile,
// the TV row (voice, phone input) and its focus, Ctrl+K and «/».

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_personal_media.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/platform/voice_input.dart';
import 'package:kinolune/screens/search_model.dart';
import 'package:kinolune/screens/search_screen.dart';
import 'package:kinolune/screens/search_sources.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

LibraryItem _item(String id) =>
    LibraryItem(id: id, kind: ItemKind.movie, title: id, filePath: '/m/$id');

SearchSource _source(
  SearchSection section,
  FutureOr<List<Object>> Function(String q) found, {
  Duration delay = Duration.zero,
}) => SearchSource(
  sections: {section},
  delay: delay,
  search: (q, limit) async => [cutHits(section, await found(q), limit)],
);

class _FakeVoice extends VoiceInput {
  _FakeVoice(this.text);

  final String? text;
  final asked = <String?>[];

  @override
  Future<bool> available() async => true;

  @override
  Future<String?> listen({String? language, String? prompt}) async {
    asked.add(language);
    return text;
  }
}

TextField _field(WidgetTester t) => t.widget<TextField>(
  find
      .descendant(
        of: find.byType(SearchScreen),
        matching: find.byType(TextField),
      )
      .first,
);

/// Whether the primary focus is inside the widget [f] finds.
bool _focusIn(Finder f) {
  final ctx = FocusManager.instance.primaryFocus?.context;
  if (ctx == null) return false;
  final target = f.evaluate().single;
  var found = identical(ctx, target);
  ctx.visitAncestorElements((e) {
    if (identical(e, target)) found = true;
    return !found;
  });
  return found;
}

void main() {
  group('UnifiedSearch', () {
    testWidgets('groups of every source in screen order; a slow source '
        'joins later', (t) async {
      final search = UnifiedSearch([
        _source(
          SearchSection.channels,
          (q) => ['channel-$q'],
          delay: const Duration(milliseconds: 300),
        ),
        _source(SearchSection.personal, (q) => ['video-$q']),
        _source(SearchSection.movies, (q) => [_item('m-$q')]),
      ]);
      addTearDown(search.dispose);

      search.update('dar');
      await t.pump(searchDebounce);
      await t.pump();
      expect(search.groups.map((g) => g.section), [
        SearchSection.movies,
        SearchSection.personal,
      ]);
      expect(search.busy, isTrue);
      await t.pump(const Duration(milliseconds: 300));
      expect(search.groups.map((g) => g.section), [
        SearchSection.movies,
        SearchSection.personal,
        SearchSection.channels,
      ]);
      expect(search.busy, isFalse);
    });

    testWidgets('a new query drops the results and slow answers of the old '
        'one', (t) async {
      final search = UnifiedSearch([
        _source(
          SearchSection.channels,
          (q) => ['channel-$q'],
          delay: const Duration(milliseconds: 300),
        ),
        _source(SearchSection.movies, (q) => [_item('m-$q')]),
      ]);
      addTearDown(search.dispose);
      search.update('ab');
      await t.pump(searchDebounce);
      search.update('xyz');
      await t.pump(searchDebounce);
      await t.pump(const Duration(milliseconds: 400));
      expect(search.query, 'xyz');
      for (final g in search.groups) {
        expect(g.items.toString(), contains('xyz'));
      }
    });

    testWidgets('an error only when every source failed', (t) async {
      final ok = UnifiedSearch([
        _source(SearchSection.movies, (q) => throw StateError('core')),
        _source(SearchSection.personal, (q) => const []),
      ]);
      addTearDown(ok.dispose);
      await ok.submit('zzz');
      expect(ok.error, isNull);
      expect(ok.isEmptyResult, isTrue);

      final failed = UnifiedSearch([
        _source(SearchSection.movies, (q) => throw StateError('core')),
      ]);
      addTearDown(failed.dispose);
      await failed.submit('zzz');
      expect(failed.error, isA<StateError>());
    });

    testWidgets('a full group says it has more; «Все» asks for the rest', (
      t,
    ) async {
      final search = UnifiedSearch([
        _source(SearchSection.movies, (q) => [for (var i = 0; i < 30; i++) _item('m$i')]),
      ], perGroup: 5);
      addTearDown(search.dispose);
      await search.submit('m');
      final movies = search.groups.single;
      expect(movies.items, hasLength(5));
      expect(movies.more, isTrue);
      final all = await search.all(SearchSection.movies, 'm');
      expect(all!.items, hasLength(30));
      expect(all.more, isFalse);
    });
  });

  group('sources', () {
    test('personal media: a locked library is never asked', () async {
      final personal = FakePersonalMedia.demo();
      final source = personalSearchSource(personal);
      // «Семья» is locked (PIN), «Домашнее видео» is open.
      expect((await source.search('Дача', 20)).single.items, isEmpty);
      final open = (await source.search('аквариум', 20)).single;
      expect(open.items.cast<PersonalHit>().map((h) => h.video.title), [
        'Аквариум',
      ]);

      final family = (await personal.libraries()).firstWhere(
        (l) => l.name == 'Семья',
      );
      await personal.unlock(family.id, '2468');
      final found = (await source.search('Дача', 20)).single;
      expect(found.items.cast<PersonalHit>().single.library.name, 'Семья');

      await personal.lockAll();
      expect((await source.search('Дача', 20)).single.items, isEmpty);
    });

    test('words match in any order, starts-with first', () {
      final words = searchWords('  Static  velvet ');
      expect(words, ['static', 'velvet']);
      expect(matchesWords(words, ['Velvet Static']), isTrue);
      expect(matchesWords(words, ['Velvet', null, 'Static Noise']), isTrue);
      expect(matchesWords(words, ['Velvet Underground']), isFalse);
      expect(
        startsFirst(['Old Moon', 'Moonlight', 'A moon'], 'moon', (s) => s),
        ['Moonlight', 'Old Moon', 'A moon'],
      );
    });

    test('recent queries are kept per profile', () async {
      final prefs = MemoryPreferences();
      final a = SearchHistory(prefs, key: Preferences.profileSearchHistory('a'));
      final b = SearchHistory(prefs, key: Preferences.profileSearchHistory('b'));
      await a.add('Dune');
      await b.load();
      expect(b.value, isEmpty);
      await b.add('Bluey');
      final again = SearchHistory(
        prefs,
        key: Preferences.profileSearchHistory('a'),
      );
      await again.load();
      expect(again.value, ['Dune']);
      await again.clear();
      await b.load();
      expect(b.value, ['Bluey']);
    });
  });

  group('screen', () {
    setUpAll(loadAppFonts);
    tearDown(() => VoiceInput.instance = const ChannelVoiceInput());

    testWidgets('music artists and albums are groups of their own', (t) async {
      setSurface(t, const Size(1280, 800));
      await t.pumpWidget(
        testApp(await demoServices(), section: Section.search),
      );
      await settle(t);
      await t.enterText(find.byType(TextField), 'velvet');
      await settle(t, 3);
      expect(find.byKey(const ValueKey('search-artists')), findsOneWidget);
      expect(find.byKey(const ValueKey('search-albums')), findsOneWidget);
      expect(find.text('Velvet Static'), findsWidgets);
    });

    testWidgets('locked personal media never shows up; open media does', (
      t,
    ) async {
      setSurface(t, const Size(1280, 800));
      final personal = FakePersonalMedia.demo();
      await t.pumpWidget(
        testApp(
          await demoServices(personal: personal),
          section: Section.search,
        ),
      );
      await settle(t);
      await t.enterText(find.byType(TextField), 'Дача');
      await settle(t, 3);
      expect(find.text('Дача, июль'), findsNothing);
      expect(find.byKey(const ValueKey('search-personal')), findsNothing);
      expect(find.text('No results for “Дача”'), findsOneWidget);

      await t.enterText(find.byType(TextField), 'Камин');
      await settle(t, 3);
      expect(find.byKey(const ValueKey('search-personal')), findsOneWidget);
      expect(find.text('Personal media'), findsOneWidget);
      expect(find.text('Камин, 1 час'), findsOneWidget);
    });

    testWidgets('«Все» opens every result of a group', (t) async {
      setSurface(t, const Size(1280, 800));
      await t.pumpWidget(
        testApp(await demoServices(), section: Section.search),
      );
      await settle(t);
      // "e" is in more episode titles than a group shows.
      await t.enterText(find.byType(TextField), 'e');
      await settle(t, 3);
      final seeAll = find.descendant(
        of: find.byKey(const ValueKey('search-episodes')),
        matching: find.text('See all'),
      );
      expect(seeAll, findsOneWidget);
      await t.tap(seeAll);
      await settle(t, 6);
      expect(
        find.byKey(const ValueKey('search-all-episodes')),
        findsOneWidget,
      );
      expect(find.byType(SearchAllScreen), findsOneWidget);
    });

    testWidgets('TV: voice and phone buttons beside the field, → reaches '
        'them, ↓ goes to the results, voice fills the field', (t) async {
      setSurface(t, const Size(1920, 1080));
      final voice = _FakeVoice('moonlit');
      VoiceInput.instance = voice;
      final prefs = MemoryPreferences();
      final s = await demoServices(preferences: prefs);
      await t.pumpWidget(
        testApp(s, formFactor: FormFactor.tv, section: Section.search),
      );
      await settle(t);
      final mic = find.byKey(const ValueKey('search-voice'));
      final phone = find.byKey(const ValueKey('search-phone'));
      expect(mic, findsOneWidget);
      expect(phone, findsOneWidget);

      // «Ввести с телефона» puts the focus in the field (UI-027 asks the
      // phones for it).
      await t.tap(phone);
      await settle(t, 2);
      expect(_field(t).focusNode!.hasFocus, isTrue);

      // At the end of the text → moves to the buttons.
      await pressKey(t, LogicalKeyboardKey.arrowRight);
      expect(_focusIn(mic), isTrue);

      // Voice: the recognized text is searched and remembered.
      await t.tap(mic);
      await settle(t, 3);
      expect(voice.asked.single, isNotNull);
      expect(_field(t).controller!.text, 'moonlit');
      expect(find.text('Moonlit Bureau'), findsWidgets);
      expect(prefs.values.values.any((v) => v.contains('moonlit')), isTrue);

      // ↓ from the field goes to the results, not to the buttons.
      _field(t).focusNode!.requestFocus();
      await settle(t, 2);
      await pressKey(t, LogicalKeyboardKey.arrowDown);
      expect(_field(t).focusNode!.hasFocus, isFalse);
      expect(_focusIn(mic), isFalse);
      expect(_focusIn(phone), isFalse);
      expect(
        _focusIn(find.byKey(const ValueKey('search-movies'))) ||
            _focusIn(find.byKey(const ValueKey('search-shows'))),
        isTrue,
      );
    });

    testWidgets('no voice button off the TV', (t) async {
      setSurface(t, const Size(1280, 800));
      VoiceInput.instance = _FakeVoice('x');
      await t.pumpWidget(
        testApp(await demoServices(), section: Section.search),
      );
      await settle(t);
      expect(find.byKey(const ValueKey('search-voice')), findsNothing);
      expect(find.byKey(const ValueKey('search-phone')), findsNothing);
    });

    testWidgets('Ctrl+K and «/» open Search with the focus in the field', (
      t,
    ) async {
      setSurface(t, const Size(1280, 800));
      await t.pumpWidget(testApp(await demoServices()));
      await settle(t);
      final shell = t.state<AppShellState>(find.byType(AppShell));
      expect(shell.section, Section.home);

      await t.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await t.sendKeyEvent(LogicalKeyboardKey.keyK);
      await t.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await settle(t, 4);
      expect(shell.section, Section.search);
      expect(_field(t).focusNode!.hasFocus, isTrue);

      // In the field «/» is just a character.
      await t.enterText(find.byType(TextField), 'a/b');
      await t.sendKeyEvent(LogicalKeyboardKey.slash, character: '/');
      await settle(t, 2);
      expect(_field(t).focusNode!.hasFocus, isTrue);

      // From another section «/» opens Search.
      shell.select(Section.home);
      await settle(t, 4);
      FocusManager.instance.primaryFocus?.unfocus();
      await settle(t, 2);
      await t.sendKeyEvent(LogicalKeyboardKey.slash, character: '/');
      await settle(t, 4);
      expect(shell.section, Section.search);
      expect(_field(t).focusNode!.hasFocus, isTrue);
    });
  });
}
