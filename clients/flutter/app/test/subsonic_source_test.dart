// SPDX-License-Identifier: MPL-2.0
//
// NET-016 in the app: adding a Subsonic server (Navidrome) with the source
// form — address, user name and password, or an OpenSubsonic API key; the
// server is checked before the source is saved — and scrobbles of what
// the music player plays.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/screens/add_source_dialog.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/source_address.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:kinolune/sources/subsonic_link.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart';
import 'package:pa_player/testing.dart';

import 'support/fake_subsonic.dart';
import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));

Future<FakeSourceManager> _openForm(
  WidgetTester tester,
  FakeSubsonicLink link,
) async {
  setSurface(tester, const Size(1280, 900));
  late FakeSourceManager manager;
  final services = await demoServices(
    subsonic: link,
    sourceManager: (library) => manager = FakeSourceManager(library),
  );
  await tester.pumpWidget(testApp(services, section: Section.settings));
  await settle(tester);
  await tester.tap(find.text(en.addSourceTitle).first);
  await settle(tester, 4);
  await tester.ensureVisible(find.text(en.sourcesTypeSubsonic));
  await tester.tap(find.text(en.sourcesTypeSubsonic));
  await settle(tester, 4);
  return manager;
}

Future<void> _enter(WidgetTester tester, String key, String text) async {
  await tester.ensureVisible(find.byKey(ValueKey(key)));
  await tester.enterText(find.byKey(ValueKey(key)), text);
  await tester.pump();
}

Future<void> _submit(WidgetTester tester) async {
  await tester.ensureVisible(find.byKey(const ValueKey('source-submit')));
  await tester.tap(find.byKey(const ValueKey('source-submit')));
  await settle(tester, 8);
}

void main() {
  setUpAll(loadAppFonts);

  test('Subsonic is a music server the core adds and scans', () {
    expect(SourceType.available, contains(SourceType.subsonic));
    expect(SourceType.subsonic.indexable, isTrue);
    expect(SourceType.subsonic.mediaServer, isFalse);
    expect(SourceType.subsonic.musicServer, isTrue);
    expect(SourceType.fromCoreKind('subsonic'), SourceType.subsonic);
    expect(SourceType.subsonic.coreKind, 'subsonic');
  });

  test('Subsonic addresses default to http and drop /rest', () {
    expect(subsonicAddress('nas:4533').address, 'http://nas:4533');
    expect(
      subsonicAddress('https://music.example/navidrome/rest/').address,
      'https://music.example/navidrome',
    );
    expect(subsonicAddress('').error, SourceFormError.urlEmpty);
    expect(
      subsonicAddress('http://anna:pw@nas:4533').error,
      SourceFormError.credentialsInAddress,
    );
  });

  testWidgets('checks the password with the server before saving', (
    tester,
  ) async {
    final link = FakeSubsonicLink();
    final manager = await _openForm(tester, link);
    expect(find.byType(SourceForm), findsOneWidget);
    expect(find.byKey(const ValueKey('subsonic-note')), findsOneWidget);
    expect(find.byKey(const ValueKey('source-advanced')), findsNothing);
    expect(find.byKey(const ValueKey('source-user')), findsOneWidget);

    await _enter(tester, 'source-url', '192.168.1.20:4533');
    await _enter(tester, 'source-user', 'anna');
    await _enter(tester, 'source-password', 'wrong');
    await _submit(tester);
    expect(link.calls.last, 'probe http://192.168.1.20:4533 anna');
    expect(find.text(en.sourcesErrorAuth), findsOneWidget);
    expect(manager.drafts, isEmpty, reason: 'nothing saved');

    await _enter(tester, 'source-password', 'secret');
    await _submit(tester);
    expect(find.byType(SourceForm), findsNothing);
    final d = manager.drafts.single;
    expect(d.type, SourceType.subsonic);
    expect(d.address, 'http://192.168.1.20:4533');
    expect(d.username, 'anna');
    expect(d.password, 'secret');
    expect(d.scanIntoLibrary, isTrue);
  });

  testWidgets('an API key goes without a user name', (tester) async {
    final link = FakeSubsonicLink();
    final manager = await _openForm(tester, link);
    await tester.ensureVisible(
      find.byKey(const ValueKey('source-subsonic-api-key')),
    );
    await tester.tap(find.byKey(const ValueKey('source-subsonic-api-key')));
    await settle(tester, 2);
    expect(find.byKey(const ValueKey('source-user')), findsNothing);
    expect(find.text(en.sourcesSubsonicKeyField), findsOneWidget);

    await _enter(tester, 'source-url', 'https://music.example');
    await _enter(tester, 'source-password', 'key-1');
    await _submit(tester);
    expect(link.calls.last, 'probe https://music.example -');
    final d = manager.drafts.single;
    expect(d.username, isNull);
    expect(d.password, 'key-1');
  });

  group('MusicScrobbler', () {
    late FakePlayerEngine engine;
    late MusicPlayerController player;
    late FakeSubsonicLink link;
    late MusicScrobbler scrobbler;

    setUp(() {
      engine = FakePlayerEngine();
      player = MusicPlayerController(
        engineFactory: ({bool video = true}) => engine,
      );
      link = FakeSubsonicLink();
      scrobbler = MusicScrobbler(player: player, link: link)..start();
    });

    tearDown(() {
      scrobbler.dispose();
      player.dispose();
    });

    void play(Duration d) {
      for (var t = Duration.zero; t < d; t += const Duration(seconds: 1)) {
        engine.advance(const Duration(seconds: 1));
      }
    }

    PlayRequest track(String id, int seconds) => PlayRequest(
      uri: 'http://127.0.0.1/$id.flac',
      itemId: id,
      title: id,
      durationMs: seconds * 1000,
    );

    test('now playing at the start, played after half the track', () async {
      await player.play([track('t1', 100), track('t2', 600)]);
      engine.emit((s) => s.copyWith(duration: const Duration(seconds: 100)));
      expect(scrobbler.sent, ['now t1']);
      play(const Duration(seconds: 49));
      expect(scrobbler.sent, ['now t1']);
      play(const Duration(seconds: 2));
      expect(scrobbler.sent, ['now t1', 'played t1']);
      play(const Duration(seconds: 30));
      expect(scrobbler.sent, hasLength(2), reason: 'once per play');
      expect(link.calls, ['scrobble t1 false', 'scrobble t1 true']);

      // A long track counts after four minutes; seeking does not count.
      player.next();
      await Future<void>.delayed(Duration.zero);
      engine.emit(
        (s) => s.copyWith(
          playlistIndex: 1,
          position: Duration.zero,
          duration: const Duration(seconds: 600),
        ),
      );
      expect(scrobbler.sent.last, 'now t2');
      engine.emit((s) => s.copyWith(position: const Duration(seconds: 200)));
      expect(scrobbler.sent.last, 'now t2', reason: 'a seek is not playback');
      play(const Duration(minutes: 4));
      expect(scrobbler.sent.last, 'played t2');
    });

    test('short tracks are not scrobbled as played', () async {
      await player.play([track('jingle', 20)]);
      engine.emit((s) => s.copyWith(duration: const Duration(seconds: 20)));
      play(const Duration(seconds: 20));
      expect(scrobbler.sent, ['now jingle']);
    });
  });
}
