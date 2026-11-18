// SPDX-License-Identifier: MPL-2.0
//
// The macOS menu bar (UI-022, ADR-NNNN; 01 §2.4): Kinolune · File · View ·
// Playback · Window · Help with the standard shortcuts. Flutter's
// `PlatformMenuBar` puts the menus into the native NSMenu; Hide, Quit,
// Services, Full Screen, Minimize and Zoom are the system's own items
// (`PlatformProvidedMenuItem`). The menus act on the shell of the active
// profile ([AppMenuModel]); before there is one (onboarding, the profile
// picker) its items are disabled. Playback goes to whatever plays now, as
// the system media keys do ([SystemMediaSession]).
//
// Windows and Linux have no menu bar: their windows keep the plain native
// title bar (runner), as research 01 §4.10 asks.

import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../branding.dart';
import '../integration/player.dart';
import '../l10n/l10n.dart';
import '../screens/about_screen.dart';
import '../screens/sources_screen.dart';
import '../services/external_links.dart';
import '../shell/app_shell.dart';

const _windowChannel = MethodChannel('app.kinolune/window');

/// Help: the project's README; problems: a new issue.
const kHelpUrl = '$kRepoUrl#readme';
const kReportIssueUrl = '$kRepoUrl/issues/new/choose';

/// Whether this build shows the macOS menu bar.
bool get appMenuPlatform => !kIsWeb && Platform.isMacOS;

/// The shell the menu bar acts on and the sections it may open.
class AppMenuModel extends ChangeNotifier {
  AppMenuModel();

  /// The app's model (the shell attaches itself after every frame).
  static final AppMenuModel instance = AppMenuModel();

  AppShellState? _shell;
  List<Section> _sections = const [];
  bool _notifyScheduled = false;

  AppShellState? get shell => _shell;

  /// Sections of the navigation now (Twitch, Music… come and go).
  List<Section> get sections => _sections;

  void attach(AppShellState shell, List<Section> sections) {
    if (identical(shell, _shell) && listEquals(sections, _sections)) return;
    _shell = shell;
    _sections = List.unmodifiable(sections);
    _notify();
  }

  void detach(AppShellState shell) {
    if (!identical(shell, _shell)) return;
    _shell = null;
    _sections = const [];
    _notify();
  }

  /// Out of the build and the unmount of the shell: the menu bar sits above
  /// it.
  void _notify() {
    if (_notifyScheduled) return;
    _notifyScheduled = true;
    scheduleMicrotask(() {
      _notifyScheduled = false;
      notifyListeners();
    });
  }
}

/// What the menu items do; tests record the calls.
abstract class AppMenuActions {
  void openSection(Section section);
  void openSettings();
  void openAbout();
  void openSources();
  void search();
  void back();
  void closeWindow();
  void playback(MediaSessionCommand command);
  void togglePlay();
  void openHelp();
  void reportIssue();
}

/// The menu items acting on [model]'s shell.
class ShellMenuActions implements AppMenuActions {
  ShellMenuActions(this.model, {SystemMediaSession? session})
    : _session = session ?? SystemMediaSession.instance;

  final AppMenuModel model;
  final SystemMediaSession _session;

  AppShellState? get _shell {
    final s = model.shell;
    return s != null && s.mounted ? s : null;
  }

  @override
  void openSection(Section section) => _shell?.select(section);

  @override
  void openSettings() => _shell?.select(Section.settings);

  void _pushSettings(Widget page) {
    final shell = _shell;
    if (shell == null) return;
    shell.select(Section.settings);
    // Settings behind a restricted profile's PIN have no navigator.
    final nav = shell.navigatorOf(Section.settings);
    nav?.popUntil((r) => r.isFirst);
    unawaited(nav?.push(MaterialPageRoute<void>(builder: (_) => page)));
  }

  @override
  void openAbout() => _pushSettings(const AboutScreen());

  @override
  void openSources() => _pushSettings(const SourcesScreen());

  @override
  void search() => _shell?.select(Section.search);

  @override
  void back() => _shell?.back();

  @override
  void closeWindow() => unawaited(_invokeWindow('close'));

  @override
  void playback(MediaSessionCommand command) => _session.send(command);

  @override
  void togglePlay() => _session.togglePlay();

  @override
  void openHelp() => unawaited(openExternalLink(Uri.parse(kHelpUrl)));

