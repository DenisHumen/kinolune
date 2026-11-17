// SPDX-License-Identifier: MPL-2.0
//
// META-035: «Тип и нумерация» on the «Исправить распознавание» page —
// the numbering row «Серия в имени + N = серия сезона S» (folded until
// asked for), the running count, the type switch, the folder rule line
// and «Забыть правило папки»; up/down leave the number fields (TV). With
// KINOLUNE_SCREENSHOTS_DIR set the frames are written (in Russian):
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/meta-035 \
//     flutter test test/folder_rules_test.dart

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_review_inbox.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/review/folder_rules_section.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/widgets/focusable.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));
final en = lookupAppLocalizations(const Locale('en'));

Finder _key(String k) => find.byKey(ValueKey(k));

Future<void> _tapKey(WidgetTester t, String key) async {
  final f = _key(key);
  await t.ensureVisible(f);
  await t.pump();
  await t.tap(f);
  await settle(t, 6);
}

/// The review page of [workId] of the demo inbox.
Future<FakeReviewInbox> _openEntry(
  WidgetTester t,
  int workId, {
  FakeReviewInbox? inbox,
}) async {
  setSurface(t, const Size(1280, 900));
  final review = inbox ?? FakeReviewInbox.demo();
  final s = await demoServices(reviewInbox: review);
  s.locale.value = const Locale('ru');
  await t.pumpWidget(testApp(s, section: Section.settings));
  await settle(t);
  await _tapKey(t, 'settings-review-inbox');
  await settle(t, 4);
  await _tapKey(t, 'review-entry-$workId');
  await settle(t, 4);
  return review;
}

Future<void> _showSection(WidgetTester t) async {
  await t.runAsync(
    () => Scrollable.ensureVisible(
      t.element(_key('folder-rules')),
      alignment: 0.1,
    ),
  );
  await settle(t, 4);
}

