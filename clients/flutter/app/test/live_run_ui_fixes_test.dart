// SPDX-License-Identifier: MPL-2.0
//
// Fixes from the live Linux run of 2026-11-20 (QA-016):
// BUG-012 the home line and the Settings count follow a metadata run;
// BUG-013 «Источники» show the keychain fallback in plain words, the core's
//         message only under «Подробнее»;
// BUG-014 an episode without a title is «Серия N» in Russian;
// BUG-016 no banner or notice card over the video player.

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/fakes/fake_review_inbox.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/notices/in_app_notices.dart';
import 'package:kinolune/screens/show_detail_screen.dart';
import 'package:kinolune/screens/sources_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/secret_storage.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));
final ru = lookupAppLocalizations(const Locale('ru'));

Finder _key(String k) => find.byKey(ValueKey(k));

/// A fresh app on [section].
Future<void> _open(WidgetTester t, AppServices s, Section section) async {
  await t.pumpWidget(const SizedBox());
  await settle(t, 2);
  await t.pumpWidget(testApp(s, section: section));
  await settle(t);
}

Future<void> _push(WidgetTester t, Widget page) async {
  final context = t.element(find.byType(Scaffold).first);
  unawaited(
    Navigator.of(
      context,
      rootNavigator: true,
    ).push(MaterialPageRoute<void>(builder: (_) => page)),
  );
  await settle(t, 8);
}

ReviewEntry _entry(int work, String kind, ReviewState state) => ReviewEntry(
  workId: work,
  itemId: '${kind == 'movie' ? 'm' : 's'}$work',
  kind: kind,
  state: state,
  title: 'Title $work', // l10n-ignore
);

/// The core's message of the live run: D-Bus and a socket path.
const _coreReason =
    'OS keychain: no secret storage is available: zbus error: Failed to '
    'connect to address `unix:path=/tmp/run/bus`: No such file or '
    'directory (os error 2)';

const _fallback = SecretStorageInfo(
  backend: 'encrypted-file',
  fallbackKind: SecretFallbackKind.unavailable,
  fallbackReason: _coreReason,
  fallbackFile: '/home/user/.local/share/kinolune/secrets.enc',
);

/// The core's source manager on a system without a keychain.
class _NoKeychainSources extends FakeSourceManager {
  _NoKeychainSources(super.library);

  @override
  String? get secretWarning => _coreReason;
}