  @override
  void reportIssue() =>
      unawaited(openExternalLink(Uri.parse(kReportIssueUrl)));

  Future<void> _invokeWindow(String method) async {
    try {
      await _windowChannel.invokeMethod<void>(method);
    } on MissingPluginException {
      // Not macOS.
    } on PlatformException catch (e) {
      debugPrint('Window $method failed: $e');
    }
  }
}

/// ⌘1…⌘7 for the sections, in sidebar order; a hidden section keeps its
/// number free.
const kSectionShortcuts = <Section, LogicalKeyboardKey>{
  Section.home: LogicalKeyboardKey.digit1,
  Section.movies: LogicalKeyboardKey.digit2,
  Section.shows: LogicalKeyboardKey.digit3,
  Section.twitch: LogicalKeyboardKey.digit4,
  Section.music: LogicalKeyboardKey.digit5,
  Section.audiobooks: LogicalKeyboardKey.digit6,
  Section.personal: LogicalKeyboardKey.digit7,
};

SingleActivator _cmd(LogicalKeyboardKey key, {bool alt = false}) =>
    SingleActivator(key, meta: true, alt: alt);

/// The menus of the menu bar. [sections] are those of the navigation now;
/// with [enabled] false (no shell yet) the app's own items are disabled.
List<PlatformMenuItem> buildAppMenus({
  required AppLocalizations l,
  required AppMenuActions actions,
  required List<Section> sections,
  bool enabled = true,
}) {
  VoidCallback? on(VoidCallback f) => enabled ? f : null;
  bool shown(Section s) => sections.contains(s);
  SingleActivator key(Section s) => _cmd(kSectionShortcuts[s]!);
  VoidCallback? open(Section s) => on(() => actions.openSection(s));
  return [
    PlatformMenu(
      label: kAppName,
      menus: [
        PlatformMenuItemGroup(
          members: [
            PlatformMenuItem(
              label: l.menuAbout(kAppName),
              onSelected: on(actions.openAbout),
            ),
          ],
        ),
        PlatformMenuItemGroup(
          members: [
            PlatformMenuItem(
              label: l.menuSettings,
              shortcut: _cmd(LogicalKeyboardKey.comma),
              onSelected: on(actions.openSettings),
            ),
          ],
        ),
        const PlatformMenuItemGroup(
          members: [
            PlatformProvidedMenuItem(
              type: PlatformProvidedMenuItemType.servicesSubmenu,
            ),
          ],
        ),
        const PlatformMenuItemGroup(
          members: [
            PlatformProvidedMenuItem(type: PlatformProvidedMenuItemType.hide),
            PlatformProvidedMenuItem(
              type: PlatformProvidedMenuItemType.hideOtherApplications,
            ),
            PlatformProvidedMenuItem(
              type: PlatformProvidedMenuItemType.showAllApplications,
            ),
          ],
        ),
        const PlatformMenuItemGroup(
          members: [
            PlatformProvidedMenuItem(type: PlatformProvidedMenuItemType.quit),
          ],
        ),
      ],
    ),
    PlatformMenu(
      label: l.menuFile,
      menus: [
        PlatformMenuItemGroup(
          members: [
            PlatformMenuItem(
              label: l.menuSources,
              shortcut: _cmd(LogicalKeyboardKey.keyO),
              onSelected: on(actions.openSources),
            ),
            PlatformMenuItem(
              label: l.menuSearch,
              shortcut: _cmd(LogicalKeyboardKey.keyF),
              onSelected: on(actions.search),
            ),
          ],
        ),
        PlatformMenuItemGroup(
          members: [
            PlatformMenuItem(
              label: l.menuCloseWindow,
              shortcut: _cmd(LogicalKeyboardKey.keyW),
              onSelected: actions.closeWindow,
            ),
          ],
        ),
      ],
    ),
    PlatformMenu(
      label: l.menuView,
      menus: [
        PlatformMenuItemGroup(
          members: [
            PlatformMenuItem(
              label: l.navHome,
              shortcut: key(Section.home),
              onSelected: open(Section.home),
            ),
            PlatformMenuItem(
              label: l.navMovies,
              shortcut: key(Section.movies),
              onSelected: open(Section.movies),
            ),
            PlatformMenuItem(
              label: l.navShows,
              shortcut: key(Section.shows),
              onSelected: open(Section.shows),
            ),
            if (shown(Section.twitch))
              PlatformMenuItem(
                label: l.navTwitch,
                shortcut: key(Section.twitch),
                onSelected: open(Section.twitch),
              ),
            if (shown(Section.music))
              PlatformMenuItem(
                label: l.navMusic,
                shortcut: key(Section.music),
                onSelected: open(Section.music),
              ),
            if (shown(Section.audiobooks))
              PlatformMenuItem(
                label: l.navAudiobooks,
                shortcut: key(Section.audiobooks),
                onSelected: open(Section.audiobooks),
              ),
            if (shown(Section.personal))
              PlatformMenuItem(
                label: l.navPersonal,
                shortcut: key(Section.personal),
                onSelected: open(Section.personal),
              ),
          ],
        ),
        PlatformMenuItemGroup(
          members: [
            PlatformMenuItem(
              label: l.menuBack,
              shortcut: _cmd(LogicalKeyboardKey.bracketLeft),
              onSelected: on(actions.back),
            ),
          ],
        ),
        const PlatformMenuItemGroup(
          members: [
            PlatformProvidedMenuItem(
              type: PlatformProvidedMenuItemType.toggleFullScreen,
            ),
          ],
        ),
      ],
    ),
    PlatformMenu(
      label: l.menuPlayback,
      menus: [
        PlatformMenuItemGroup(
          members: [
            // ⌘P, not Space: a menu key equivalent without ⌘ would take the
            // space from the search field. The player keeps Space itself.
            PlatformMenuItem(
              label: l.menuPlayPause,
              shortcut: _cmd(LogicalKeyboardKey.keyP),
              onSelected: actions.togglePlay,
            ),
            PlatformMenuItem(
              label: l.menuStop,
              shortcut: _cmd(LogicalKeyboardKey.period),
              onSelected: () => actions.playback(MediaSessionCommand.stop),
            ),
          ],
        ),
        PlatformMenuItemGroup(
          members: [
            PlatformMenuItem(
              label: l.menuNext,
              shortcut: _cmd(LogicalKeyboardKey.arrowRight, alt: true),
              onSelected: () => actions.playback(MediaSessionCommand.next),
            ),
            PlatformMenuItem(
              label: l.menuPrevious,
              shortcut: _cmd(LogicalKeyboardKey.arrowLeft, alt: true),
              onSelected: () => actions.playback(MediaSessionCommand.previous),
            ),
          ],
        ),
      ],
    ),
    PlatformMenu(
      label: l.menuWindow,
      menus: const [
        PlatformMenuItemGroup(
          members: [
            PlatformProvidedMenuItem(
              type: PlatformProvidedMenuItemType.minimizeWindow,
            ),
            PlatformProvidedMenuItem(
              type: PlatformProvidedMenuItemType.zoomWindow,
            ),
          ],
        ),
        PlatformMenuItemGroup(
          members: [
            PlatformProvidedMenuItem(
              type: PlatformProvidedMenuItemType.arrangeWindowsInFront,
            ),
          ],
        ),
      ],
    ),
    PlatformMenu(
      label: l.menuHelp,
      menus: [
        PlatformMenuItem(
          label: l.menuHelpGuide(kAppName),
          onSelected: actions.openHelp,
        ),
        PlatformMenuItem(
          label: l.menuReportIssue,
          onSelected: actions.reportIssue,
        ),
      ],
    ),
  ];
}

/// The menu bar over the app (macOS only; elsewhere [child] as is).
class AppMenuBar extends StatelessWidget {
  const AppMenuBar({
    super.key,
    required this.child,
    this.enabled,
    this.actions,
    this.model,
  });

  final Widget child;

  /// Defaults to macOS.
  final bool? enabled;
  final AppMenuActions? actions;
  final AppMenuModel? model;

  @override
  Widget build(BuildContext context) {
    if (!(enabled ?? appMenuPlatform)) return child;
    final m = model ?? AppMenuModel.instance;
    final a = actions ?? ShellMenuActions(m);
    return ListenableBuilder(
      listenable: m,
      builder: (context, child) => PlatformMenuBar(
        menus: buildAppMenus(
          l: context.l10n,
          actions: a,
          sections: m.sections,
          enabled: m.shell != null,
        ),
        child: child!,
      ),
      child: child,
    );
  }
}
