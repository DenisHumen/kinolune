// SPDX-License-Identifier: MPL-2.0
//
// Show Detail (UI-010): golden images on the four densities, initial focus
// on "Resume S2 · E1", season tabs with Specials last, the episode order
// (TMDB → AniDB rebuilds the episodes; absolute), Hide Spoilers and the v0
// fallback. The goldens use the default test font, like
// test/movie_detail_test.dart. Refresh deliberately:
//   flutter test --update-goldens test/show_detail_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/show_detail_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/widgets/focusable.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final _now = DateTime(2026, 11, 14, 20);

const _showId = 's-kaiju';

final _show = LibraryItem(
  id: _showId,
  kind: ItemKind.show,
  title: 'Kaiju Patrol',
  year: 2024,
  overview: 'A city defense corps keeps monsters out of the bay.',
  filePath: '/media/Shows/Kaiju Patrol',
  addedAt: _now,
  matchConfidence: .96,
);

LibraryItem _ep(int s, int e, String title) => LibraryItem(
  id: 'e-$s-$e',
  kind: ItemKind.episode,
  title: title,
  showId: _showId,
  season: s,
  episode: e,
  overview: 'What happens in $title.',
  filePath:
      '/media/Shows/Kaiju Patrol/Season $s/Kaiju.Patrol.S0${s}E0$e.1080p.mkv',
  durationMs: 24 * 60000,
  addedAt: _now,
);

final _episodes = [
  _ep(1, 1, 'First Alarm'),
  _ep(1, 2, 'Tide Line'),
  _ep(1, 3, 'Deep Water'),
  _ep(2, 1, 'Second Wave'),
  _ep(2, 2, 'Breakwater'),
  _ep(0, 1, 'Recap Special'),
];

/// AniDB, as anime mappings number it: TMDB S1E3 opens the second AniDB
/// entry, which also holds TMDB season 2.
const _anidb = {
  'e-1-1': (1, 1),
  'e-1-2': (1, 2),
  'e-1-3': (2, 1),
  'e-2-1': (2, 2),
  'e-2-2': (2, 3),
  'e-0-1': (0, 1),
};

Progress _watched(String id, int hoursAgo) => Progress(
  itemId: id,
  positionMs: 24 * 60000,
  durationMs: 24 * 60000,
  updatedAt: _now.subtract(Duration(hours: hoursAgo)),
  watched: true,
);

/// S1 watched, S2E1 started 10 minutes in, S2E2 not started.
final _progress = [
  _watched('e-1-1', 30),
  _watched('e-1-2', 29),
  _watched('e-1-3', 28),
  Progress(
    itemId: 'e-2-1',
    positionMs: 10 * 60000,
    durationMs: 24 * 60000,
    updatedAt: _now.subtract(const Duration(minutes: 5)),
  ),
];

const _showDetails = ItemDetails(
  genres: ['Animation', 'Action'],
  ratings: [ExternalRating(source: 'tmdb', value: 8.1, votes: 900)],
  credits: [
    CreditPerson(name: 'Ann Example', character: 'Captain Oda'),
    CreditPerson(name: 'Bob Example', character: 'Rin'),
    CreditPerson(name: 'Carl Example', crew: true, job: 'Director'),
  ],
);

const _episodeTech = ItemDetails(
  tech: TechInfo(
    container: 'matroska,webm',
    sizeBytes: 1400000000,
    durationMs: 24 * 60000,
    videoCodec: 'hevc',
    width: 1920,
    height: 1080,
    bitDepth: 10,
    frameRate: 23.976,
    audioFormats: ['eac3'],
    audioLanguages: ['jpn', 'eng'],
    subtitleLanguages: ['eng', 'rus'],
  ),
);

Future<(AppServices, FakeLibrary)> _services({
  List<Progress> progress = const [],
  bool hideSpoilers = false,
  Preferences? preferences,
}) async {
  final lib = FakeLibrary.items(
    [_show, ..._episodes],
    progress: progress,
    details: {_showId: _showDetails, 'e-2-1': _episodeTech},
    anidb: _anidb,
  );
  final s = AppServices(
    library: lib,
    secureStore: MemorySecureStore(),
    preferences: preferences,
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    clock: () => _now,
  );
  await s.init();
  if (hideSpoilers) await s.setHideSpoilers(true);
  return (s, lib);
}

