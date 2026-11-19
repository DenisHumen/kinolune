// SPDX-License-Identifier: MPL-2.0
//
// UI-044: plain-language errors. The coverage test reads the common codes
// of the core straight from `pa_runtime::error_codes`
// (core/crates/pa-runtime/src/error_code.rs) and checks that each one has
// a ru and an en sentence and an «Исправить» action; a code added to the
// core without words in the app fails here. Also: source errors get their
// action (a NAS that does not let us in → the source's form), the raw
// message is only under «Подробнее», and unknown codes fall back to the
// report.

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/errors/friendly_errors.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/offline/offline_downloads.dart';
import 'package:kinolune/screens/add_source_dialog.dart';
import 'package:kinolune/screens/sources_screen.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:kinolune/sources/source_text.dart';
import 'package:kinolune/sync/device_sync.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_core/pa_core.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));
final ru = lookupAppLocalizations(const Locale('ru'));

/// The repository root, from the app's directory (where tests run).
Directory _repo() {
  var dir = Directory.current.absolute;
  while (!File('${dir.path}/core/crates/pa-runtime/Cargo.toml').existsSync()) {
    final up = dir.parent;
    if (up.path == dir.path) throw StateError('repository root not found');
    dir = up;
  }
  return dir;
}

/// The codes of `pub mod common` in `error_code.rs`, as listed in `ALL`.
List<String> coreCommonCodes() {
  final src = File(
    '${_repo().path}/core/crates/pa-runtime/src/error_code.rs',
  ).readAsStringSync();
  final start = src.indexOf('pub mod common {');
  expect(start, greaterThanOrEqualTo(0), reason: 'no `pub mod common`');
  final body = src.substring(start, src.indexOf('\n}', start));
  final consts = {
    for (final m in RegExp(
      r'pub const ([A-Z_]+): &str = "([a-z0-9_]+)";',
    ).allMatches(body))
      m.group(1)!: m.group(2)!,
  };
  final all = RegExp(r'pub const ALL: &\[&str\] = &\[([^\]]*)\]')
      .firstMatch(body)!
      .group(1)!
      .split(',')
      .map((s) => s.trim())
      .where((s) => s.isNotEmpty)
      .toList();
  expect(all.toSet(), consts.keys.toSet(), reason: '`ALL` lists every code');
  return [for (final name in all) consts[name]!];
}

