// SPDX-License-Identifier: MPL-2.0
//
// Adaptive navigation (UI-002; 01 §2.5–2.6, §4.3, §4.10 п.4). The chrome
// follows the window size class (breakpoints.dart), not the device:
// * compact (< 600) — bottom tabs «Home · Library · Twitch · Music · Personal ·
//   Search»; Library holds Movies and TV Shows, profile and settings sit
//   behind the avatar on Home;
// * medium (< 1024) — rail (narrow sidebar);
// * expanded and large — full sidebar;
// * Android TV — top menu, D-pad focus; moving focus along the menu switches
//   the section (tvOS tab bar behaviour), Back goes up to the menu, then Home.
// The Twitch section is hidden unless enabled in Settings, and for
// restricted profiles (UI-015); Music when a profile's folder white list
// leaves no tracks. Settings of a restricted profile and Search with Search
// Lock show a PIN pad instead of their pages (profile_locks.dart).

import 'dart:io';

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../files/files_controller.dart' show UploadRequest;
import '../files/personal_files.dart' show uploadToPersonal;
import '../files/share_intake.dart';
import '../integration/player.dart';
import '../l10n/l10n.dart';
import '../personal/personal_screen.dart';
import '../screens/home_screen.dart';
import '../screens/library_grid_screen.dart';
import '../screens/audiobooks/audiobooks_screen.dart';
import '../screens/music_screen.dart';
import '../screens/playback.dart';
import '../screens/search_screen.dart';
import '../screens/settings_screen.dart';
import '../integration/library.dart';
import '../services/app_services.dart';
import '../screens/twitch_screen.dart';
import '../platform/android_tv.dart';
import '../platform/form_factor.dart';
import '../profiles/profile_avatar.dart';
import '../profiles/profile_locks.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import '../widgets/logo.dart';
import '../widgets/scan_status.dart';
import 'breakpoints.dart';
import 'section_active.dart';

enum Section {
  home(Icons.home_outlined, Icons.home_rounded),
  movies(Icons.movie_outlined, Icons.movie_rounded),
  shows(Icons.tv_outlined, Icons.tv_rounded),
  twitch(Icons.live_tv_outlined, Icons.live_tv_rounded),
  music(Icons.library_music_outlined, Icons.library_music_rounded),
  audiobooks(Icons.menu_book_outlined, Icons.menu_book_rounded),
  personal(Icons.folder_special_outlined, Icons.folder_special_rounded),
  search(Icons.search, Icons.search),
  settings(Icons.settings_outlined, Icons.settings_rounded);

  const Section(this.icon, this.selectedIcon);
  final IconData icon;
  final IconData selectedIcon;

  String label(AppLocalizations l) => switch (this) {
    home => l.navHome,
    movies => l.navMovies,
    shows => l.navShows,
    twitch => l.navTwitch,
    music => l.navMusic,
    audiobooks => l.navAudiobooks,
    personal => l.navPersonal,
    search => l.navSearch,
    settings => l.navSettings,
  };
}

class AppShell extends StatefulWidget {
  const AppShell({super.key, this.initialSection = Section.home});

  final Section initialSection;

  @override
  State<AppShell> createState() => AppShellState();
}

class AppShellState extends State<AppShell> {
  late Section _section = widget.initialSection;
  final _navKeys = {
    for (final s in Section.values) s: GlobalKey<NavigatorState>(),
  };
  late final _observers = {
    for (final s in Section.values) s: _RebuildObserver(() => setState(() {})),
  };
  final _menuFocus = {
    for (final s in Section.values) s: FocusNode(debugLabel: 'menu-${s.name}'),
  };

  Section _lastLibrary = Section.movies;
  AppServices? _services;
  bool _showTwitch = false;
  bool _showMusic = true;

  Section get section => _section;

  /// PLAY-AND-009: Watch Next and launcher links on Android TV.
  AndroidTvLauncher? _tv;
  ShareIntake? _share;

  /// The navigator of section [s] (launcher links push onto it).
  NavigatorState? navigatorOf(Section s) => _navKeys[s]!.currentState;

