// SPDX-License-Identifier: MPL-2.0
//
// UI-037: watched state on episode stills and posters. The four states of a
// 16:9 still (started, watched, not watched, next) and Hide Spoilers, the two
// kinds of 2:3 poster (movie, show) in golden images on a dark and a light
// surface and in en-XA; a started still split at its share; screen-reader
// text; the blurred copy made once and cached. Goldens use the default test
// font like test/home_golden_test.dart. Refresh deliberately:
//   flutter test --update-goldens test/watched_art_test.dart

import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/library/show_unwatched.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/theme/kinolune_theme.dart';
import 'package:kinolune/widgets/artwork.dart';
import 'package:kinolune/widgets/format.dart' show watchedLeft;
import 'package:kinolune/widgets/watched_art.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

const _sheetKey = ValueKey('sheet');

Widget _app(
  Widget child, {
  Locale locale = const Locale('en'),
  Brightness brightness = Brightness.dark,
}) {
  final dark = brightness == Brightness.dark;
  return MaterialApp(
    debugShowCheckedModeBanner: false,
    locale: locale,
    supportedLocales: AppLocalizations.supportedLocales,
    localizationsDelegates: kLocalizationsDelegates,
    theme: dark
        ? buildKinoluneTheme(FormFactor.desktop)
        : ThemeData(
            brightness: Brightness.light,
            colorSchemeSeed: KColors.accent,
          ),
    home: Scaffold(
      backgroundColor: dark ? KColors.bgBase : const Color(0xFFF5F5F7),
      body: Center(child: child),
    ),
  );
}

Progress _p(double f, {int minutes = 40, bool watched = false}) => Progress(
  itemId: 'x',
  positionMs: (minutes * 60000 * f).round(),
  durationMs: minutes * 60000,
  updatedAt: DateTime(2026, 11, 17),
  watched: watched,
);

Widget _still(Widget child, {double width = 240}) => SizedBox(
  width: width,
  height: width * 9 / 16,
  child: ClipRRect(borderRadius: BorderRadius.circular(9), child: child),
);

Widget _poster(Widget marks, String seed, String title) => SizedBox(
  width: 120,
  height: 180,
  child: ClipRRect(
    borderRadius: BorderRadius.circular(9),
    child: Stack(
      fit: StackFit.expand,
      children: [
        Artwork(
          url: '${kDemoArtScheme}poster',
          seed: seed,
          style: ArtStyle.poster,
          title: title,
        ),
        marks,
      ],
    ),
  ),
);

