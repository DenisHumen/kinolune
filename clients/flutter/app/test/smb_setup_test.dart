// SPDX-License-Identifier: MPL-2.0
//
// UI-028: adding an SMB folder step by step (server, sign in, pick the
// folder), finding a typed path, plain errors, "Choose folder" in the edit
// form and on a broken source; phone, desktop and TV with the remote.
// With KINOLUNE_SCREENSHOTS_DIR set the frames are written there:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress \
//     flutter test test/smb_setup_test.dart

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_smb_server.dart';
import 'package:kinolune/fakes/fake_source_manager.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sources/discovery.dart';
import 'package:kinolune/sources/source_address.dart';
import 'package:kinolune/sources/source_manager.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final en = lookupAppLocalizations(const Locale('en'));
final ru = lookupAppLocalizations(const Locale('ru'));

FakeSourceManager _fake(AppServices s) => s.sourceManager as FakeSourceManager;

const _nas = StaticSourceDiscovery([
  DiscoveredSource(
    type: SourceType.smb,
    name: 'Home NAS',
    host: 'nas.local',
    url: 'smb://192.168.1.10/',
  ),
]);

Future<AppServices> _open(
  WidgetTester t, {
  Size size = const Size(1280, 900),
  FormFactor formFactor = FormFactor.desktop,
  SourceDiscovery discovery = const NoSourceDiscovery(),
  Locale? locale,
  double dpr = 1,
}) async {
  setSurface(t, size, dpr: dpr);
  final services = await demoServices(discovery: discovery);
  if (locale != null) services.locale.value = locale;
  await t.pumpWidget(
    testApp(services, formFactor: formFactor, section: Section.settings),
  );
  await settle(t);
  return services;
}

Future<void> _tapKey(WidgetTester t, String key) async {
  final f = find.byKey(ValueKey(key));
  await t.ensureVisible(f);
  await t.pump();
  await t.tap(f);
  await settle(t, 6);
}

Future<void> _enter(WidgetTester t, String key, String text) async {
  final f = find.byKey(ValueKey(key));
  await t.ensureVisible(f);
  await t.enterText(f, text);
  await t.pump();
}

Future<void> _startSmb(WidgetTester t, AppLocalizations l) async {
  await t.ensureVisible(find.text(l.addSourceTitle).first);
  await t.tap(find.text(l.addSourceTitle).first);
  await settle(t, 6);
  await t.tap(find.text(l.sourcesTypeSmb));
  await settle(t, 6);
}

Future<void> _signIn(WidgetTester t, {String password = 'secret'}) async {
  await _enter(t, 'smb-user', 'alex');
  await _enter(t, 'smb-password', password);
  await _tapKey(t, 'smb-connect');
}

