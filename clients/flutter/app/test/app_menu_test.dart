// SPDX-License-Identifier: MPL-2.0
//
// UI-022: the macOS menu bar — its menus and standard shortcuts, the
// items wired to the shell and the playback session, and the menus as they
// reach the platform (`flutter/menu` is mocked: the test host is not macOS).

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/app.dart';
import 'package:kinolune/integration/player.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/app_menu.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/about_screen.dart';
import 'package:kinolune/screens/sources_screen.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart' show MediaSessionInfo, MediaSessionKind;

import 'support/test_app.dart';

class _Recorder implements AppMenuActions {
  final calls = <String>[];

  @override
  void openSection(Section section) => calls.add('section:${section.name}');
  @override
  void openSettings() => calls.add('settings');
  @override
  void openAbout() => calls.add('about');
  @override
  void openSources() => calls.add('sources');
  @override
  void search() => calls.add('search');
  @override
  void back() => calls.add('back');
  @override
  void closeWindow() => calls.add('close');
  @override
  void playback(MediaSessionCommand command) =>
      calls.add('playback:${command.name}');
  @override
  void togglePlay() => calls.add('toggle');
  @override
  void openHelp() => calls.add('help');
  @override
  void reportIssue() => calls.add('issue');
}

final _en = lookupAppLocalizations(const Locale('en'));

const _allSections = [
  Section.home,
  Section.movies,
  Section.shows,
  Section.music,
  Section.search,
  Section.settings,
];

/// Every item of [menus] by label (groups flattened).
Map<String, PlatformMenuItem> _items(List<PlatformMenuItem> menus) {
  final out = <String, PlatformMenuItem>{};
  void walk(List<PlatformMenuItem> items) {
    for (final i in items) {
      switch (i) {
        case PlatformMenu(:final menus):
          out[i.label] = i;
          walk(menus);
        case PlatformMenuItemGroup(:final members):
          walk(members);
        case PlatformProvidedMenuItem():
          break;
        default:
          out[i.label] = i;
      }
    }
  }

  walk(menus);
  return out;
}

String _shortcut(PlatformMenuItem item) {
  final s = item.shortcut! as SingleActivator;
  return [
    if (s.control) '⌃',
    if (s.alt) '⌥',
    if (s.shift) '⇧',
    if (s.meta) '⌘',
    s.trigger.keyLabel,
  ].join();
}

/// Item labels of the menus as the platform got them, with their ids.
Map<String, int> _sentItems(Object? menus) {
  final out = <String, int>{};
  void walk(Object? node) {
    if (node is List) {
      for (final n in node) {
        walk(n);
      }
    } else if (node is Map) {
      final label = node['label'];
      final id = node['id'];
      if (label is String && id is int) out[label] = id;
      walk(node['children']);
      for (final v in node.values) {
        if (v is List) walk(v);
      }
    }
  }

  walk(menus);
  return out;
}