/// Every state on one sheet.
class _Sheet extends StatelessWidget {
  const _Sheet();

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final started = _p(.4);
    return RepaintBoundary(
      key: _sheetKey,
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Wrap(
              spacing: 12,
              runSpacing: 12,
              children: [
                _still(
                  WatchedStill(
                    url: '${kDemoArtScheme}still',
                    seed: 'ep-started',
                    state: WatchArt.started,
                    fraction: started.fraction,
                    remaining: watchedLeft(l, started),
                  ),
                ),
                _still(
                  const WatchedStill(
                    url: '${kDemoArtScheme}still',
                    seed: 'ep-watched',
                    state: WatchArt.watched,
                  ),
                ),
                _still(
                  const WatchedStill(
                    url: '${kDemoArtScheme}still',
                    seed: 'ep-unwatched',
                  ),
                ),
                _still(
                  const WatchedStill(
                    url: '${kDemoArtScheme}still',
                    seed: 'ep-next',
                    next: true,
                    radius: 9,
                  ),
                ),
                _still(
                  const WatchedStill(
                    url: '${kDemoArtScheme}still',
                    seed: 'ep-hidden',
                    hiddenLabel: 'S2 · E5', // l10n-ignore
                  ),
                ),
              ],
            ),
            const SizedBox(height: 16),
            Wrap(
              spacing: 12,
              children: [
                _poster(
                  const PosterWatchMarks(
                    itemId: 'm1',
                    state: WatchArt.started,
                    fraction: .62,
                  ),
                  'movie-started',
                  'Quiet Orbit', // l10n-ignore
                ),
                _poster(
                  const PosterWatchMarks(itemId: 'm2', state: WatchArt.watched),
                  'movie-watched',
                  'Northbound', // l10n-ignore
                ),
                _poster(
                  const Stack(
                    fit: StackFit.expand,
                    children: [
                      PosterWatchMarks(
                        itemId: 's1',
                        state: WatchArt.started,
                        episodesLeft: 5,
                      ),
                      Positioned(left: 6, top: 6, child: ReviewMark()),
                    ],
                  ),
                  'show-left',
                  'Moonlit Bureau', // l10n-ignore
                ),
                _poster(
                  const PosterWatchMarks(
                    itemId: 's2',
                    state: WatchArt.unwatched,
                    episodesLeft: 0,
                  ),
                  'show-done',
                  'The Archive', // l10n-ignore
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

void main() {
  group('thresholds (LIB-011)', () {
    test('started from 2 %, watched past 95 % or when marked', () {
      expect(watchArtOf(null), WatchArt.unwatched);
      expect(watchArtOf(_p(.01)), WatchArt.unwatched);
      expect(watchArtOf(_p(continueWatchingMin)), WatchArt.started);
      expect(watchArtOf(_p(.4)), WatchArt.started);
      expect(watchArtOf(_p(continueWatchingMax)), WatchArt.started);
      expect(watchArtOf(_p(.96)), WatchArt.watched);
      expect(watchArtOf(_p(.1, watched: true)), WatchArt.watched);
      expect(
        watchArtOf(
          Progress(
            itemId: 'x',
            positionMs: 5000,
            durationMs: 0,
            updatedAt: DateTime(2026),
          ),
        ),
        WatchArt.unwatched,
      );
    });

    test('unwatched episodes count regular ones only', () {
      ShowEpisode ep(int season, int n, [Progress? p]) => ShowEpisode(
        item: LibraryItem(
          id: 'e$season$n',
          kind: ItemKind.episode,
          title: 'E$n',
          filePath: '/e$season$n.mkv',
          season: season,
          episode: n,
        ),
        progress: p,
      );
      expect(
        unwatchedEpisodes([
          ep(0, 1),
          ep(1, 1, _p(1, watched: true)),
          ep(1, 2, _p(.97)),
          ep(1, 3, _p(.4)),
          ep(1, 4),
        ]),
        2,
      );
    });
  });

  testWidgets('40 % splits the still 40/60', (t) async {
    setSurface(t, const Size(600, 400));
    await t.pumpWidget(
      _app(
        SizedBox(
          key: const ValueKey('box'),
          width: 320,
          height: 180,
          child: WatchedStill(
            url: '${kDemoArtScheme}still',
            seed: 'split',
            state: WatchArt.started,
            fraction: _p(.4).fraction,
          ),
        ),
      ),
    );
    await t.pump();
    final box = t.getRect(find.byKey(const ValueKey('box')));
    final blur = t.getRect(find.byKey(watchedBlurKey));
    expect(blur.left - box.left, moreOrLessEquals(128));
    expect(blur.width, moreOrLessEquals(192));
    expect(blur.right, moreOrLessEquals(box.right));
    expect(blur.height, moreOrLessEquals(180));
    final line = t.getRect(find.byKey(watchedSplitKey));
    expect(line.center.dx - box.left, moreOrLessEquals(128));
    // The blurred part is the whole art blurred and clipped: it lines up
    // with the sharp art under it.
    final art = t.getRect(
      find.descendant(
        of: find.byKey(watchedBlurKey),
        matching: find.byType(BlurredArt),
      ),
    );
    expect(art, box);
    // The sharp share is left uncovered.
    expect(
      find.descendant(
        of: find.byType(WatchedStill),
        matching: find.byType(GeneratedArt),
      ),
      findsOneWidget,
    );
  });

  testWidgets('the blurred copy is small, made once and cached', (t) async {
    setSurface(t, const Size(800, 400));
    PaintingBinding.instance.imageCache.clear();
    Widget still(String seed) => SizedBox(
      width: 320,
      height: 180,
      child: WatchedStill(
        url: '${kDemoArtScheme}still',
        seed: seed,
        state: WatchArt.started,
        fraction: .3,
      ),
    );
    await t.pumpWidget(
      _app(Row(mainAxisSize: MainAxisSize.min, children: [still('a')])),
    );
    await t.pump();
    const key = BlurredArtImage(
      url: '${kDemoArtScheme}still',
      seed: 'a',
      sourceWidth: 320,
    );
    expect(PaintingBinding.instance.imageCache.containsKey(key), isTrue);
    final raw = t.widget<RawImage>(
      find.descendant(
        of: find.byKey(watchedBlurKey),
        matching: find.byType(RawImage),
      ),
    );
    expect(raw.image!.width, blurredArtWidth);
    expect(raw.image!.height, blurredArtWidth * 9 ~/ 16);
    final before = PaintingBinding.instance.imageCache.currentSize;
    // A second still of the same art reuses the copy.
    await t.pumpWidget(
      _app(
        Row(mainAxisSize: MainAxisSize.min, children: [still('a'), still('a')]),
      ),
    );
    await t.pump();
    expect(PaintingBinding.instance.imageCache.currentSize, before);
  });

  testWidgets('a file or remote art is decoded small, then blurred', (t) async {
    // A local file stands in for the LIB-013 cache file of remote art.
    final path = File('assets/placeholders/backdrop.png').absolute.path;
    final info = await t.runAsync(() async {
      final done = Completer<ImageInfo>();
      final stream = BlurredArtImage(
        url: path,
        seed: 'f',
        sourceWidth: 640,
      ).resolve(ImageConfiguration.empty);
      stream.addListener(
        ImageStreamListener(
          (i, _) => done.complete(i),
          onError: (e, st) => done.completeError(e, st!),
        ),
      );
      return done.future;
    });
    expect(info!.image.width, blurredArtWidth);
    expect(info.image.height, lessThan(blurredArtWidth));
  });

  group('screen reader', () {
    testWidgets('a started still reads its share and time left', (t) async {
      final semantics = t.ensureSemantics();
      final l = lookupAppLocalizations(const Locale('ru'));
      final p = _p(.4);
      await t.pumpWidget(
        _app(
          locale: const Locale('ru'),
          _still(
            WatchedStill(
              url: '${kDemoArtScheme}still',
              seed: 's',
              state: WatchArt.started,
              fraction: p.fraction,
              remaining: watchedLeft(l, p),
            ),
          ),
        ),
      );
      await t.pump();
      expect(
        find.bySemanticsLabel('просмотрено 40 %, ещё 24 мин'),
        findsOneWidget,
      );
      semantics.dispose();
    });

    testWidgets('watched, not watched, next, hidden', (t) async {
      final semantics = t.ensureSemantics();
      await t.pumpWidget(
        _app(
          const Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              SizedBox(
                width: 160,
                height: 90,
                child: WatchedStill(
                  url: '${kDemoArtScheme}still',
                  seed: 'w',
                  state: WatchArt.watched,
                ),
              ),
              SizedBox(
                width: 160,
                height: 90,
                child: WatchedStill(
                  url: '${kDemoArtScheme}still',
                  seed: 'n',
                  next: true,
                ),
              ),
              SizedBox(
                width: 160,
                height: 90,
                child: WatchedStill(
                  url: '${kDemoArtScheme}still',
                  seed: 'h',
                  hiddenLabel: 'S1 · E3', // l10n-ignore
                ),
              ),
            ],
          ),
        ),
      );
      await t.pump();
      expect(find.bySemanticsLabel('Watched'), findsOneWidget);
      expect(find.bySemanticsLabel('Not watched, Up next'), findsOneWidget);
      expect(
        find.bySemanticsLabel('S1 · E3: still hidden to avoid spoilers'),
        findsOneWidget,
      );
      // Hide Spoilers: the cached blurred copy, no live filter.
      expect(find.byKey(watchedHiddenKey), findsOneWidget);
      expect(find.byType(ImageFiltered), findsNothing);
      semantics.dispose();
    });

    testWidgets('posters read progress and episodes left', (t) async {
      final semantics = t.ensureSemantics();
      await t.pumpWidget(
        _app(
          locale: const Locale('ru'),
          const Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              SizedBox(
                width: 120,
                height: 180,
                child: PosterWatchMarks(
                  itemId: 'm',
                  state: WatchArt.started,
                  fraction: .4,
                ),
              ),
              SizedBox(
                width: 120,
                height: 180,
                child: PosterWatchMarks(
                  itemId: 's',
                  state: WatchArt.started,
                  episodesLeft: 3,
                ),
              ),
              SizedBox(
                width: 120,
                height: 180,
                child: PosterWatchMarks(
                  itemId: 'd',
                  state: WatchArt.unwatched,
                  episodesLeft: 0,
                ),
              ),
            ],
          ),
        ),
      );
      await t.pump();
      expect(find.bySemanticsLabel('просмотрено 40 %'), findsOneWidget);
      expect(find.bySemanticsLabel('3 непросмотренные серии'), findsOneWidget);
      expect(find.bySemanticsLabel('Просмотрено'), findsOneWidget);
      expect(find.byKey(const ValueKey('badge-progress-m')), findsOneWidget);
      expect(find.byKey(const ValueKey('badge-left-s')), findsOneWidget);
      expect(find.byKey(const ValueKey('badge-watched-d')), findsOneWidget);
      semantics.dispose();
    });
  });

  group('goldens', () {
    for (final (name, brightness, locale) in <(String, Brightness, Locale)>[
      ('dark', Brightness.dark, const Locale('en')),
      ('light', Brightness.light, const Locale('en')),
      ('dark_en_XA', Brightness.dark, kPseudoLocale),
      ('light_en_XA', Brightness.light, kPseudoLocale),
    ]) {
      testWidgets('states, $name', (t) async {
        setSurface(t, const Size(1320, 400));
        await t.pumpWidget(
          _app(const _Sheet(), brightness: brightness, locale: locale),
        );
        await t.pump();
        await expectLater(
          find.byKey(_sheetKey),
          matchesGoldenFile('goldens/watched_states_$name.png'),
        );
      });
    }
  });
}
