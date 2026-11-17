// SPDX-License-Identifier: MPL-2.0
//
// MUS-018: the «Аудиокниги» section is in the navigation only while the
// library has books; a book page with chapters; «Продолжить с …» from the
// saved place; the player with ±30 s, speed and the sleep timer; the
// place saved through the library; music and a book never play at once.
// With KINOLUNE_SCREENSHOTS_DIR set the frames are written (in Russian):
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/mus-018 \
//     flutter test test/audiobooks_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/widgets/artwork.dart' show kDemoArtScheme;
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

Finder _key(String k) => find.byKey(ValueKey(k));

Future<void> _tapKey(WidgetTester t, String key) async {
  final f = _key(key).first;
  await t.ensureVisible(f);
  await t.pump();
  await t.tap(f);
  await settle(t, 6);
}

const _min = 60 * 1000;

LibraryItem _part(String id, String path, int minutes, String title) =>
    LibraryItem(
      id: id,
      kind: ItemKind.track,
      title: title,
      posterUrl: '${kDemoArtScheme}album',
      filePath: path,
      durationMs: minutes * _min,
    );

/// A folder of three parts by invented authors.
Audiobook _lighthouse({BookProgress? progress}) => Audiobook(
  key: 'file:aaaa',
  title: 'Дом у маяка',
  author: 'Анна Северова',
  narrator: 'Илья Громов',
  series: 'Северные истории',
  seriesNumber: 2,
  sourceId: 1,
  relPath: 'Аудиокниги/Анна Северова - Дом у маяка',
  coverUri: '${kDemoArtScheme}album',
  durationMs: 120 * _min,
  parts: [
    for (var n = 1; n <= 3; n++)
      _part('t$n', '/media/Аудиокниги/Дом у маяка/0$n.mp3', 40, 'Часть $n'),
  ],
  chapters: [
    for (var n = 0; n < 3; n++)
      BookChapter(
        title: 'Часть ${n + 1}',
        part: n,
        startMs: 0,
        bookStartMs: n * 40 * _min,
        durationMs: 40 * _min,
      ),
  ],
  progress: progress,
);

/// An m4b with chapters.
Audiobook _clockmaker() => Audiobook(
  key: 'file:bbbb',
  title: 'Часовщик',
  author: 'Пётр Ланской',
  sourceId: 1,
  relPath: 'Аудиокниги/Часовщик.m4b',
  coverUri: '${kDemoArtScheme}album2',
  durationMs: 90 * _min,
  parts: [
    _part('t9', '/media/Аудиокниги/Часовщик.m4b', 90, 'Часовщик'),
  ],
  chapters: [
    for (final (i, title) in ['Пролог', 'Мастерская', 'Буря', 'Эпилог'].indexed)
      BookChapter(
        title: title,
        part: 0,
        startMs: i * 20 * _min,
        bookStartMs: i * 20 * _min,
        durationMs: i == 3 ? 30 * _min : 20 * _min,
      ),
  ],
);

/// At the start of the second part.
final _started = BookProgress(
  positionMs: 40 * _min,
  durationMs: 120 * _min,
  updatedAt: DateTime(2026, 11, 14, 19),
);

Future<(AppServices, FakeLibrary, List<FakePlayerEngine>)> _open(
  WidgetTester t, {
  Size size = const Size(1280, 800),
  FormFactor formFactor = FormFactor.desktop,
  Section section = Section.home,
  double dpr = 1,
  List<Audiobook>? books,
}) async {
  setSurface(t, size, dpr: dpr);
  final engines = <FakePlayerEngine>[];
  final library = FakeLibrary.demo(scanStepDelay: Duration.zero);
  library.audiobookList.value =
      books ?? [_lighthouse(progress: _started), _clockmaker()];
  final s = await demoServices(
    library: library,
    engineFactory: ({bool video = true}) {
      final e = FakePlayerEngine(
        loadedDuration: const Duration(minutes: 40),
      );
      engines.add(e);
      return e;
    },
  );
  s.locale.value = const Locale('ru');
  await t.pumpWidget(testApp(s, formFactor: formFactor, section: section));
  await settle(t);
  return (s, library, engines);
}

Future<void> _close(WidgetTester t) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
}

