// SPDX-License-Identifier: MPL-2.0
//
// Ukrainian interface language (UI-052): app_uk.arb translates every key of
// app_en.arb with the same placeholders and Ukrainian plural forms, the
// language picker offers it, and the metadata language follows it.

import 'dart:convert';
import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/services/app_services.dart';

Map<String, dynamic> _arb(String locale) =>
    jsonDecode(File('lib/l10n/app_$locale.arb').readAsStringSync())
        as Map<String, dynamic>;

Set<String> _keys(Map<String, dynamic> arb) =>
    arb.keys.where((k) => !k.startsWith('@')).toSet();

/// Placeholder names used in an ICU message (`{name}` and `{name, plural…`).
Set<String> _placeholders(String message) => {
  for (final m in RegExp(r'\{(\w+)[,}]').allMatches(message)) m.group(1)!,
};

void main() {
  group('app_uk.arb', () {
    final en = _arb('en');
    final uk = _arb('uk');

    test('declares the uk locale', () {
      expect(uk['@@locale'], 'uk');
    });

    test('has exactly the keys of en', () {
      final missing = _keys(en).difference(_keys(uk));
      final extra = _keys(uk).difference(_keys(en));
      expect(missing, isEmpty, reason: 'Untranslated keys in app_uk.arb');
      expect(extra, isEmpty, reason: 'Keys missing from app_en.arb');
    });

    test('every key keeps the placeholders of en', () {
      for (final k in _keys(en)) {
        final u = uk[k] as String?;
        if (u == null) continue;
        expect(u.trim(), isNotEmpty, reason: k);
        expect(
          _placeholders(u),
          _placeholders(en[k] as String),
          reason: 'Placeholders differ in "$k"',
        );
      }
    });

    test('Ukrainian plurals have one, few, many and other', () {
      for (final k in _keys(en)) {
        if (!(en[k] as String).contains(', plural,')) continue;
        final u = uk[k] as String;
        for (final form in ['one{', 'few{', 'many{', 'other{']) {
          expect(u, contains(form), reason: '"$k" lacks $form…}');
        }
      }
    });

    test('no letters that Ukrainian does not have', () {
      final russianOnly = RegExp('[ЫыЭэЁёЪъ]');
      for (final k in _keys(uk)) {
        // Endonyms are the same in every locale.
        if (k == 'settingsLanguageRussian') continue;
        expect(
          russianOnly.hasMatch(uk[k] as String),
          isFalse,
          reason: '"$k" looks Russian: ${uk[k]}',
        );
      }
    });

    test('the language name is an endonym in every locale', () {
      for (final locale in ['en', 'ru', 'uk']) {
        expect(_arb(locale)['settingsLanguageUkrainian'], 'Українська');
      }
    });
  });

  group('messages', () {
    final uk = lookupAppLocalizations(const Locale('uk'));

    test('Ukrainian plural forms', () {
      expect(uk.libraryMovieCount(1), '1 фільм');
      expect(uk.libraryMovieCount(3), '3 фільми');
      expect(uk.libraryMovieCount(5), '5 фільмів');
      expect(uk.libraryMovieCount(11), '11 фільмів');
      expect(uk.libraryMovieCount(21), '21 фільм');
      expect(uk.libraryMovieCount(0), '0 фільмів');
      expect(uk.showEpisodeCount(2), '2 серії');
      expect(uk.showEpisodeCount(25), '25 серій');
      expect(uk.musicTrackCount(1), '1 трек');
      expect(uk.musicTrackCount(4), '4 треки');
    });

    test('main navigation is in Ukrainian', () {
      expect(uk.navSettings, 'Налаштування');
      expect(uk.navLibrary, 'Медіатека');
      expect(uk.homeContinueWatching, 'Продовжити перегляд');
    });
  });

  group('language choice', () {
    test('uk is a supported locale and parses from its tag', () {
      expect(AppLocalizations.supportedLocales, contains(const Locale('uk')));
      expect(parseLocaleTag('uk'), const Locale('uk'));
      expect(localeTag(const Locale('uk')), 'uk');
    });

    test('metadata follows the Ukrainian interface', () {
      expect(metadataLanguageFor(const Locale('uk')), 'uk-UA');
      expect(metadataLanguageFor(const Locale('uk', 'UA')), 'uk-UA');
      expect(metadataLanguageFor(const Locale('ru')), 'ru-RU');
      expect(metadataLanguageFor(const Locale('de')), 'en-US');
      expect(AppServices.metadataLanguages, contains('uk-UA'));
    });
  });
}