  /// Whether the Twitch section is in the navigation (Settings toggle, not
  /// hidden for the profile).
  bool get showTwitch => _showTwitch;

  /// Whether the Music section is in the navigation (UI-015).
  bool get showMusic => _showMusic;

  /// Whether the Audiobooks section is in the navigation (MUS-018): only
  /// while the library has books.
  bool get showAudiobooks => _showBooks;
  bool _showBooks = false;

  /// Whether the Personal media section is in the navigation (UI-025):
  /// wherever the library can keep personal media.
  bool get showPersonal => _services?.personal != null;

  /// Sections of the sidebar, rail and TV menu, in order.
  List<Section> get menuSections => [
    Section.home,
    Section.movies,
    Section.shows,
    if (_showTwitch) Section.twitch,
    if (_showMusic) Section.music,
    if (_showBooks) Section.audiobooks,
    if (showPersonal) Section.personal,
    Section.search,
    Section.settings,
  ];

  void select(Section s) {
    if (s == Section.twitch && !_showTwitch) s = Section.home;
    if (s == Section.music && !_showMusic) s = Section.home;
    if (s == Section.audiobooks && !_showBooks) s = Section.home;
    if (s == Section.personal && !showPersonal) s = Section.home;
    // Leaving Settings locks them again for a restricted profile.
    if (_section == Section.settings && s != Section.settings) {
      _services?.profileControl.relockSettings();
    }
    if (s == _section) {
      // Re-selecting a section pops it to its root.
      _navKeys[s]!.currentState?.popUntil((r) => r.isFirst);
    }
    if (s == Section.movies || s == Section.shows) _lastLibrary = s;
    setState(() => _section = s);
  }

  /// The phone Library tab: the last of Movies / TV Shows that was open.
  void selectLibrary() => select(
    _section == Section.movies || _section == Section.shows
        ? _section
        : _lastLibrary,
  );

  void _onShowTwitch() {
    final s = _services!;
    final show = s.twitchVisible;
    final music = !s.profileControl.musicHidden;
    if (show == _showTwitch && music == _showMusic) return;
    setState(() {
      _showTwitch = show;
      _showMusic = music;
      if (!show && _section == Section.twitch) _section = Section.home;
      if (!music && _section == Section.music) _section = Section.home;
    });
  }

  void _onShowBooks() {
    final show = _services!.audiobooks.shown.value;
    if (show == _showBooks) return;
    setState(() {
      _showBooks = show;
      if (!show && _section == Section.audiobooks) _section = Section.home;
    });
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final services = Services.of(context);
    if (!identical(services, _services)) {
      _services?.twitchShown.removeListener(_onShowTwitch);
      _services?.profileControl.removeListener(_onShowTwitch);
      _services?.audiobooks.shown.removeListener(_onShowBooks);
      services.audiobooks.shown.addListener(_onShowBooks);
      _showBooks = services.audiobooks.shown.value;
      _services = services..twitchShown.addListener(_onShowTwitch);
      services.profileControl.addListener(_onShowTwitch);
      _showTwitch = services.twitchVisible;
      _showMusic = !services.profileControl.musicHidden;
      if (!_showTwitch && _section == Section.twitch) _section = Section.home;
      if (!_showMusic && _section == Section.music) _section = Section.home;
      // Books load after the first frame: a start in the section waits.
      if (!_showBooks &&
          _section == Section.audiobooks &&
          widget.initialSection != Section.audiobooks) {
        _section = Section.home;
      }
      _startTv(services);
      _startShare(services);
    }
  }

  void _startTv(AppServices services) {
    _tv?.dispose();
    _tv = null;
    if (kIsWeb || !Platform.isAndroid) return;
    if (FormFactorScope.of(context) != FormFactor.tv) return;
    _tv = AndroidTvLauncher(
      services: services,
      onLink: (link) => openLauncherLink(this, link),
    );
    _tv!.start().ignore();
  }

  /// UI-035: files shared to Kinolune go to «Личное».
  void _startShare(AppServices services) {
    _share?.dispose();
    _share = null;
    if (kIsWeb || !Platform.isAndroid || services.filesControl == null) return;
    _share = ShareIntake(onShared: _onShared);
    _share!.start().ignore();
  }