void main() {
  setUpAll(loadAppFonts);

  testWidgets('no books: no section', (t) async {
    final (s, _, _) = await _open(t, books: const []);
    expect(find.text(ru.navAudiobooks), findsNothing);
    expect(s.audiobooks.shown.value, isFalse);
    await _close(t);
  });

  for (final (name, ff, size, dpr) in <(String, FormFactor, Size, double)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800), 1),
    ('phone', FormFactor.phone, const Size(390, 844), 2),
  ]) {
    testWidgets('books have their own section on $name', (t) async {
      await _open(t, size: size, formFactor: ff, dpr: dpr);
      expect(find.text(ru.navAudiobooks), findsWidgets);
      await t.tap(find.text(ru.navAudiobooks).last);
      await settle(t, 8);
      expect(_key('audiobooks-screen'), findsOneWidget);
      // Started first, then all books.
      expect(_key('audiobooks-continue'), findsOneWidget);
      expect(find.text('Дом у маяка'), findsWidgets);
      expect(find.text('Часовщик'), findsOneWidget);
      expect(
        find.text(ru.audiobookLeft(ru.formatHoursMinutes(1, 20))),
        findsWidgets,
      );
      await capture(t, '01-audiobooks-$name');

      // The book page: chapters, narrator, series, «Продолжить с 40:00».
      await _tapKey(t, 'audiobook-card-0');
      expect(find.text(ru.audiobookContinueFrom('40:00')), findsOneWidget);
      expect(find.textContaining(ru.audiobookNarrator('Илья Громов')), findsOneWidget);
      expect(
        find.textContaining(ru.audiobookSeriesNumber('Северные истории', '2')),
        findsOneWidget,
      );
      expect(_key('audiobook-chapter-2'), findsOneWidget);
      await capture(t, '02-audiobook-page-$name');
      await _close(t);
    });

    testWidgets('the player on $name: resume, ±30 s, speed, sleep timer', (
      t,
    ) async {
      final (s, library, engines) = await _open(
        t,
        size: size,
        formFactor: ff,
        dpr: dpr,
        section: Section.audiobooks,
      );
      await settle(t, 4);
      await _tapKey(t, 'audiobook-card-0');
      await _tapKey(t, 'audiobook-listen');
      expect(_key('book-player'), findsOneWidget);
      final e = engines.lastWhere((e) => e.uris.length == 3);
      // Resumes at 40:00 = the start of the second part.
      expect(e.calls.first, 'open:3:1:0:true');
      final player = s.audiobooks.player;
      expect(player.position, const Duration(minutes: 40));
      expect(find.text('Часть 2'), findsWidgets);

      await _tapKey(t, 'book-forward30');
      expect(player.position, const Duration(minutes: 40, seconds: 30));
      await _tapKey(t, 'book-back30');
      await _tapKey(t, 'book-back30');
      expect(player.position, const Duration(minutes: 39, seconds: 30));
      // Back over the part boundary: the first part again.
      expect(e.state.value.playlistIndex, 0);

      await _tapKey(t, 'book-speed');
      await _tapKey(t, 'book-speed-1.5');
      expect(player.speed, 1.5);
      expect(e.state.value.rate, 1.5);
      expect(find.text(ru.audiobookSpeedValue('1.5')), findsOneWidget);

      await _tapKey(t, 'book-sleep');
      await _tapKey(t, 'book-sleep-end-of-chapter');
      expect(player.sleepTimer, const SleepTimer.endOfChapter());
      expect(find.text(ru.audiobookSleepEndOfChapter), findsOneWidget);
      await capture(t, '03-book-player-$name');

      // «Часть 1» ends at 40:00: the timer pauses there.
      e.advance(const Duration(seconds: 31));
      await settle(t, 2);
      expect(player.playing, isFalse);
      expect(player.position, const Duration(minutes: 40));
      // The place went to the library.
      expect(library.bookPlaces['file:aaaa']?.positionMs, 40 * _min);

      // The chapter list jumps.
      await _tapKey(t, 'book-chapters');
      await _tapKey(t, 'book-chapter-2');
      expect(player.position, const Duration(minutes: 80));
      expect(library.bookPlaces['file:aaaa']?.positionMs, 80 * _min);

      // Closed: the mini bar of the book.
      await t.tap(find.byTooltip(ru.audiobookClose));
      await settle(t, 6);
      expect(_key('book-mini-bar'), findsOneWidget);
      await capture(t, '04-book-mini-bar-$name');
      await _close(t);
    });
  }

  testWidgets('a book and music never play at once', (t) async {
    final (s, _, _) = await _open(t, section: Section.audiobooks);
    final book = s.audiobooks.books.last;
    await s.audiobooks.play(book);
    await settle(t, 2);
    expect(s.audiobooks.player.playing, isTrue);
    final tracks = await libraryMusic(s.library).musicTracks().first;
    await s.music.play([
      PlayRequest(uri: tracks.first.item.playbackUri, title: 'x'),
    ]);
    await settle(t, 2);
    expect(s.music.playing, isTrue);
    expect(s.audiobooks.player.playing, isFalse);
    s.audiobooks.player.play();
    await settle(t, 2);
    expect(s.music.playing, isFalse);
    await _close(t);
  });

  testWidgets('the speed is remembered', (t) async {
    final (s, _, _) = await _open(t);
    await s.audiobooks.setSpeed(1.75);
    expect(await s.preferences.read('audiobook_speed'), '1.75');
    await _close(t);
  });
}