void main() {
  setUpAll(loadAppFonts);

  test('the numbering in words', () {
    expect(numberingSummary(ru, null), ru.reviewNumberingAsNamed);
    expect(
      numberingSummary(ru, const FolderRule(id: 1)),
      ru.reviewNumberingAsNamed,
    );
    expect(
      numberingSummary(ru, const FolderRule(id: 1, season: 2, offset: -28)),
      ru.reviewNumberingRule('− 28', 2),
    );
    expect(
      numberingSummary(en, const FolderRule(id: 1, offset: 12)),
      en.reviewNumberingRuleSameSeason('+ 12'),
    );
    expect(
      numberingSummary(en, const FolderRule(id: 1, absolute: true)),
      en.reviewNumberingRuleAbsolute('+ 0'),
    );
  });

  testWidgets('numbering: folded, then «+ N = серия сезона S»', (t) async {
    final inbox = await _openEntry(t, 901);
    await _showSection(t);
    // Nothing to touch: one line, the type and the rule of the folder.
    expect(
      find.text(ru.reviewNumberingAsNamed),
      findsOneWidget,
      reason: 'the numbering is folded by default',
    );
    expect(_key('folder-numbering-editor'), findsNothing);
    expect(
      find.text(ru.reviewFolderRule('Anime/Fairy Tail OVA')),
      findsOneWidget,
    );
    expect(_key('folder-forget'), findsOneWidget);
    bool? selected(String kind) => t
        .widget<KFocusable>(
          find.descendant(
            of: _key('folder-kind-$kind'),
            matching: find.byType(KFocusable),
          ),
        )
        .selected;
    expect(selected('anime'), isTrue);
    expect(selected('show'), isFalse);
    await capture(t, 'meta-035-folded');

    await _tapKey(t, 'folder-numbering-edit');
    expect(_key('folder-numbering-editor'), findsOneWidget);
    // Unchanged numbering: nothing to apply.
    final apply = t.widget<KButton>(_key('folder-numbering-apply'));
    expect(apply.onPressed, isNull);

    await t.enterText(_key('folder-offset'), '-12');
    await _tapKey(t, 'folder-season-more');
    await _tapKey(t, 'folder-season-more');
    expect(t.widget<TextField>(_key('folder-season')).controller!.text, '2');
    expect(find.text(ru.reviewNumberingExample(13, 2, 1)), findsOneWidget);
    await _showSection(t);
    await capture(t, 'meta-035-numbering');

    await _tapKey(t, 'folder-numbering-apply');
    expect(inbox.log, ['numbering:s901:2:-12:ep']);
    expect(_key('folder-numbering-editor'), findsNothing);
    expect(find.text(ru.reviewNumberingRule('− 12', 2)), findsOneWidget);
  });

  testWidgets('running count and the season «как в имени»', (t) async {
    final inbox = await _openEntry(t, 901);
    await _tapKey(t, 'folder-numbering-edit');
    // − on an empty season stays «как в имени»; + then − − is back.
    await _tapKey(t, 'folder-season-less');
    expect(t.widget<TextField>(_key('folder-season')).controller!.text, '');
    await _tapKey(t, 'folder-season-more');
    await _tapKey(t, 'folder-season-less');
    await _tapKey(t, 'folder-season-less');
    expect(t.widget<TextField>(_key('folder-season')).controller!.text, '');
    await _tapKey(t, 'folder-offset-more');
    expect(
      find.text(ru.reviewNumberingExampleSameSeason(1, 2)),
      findsOneWidget,
    );

    await _tapKey(t, 'folder-absolute');
    expect(_key('folder-season'), findsNothing);
    expect(find.text(ru.reviewNumberingIsAbsolute), findsOneWidget);
    expect(find.text(ru.reviewNumberingExampleAbsolute(1, 2)), findsOneWidget);
    await _tapKey(t, 'folder-numbering-apply');
    expect(inbox.log, ['numbering:s901:-:1:abs']);
    expect(find.text(ru.reviewNumberingRuleAbsolute('+ 1')), findsOneWidget);

    // Cancel drops an edit.
    await _tapKey(t, 'folder-numbering-edit');
    await _tapKey(t, 'folder-offset-more');
    await _tapKey(t, 'folder-numbering-cancel');
    expect(inbox.log, hasLength(1));
    expect(find.text(ru.reviewNumberingRuleAbsolute('+ 1')), findsOneWidget);
  });

  testWidgets('«Забыть правило папки»', (t) async {
    final inbox = await _openEntry(t, 901);
    await _tapKey(t, 'folder-forget');
    expect(inbox.log, ['forget:s901']);
    expect(_key('folder-forget'), findsNothing);
    expect(
      find.text(ru.reviewFolderRule('Anime/Fairy Tail OVA')),
      findsNothing,
    );
    expect(find.text(ru.reviewFolderForgotten), findsOneWidget);
  });

  testWidgets('the type switch applies to the folder and closes', (t) async {
    final inbox = await _openEntry(t, 901);
    await _tapKey(t, 'folder-kind-show');
    expect(inbox.log, ['kind:s901:show']);
    // Back on the list.
    expect(_key('folder-rules'), findsNothing);
    expect(_key('review-entry-901'), findsOneWidget);
  });

  testWidgets('a movie: the type, no numbering, no rule yet', (t) async {
    final inbox = await _openEntry(t, 902);
    await _showSection(t);
    expect(_key('folder-kind-movie'), findsOneWidget);
    expect(_key('folder-numbering-summary'), findsNothing);
    expect(_key('folder-forget'), findsNothing);
    await capture(t, 'meta-035-movie');
    // The selected type does nothing.
    await _tapKey(t, 'folder-kind-movie');
    expect(inbox.log, isEmpty);
  });

  testWidgets('up and down leave a number field (TV)', (t) async {
    await _openEntry(t, 901);
    await _tapKey(t, 'folder-numbering-edit');
    await t.tap(_key('folder-offset'));
    await settle(t, 2);
    bool inField() =>
        FocusManager.instance.primaryFocus?.context
            ?.findAncestorWidgetOfExactType<TextField>() !=
        null;
    expect(inField(), isTrue);
    await pressKey(t, LogicalKeyboardKey.arrowDown);
    expect(inField(), isFalse, reason: 'down left the field');
    await t.tap(_key('folder-offset'));
    await settle(t, 2);
    await pressKey(t, LogicalKeyboardKey.arrowUp);
    expect(inField(), isFalse, reason: 'up left the field');
  });

  testWidgets('an error stays on the page', (t) async {
    final inbox = _FailingInbox();
    await _openEntry(t, 901, inbox: inbox);
    await _tapKey(t, 'folder-numbering-edit');
    await _tapKey(t, 'folder-offset-more');
    await _tapKey(t, 'folder-numbering-apply');
    expect(_key('folder-error'), findsOneWidget);
    expect(_key('folder-numbering-editor'), findsOneWidget);
  });
}

class _FailingInbox extends FakeReviewInbox {
  _FailingInbox()
    : super(
        entries: const [
          ReviewEntry(
            workId: 901,
            itemId: 's901',
            kind: 'anime',
            state: ReviewState.check,
            title: 'Хвост феи',
          ),
        ],
        folderRules: const {
          's901': FolderRules(
            kind: 'anime',
            rules: [FolderRule(id: 1, folder: 'Anime/Fairy Tail OVA')],
          ),
        },
      );

  @override
  Future<void> setNumbering(
    String itemId, {
    int? season,
    int offset = 0,
    bool absolute = false,
  }) async => throw StateError('work 901 has no folder of its own');
}