  Future<void> _onShared(List<String> paths) async {
    if (!mounted) return;
    select(Section.personal);
    await uploadToPersonal(context, [
      for (final p in paths) UploadRequest(p, ownedCopy: true),
    ]);
  }

  @override
  void initState() {
    super.initState();
    if (_section == Section.shows) _lastLibrary = Section.shows;
    // Nested navigators grab focus for their first route; on TV the menu tab
    // must own the initial focus instead.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted && KDensity.of(context).isTv) {
        _menuFocus[_section]!.requestFocus();
      }
    });
  }

  @override
  void dispose() {
    _services?.twitchShown.removeListener(_onShowTwitch);
    _services?.profileControl.removeListener(_onShowTwitch);
    _services?.audiobooks.shown.removeListener(_onShowBooks);
    _tv?.dispose();
    _share?.dispose();
    for (final f in _menuFocus.values) {
      f.dispose();
    }
    super.dispose();
  }

  Widget _rootFor(Section s) => switch (s) {
    Section.home => HomeScreen(
      onOpenSection: (i) => select(
        const [Section.home, Section.movies, Section.shows, Section.music][i],
      ),
      // TWI-013: started Twitch VODs join the shared Continue row while the
      // section is shown.
      continueSources: () => [
        if (_services case final s?)
          s.twitch.continueEntries(enabled: s.twitchShown),
      ],
    ),
    Section.movies => const LibraryGridScreen(kind: ItemKind.movie),
    Section.shows => const LibraryGridScreen(kind: ItemKind.show),
    Section.twitch => const TwitchScreen(),
    Section.music => const MusicScreen(),
    Section.audiobooks => const AudiobooksScreen(),
    Section.personal => const PersonalScreen(),
    Section.search => const SearchScreen(),
    Section.settings => const SettingsScreen(),
  };

  /// The navigator of [s]; Settings and Search behind their locks
  /// (UI-015): a locked section builds none of its pages.
  Widget _sectionNavigator(Section s) {
    final nav = HeroControllerScope.none(
      child: Navigator(
        key: _navKeys[s],
        observers: [_observers[s]!],
        routeDirectionalTraversalEdgeBehavior:
            TraversalEdgeBehavior.parentScope,
        onGenerateRoute: (_) =>
            MaterialPageRoute<void>(builder: (_) => _rootFor(s)),
      ),
    );
    return switch (s) {
      Section.settings => SectionLock(kind: LockKind.settings, child: nav),
      Section.search => SectionLock(kind: LockKind.search, child: nav),
      _ => nav,
    };
  }

  bool get _nestedCanPop => _navKeys[_section]!.currentState?.canPop() ?? false;

  void _handleBack() {
    final nav = _navKeys[_section]!.currentState;
    if (nav != null && nav.canPop()) {
      nav.pop();
      return;
    }
    final d = KDensity.of(context);
    if (d.isTv && !(_menuFocus[_section]!.hasFocus)) {
      _menuFocus[_section]!.requestFocus();
      return;
    }
    if (_section != Section.home) {
      setState(() => _section = Section.home);
      if (d.isTv) _menuFocus[Section.home]!.requestFocus();
    }
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final size = WindowSize.of(context);
    final layout = NavLayout.resolve(FormFactorScope.of(context), size);

    final stack = IndexedStack(
      index: _section.index,
      children: [
        for (final s in Section.values)
          SectionActive(active: s == _section, child: _sectionNavigator(s)),
      ],
    );
    // UI-021: the pages get their own semantics container. A route's modal
    // barrier blocks the semantics of everything painted before it in the
    // same container, which hid the sidebar and the rail from TalkBack,
    // VoiceOver, Narrator and Orca.
    final content = Semantics(
      container: true,
      child: NavLayoutScope(layout: layout, child: stack),
    );

    final canExit =
        _section == Section.home &&
        !_nestedCanPop &&
        !(d.isTv && !_menuFocus[Section.home]!.hasFocus);
    final Widget body = switch (layout) {
      NavLayout.topMenu => _TvLayout(shell: this, content: content),
      NavLayout.sidebar => _SidebarLayout(
        shell: this,
        content: content,
        width: size == WindowSize.large ? 264 : 232,
      ),
      NavLayout.rail => _RailLayout(shell: this, content: content),
      NavLayout.tabs => _PhoneLayout(shell: this, content: content),
    };
    return PopScope(
      canPop: canExit,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) _handleBack();
      },
      child: Shortcuts(
        shortcuts: const {
          SingleActivator(LogicalKeyboardKey.escape): _BackIntent(),
        },
        child: Actions(
          actions: {
            _BackIntent: CallbackAction<_BackIntent>(
              onInvoke: (_) => _handleBack(),
            ),
          },
          child: body,
        ),
      ),
    );
  }
}