Future<void> _open(
  WidgetTester t,
  AppServices s, {
  FormFactor ff = FormFactor.desktop,
  Size size = const Size(1280, 800),
}) async {
  setSurface(t, size);
  await t.pumpWidget(testApp(s, formFactor: ff, section: Section.shows));
  await settle(t, 10);
  await t.tap(find.text('Kaiju Patrol').first);
  await settle(t, 10);
  expect(find.byType(ShowDetailScreen), findsOneWidget);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

KButton? _focusedButton() => FocusManager.instance.primaryFocus?.context
    ?.findAncestorWidgetOfExactType<KButton>();

String _playLabel(WidgetTester t) =>
    t.widget<KButton>(find.byKey(showPlayKey)).label;

/// Scrolls the page back to the hero.
Future<void> _scrollTop(WidgetTester t) async {
  final page = find
      .descendant(
        of: find.byType(ShowDetailScreen),
        matching: find.byType(Scrollable),
      )
      .first;
  t.state<ScrollableState>(page).position.jumpTo(0);
  await settle(t, 2);
}

Future<void> _pickOrder(WidgetTester t, EpisodeOrder o) async {
  await _scrollTop(t);
  await t.ensureVisible(find.byKey(showOrderKey));
  await settle(t, 2);
  await t.tap(find.byKey(showOrderKey));
  await settle(t, 6);
  await t.tap(find.byKey(ValueKey('show-order-${o.name}')));
  await settle(t, 8);
}

/// Episode ids of the cards on the page, in order.
List<String> _cards(WidgetTester t) => [
  for (final e in t.widgetList(
    find.byWidgetPredicate(
      (w) =>
          w.key is ValueKey<String> &&
          (w.key! as ValueKey<String>).value.startsWith('show-episode-e-'),
    ),
  ))
    (e.key! as ValueKey<String>).value.substring('show-episode-'.length),
];

Future<void> _tapSeason(WidgetTester t, int s) async {
  await t.ensureVisible(find.byKey(ValueKey('show-season-$s')));
  await settle(t, 2);
  await t.tap(find.byKey(ValueKey('show-season-$s')));
  await settle(t, 6);
}

/// A library without the core: only the [Library] interface (v0).
class _V0Library implements Library {
  _V0Library(this._f);
  final FakeLibrary _f;

  @override
  Future<List<MediaSource>> sources() => _f.sources();
  @override
  Future<MediaSource> addSource(MediaSource s) => _f.addSource(s);
  @override
  Future<void> removeSource(String id) => _f.removeSource(id);
  @override
  Stream<ScanProgress> scan({String? sourceId}) => _f.scan(sourceId: sourceId);
  @override
  Future<void> setTmdbApiKey(String? key) => _f.setTmdbApiKey(key);
  @override
  Future<String?> tmdbApiKey() => _f.tmdbApiKey();
  @override
  Future<void> setMetadataLanguage(String? language) =>
      _f.setMetadataLanguage(language);
  @override
  Stream<List<LibraryItem>> movies() => _f.movies();
  @override
  Stream<List<LibraryItem>> shows() => _f.shows();
  @override
  Stream<List<LibraryItem>> episodes(String showId) => _f.episodes(showId);
  @override
  Stream<List<LibraryItem>> tracks() => _f.tracks();
  @override
  Stream<List<(LibraryItem, Progress)>> continueWatching() =>
      _f.continueWatching();
  @override
  Stream<List<LibraryItem>> recentlyAdded({
    ItemKind kind = ItemKind.movie,
    int limit = 20,
  }) => _f.recentlyAdded(kind: kind, limit: limit);
  @override
  Future<LibraryItem?> item(String id) => _f.item(id);
  @override
  Future<Progress?> progress(String itemId) => _f.progress(itemId);
  @override
  Future<LibraryItem?> nextEpisode(String itemId) => _f.nextEpisode(itemId);
  @override
  Future<List<LibraryItem>> search(String query, {int limit = 100}) =>
      _f.search(query, limit: limit);
  @override
  Future<List<SearchGroup>> searchGrouped(
    String query, {
    int perGroup = searchPerGroupDefault,
  }) => _f.searchGrouped(query, perGroup: perGroup);
  @override
  Future<void> saveProgress(Progress p) => _f.saveProgress(p);
  @override
  Future<void> fixMatch(
    String itemId, {
    int? tmdbId,
    String? title,
    int? year,
  }) => _f.fixMatch(itemId, tmdbId: tmdbId, title: title, year: year);
  @override
  Future<void> startWatching({
    Duration networkInterval = const Duration(minutes: 15),
    bool scanOnStart = true,
  }) async {}
  @override
  Future<void> stopWatching() async {}
  @override
  Future<void> close() => _f.close();
}

void main() {
  group('golden', () {
    final densities = <(String, FormFactor, Size)>[
      ('phone', FormFactor.phone, const Size(390, 844)),
      ('tablet', FormFactor.phone, const Size(820, 1180)),
      ('desktop', FormFactor.desktop, const Size(1280, 800)),
      ('tv', FormFactor.tv, const Size(960, 540)),
    ];
    for (final (name, ff, size) in densities) {
      testWidgets('show detail on $name', (t) async {
        final (s, _) = await _services(
          progress: _progress,
          hideSpoilers: true,
        );
        await _open(t, s, ff: ff, size: size);
        await expectLater(
          find.byKey(screenshotKey),
          matchesGoldenFile('goldens/show_detail_$name.png'),
        );
        await _close(t);
      });
    }
  });

  for (final (name, ff, size) in const [
    ('desktop', FormFactor.desktop, Size(1280, 800)),
    ('tv', FormFactor.tv, Size(960, 540)),
  ]) {
    testWidgets('initial focus is on Resume S2 · E1 on $name', (t) async {
      final (s, _) = await _services(progress: _progress);
      await _open(t, s, ff: ff, size: size);
      expect(_focusedButton()?.key, showPlayKey);
      expect(_playLabel(t), 'Resume S2 · E1');
      // The tab of the episode to resume is open (on TV below the fold).
      if (ff != FormFactor.tv) expect(_cards(t), ['e-2-1', 'e-2-2']);
      expect(find.byKey(const ValueKey('show-resume-source')), findsOneWidget);
      await _close(t);
    });
  }

  testWidgets('after a finished episode the button plays the next one', (
    t,
  ) async {
    final (s, _) = await _services(
      progress: [_watched('e-1-1', 2), _watched('e-1-2', 1)],
    );
    await _open(t, s);
    expect(_playLabel(t), 'Play S1 · E3');
    expect(find.byKey(const ValueKey('show-start-over')), findsNothing);
    await _close(t);
  });

  testWidgets('without progress the button plays S1 · E1', (t) async {
    final (s, _) = await _services();
    await _open(t, s);
    expect(_playLabel(t), 'Play S1 · E1');
    await _close(t);
  });

  testWidgets('season tabs: seasons, then Specials', (t) async {
    final (s, _) = await _services(progress: _progress);
    await _open(t, s);
    final chips = [
      for (final w in t.widgetList(
        find.byWidgetPredicate(
          (w) =>
              w.key is ValueKey<String> &&
              (w.key! as ValueKey<String>).value.startsWith('show-season-'),
        ),
      ))
        (w.key! as ValueKey<String>).value,
    ];
    expect(chips, ['show-season-1', 'show-season-2', 'show-season-0']);
    expect(find.text('Specials'), findsOneWidget);
    await _tapSeason(t, 0);
    expect(_cards(t), ['e-0-1']);
    await _tapSeason(t, 1);
    expect(_cards(t), ['e-1-1', 'e-1-2', 'e-1-3']);
    await _close(t);
  });

  testWidgets('switching from TMDB to AniDB order rebuilds the episodes', (
    t,
  ) async {
    final (s, lib) = await _services(progress: _progress);
    await _open(t, s);
    expect(find.text('TMDB order'), findsOneWidget);
    await _tapSeason(t, 1);
    expect(_cards(t), ['e-1-1', 'e-1-2', 'e-1-3']);
    await _tapSeason(t, 2);
    expect(_cards(t), ['e-2-1', 'e-2-2']);

    await _pickOrder(t, EpisodeOrder.anidb);
    expect(lib.storedEpisodeOrder(_showId), EpisodeOrder.anidb);
    expect(find.text('AniDB order'), findsOneWidget);
    // TMDB S2E1 is AniDB entry 2, episode 2.
    expect(_playLabel(t), 'Resume S2 · E2');
    expect(_cards(t), ['e-1-3', 'e-2-1', 'e-2-2']);
    expect(find.textContaining('S2 · E1'), findsWidgets);
    await _tapSeason(t, 1);
    expect(_cards(t), ['e-1-1', 'e-1-2']);

    // The stored order is used when the page opens again.
    await _close(t);
    await _open(t, s);
    expect(find.text('AniDB order'), findsOneWidget);
    expect(_cards(t), ['e-1-3', 'e-2-1', 'e-2-2']);
    await _close(t);
  });

  testWidgets('absolute order: one run of episodes, Specials apart', (t) async {
    final (s, _) = await _services(progress: _progress);
    await _open(t, s, size: const Size(1920, 1080));
    await _pickOrder(t, EpisodeOrder.absolute);
    expect(_playLabel(t), 'Resume E4');
    expect(find.text('Episodes'), findsOneWidget);
    expect(_cards(t), ['e-1-1', 'e-1-2', 'e-1-3', 'e-2-1', 'e-2-2']);
    expect(find.text('Specials'), findsOneWidget);
    await _close(t);
  });

  testWidgets('Hide Spoilers blurs only episodes not started', (t) async {
    final (s, _) = await _services(progress: _progress);
    await _open(t, s);
    expect(find.byKey(const ValueKey('show-spoiler-blur')), findsNothing);
    await s.setHideSpoilers(true);
    await settle(t, 4);
    // Season 2: E1 started, E2 not.
    expect(find.byKey(const ValueKey('show-spoiler-blur')), findsOneWidget);
    expect(
      find.descendant(
        of: find.byKey(showEpisodeKey('e-2-2')),
        matching: find.byType(ImageFiltered),
      ),
      findsNWidgets(2),
    );
    expect(
      find.descendant(
        of: find.byKey(showEpisodeKey('e-2-1')),
        matching: find.byType(ImageFiltered),
      ),
      findsNothing,
    );
    // Watched season 1: nothing hidden.
    await _tapSeason(t, 1);
    expect(find.byKey(const ValueKey('show-spoiler-blur')), findsNothing);
    expect(find.byKey(const ValueKey('show-episode-watched')), findsNWidgets(3));
    await _close(t);
  });

  testWidgets('Hide Spoilers is off by default and kept in preferences', (
    t,
  ) async {
    final prefs = MemoryPreferences();
    final (s, _) = await _services(preferences: prefs);
    expect(s.hideSpoilers.value, isFalse);
    setSurface(t, const Size(1280, 800));
    await t.pumpWidget(testApp(s, section: Section.settings));
    await settle(t, 10);
    final tile = find.byKey(const ValueKey('settings-hide-spoilers'));
    await t.ensureVisible(tile);
    await settle(t, 4);
    await t.tap(tile);
    await settle(t, 4);
    expect(s.hideSpoilers.value, isTrue);
    expect(prefs.values[Preferences.hideSpoilers], 'true');
    final again = AppServices(
      library: FakeLibrary.items(const []),
      secureStore: MemorySecureStore(),
      preferences: prefs,
    );
    await again.init();
    expect(again.hideSpoilers.value, isTrue);
    await _close(t);
  });

  testWidgets('v0 fallback: episodes, resume and absolute order', (t) async {
    final fake = FakeLibrary.items(
      [_show, ..._episodes],
      progress: _progress,
      anidb: _anidb,
    );
    final s = AppServices(
      library: _V0Library(fake),
      secureStore: MemorySecureStore(),
      playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
      clock: () => _now,
    );
    await s.init();
    await _open(t, s, size: const Size(1920, 1080));
    expect(_playLabel(t), 'Resume S2 · E1');
    expect(_cards(t), ['e-2-1', 'e-2-2']);
    // No AniDB without the core; absolute is computed in the app.
    await t.tap(find.byKey(showOrderKey));
    await settle(t, 6);
    expect(
      find.byKey(const ValueKey('show-order-anidb')),
      findsNothing,
    );
    await t.tap(find.byKey(const ValueKey('show-order-absolute')));
    await settle(t, 8);
    expect(_playLabel(t), 'Resume E4');
    expect(_cards(t), ['e-1-1', 'e-1-2', 'e-1-3', 'e-2-1', 'e-2-2']);
    // No cast or technical info without details.
    expect(find.text('Cast & Crew'), findsNothing);
    await _close(t);
  });
}
