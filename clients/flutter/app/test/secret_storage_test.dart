// SPDX-License-Identifier: MPL-2.0
//
// CORE-007: when the OS keychain does not work the core keeps secrets in an
// encrypted file; the app says so once, Settings → Privacy says where they
// are, and a secret error in the Sources flow is a readable sentence instead
// of an endless spinner.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/services/preferences.dart';
import 'package:kinolune/services/secret_storage.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:kinolune/sources/source_text.dart';
import 'package:kinolune/widgets/secret_storage_text.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));
final ru = lookupAppLocalizations(const Locale('ru'));

const _fallback = SecretStorageInfo(
  backend: 'encrypted-file',
  fallbackKind: SecretFallbackKind.locked,
  fallbackReason:
      'OS keychain: secret storage is locked or access was denied: '
      'the default keyring is locked; unlocking it needs a prompt',
  fallbackFile: '/home/user/.local/share/kinolune/secrets.enc',
);

/// Its `create` fails like a Rust bridge call that is not a SourcesError.
class _CrashingSourceManager extends FakeSourceManager {
  _CrashingSourceManager(super.library);

  @override
  Future<ManagedSource> create(SourceDraft draft) async =>
      throw StateError('bridge call failed');
}

Future<void> _submitSmb(WidgetTester tester) async {
  await tester.tap(find.text(en.addSourceTitle).first);
  await settle(tester, 4);
  await tester.tap(find.text(en.sourcesTypeSmb));
  await settle(tester, 4);
  for (final (key, text) in [('source-host', 'nas'), ('source-share', 'tv')]) {
    await tester.ensureVisible(find.byKey(ValueKey(key)));
    await tester.enterText(find.byKey(ValueKey(key)), text);
    await tester.pump();
  }
  await tester.ensureVisible(find.byKey(const ValueKey('source-submit')));
  await tester.tap(find.byKey(const ValueKey('source-submit')));
  await settle(tester, 8);
}

void main() {
  group('secret errors of the core', () {
    test('locked and timed-out keychains are told apart', () {
      expect(
        classifySourceError(
          'secrets',
          'secret store: secret storage did not answer: '
              'secret-service set took longer than 10 s',
        ),
        SourceErrorCode.secretStoreTimeout,
      );
      expect(
        classifySourceError(
          'secrets',
          'secret store: secret storage is locked or access was denied: x',
        ),
        SourceErrorCode.secretStoreLocked,
      );
      expect(
        classifySourceError('secrets', 'secret store: secret storage failed'),
        SourceErrorCode.secretStore,
      );
    });

    test('texts in en and ru', () {
      for (final l in [en, ru]) {
        expect(
          sourceErrorText(l, SourceErrorCode.secretStoreTimeout),
          isNot(sourceErrorText(l, SourceErrorCode.secretStoreLocked)),
        );
        for (final kind in [...SecretFallbackKind.values, null]) {
          expect(secretFallbackWhy(l, kind), isNotEmpty);
        }
        expect(secretStorageText(l, _fallback), contains('secrets.enc'));
        expect(
          secretStorageText(l, const SecretStorageInfo(backend: 'keychain')),
          contains('Keychain'),
        );
      }
    });
  });

  group('fallback notice', () {
    test('is pending once, until dismissed', () async {
      final prefs = MemoryPreferences();
      final s = await demoServices(
        preferences: prefs,
        secretStorage: _fallback,
      );
      expect(s.secretNoticePending, isTrue);
      await s.dismissSecretNotice();
      expect(s.secretNoticePending, isFalse);
      expect(
        await prefs.read(Preferences.secretFallbackNoticeShown),
        'true',
      );
      final again = await demoServices(
        preferences: prefs,
        secretStorage: _fallback,
      );
      expect(again.secretNoticePending, isFalse);
    });

    test('not shown when the keychain works', () async {
      final s = await demoServices(
        secretStorage: const SecretStorageInfo(backend: 'secret-service'),
      );
      expect(s.secretNoticePending, isFalse);
    });

    testWidgets('banner with the reason; "Got it" hides it', (tester) async {
      setSurface(tester, const Size(1280, 900));
      final s = await demoServices(secretStorage: _fallback);
      await tester.pumpWidget(testApp(s));
      await settle(tester);
      final banner = find.byKey(const ValueKey('secret-fallback-notice'));
      expect(banner, findsOneWidget);
      expect(find.text(secretFallbackNotice(en, _fallback)), findsOneWidget);
      expect(find.textContaining(en.secretsWhyLocked), findsWidgets);
      await tester.tap(find.text(en.secretsNoticeOk));
      await settle(tester, 4);
      expect(banner, findsNothing);
      expect(s.secretNoticePending, isFalse);
    });
  });

  testWidgets('Settings → Privacy says where passwords are', (tester) async {
    setSurface(tester, const Size(1280, 4000));
    final s = await demoServices(secretStorage: _fallback);
    await s.dismissSecretNotice();
    await tester.pumpWidget(testApp(s, section: Section.settings));
    await settle(tester);
    final tile = find.byKey(const ValueKey('settings-secret-storage'));
    await tester.scrollUntilVisible(
      tile,
      300,
      scrollable: find.byType(Scrollable).first,
    );
    await settle(tester, 2);
    expect(find.text(en.settingsPrivacySecrets), findsOneWidget);
    expect(find.text(secretStorageText(en, _fallback)), findsOneWidget);
  });

  group('add source', () {
    testWidgets('a keychain that did not answer is a readable error', (
      tester,
    ) async {
      setSurface(tester, const Size(1280, 900));
      final s = await demoServices();
      (s.sourceManager as FakeSourceManager).nextError =
          SourceException.fromCore(
            'secrets',
            'secret store: secret storage did not answer: '
                'secret-service set took longer than 10 s',
          );
      await tester.pumpWidget(testApp(s, section: Section.settings));
      await settle(tester);
      await _submitSmb(tester);
      expect(find.text(en.sourcesErrorSecretTimeout), findsOneWidget);
      // Not busy: the form can be sent again.
      final submit = tester.widget<FilledButton>(
        find.byKey(const ValueKey('source-submit')),
      );
      expect(submit.onPressed, isNotNull);
    });

    testWidgets('any other failure is shown, not an endless spinner', (
      tester,
    ) async {
      setSurface(tester, const Size(1280, 900));
      final s = await demoServices(sourceManager: _CrashingSourceManager.new);
      await tester.pumpWidget(testApp(s, section: Section.settings));
      await settle(tester);
      await _submitSmb(tester);
      expect(find.text(en.sourcesErrorOther), findsOneWidget);
      final submit = tester.widget<FilledButton>(
        find.byKey(const ValueKey('source-submit')),
      );
      expect(submit.onPressed, isNotNull);
    });
  });
}