class _BackIntent extends Intent {
  const _BackIntent();
}

class _RebuildObserver extends NavigatorObserver {
  _RebuildObserver(this.onChange);
  final VoidCallback onChange;

  void _later() =>
      WidgetsBinding.instance.addPostFrameCallback((_) => onChange());

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) => _later();
  @override
  void didPop(Route<dynamic> route, Route<dynamic>? previousRoute) => _later();
}

/// Mini player docked over the content when music is queued, or a book is
/// open (MUS-018): the one that played last.
class _MiniPlayer extends StatelessWidget {
  const _MiniPlayer({this.maxWidth = 560});

  final double maxWidth;

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final books = services.audiobooks;
    return ListenableBuilder(
      listenable: Listenable.merge([services.music, books]),
      builder: (context, _) {
        final book =
            books.player.book != null &&
            (services.music.current == null || books.bookIsLatest);
        if (services.music.current == null && !book) {
          return const SizedBox.shrink();
        }
        return ConstrainedBox(
          constraints: BoxConstraints(maxWidth: maxWidth),
          child: DecoratedBox(
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(14),
              boxShadow: const [
                BoxShadow(
                  color: Color(0x80000000),
                  blurRadius: 24,
                  offset: Offset(0, 8),
                ),
              ],
            ),
            child: book
                ? const BookMiniBar()
                : NowPlayingBar(
                    controller: services.music,
                    onOpen: () => openNowPlaying(context),
                  ),
          ),
        );
      },
    );
  }
}

/// System insets (edge to edge on Android, the notch on iOS). The shell
/// does not pad its pages: their headers start below the status bar
/// ([KDensity.safeTop], used by `PageTitle` and the Home hero) while
/// backdrops run under it. This scrim keeps the clock and the status icons
/// legible over content scrolled beneath them. Without a top inset (desktop,
/// TV) it draws nothing.
class _StatusBarScrim extends StatelessWidget {
  const _StatusBarScrim();

  @override
  Widget build(BuildContext context) {
    final top = MediaQuery.paddingOf(context).top;
    // Always positioned: a non-positioned child would size the Stack.
    return Positioned(
      left: 0,
      top: 0,
      right: 0,
      height: top,
      child: top <= 0
          ? const SizedBox.shrink()
          : IgnorePointer(
              child: ColoredBox(
                key: const ValueKey('status-bar-scrim'),
                color: KColors.bgBase.withValues(alpha: .72),
              ),
            ),
    );
  }
}

// ---------------------------------------------------------------- desktop

/// Library scan progress at the bottom of the sidebar and the rail.
class _ScanProgress extends StatelessWidget {
  const _ScanProgress({this.compact = false});

  final bool compact;

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    if (!services.scanning) return const SizedBox.shrink();
    final p =
        services.scanProgress ??
        const ScanProgress(phase: ScanPhase.connecting);
    if (compact) {
      // LIB-023: the rail shows the bar; the tooltip says what runs.
      return Tooltip(
        message: scanStatusText(context.l10n, p),
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: LinearProgressIndicator(value: scanFraction(p)),
        ),
      );
    }
    return Padding(
      padding: const EdgeInsets.all(20),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            context.l10n.shellScanningLibrary,
            style: Theme.of(context).textTheme.labelMedium,
          ),
          const SizedBox(height: 8),
          ScanStatusView(progress: p, dense: true),
        ],
      ),
    );
  }
}

