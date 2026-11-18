// SPDX-License-Identifier: MPL-2.0
//
// LIB-014: «Сохранить в файлах» on the «Исправить распознавание» page —
// «Записать NFO» (and the question before an existing NFO is replaced)
// and «Добавить ID в имя папки» with the new name shown first. With
// KINOLUNE_SCREENSHOTS_DIR set the frames are written (in Russian):
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/lib-014 \
//     flutter test test/knowledge_section_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_review_inbox.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

Finder _key(String k) => find.byKey(ValueKey(k));

Future<void> _tapKey(WidgetTester t, String key) async {
  final f = _key(key);
  await t.ensureVisible(f);
  await t.pump();
  await t.tap(f);
  await settle(t, 6);
}

Future<FakeReviewInbox> _openEntry(WidgetTester t, int workId) async {
  setSurface(t, const Size(1280, 900));
  final review = FakeReviewInbox.demo();
  final s = await demoServices(reviewInbox: review);
  s.locale.value = const Locale('ru');
  await t.pumpWidget(testApp(s, section: Section.settings));
  await settle(t);
  await _tapKey(t, 'settings-review-inbox');
  await settle(t, 4);
  await _tapKey(t, 'review-entry-$workId');
  await settle(t, 4);
  await t.runAsync(
    () =>
        Scrollable.ensureVisible(t.element(_key('knowledge')), alignment: 0.3),
  );
  await settle(t, 4);
  return review;
}

void main() {
  setUpAll(loadAppFonts);

  testWidgets('NFO and the id in the folder name', (t) async {
    final review = await _openEntry(t, 901);
    expect(find.text(ru.reviewKnowledgeSection), findsOneWidget);
    expect(find.text('/Anime/Fairy Tail OVA/tvshow.nfo'), findsOneWidget);
    expect(
      find.text(ru.reviewIdRenameHint('Fairy Tail OVA [tmdbid-46261]')),
      findsOneWidget,
    );
    await capture(t, 'lib-014-section');

    // A new NFO: written at once.
    await _tapKey(t, 'knowledge-nfo');
    expect(review.log, contains('nfo:s901:keep'));
    expect(find.text(ru.reviewNfoDone), findsOneWidget);
    await settle(t, 40);

    // Now it exists: the page says so and asks before replacing.
    expect(
      find.text(ru.reviewNfoExists('/Anime/Fairy Tail OVA/tvshow.nfo')),
      findsOneWidget,
    );
    await _tapKey(t, 'knowledge-nfo');
    expect(_key('knowledge-replace-dialog'), findsOneWidget);
    await capture(t, 'lib-014-replace');
    await _tapKey(t, 'knowledge-replace-cancel');
    expect(review.log.where((e) => e.startsWith('nfo:')), hasLength(1));
    await _tapKey(t, 'knowledge-nfo');
    await _tapKey(t, 'knowledge-replace-ok');
    expect(review.log, contains('nfo:s901:replace'));
    await settle(t, 40);

    // The rename; afterwards there is nothing more to rename.
    await _tapKey(t, 'knowledge-rename');
    expect(review.log, contains('rename_ids:s901'));
    expect(find.text(ru.reviewIdRenameDone), findsOneWidget);
    expect(_key('knowledge-rename'), findsNothing);
  });

  testWidgets('a work without ids has nothing to write', (t) async {
    await _openEntry(t, 903);
    expect(_key('knowledge-no-ids'), findsOneWidget);
    expect(_key('knowledge-nfo'), findsNothing);
    expect(_key('knowledge-rename'), findsNothing);
  });
}