void main() {
  setUpAll(loadAppFonts);

  group('BUG-012', () {
    testWidgets('a metadata run changes the home line at once', (t) async {
      setSurface(t, const Size(1280, 900));
      final inbox = FakeReviewInbox(
        entries: [
          _entry(1, 'show', ReviewState.check),
          _entry(2, 'show', ReviewState.manual),
        ],
      );
      final s = await demoServices(reviewInbox: inbox);
      s.locale.value = const Locale('ru');
      await _open(t, s, Section.home);
      expect(find.text(ru.reviewHomeLineShows(2)), findsOneWidget);

      // TMDB adds five movies to check: the line counts all seven titles.
      inbox.addFromMetadataRun([
        for (var i = 3; i <= 5; i++) _entry(i, 'movie', ReviewState.check),
        for (var i = 6; i <= 7; i++) _entry(i, 'movie', ReviewState.manual),
      ]);
      await settle(t, 4);
      expect(find.text(ru.reviewHomeLineShows(2)), findsNothing);
      expect(find.text(ru.reviewHomeLineTitles(7)), findsOneWidget);
    });

    testWidgets('and the Settings count without leaving the page', (t) async {
      setSurface(t, const Size(1280, 900));
      final inbox = FakeReviewInbox(
        entries: [
          _entry(1, 'show', ReviewState.manual),
          _entry(2, 'show', ReviewState.manual),
        ],
      );
      final s = await demoServices(reviewInbox: inbox);
      s.locale.value = const Locale('ru');
      await _open(t, s, Section.settings);
      final tile = _key('settings-review-inbox');
      await t.ensureVisible(tile);
      await settle(t, 2);
      expect(find.text(ru.reviewInboxSubtitle(0, 2)), findsOneWidget);

      inbox.addFromMetadataRun([
        for (var i = 3; i <= 5; i++) _entry(i, 'movie', ReviewState.check),
        for (var i = 6; i <= 7; i++) _entry(i, 'movie', ReviewState.manual),
      ]);
      await settle(t, 4);
      expect(find.text(ru.reviewInboxSubtitle(3, 4)), findsOneWidget);
      expect(find.text(ru.reviewInboxSubtitle(0, 2)), findsNothing);
    });
  });

  group('BUG-013', () {
    for (final (code, l) in [('ru', ru), ('en', en)]) {
      testWidgets('no keychain: plain words, details folded ($code)', (
        t,
      ) async {
        setSurface(t, const Size(1280, 900));
        final s = await demoServices(
          secretStorage: _fallback,
          sourceManager: _NoKeychainSources.new,
        );
        await s.dismissSecretNotice();
        s.locale.value = Locale(code);
        await _open(t, s, Section.settings);
        await _push(t, const SourcesScreen());
        final notice = _key('sources-secret-warning');
        expect(notice, findsOneWidget);
        final plain = l.sourcesSecretFallback(
          l.secretsWhyUnavailable,
          'Kinolune', // l10n-ignore
        );
        expect(
          find.descendant(of: notice, matching: find.text(plain)),
          findsOneWidget,
        );
        // Nothing of the core's message on the main line.
        for (final raw in ['zbus', 'unix:path', 'OS keychain']) {
          expect(find.textContaining(raw), findsNothing);
        }
        // «Подробнее» opens it.
        await t.tap(
          find.descendant(
            of: notice,
            matching: find.text(l.sourcesErrorDetails),
          ),
        );
        await settle(t, 4);
        expect(find.text(_coreReason), findsOneWidget);
      });
    }
  });

  group('BUG-014', () {
    testWidgets('an unrecognized series has «Серия N», not «Episode N»', (
      t,
    ) async {
      setSurface(t, const Size(1280, 900));
      // The app names untitled episodes in its language…
      final first = await demoServices(empty: true);
      first.locale.value = const Locale('ru');
      await _open(t, first, Section.home);
      expect(UntitledEpisode.title(13), 'Серия 13');
      addTearDown(() => UntitledEpisode.title = UntitledEpisode.english);

      // …so the library hands them out like that (the core adapter calls
      // it for an episode with no title of its own).
      const show = LibraryItem(
        id: 's1',
        kind: ItemKind.show,
        title: 'Kaijuu Test', // l10n-ignore
        filePath: '/media/Kaijuu Test', // l10n-ignore
      );
      final lib = FakeLibrary.items([
        show,
        for (final e in [13, 14])
          LibraryItem(
            id: 'e$e',
            kind: ItemKind.episode,
            title: UntitledEpisode.title(e),
            showId: 's1',
            season: 2,
            episode: e,
            filePath: '/media/Kaijuu Test - $e.mkv', // l10n-ignore
          ),
      ]);
      final s = await demoServices(library: lib);
      s.locale.value = const Locale('ru');
      await _open(t, s, Section.home);
      await _push(t, const ShowDetailScreen(show: show));
      expect(find.text('Серия 13'), findsWidgets);
      expect(find.textContaining('Episode'), findsNothing);

      // English keeps «Episode N».
      s.locale.value = const Locale('en');
      await settle(t, 4);
      expect(UntitledEpisode.title(13), en.episodeUntitled(13));
      expect(en.episodeUntitled(13), 'Episode 13');
    });
  });

  group('BUG-016', () {
    testWidgets('no banner or card over the video; both come back after', (
      t,
    ) async {
      setSurface(t, const Size(1280, 900));
      final s = await demoServices(secretStorage: _fallback);
      s.locale.value = const Locale('ru');
      await _open(t, s, Section.home);
      final banner = _key('secret-fallback-notice');
      expect(banner, findsOneWidget);

      await _push(
        t,
        PlayerScreen(
          request: const PlayRequest(uri: '/m/e1.mkv', title: 'Movie'),
          onProgress: (_, _) {},
          engineFactory: ({bool video = true}) => FakePlayerEngine(),
        ),
      );
      expect(find.byType(PlayerScreen), findsOneWidget);
      expect(PlayerScreen.anyOpen.value, isTrue);
      expect(banner, findsNothing);

      // A notice that comes while the video plays waits for it.
      s.notices.post(
        InAppNotice(
          id: 'test',
          kind: InAppNoticeKind.download,
          title: (_) => 'Downloaded', // l10n-ignore
          duration: const Duration(seconds: 2),
        ),
      );
      await settle(t, 40);
      expect(find.text('Downloaded'), findsNothing);
      expect(s.notices.current?.id, 'test');

      Navigator.of(t.element(find.byType(PlayerScreen))).pop();
      await settle(t, 8);
      expect(find.byType(PlayerScreen), findsNothing);
      expect(PlayerScreen.anyOpen.value, isFalse);
      // Not dismissed: the banner is back, and the card shows now.
      expect(banner, findsOneWidget);
      expect(find.text('Downloaded'), findsOneWidget);
    });
  });
}