void main() {
  group('buildAppMenus', () {
    test('Kinolune · File · View · Playback · Window · Help', () {
      final menus = buildAppMenus(
        l: _en,
        actions: _Recorder(),
        sections: _allSections,
      );
      expect([for (final m in menus) m.label], [
        'Kinolune',
        'File',
        'View',
        'Playback',
        'Window',
        'Help',
      ]);
      // The system's own items: Services, Hide, Quit, Full Screen, Minimize.
      final provided = <PlatformProvidedMenuItemType>[];
      void walk(List<PlatformMenuItem> items) {
        for (final i in items) {
          if (i is PlatformProvidedMenuItem) provided.add(i.type);
          if (i is PlatformMenu) walk(i.menus);
          if (i is PlatformMenuItemGroup) walk(i.members);
        }
      }

      walk(menus);
      expect(
        provided,
        containsAll([
          PlatformProvidedMenuItemType.servicesSubmenu,
          PlatformProvidedMenuItemType.hide,
          PlatformProvidedMenuItemType.hideOtherApplications,
          PlatformProvidedMenuItemType.showAllApplications,
          PlatformProvidedMenuItemType.quit,
          PlatformProvidedMenuItemType.toggleFullScreen,
          PlatformProvidedMenuItemType.minimizeWindow,
          PlatformProvidedMenuItemType.zoomWindow,
          PlatformProvidedMenuItemType.arrangeWindowsInFront,
        ]),
      );
    });

    test('standard shortcuts', () {
      final items = _items(
        buildAppMenus(l: _en, actions: _Recorder(), sections: _allSections),
      );
      final expected = {
        'Settings…': '⌘,',
        'Sources…': '⌘O',
        'Search': '⌘F',
        'Close Window': '⌘W',
        'Home': '⌘1',
        'Movies': '⌘2',
        'TV Shows': '⌘3',
        'Music': '⌘5',
        'Back': '⌘[',
        'Play/Pause': '⌘P',
        'Stop': '⌘.',
        'Next': '⌥⌘Arrow Right',
        'Previous': '⌥⌘Arrow Left',
      };
      for (final MapEntry(:key, :value) in expected.entries) {
        expect(_shortcut(items[key]!), value, reason: key);
      }
      expect(items['About Kinolune']!.shortcut, isNull);
    });

    test('hidden sections are not in View, shown ones are', () {
      List<String> view(List<Section> sections) {
        final menus = buildAppMenus(
          l: _en,
          actions: _Recorder(),
          sections: sections,
        );
        return _items(
          (menus[2] as PlatformMenu).menus,
        ).keys.where((k) => k != 'Back').toList();
      }

      expect(view(_allSections), ['Home', 'Movies', 'TV Shows', 'Music']);
      expect(
        view([..._allSections, Section.twitch, Section.personal]),
        ['Home', 'Movies', 'TV Shows', 'Streams', 'Music', 'Personal'],
      );
    });

    test('every item calls its action', () {
      final r = _Recorder();
      final items = _items(
        buildAppMenus(
          l: _en,
          actions: r,
          sections: [..._allSections, Section.twitch],
        ),
      );
      for (final label in [
        'About Kinolune',
        'Settings…',
        'Sources…',
        'Search',
        'Close Window',
        'Movies',
        'Streams',
        'Back',
        'Play/Pause',
        'Stop',
        'Next',
        'Previous',
        'Kinolune Help',
        'Report a Problem…',
      ]) {
        items[label]!.onSelected!();
      }
      expect(r.calls, [
        'about',
        'settings',
        'sources',
        'search',
        'close',
        'section:movies',
        'section:twitch',
        'back',
        'toggle',
        'playback:stop',
        'playback:next',
        'playback:previous',
        'help',
        'issue',
      ]);
    });

    test('without a shell the app items are off, the window ones not', () {
      final items = _items(
        buildAppMenus(
          l: _en,
          actions: _Recorder(),
          sections: const [],
          enabled: false,
        ),
      );
      expect(items['Settings…']!.onSelected, isNull);
      expect(items['Home']!.onSelected, isNull);
      expect(items['Close Window']!.onSelected, isNotNull);
      expect(items['Play/Pause']!.onSelected, isNotNull);
    });
  });

  group('ShellMenuActions', () {
    testWidgets('acts on the shell', (t) async {
      setSurface(t, const Size(1280, 800));
      await t.pumpWidget(testApp(await demoServices()));
      await settle(t);
      final model = AppMenuModel.instance;
      final shell = t.state<AppShellState>(find.byType(AppShell));
      expect(model.shell, same(shell));
      expect(model.sections, shell.menuSections);
      final actions = ShellMenuActions(model);

      actions.openSection(Section.movies);
      await settle(t);
      expect(shell.section, Section.movies);

      actions.search();
      await settle(t);
      expect(shell.section, Section.search);

      actions.openAbout();
      await settle(t);
      expect(shell.section, Section.settings);
      expect(find.byType(AboutScreen), findsOneWidget);

      actions.back();
      await settle(t);
      expect(find.byType(AboutScreen), findsNothing);

      actions.openSources();
      await settle(t);
      expect(find.byType(SourcesScreen), findsOneWidget);
    });

    test('playback goes to what plays now', () {
      final session = SystemMediaSession(enabled: false);
      final actions = ShellMenuActions(AppMenuModel(), session: session);
      final got = <MediaSessionCommand>[];
      final claim = session.claim((c, _) => got.add(c));
      // Nothing plays yet: Play/Pause plays.
      actions.togglePlay();
      claim.update(
        const MediaSessionInfo(
          kind: MediaSessionKind.video,
          title: 'Harbor Lights',
          playing: true,
        ),
      );
      actions.togglePlay();
      actions.playback(MediaSessionCommand.next);
      expect(got, [
        MediaSessionCommand.play,
        MediaSessionCommand.pause,
        MediaSessionCommand.next,
      ]);
      claim.release();
      // Nothing plays: the items do nothing.
      actions.togglePlay();
      expect(got, hasLength(3));
    });
  });

  testWidgets('the menu bar reaches the platform and its items work', (
    t,
  ) async {
    final sent = <Object?>[];
    t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.menu,
      (call) async {
        if (call.method == 'Menu.setMenus') sent.add(call.arguments);
        return null;
      },
    );
    addTearDown(
      () => t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.menu,
        null,
      ),
    );
    setSurface(t, const Size(1280, 800));
    final services = await demoServices();
    await t.pumpWidget(
      KinoluneApp(
        services: services,
        formFactor: FormFactor.desktop,
        menuBar: true,
      ),
    );
    await settle(t);
    expect(sent, isNotEmpty);
    final items = _sentItems(sent.last);
    for (final label in [
      'Kinolune',
      'File',
      'View',
      'Playback',
      'Window',
      'Help',
      'Settings…',
      'Movies',
    ]) {
      expect(items, contains(label));
    }

    // The user picks View → Movies.
    await t.binding.defaultBinaryMessenger.handlePlatformMessage(
      SystemChannels.menu.name,
      SystemChannels.menu.codec.encodeMethodCall(
        MethodCall('Menu.selectedCallback', items['Movies']),
      ),
      (_) {},
    );
    await settle(t);
    expect(
      t.state<AppShellState>(find.byType(AppShell)).section,
      Section.movies,
    );

    // Switching Streams on adds it to View.
    await services.setShowTwitch(true);
    await settle(t);
    expect(_sentItems(sent.last), contains('Streams'));
  });
}
