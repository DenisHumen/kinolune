// SPDX-License-Identifier: MPL-2.0
//
// META-030: the anime database in Settings (daily update switch, status,
// "Check now") and its ODbL attribution in About. With
// KINOLUNE_SCREENSHOTS_DIR set the frames are written there:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/meta-030 \
//     flutter test test/anime_db_test.dart

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:intl/date_symbol_data_local.dart';
import 'package:kinolune/branding.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/anime_db_settings.dart';
import 'package:kinolune/services/anime_db_asset.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));
final en = lookupAppLocalizations(const Locale('en'));

/// An asset bundle with (or without) the bundled database.
class _Bundle extends CachingAssetBundle {
  _Bundle(this.bytes);
  final Uint8List? bytes;

  @override
  Future<ByteData> load(String key) async {
    final b = bytes;
    if (key != kAnimeDbAsset || b == null) throw FlutterError('no $key');
    return ByteData.sublistView(b);
  }
}

void main() {
  setUpAll(() async {
    await loadAppFonts();
    await initializeDateFormatting();
  });

  test('status line: origin, release, titles, last check', () {
    expect(animeDbStatusText(ru, null, 'ru'), ru.settingsAnimeDbNone);
    final bundled = animeDbStatusText(
      en,
      const AnimeDbStatus(
        origin: AnimeDbOrigin.bundled,
        tag: '2026-45',
        lastUpdate: '2026-11-08',
      ),
      'en',
    );
    expect(bundled, contains('Built in, release 2026-45 of 2026-11-08'));
    expect(bundled, contains(en.settingsAnimeDbNeverChecked));
    final updated = animeDbStatusText(
      ru,
      AnimeDbStatus(
        origin: AnimeDbOrigin.downloaded,
        tag: '2026-46',
        lastUpdate: '2026-11-15',
        entries: 38512,
        checkedAt: DateTime(2026, 11, 15, 14, 5),
      ),
      'ru',
    );
    expect(updated, contains('Обновлена, выпуск 2026-46 от 2026-11-15'));
    expect(updated, contains('38512 тайтлов'));
    expect(updated, contains('14:05'));
  });

  test('the bundled database is copied once and replaced when it changes',
      () async {
    final tmp = await Directory.systemTemp.createTemp('kinolune-anime-db');
    addTearDown(() => tmp.delete(recursive: true));
    Future<Directory> support() async => tmp;
    // A build without the file: nothing.
    expect(
      await installBundledAnimeDb(bundle: _Bundle(null), supportDir: support),
      isNull,
    );
    final v1 = Uint8List.fromList(List.generate(1000, (i) => i % 251));
    final path = await installBundledAnimeDb(
      bundle: _Bundle(v1),
      supportDir: support,
    );
    expect(path, isNotNull);
    expect(File(path!).readAsBytesSync(), v1);
    final stamp = File(path).lastModifiedSync();
    // Same build: left as it is.
    await installBundledAnimeDb(bundle: _Bundle(v1), supportDir: support);
    expect(File(path).lastModifiedSync(), stamp);
    // A new build with another database of the same size: replaced.
    final v2 = Uint8List.fromList(List.generate(1000, (i) => (i * 7) % 251));
    await installBundledAnimeDb(bundle: _Bundle(v2), supportDir: support);
    expect(File(path).readAsBytesSync(), v2);
    expect(File('$path.tmp').existsSync(), isFalse);
  });

  testWidgets('the daily update switch is on by default and remembered', (
    t,
  ) async {
    setSurface(t, const Size(1280, 800));
    final services = await demoServices();
    expect(services.animeDataUpdates.value, isTrue);
    services.locale.value = const Locale('ru');
    await t.pumpWidget(
      testApp(
        services,
        formFactor: FormFactor.desktop,
        section: Section.settings,
      ),
    );
    await settle(t);
    final tile = find.byKey(const ValueKey('settings-anime-db-updates'));
    await t.ensureVisible(tile);
    await settle(t, 2);
    expect(find.text(ru.settingsAnimeDbUpdates), findsOneWidget);
    expect(find.text(ru.settingsAnimeDbNone), findsOneWidget);
    await t.tap(tile);
    await settle(t, 2);
    expect(services.animeDataUpdates.value, isFalse);
    expect(
      await services.preferences.read(Preferences.animeDataUpdates),
      'false',
    );
    await t.tap(tile);
    await settle(t, 2);
    expect(services.animeDataUpdates.value, isTrue);
    expect(await services.preferences.read(Preferences.animeDataUpdates), isNull);
    await t.pumpWidget(const SizedBox());
    await settle(t, 2);
  });

  for (final (name, ff, size) in <(String, FormFactor, Size)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('phone', FormFactor.phone, const Size(390, 844)),
  ]) {
    testWidgets('screenshots on $name', (t) async {
      setSurface(t, size, dpr: name == 'phone' ? 2 : 1);
      final services = await demoServices();
      services.locale.value = const Locale('ru');
      services.animeDbStatus.value = AnimeDbStatus(
        origin: AnimeDbOrigin.downloaded,
        tag: '2026-46',
        lastUpdate: '2026-11-15',
        entries: 38512,
        checkedAt: DateTime(2026, 11, 15, 14, 5),
      );
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.settings),
      );
      await settle(t);
      final status = find.byKey(const ValueKey('settings-anime-db-status'));
      await t.ensureVisible(status);
      await settle(t, 4);
      expect(find.text(ru.settingsAnimeDbCheckNow), findsOneWidget);
      await capture(t, '0${name == 'desktop' ? 1 : 2}-settings-$name');

      if (name == 'desktop') {
        // About → attribution: the ODbL notice.
        final about = find.text(ru.settingsAboutApp(kAppName));
        await t.ensureVisible(about);
        await settle(t, 2);
        await t.tap(about);
        await settle(t, 8);
        final notice = find.text(ru.aboutAnimeDbHeading);
        await t.ensureVisible(notice);
        await settle(t, 4);
        expect(notice, findsOneWidget);
        await capture(t, '03-about-attribution-$name');
      }
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  }
}