void main() {
  setUpAll(loadAppFonts);

  group('what the user pastes', () {
    test('any server text is taken apart', () {
      final cases = <String, (String, String, String?)>{
        '192.168.1.23': ('192.168.1.23', '', null),
        r'\\nas\Media\Movies': ('nas', 'Media/Movies', null),
        'smb://nas:445/Media/': ('nas:445', 'Media', null),
        'nas/home/alex/films': ('nas', 'home/alex/films', null),
        '//nas/share': ('nas', 'share', null),
        'alex@nas/x': ('nas', 'x', 'alex'),
        'alex:pw@nas': ('nas', '', 'alex'),
        '/home/alex/smb_shared/torrents/downloads': (
          '',
          'home/alex/smb_shared/torrents/downloads',
          null,
        ),
        ' "smb://nas/a b" ': ('nas', 'a b', null),
      };
      for (final MapEntry(:key, value: (host, path, user)) in cases.entries) {
        final p = parseSmbInput(key);
        expect((p.host, p.path, p.user), (host, path, user), reason: key);
      }
      final p = parseSmbInput('nas/home/alex/films');
      expect((p.share, p.folder), ('home', 'alex/films'));
    });

    test('a wrong share name is not a moved folder', () {
      expect(
        classifySourceMessage(
          'source 3: not found: smb://192.168.1.23/home: share not found: '
          'Connect failed: Tree Connect failed (NT status 0xC00000CC)',
        ),
        SourceErrorCode.shareNotFound,
      );
      expect(
        classifySourceMessage('source 3: not found: smb://nas/media/Movies'),
        SourceErrorCode.pathNotFound,
      );
    });
  });

  group('add flow', () {
    testWidgets('the owner\'s typed path is found and saved', (t) async {
      final services = await _open(t);
      await _startSmb(t, en);
      expect(find.text(en.smbSetupStep(1, 3)), findsOneWidget);
      await _enter(t, 'smb-server', '192.168.1.10');
      // What was typed in alpha.7: a path as on the server's disk.
      await _enter(t, 'smb-path', 'home/alex/smb_shared/torrents/downloads');
      await _tapKey(t, 'smb-next');
      expect(find.text(en.smbSetupLoginHeading('192.168.1.10')), findsOne);
      await _signIn(t);
      expect(
        find.text(en.smbSetupFoundOne('smb_shared', 'torrents/downloads')),
        findsOneWidget,
      );
      expect(
        find.text('192.168.1.10 › smb_shared › torrents › downloads'),
        findsOneWidget,
      );
      expect(find.text(en.smbSetupFiles(2)), findsOneWidget);
      await _tapKey(t, 'smb-use-folder');

      final d = _fake(services).drafts.single;
      expect(d.type, SourceType.smb);
      expect(d.address, 'smb://192.168.1.10/smb_shared/torrents/downloads');
      expect(d.name, 'downloads');
      expect(d.username, 'alex');
      expect(d.password, 'secret');
      expect(find.byKey(const ValueKey('smb-browser')), findsNothing);
    });

    testWidgets('a pasted full address works the same', (t) async {
      final services = await _open(t);
      await _startSmb(t, en);
      await _enter(t, 'smb-server', r'\\192.168.1.10\home\alex\Documents');
      await _tapKey(t, 'smb-next');
      await _signIn(t);
      expect(find.text(en.smbSetupFoundOne('alex', 'Documents')), findsOne);
      await _tapKey(t, 'smb-use-folder');
      expect(
        _fake(services).drafts.single.address,
        'smb://192.168.1.10/alex/Documents',
      );
    });

    testWidgets('pick a folder without knowing any names', (t) async {
      final services = await _open(t, discovery: _nas);
      await _startSmb(t, en);
      // The server found on the network: one tap.
      await _tapKey(t, 'smb-server-192.168.1.10');
      await _signIn(t);
      // Shared folders of the server, nothing hidden or technical.
      for (final s in ['Media', 'smb_shared', 'public']) {
        expect(find.byKey(ValueKey('smb-share-$s')), findsOneWidget);
      }
      final use = t.widget<FilledButton>(
        find.descendant(
          of: find.byKey(const ValueKey('smb-use-folder')),
          matching: find.byType(FilledButton),
        ),
      );
      expect(use.onPressed, isNull, reason: 'a share must be opened first');
      expect(find.text(en.smbSetupPickShareFirst), findsOneWidget);
      await _tapKey(t, 'smb-share-Media');
      await _tapKey(t, 'smb-folder-Movies');
      expect(find.text('192.168.1.10 › Media › Movies'), findsOneWidget);
      await _tapKey(t, 'smb-up');
      expect(find.text('192.168.1.10 › Media'), findsOneWidget);
      await _tapKey(t, 'smb-folder-Movies');
      await _tapKey(t, 'smb-use-folder');
      expect(
        _fake(services).drafts.single.address,
        'smb://192.168.1.10/Media/Movies',
      );
    });

    testWidgets('a discovered server in the kind list skips step 1', (
      t,
    ) async {
      await _open(t, discovery: _nas);
      await t.ensureVisible(find.text(en.addSourceTitle).first);
      await t.tap(find.text(en.addSourceTitle).first);
      await settle(t, 6);
      await t.tap(find.text('Home NAS'));
      await settle(t, 6);
      expect(find.text(en.smbSetupStep(2, 3)), findsOneWidget);
      expect(find.text(en.smbSetupLoginHeading('192.168.1.10')), findsOne);
    });

    testWidgets('guest sees the open folders only', (t) async {
      final services = await _open(t);
      await _startSmb(t, en);
      await _enter(t, 'smb-server', '192.168.1.10');
      await _tapKey(t, 'smb-next');
      await _tapKey(t, 'smb-guest');
      expect(find.byKey(const ValueKey('smb-user')), findsNothing);
      await _tapKey(t, 'smb-connect');
      expect(find.byKey(const ValueKey('smb-share-public')), findsOneWidget);
      expect(find.byKey(const ValueKey('smb-share-Media')), findsNothing);
      await _tapKey(t, 'smb-share-public');
      await _tapKey(t, 'smb-use-folder');
      final d = _fake(services).drafts.single;
      expect(d.address, 'smb://192.168.1.10/public');
      expect(d.username, isNull);
      expect(d.password, isNull);
    });

    testWidgets('errors say what to do', (t) async {
      final services = await _open(t);
      await _startSmb(t, en);
      await _tapKey(t, 'smb-next');
      expect(find.text(en.smbSetupServerEmpty), findsOneWidget);
      await _enter(t, 'smb-server', '192.168.1.99');
      await _tapKey(t, 'smb-next');
      await _signIn(t);
      expect(find.text(en.smbErrorUnreachable), findsOneWidget);
      await t.tap(find.byTooltip(en.commonBack));
      await settle(t, 4);
      await _enter(t, 'smb-server', '192.168.1.10');
      await _tapKey(t, 'smb-next');
      await _signIn(t, password: 'wrong');
      expect(find.text(en.smbErrorAuth), findsOneWidget);
      expect(find.text(en.sourcesErrorPathNotFound), findsNothing);
      // Still on the sign-in step, ready for another try.
      expect(find.byKey(const ValueKey('smb-password')), findsOneWidget);
      await _signIn(t);
      expect(find.byKey(const ValueKey('smb-share-Media')), findsOneWidget);
      // A guest refused by a server that wants a password.
      _fake(services).smbServers = [
        FakeSmbServer(address: '192.168.1.10', guestShares: const {}),
      ];
      await t.tap(find.byTooltip(en.commonBack));
      await settle(t, 4);
      await _tapKey(t, 'smb-guest');
      _fake(services).nextError = const SourceException(
        SourceErrorCode.permissionDenied,
        'permission denied: smb://192.168.1.10/IPC\$',
      );
      await _tapKey(t, 'smb-connect');
      expect(find.text(en.smbErrorGuestRefused), findsOneWidget);
    });

    testWidgets('a path that is nowhere: pick from the list', (t) async {
      await _open(t);
      await _startSmb(t, en);
      await _enter(t, 'smb-server', '192.168.1.10/home/alex/nope');
      await _tapKey(t, 'smb-next');
      await _signIn(t);
      expect(
        find.text(en.smbSetupNotFound('home/alex/nope')),
        findsOneWidget,
      );
      expect(find.byKey(const ValueKey('smb-share-Media')), findsOneWidget);
    });

    testWidgets('several places fit: the user picks one', (t) async {
      final services = await _open(t);
      _fake(services).smbServers = [
        FakeSmbServer(
          address: 'nas',
          shares: const {
            'Media': ['Movies/a.mkv'],
            'Movies': ['b.mkv'],
          },
        ),
      ];
      await _startSmb(t, en);
      await _enter(t, 'smb-server', 'nas/Media/Movies');
      await _tapKey(t, 'smb-next');
      await _signIn(t);
      expect(find.text(en.smbSetupFoundMany), findsOneWidget);
      await _tapKey(t, 'smb-candidate-Movies');
      await _tapKey(t, 'smb-use-folder');
      expect(_fake(services).drafts.single.address, 'smb://nas/Movies');
    });

    testWidgets('advanced options stay folded and are used', (t) async {
      final services = await _open(t);
      await _startSmb(t, en);
      await _enter(t, 'smb-server', '192.168.1.10');
      await _tapKey(t, 'smb-next');
      expect(find.byKey(const ValueKey('source-domain')), findsNothing);
      await _tapKey(t, 'smb-advanced');
      await _enter(t, 'source-domain', 'HOME');
      await t.tap(find.text(en.sourcesFormSmbVersionSmb3));
      await settle(t, 2);
      await _signIn(t);
      final login = _fake(services).smbLogins.last;
      expect(login.smb.domain, 'HOME');
      expect(login.smb.version, 'smb3');
      expect(login.toString(), isNot(contains('secret')));
    });
  });

  group('fixing an existing source', () {
    Future<(AppServices, ManagedSource)> broken(WidgetTester t) async {
      final services = await _open(t);
      final m = await services.createSource(
        const SourceDraft(
          type: SourceType.smb,
          name: 'Films',
          address: 'smb://192.168.1.10/home/alex/smb_shared/torrents/downloads',
          username: 'alex',
          password: 'secret',
        ),
      );
      _fake(services).drafts.clear();
      _fake(services).nextError = const SourceException(
        SourceErrorCode.shareNotFound,
        'source 1: not found: smb://192.168.1.10/home: share not found: '
        'Connect failed: Tree Connect failed (NT status 0xC00000CC)',
      );
      await services.checkSource(m.id);
      await settle(t, 4);
      return (services, m);
    }

    testWidgets('edit form: "Choose folder" fixes the share', (t) async {
      final (services, m) = await broken(t);
      await t.ensureVisible(find.text(en.sourcesManage));
      await t.tap(find.text(en.sourcesManage));
      await settle(t, 8);
      await t.ensureVisible(find.text(en.sourcesEdit).first);
      await t.tap(find.text(en.sourcesEdit).first);
      await settle(t, 6);
      expect(find.text('home › alex › smb_shared › torrents › downloads'), findsOne);
      await _tapKey(t, 'source-choose-folder');
      // Opened right where the folder really is, with the stored password.
      expect(
        find.text(en.smbSetupFoundOne('smb_shared', 'torrents/downloads')),
        findsOneWidget,
      );
      expect(_fake(services).smbLogins.last.sourceId, m.id);
      expect(_fake(services).smbLogins.last.password, isNull);
      await _tapKey(t, 'smb-use-folder');
      expect(find.text('smb_shared › torrents › downloads'), findsOneWidget);
      await _tapKey(t, 'source-submit');
      final d = _fake(services).drafts.single;
      expect(d.address, 'smb://192.168.1.10/smb_shared/torrents/downloads');
      expect(d.password, isNull, reason: 'the stored password is kept');
    });

    testWidgets('a broken source offers the fix on its card', (t) async {
      final (services, m) = await broken(t);
      await t.ensureVisible(find.text(en.sourcesManage));
      await t.tap(find.text(en.sourcesManage));
      await settle(t, 8);
      expect(find.text(en.sourcesErrorShareNotFound), findsOneWidget);
      expect(find.text(en.sourcesErrorPathNotFound), findsNothing);
      await _tapKey(t, 'source-fix-folder-${m.id}');
      await _tapKey(t, 'smb-use-folder');
      final d = _fake(services).drafts.single;
      expect(d.address, 'smb://192.168.1.10/smb_shared/torrents/downloads');
      expect(d.username, 'alex');
      expect(d.password, isNull);
      expect(find.text(en.sourcesErrorShareNotFound), findsNothing);
    });
  });

  group('form factors', () {
    testWidgets('phone: every step fits', (t) async {
      final services = await _open(
        t,
        size: const Size(390, 844),
        formFactor: FormFactor.phone,
        locale: const Locale('ru'),
        dpr: 2,
      );
      await _startSmb(t, ru);
      await _enter(t, 'smb-server', '192.168.1.10');
      await _tapKey(t, 'smb-next');
      await _signIn(t);
      await _tapKey(t, 'smb-share-Media');
      await _tapKey(t, 'smb-use-folder');
      expect(t.takeException(), isNull);
      expect(_fake(services).drafts.single.address, 'smb://192.168.1.10/Media');
    });

    testWidgets('TV: the remote picks a server and a folder', (t) async {
      final services = await _open(
        t,
        size: const Size(1920, 1080),
        formFactor: FormFactor.tv,
        discovery: _nas,
      );
      await _startSmb(t, en);
      // The found server has the focus: OK opens the sign-in.
      await t.sendKeyEvent(LogicalKeyboardKey.select);
      await settle(t, 6);
      expect(find.text(en.smbSetupStep(2, 3)), findsOneWidget);
      // Text comes from the on-screen keyboard or a phone (UI-027).
      await _signIn(t);
      // The first shared folder has the focus; OK opens it.
      final first = find.byKey(const ValueKey('smb-share-Media'));
      expect(Focus.of(t.element(first)).hasFocus, isTrue);
      await t.sendKeyEvent(LogicalKeyboardKey.select);
      await settle(t, 6);
      expect(find.text('192.168.1.10 › Media'), findsOneWidget);
      // Down to "Use this folder", then OK.
      final use = find.byKey(const ValueKey('smb-use-folder'));
      for (var i = 0; i < 12; i++) {
        final focused = FocusManager.instance.primaryFocus?.context;
        if (focused != null &&
            find
                .descendant(of: use, matching: find.byWidget(focused.widget))
                .evaluate()
                .isNotEmpty) {
          break;
        }
        await t.sendKeyEvent(LogicalKeyboardKey.arrowDown);
        await settle(t, 2);
      }
      await t.sendKeyEvent(LogicalKeyboardKey.select);
      await settle(t, 6);
      expect(_fake(services).drafts.single.address, 'smb://192.168.1.10/Media');
    });
  });

  group('screenshots', () {
    for (final (name, ff, size, dpr) in <(String, FormFactor, Size, double)>[
      ('desktop', FormFactor.desktop, const Size(1280, 800), 1),
      ('phone', FormFactor.phone, const Size(390, 844), 2),
      ('tv', FormFactor.tv, const Size(1920, 1080), 1),
    ]) {
      testWidgets('steps on $name', (t) async {
        await _open(
          t,
          size: size,
          formFactor: ff,
          discovery: _nas,
          locale: const Locale('ru'),
          dpr: dpr,
        );
        await _startSmb(t, ru);
        await capture(t, 'smb-easy/01-server-$name');
        await _tapKey(t, 'smb-server-192.168.1.10');
        await _enter(t, 'smb-user', 'alex');
        await _enter(t, 'smb-password', 'secret');
        await settle(t, 2);
        await capture(t, 'smb-easy/02-sign-in-$name');
        await _tapKey(t, 'smb-connect');
        await capture(t, 'smb-easy/03-shared-folders-$name');
        await _tapKey(t, 'smb-share-smb_shared');
        await _tapKey(t, 'smb-folder-torrents');
        await _tapKey(t, 'smb-folder-downloads');
        await capture(t, 'smb-easy/04-use-this-folder-$name');
      });
    }

    testWidgets('typed path, wrong password and the fix on desktop', (
      t,
    ) async {
      final services = await _open(t, locale: const Locale('ru'));
      await _startSmb(t, ru);
      // The owner's case: a path as on the server's disk, found by itself.
      await _enter(t, 'smb-server', '192.168.1.10');
      await _enter(t, 'smb-path', '/home/alex/smb_shared/torrents/downloads');
      await settle(t, 2);
      await capture(t, 'smb-easy/05-typed-path-desktop');
      await _tapKey(t, 'smb-next');
      await _enter(t, 'smb-user', 'alex');
      await _enter(t, 'smb-password', 'wrong');
      await _tapKey(t, 'smb-connect');
      await capture(t, 'smb-easy/06-wrong-password-desktop');
      await _enter(t, 'smb-password', 'secret');
      await _tapKey(t, 'smb-connect');
      await capture(t, 'smb-easy/07-path-found-desktop');
      await _tapKey(t, 'smb-use-folder');

      // A source saved with the wrong share before UI-028.
      final m = await services.createSource(
        const SourceDraft(
          type: SourceType.smb,
          name: 'Фильмы',
          address: 'smb://192.168.1.10/home/alex/smb_shared/torrents/downloads',
          username: 'alex',
          password: 'secret',
        ),
      );
      _fake(services).nextError = const SourceException(
        SourceErrorCode.shareNotFound,
        'not found: smb://192.168.1.10/home: share not found',
      );
      await services.checkSource(m.id);
      await t.ensureVisible(find.text(ru.sourcesManage));
      await t.tap(find.text(ru.sourcesManage));
      await settle(t, 8);
      final fix = find.byKey(ValueKey('source-fix-folder-${m.id}'));
      await t.ensureVisible(fix);
      await settle(t, 2);
      await capture(t, 'smb-easy/08-broken-source-desktop');
      await t.tap(fix);
      await settle(t, 6);
      await capture(t, 'smb-easy/09-fix-picker-desktop');
    });
  });
}
