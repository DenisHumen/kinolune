// SPDX-License-Identifier: MPL-2.0
//
// PLAY-008 in the app: the OpenSubtitles account is kept in the secure
// store and checked from Settings → Subtitle style; playVideo gives the
// player «Найти субтитры», whose download is loaded and selected. The demo
// library answers without network. With KINOLUNE_SCREENSHOTS_DIR set the
// frames are written for docs/screenshots/progress/play-008:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/play-008 \
//     flutter test test/online_subtitles_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/playback.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/subtitles/online_subtitles.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_library/pa_library.dart'
    show OpenSubtitlesAccount, LibraryOnlineSubtitles;
import 'package:pa_player/pa_player.dart'
    show OsdPanel, PlayerScreen, PlayerScreenState, SubtitleSearchFailure;
import 'package:pa_player/testing.dart';

import 'support/test_app.dart';

const _episode = 'show-moonlit-bureau-s2e4';

Future<AppServices> _services(FakePlayerEngine engine) async {
  final services = AppServices(
    library: FakeLibrary.demo(scanStepDelay: Duration.zero),
    secureStore: MemorySecureStore(),
    playerEngineFactory: ({bool video = true}) => engine,
  );
  await services.init();
  await services.setShowAdvancedSettings(true);
  return services;
}

void main() {
  setUpAll(loadAppFonts);

  test('the account is kept in the secure store, blanks removed', () async {
    final store = MemorySecureStore();
    await writeOpenSubtitlesAccount(
      store,
      const OpenSubtitlesAccount(
        username: ' viewer ',
        password: 'pa ss',
        apiKey: '  ',
        subdlApiKey: ' sd-key ',
      ),
    );
    expect(await store.read(OpenSubtitlesKeys.username), 'viewer');
    expect(await store.read(OpenSubtitlesKeys.apiKey), isNull);
    expect(await store.read(OpenSubtitlesKeys.subdlApiKey), 'sd-key');
    final a = await readOpenSubtitlesAccount(store);
    expect((a.username, a.password, a.apiKey), ('viewer', 'pa ss', null));
    expect(a.subdlApiKey, 'sd-key');
    expect(a.hasLogin, isTrue);
  });

  test('the search maps the library offers and failures', () async {
    final library = FakeLibrary.demo(scanStepDelay: Duration.zero);
    final store = MemorySecureStore();
    final item = (await library.item(_episode))!;
    final search = subtitleSearchFor(library, store, item)!;
    final found = await search.search(const ['rus']);
    expect(found.failure, isNull);
    expect(found.offers.first.hashMatch, isTrue);
    expect(found.quota, isNull, reason: 'no account: unknown until a download');
    // Both providers in one list, each offer with its provider's name.
    expect(found.offers.first.provider, 'OpenSubtitles');
    final subdl = found.offers.where((o) => o.provider == 'SubDL').single;
    expect(subdl.id, 'subdl:0:/subtitle/demo.zip');
    final got = await search.download(found.offers.first);
    expect(got.subtitle!.language, 'rus');
    expect(got.subtitle!.title, endsWith('.ru.ass'));

    await store.write(OpenSubtitlesKeys.username, 'wrong');
    await store.write(OpenSubtitlesKeys.password, 'x');
    final refused = await search.search(const ['rus']);
    expect(refused.failure, SubtitleSearchFailure.login);

    // Music tracks have no subtitle search.
    final track = (await library.tracks().first).first;
    expect(subtitleSearchFor(library, store, track), isNull);
    expect(library, isA<LibraryOnlineSubtitles>());
  });

  for (final (name, ff, size, dpr) in <(String, FormFactor, Size, double)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800), 1),
    ('phone', FormFactor.phone, const Size(412, 892), 2),
  ]) {
    testWidgets('Settings: sign in and see the downloads left ($name)', (
      t,
    ) async {
      setSurface(t, size, dpr: dpr);
      final services = await _services(FakePlayerEngine());
      services.locale.value = const Locale('ru');
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.settings),
      );
      await settle(t);
      final style = find.text('Стиль субтитров').last;
      await t.ensureVisible(style);
      await settle(t, 4);
      await t.tap(style);
      await settle(t);

      final login = find.byKey(const ValueKey('opensubtitles-login'));
      await t.ensureVisible(login);
      await settle(t, 4);
      expect(find.text('Субтитры из интернета'), findsOneWidget);
      await t.enterText(login, 'viewer');
      await t.enterText(
        find.byKey(const ValueKey('opensubtitles-password')),
        'secret',
      );
      final save = find.byKey(const ValueKey('opensubtitles-save'));
      await t.ensureVisible(save);
      await settle(t, 4);
      await t.tap(save);
      await settle(t);
      expect(
        find.text('Вход выполнен. Осталось скачиваний сегодня: 18 из 20'),
        findsOneWidget,
      );
      expect(
        await services.secureStore.read(OpenSubtitlesKeys.username),
        'viewer',
      );
      await t.ensureVisible(find.text('Субтитры из интернета'));
      await settle(t, 4);
      await capture(t, '02-settings-account-$name');

      await t.enterText(login, 'wrong');
      await t.tap(save);
      await settle(t);
      expect(find.text('Неверный логин или пароль'), findsOneWidget);

      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });

    testWidgets('player: find, download and switch ($name)', (t) async {
      setSurface(t, size, dpr: dpr);
      final engine = FakePlayerEngine(
        loadedDuration: const Duration(minutes: 40),
      );
      final services = await _services(engine);
      services.locale.value = const Locale('ru');
      await writeOpenSubtitlesAccount(
        services.secureStore,
        const OpenSubtitlesAccount(username: 'viewer', password: 'secret'),
      );
      await t.pumpWidget(testApp(services, formFactor: ff));
      await settle(t);
      final item = (await services.library.item(_episode))!;
      playVideo(t.element(find.byType(Scaffold).first), item);
      await settle(t);
      final player = t.state<PlayerScreenState>(find.byType(PlayerScreen));

      player.osd.openPanel(OsdPanel.subtitles);
      await settle(t);
      final row = find.byKey(const ValueKey('sub-search'));
      expect(row, findsOneWidget);
      await capture(t, '00-subtitle-menu-$name');
      await t.tap(row);
      await settle(t);
      expect(find.text('Осталось скачиваний: 18 из 20'), findsOneWidget);
      expect(find.byKey(const ValueKey('subsearch-offer-1')), findsOneWidget);
      await capture(t, '01-find-subtitles-$name');

      await t.tap(find.byKey(const ValueKey('subsearch-offer-1')));
      await settle(t);
      final selected = engine.state.value.subtitleTrack!;
      expect(selected.language, 'rus');
      expect(selected.title, endsWith('.ru.ass'));
      expect(find.byKey(const ValueKey('panel-subtitleSearch')), findsNothing);
      final memory = await services.subtitleMemory(item.showId!);
      expect(memory!.choice!.language, 'rus');

      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