void main() {
  group('coverage of pa_runtime::error_codes', () {
    final codes = coreCommonCodes();

    test('the core has the common codes', () {
      expect(codes.length, greaterThanOrEqualTo(15));
      expect(codes, containsAll(['not_found', 'unreachable', 'no_space']));
    });

    for (final code in codes) {
      test('$code has ru and en text and an action', () {
        expect(
          commonErrorFixes.containsKey(code),
          isTrue,
          reason: 'no action for $code in commonErrorFixes',
        );
        final e = knownErrorText(en, code);
        final r = knownErrorText(ru, code);
        expect(e, isNotNull, reason: 'no en text for $code');
        expect(r, isNotNull, reason: 'no ru text for $code');
        expect(e!.trim(), isNotEmpty);
        expect(r!.trim(), isNotEmpty);
        expect(r, isNot(e), reason: '$code is not translated');
        // The words never show the code itself.
        expect(e.contains(code), isFalse);
        expect(r.contains(code), isFalse);
        final fix = errorFixFor(code);
        expect(errorFixLabel(en, fix), isNotEmpty);
        expect(errorFixLabel(ru, fix), isNotEmpty);
        expect(errorFixHint(ru, fix), isNotEmpty);
      });
    }

    test('the app has no action for a code the core dropped', () {
      expect(commonErrorFixes.keys.toSet(), codes.toSet());
    });
  });

  test('every action has a ru and an en label and hint', () {
    for (final fix in ErrorFix.values) {
      for (final l in [en, ru]) {
        expect(errorFixLabel(l, fix), isNotEmpty);
        expect(errorFixHint(l, fix), isNotEmpty);
      }
    }
    expect(errorFixLabel(ru, ErrorFix.editSource), 'Исправить');
    expect(errorFixLabel(ru, ErrorFix.retry), 'Повторить');
  });

  test('a NAS that does not let us in opens its form', () {
    expect(sourceErrorFix(SourceErrorCode.authFailed), ErrorFix.editSource);
    expect(
      sourceErrorFix(SourceErrorCode.permissionDenied),
      ErrorFix.editSource,
    );
    expect(sourceErrorFix(SourceErrorCode.unreachable), ErrorFix.retry);
    expect(sourceErrorFix(SourceErrorCode.sourceMissing), ErrorFix.openSources);
    // Every source error has words and an action.
    for (final c in SourceErrorCode.values) {
      expect(sourceErrorText(ru, c), isNotEmpty);
      expect(errorFixHint(ru, sourceErrorFix(c)), isNotEmpty);
    }
  });

  test('domain codes and unknown codes', () {
    expect(errorText(ru, 'full'), ru.errorNoSpace);
    expect(errorFixFor('full'), ErrorFix.freeSpace);
    expect(errorText(en, 'unavailable'), en.errorUnreachable);
    // A code from a newer core: general words and the report.
    expect(errorText(ru, 'brand_new_code'), ru.errorUnknown);
    expect(errorFixFor('brand_new_code'), ErrorFix.report);
  });

  test('errors of the core are shown by their code, never raw', () {
    const e = LibraryError(code: 'no_space', message: 'ENOSPC 0xC000007F');
    expect(errorCodeOf(e), 'no_space');
    expect(errorTextOf(ru, e), ru.errorNoSpace);
    expect(errorTextOf(ru, e).contains('0xC'), isFalse);
    expect(errorDetailOf(e), 'no_space: ENOSPC 0xC000007F');
    expect(
      errorTextOf(en, const SourcesError(code: 'unreachable', message: 'x')),
      en.errorUnreachable,
    );
    expect(errorTextOf(ru, const OfflineException('busy')), ru.errorBusy);
    expect(
      errorTextOf(ru, const SyncException(SyncErrorCode.other)),
      ru.errorSync,
    );
    expect(errorTextOf(ru, StateError('boom')), ru.errorInternal);
  });

  group('source card', () {
    Future<(AppServices, FakeSourceManager)> services() async {
      late FakeSourceManager manager;
      final s = await demoServices(
        sourceManager: (lib) => manager = FakeSourceManager(lib),
      );
      return (s, manager);
    }

    Future<void> open(WidgetTester t, AppServices s) async {
      await t.pumpWidget(testApp(s));
      await settle(t, 4);
      final nav = t.state<NavigatorState>(find.byType(Navigator).first);
      // ignore: unawaited_futures
      nav.push(
        MaterialPageRoute<void>(
          builder: (_) => const SourcesScreen(checkOnOpen: false),
        ),
      );
      await settle(t, 6);
    }

    testWidgets('a refused password: words, «Исправить» opens the form, the '
        'code only under «Подробнее»', (t) async {
      setSurface(t, const Size(1280, 900));
      final (s, manager) = await services();
      s.locale.value = const Locale('ru');
      final source = s.managedSources.first;
      manager.nextError = const SourceException(
        SourceErrorCode.authFailed,
        'authentication failed: NT_STATUS_LOGON_FAILURE 0xC000006D',
      );
      await s.checkSource(source.id).catchError((_) => source.status);
      await open(t, s);

      expect(find.text(ru.sourcesErrorAuth), findsOneWidget);
      // The raw status is folded away.
      expect(find.textContaining('0xC000006D'), findsNothing);
      final fix = find.byKey(ValueKey('source-fix-${source.id}'));
      expect(fix, findsOneWidget);
      expect(
        find.descendant(of: fix, matching: find.text(ru.errorFix)),
        findsOneWidget,
      );

      await t.tap(find.text(ru.sourcesErrorDetails).first);
      await settle(t, 4);
      expect(find.textContaining('0xC000006D'), findsOneWidget);

      await capture(t, '03-source-error-fix');
      await t.ensureVisible(fix);
      await t.tap(fix);
      await settle(t, 8);
      expect(find.byType(SourceForm), findsOneWidget);
      await t.pumpWidget(const SizedBox());
      await settle(t, 2);
    });
  });
}