class _SidebarLayout extends StatelessWidget {
  const _SidebarLayout({
    required this.shell,
    required this.content,
    this.width = 232,
  });

  final AppShellState shell;
  final Widget content;
  final double width;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      key: const ValueKey('nav-sidebar'),
      body: Row(
        children: [
          Container(
            width: width,
            decoration: BoxDecoration(
              color: KColors.bgElevated,
              border: Border(right: BorderSide(color: KColors.stroke)),
            ),
            child: SafeArea(
              right: false,
              child: FocusTraversalGroup(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    const Padding(
                      padding: EdgeInsets.fromLTRB(20, 22, 20, 26),
                      child: KinoluneLogo(size: 30),
                    ),
                    for (final s in shell.menuSections) ...[
                      if (s == Section.search) const SizedBox(height: 18),
                      _SidebarItem(
                        section: s,
                        selected: shell.section == s,
                        onTap: () => shell.select(s),
                      ),
                    ],
                    const Spacer(),
                    const _ScanProgress(),
                  ],
                ),
              ),
            ),
          ),
          Expanded(
            child: Stack(
              children: [
                Positioned.fill(child: content),
                const _StatusBarScrim(),
                const Positioned(
                  left: 24,
                  right: 24,
                  bottom: 20,
                  child: Center(child: _MiniPlayer()),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _SidebarItem extends StatelessWidget {
  const _SidebarItem({
    required this.section,
    required this.selected,
    required this.onTap,
  });

  final Section section;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 2),
      child: KFocusable(
        onActivate: onTap,
        focusScale: 1.0,
        light: false,
        radius: 10,
        semanticLabel: section.label(context.l10n),
        selected: selected,
        builder: (context, hovered) => AnimatedContainer(
          duration: KMotion.focus,
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
          color: selected
              ? Colors.white.withValues(alpha: .1)
              : hovered
              ? Colors.white.withValues(alpha: .05)
              : Colors.transparent,
          child: Row(
            children: [
              Icon(
                selected ? section.selectedIcon : section.icon,
                size: 20,
                color: selected ? KColors.accentSoft : KColors.textSecondary,
              ),
              const SizedBox(width: 14),
              Text(
                section.label(context.l10n),
                style: theme.textTheme.titleMedium?.copyWith(
                  fontWeight: selected ? FontWeight.w700 : FontWeight.w500,
                  color: selected ? Colors.white : KColors.textSecondary,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

// ---------------------------------------------------------------- rail

/// Medium windows (tablet in portrait, narrow desktop window): icons with
/// short labels in a narrow column.
class _RailLayout extends StatelessWidget {
  const _RailLayout({required this.shell, required this.content});

  final AppShellState shell;
  final Widget content;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      key: const ValueKey('nav-rail'),
      body: Row(
        children: [
          Container(
            width: 88,
            decoration: BoxDecoration(
              color: KColors.bgElevated,
              border: Border(right: BorderSide(color: KColors.stroke)),
            ),
            child: SafeArea(
              right: false,
              child: FocusTraversalGroup(
                child: Column(
                  children: [
                    const Padding(
                      padding: EdgeInsets.fromLTRB(0, 20, 0, 18),
                      child: KinoluneLogo(size: 32, showName: false),
                    ),
                    Expanded(
                      child: SingleChildScrollView(
                        child: Column(
                          children: [
                            for (final s in shell.menuSections) ...[
                              if (s == Section.search)
                                const SizedBox(height: 14),
                              _RailItem(
                                section: s,
                                selected: shell.section == s,
                                onTap: () => shell.select(s),
                              ),
                            ],
                          ],
                        ),
                      ),
                    ),
                    const _ScanProgress(compact: true),
                  ],
                ),
              ),
            ),
          ),
          Expanded(
            child: Stack(
              children: [
                Positioned.fill(child: content),
                const _StatusBarScrim(),
                const Positioned(
                  left: 16,
                  right: 16,
                  bottom: 16,
                  child: Center(child: _MiniPlayer()),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _RailItem extends StatelessWidget {
  const _RailItem({
    required this.section,
    required this.selected,
    required this.onTap,
  });

  final Section section;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final l = context.l10n;
    final label = section == Section.shows ? l.navShowsShort : section.label(l);
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
      child: KFocusable(
        onActivate: onTap,
        focusScale: 1.0,
        light: false,
        radius: 12,
        semanticLabel: section.label(l),
        selected: selected,
        builder: (context, hovered) => Padding(
          padding: const EdgeInsets.symmetric(vertical: 6),
          child: Column(
            children: [
              AnimatedContainer(
                duration: KMotion.focus,
                width: 56,
                height: 32,
                decoration: BoxDecoration(
                  color: selected
                      ? Colors.white.withValues(alpha: .14)
                      : hovered
                      ? Colors.white.withValues(alpha: .06)
                      : Colors.transparent,
                  borderRadius: BorderRadius.circular(16),
                ),
                child: Icon(
                  selected ? section.selectedIcon : section.icon,
                  size: 22,
                  color: selected ? KColors.accentSoft : KColors.textSecondary,
                ),
              ),
              const SizedBox(height: 4),
              Text(
                label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                textAlign: TextAlign.center,
                style: theme.textTheme.labelSmall?.copyWith(
                  fontWeight: selected ? FontWeight.w700 : FontWeight.w500,
                  color: selected ? Colors.white : KColors.textSecondary,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

// ---------------------------------------------------------------- phone

class _PhoneLayout extends StatelessWidget {
  const _PhoneLayout({required this.shell, required this.content});

  final AppShellState shell;
  final Widget content;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    // Section.movies stands for the Library tab (Movies and TV Shows).
    final tabs = [
      Section.home,
      Section.movies,
      if (shell.showTwitch) Section.twitch,
      if (shell.showMusic) Section.music,
      if (shell.showAudiobooks) Section.audiobooks,
      if (shell.showPersonal) Section.personal,
      Section.search,
    ];
    final current = shell.section == Section.shows
        ? Section.movies
        : shell.section;
    final idx = tabs.indexOf(current);
    // Settings has no tab (it sits behind the avatar on Home). NavigationBar
    // always marks some index as selected, so with no matching tab every
    // destination is drawn in the unselected style (UI-023).
    final none = idx < 0;
    final scheme = Theme.of(context).colorScheme;
    final navTheme = NavigationBarTheme.of(context);
    Widget bar = NavigationBar(
      selectedIndex: none ? 0 : idx,
      labelBehavior: NavigationDestinationLabelBehavior.alwaysShow,
      onDestinationSelected: (i) => tabs[i] == Section.movies
          ? shell.selectLibrary()
          : shell.select(tabs[i]),
      destinations: [
        for (final s in tabs)
          if (s == Section.movies)
            NavigationDestination(
              icon: const Icon(Icons.video_library_outlined),
              selectedIcon: Icon(
                none
                    ? Icons.video_library_outlined
                    : Icons.video_library_rounded,
              ),
              label: l.navLibrary,
            )
          else
            NavigationDestination(
              icon: Icon(s.icon),
              selectedIcon: Icon(none ? s.icon : s.selectedIcon),
              label: s.label(l),
            ),
      ],
    );
    if (none) {
      bar = NavigationBarTheme(
        data: navTheme.copyWith(
          indicatorColor: Colors.transparent,
          iconTheme: WidgetStatePropertyAll(
            IconThemeData(color: scheme.onSurfaceVariant, size: 24),
          ),
          labelTextStyle: WidgetStatePropertyAll(
            (navTheme.labelTextStyle?.resolve(const {}) ??
                    Theme.of(context).textTheme.labelMedium)
                ?.copyWith(color: scheme.onSurfaceVariant),
          ),
        ),
        child: bar,
      );
    }
    return Scaffold(
      key: const ValueKey('nav-tabs'),
      body: Stack(
        children: [
          Positioned.fill(child: content),
          const _StatusBarScrim(),
          if (shell.section == Section.home && !shell._nestedCanPop)
            Positioned(
              right: 8,
              top: MediaQuery.paddingOf(context).top + 4,
              child: IconButton(
                key: const ValueKey('nav-profile'),
                tooltip: l.navProfile,
                onPressed: () => shell.select(Section.settings),
                icon: switch (Services.of(context).profileControl.active) {
                  // UI-015: the active profile's avatar.
                  final p? => ProfileAvatar.of(p, size: 32),
                  null => CircleAvatar(
                    radius: 16,
                    backgroundColor: KColors.bgElevated2,
                    foregroundColor: KColors.textPrimary,
                    child: const Icon(Icons.person_rounded, size: 20),
                  ),
                },
              ),
            ),
          const Positioned(left: 8, right: 8, bottom: 8, child: _MiniPlayer()),
        ],
      ),
      bottomNavigationBar: bar,
    );
  }
}

// ---------------------------------------------------------------- TV

class _TvLayout extends StatelessWidget {
  const _TvLayout({required this.shell, required this.content});

  final AppShellState shell;
  final Widget content;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    return Scaffold(
      key: const ValueKey('nav-top-menu'),
      body: Column(
        children: [
          Padding(
            padding: EdgeInsets.fromLTRB(
              d.pageInsets.left,
              18,
              d.pageInsets.right,
              6,
            ),
            child: FocusTraversalGroup(
              child: Row(
                children: [
                  const KinoluneLogo(size: 26),
                  const SizedBox(width: 36),
                  // Scrolls when the labels do not fit (long translations,
                  // the Streams section, TWI-013); left-aligned otherwise.
                  Expanded(
                    child: SingleChildScrollView(
                      scrollDirection: Axis.horizontal,
                      clipBehavior: Clip.none,
                      child: Row(
                        children: [
                          for (final s in shell.menuSections)
                            if (s != Section.search &&
                                s != Section.settings) ...[
                              _TvTab(shell: shell, section: s),
                              const SizedBox(width: 6),
                            ],
                        ],
                      ),
                    ),
                  ),
                  _TvTab(shell: shell, section: Section.search, iconOnly: true),
                  const SizedBox(width: 6),
                  _TvTab(
                    shell: shell,
                    section: Section.settings,
                    iconOnly: true,
                  ),
                ],
              ),
            ),
          ),
          Expanded(
            child: Stack(
              children: [
                Positioned.fill(child: content),
                const Positioned(
                  right: 40,
                  bottom: 24,
                  child: _MiniPlayer(maxWidth: 380),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _TvTab extends StatelessWidget {
  const _TvTab({
    required this.shell,
    required this.section,
    this.iconOnly = false,
  });

  final AppShellState shell;
  final Section section;
  final bool iconOnly;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final selected = shell.section == section;
    return KFocusable(
      selected: selected,
      focusNode: shell._menuFocus[section],
      autofocus: section == shell.widget.initialSection,
      onActivate: () => shell.select(section),
      onFocusChange: (f) {
        if (f && shell.section != section) shell.select(section);
      },
      radius: 999,
      light: false,
      focusScale: 1.06,
      semanticLabel: section.label(context.l10n),
      builder: (context, focused) {
        final fg = focused
            ? KColors.focusLabel
            : (selected ? Colors.white : KColors.textSecondary);
        return AnimatedContainer(
          duration: KMotion.focus,
          padding: EdgeInsets.symmetric(
            horizontal: iconOnly ? 10 : 16,
            vertical: 7,
          ),
          decoration: BoxDecoration(
            color: focused
                ? KColors.focusFill
                : (selected
                      ? Colors.white.withValues(alpha: .14)
                      : Colors.transparent),
            borderRadius: BorderRadius.circular(999),
          ),
          child: iconOnly
              ? Icon(
                  selected ? section.selectedIcon : section.icon,
                  size: 20,
                  color: fg,
                )
              : Text(
                  section.label(context.l10n),
                  style: theme.textTheme.titleMedium?.copyWith(
                    color: fg,
                    fontWeight: selected || focused
                        ? FontWeight.w700
                        : FontWeight.w500,
                  ),
                ),
        );
      },
    );
  }
}
