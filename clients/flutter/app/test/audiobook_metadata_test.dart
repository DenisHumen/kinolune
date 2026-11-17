// SPDX-License-Identifier: MPL-2.0
//
// MUS-019: the book page shows the Open Library cover and description of a
// matched book (folded, «Показать полностью»), a book that was not
// recognized for sure says so instead, and «О программе» credits Open
// Library. With KINOLUNE_SCREENSHOTS_DIR set the frames are written:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/mus-019 \
//     flutter test test/audiobook_metadata_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/widgets/artwork.dart' show kDemoArtScheme;
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

Finder _key(String k) => find.byKey(ValueKey(k));

const _min = 60 * 1000;

/// Invented book and text: no real descriptions in tests.
const _about =
    'Смотритель маяка на северном острове получает письма, которые никто '
    'не мог отправить. Он ищет отправителя среди рыбаков, метеорологов и '
    'старых карт, а зима отрезает остров от материка. Чем ближе ответ, тем '
    'меньше он уверен, что хочет его знать. Тихая история о памяти, '
    'одиночестве и свете, который зажигают для других. Вторая книга '
    'цикла «Северные истории»; её можно читать отдельно от первой.';

Audiobook _book({
  required String key,
  required String title,
  String? author,
  String? description,
  bool needsReview = false,
}) => Audiobook(
  key: key,
  title: title,
  author: author,
  narrator: 'Илья Громов',
  sourceId: 1,
  relPath: 'Аудиокниги/$title',
  coverUri: '${kDemoArtScheme}album',
  description: description,
  needsReview: needsReview,
  durationMs: 120 * _min,
  parts: [
    LibraryItem(
      id: '$key-1',
      kind: ItemKind.track,
      title: 'Часть 1',
      filePath: '/media/Аудиокниги/$title/01.mp3',
      durationMs: 120 * _min,
    ),
  ],
  chapters: const [
    BookChapter(
      title: 'Часть 1',
      part: 0,
      startMs: 0,
      bookStartMs: 0,
      durationMs: 120 * _min,
    ),
  ],
);

Future<void> _open(WidgetTester t, Size size, FormFactor ff, double dpr) async {
  setSurface(t, size, dpr: dpr);
  final library = FakeLibrary.demo(scanStepDelay: Duration.zero);
  library.audiobookList.value = [
    _book(
      key: 'file:aaaa',
      title: 'Дом у маяка',
      author: 'Анна Северова',
      description: _about,
    ),
    _book(key: 'file:bbbb', title: 'Записи без подписи', needsReview: true),
  ];
  final s = await demoServices(library: library);
  s.locale.value = const Locale('ru');
  await t.pumpWidget(testApp(s, formFactor: ff));
  await settle(t);
  await t.tap(find.text(ru.navAudiobooks).last);
  await settle(t, 8);
}

Future<void> _openBook(WidgetTester t, String title) async {
  final f = find.text(title).first;
  await t.ensureVisible(f);
  await t.pump();
  await t.tap(f);
  await settle(t, 6);
}

void main() {
  setUpAll(loadAppFonts);

  for (final (name, ff, size, dpr) in <(String, FormFactor, Size, double)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800), 1),
    ('phone', FormFactor.phone, const Size(390, 844), 2),
  ]) {
    testWidgets('a matched book shows its description on $name', (t) async {
      await _open(t, size, ff, dpr);
      await _openBook(t, 'Дом у маяка');
      expect(find.text(ru.audiobookAbout), findsOneWidget);
      expect(find.text(ru.audiobookDescriptionSource), findsOneWidget);
      final text = t.widget<Text>(_key('audiobook-description'));
      expect(text.data, _about);
      if (ff == FormFactor.phone) {
        // Folded on a phone; the button opens it all.
        expect(text.maxLines, 5);
        expect(find.text(ru.audiobookShowMore), findsOneWidget);
        await capture(t, '01-book-description-folded-$name');
        await t.tap(_key('audiobook-description-more'));
        await settle(t, 4);
        expect(t.widget<Text>(_key('audiobook-description')).maxLines, isNull);
        expect(find.text(ru.audiobookShowLess), findsOneWidget);
      }
      expect(_key('audiobook-needs-review'), findsNothing);
      await capture(t, '02-book-description-$name');
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }

  testWidgets('a book not recognized for sure says so', (t) async {
    await _open(t, const Size(1280, 800), FormFactor.desktop, 1);
    await _openBook(t, 'Записи без подписи');
    expect(_key('audiobook-needs-review'), findsOneWidget);
    expect(find.text(ru.audiobookNeedsReview), findsOneWidget);
    expect(_key('audiobook-description'), findsNothing);
    await capture(t, '03-book-needs-review-desktop');
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });
}
