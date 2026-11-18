// SPDX-License-Identifier: MPL-2.0

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import 'branding.dart';
import 'fakes/fake_library.dart';
import 'fakes/fake_personal_media.dart';
import 'fakes/fake_review_inbox.dart';
import 'features/updates/update_section.dart';
import 'flavor.dart';
import 'l10n/l10n.dart';
import 'licenses.dart';
import 'notices/notice_feeds.dart';
import 'notices/notice_host.dart';
import 'personal/personal_screen.dart' show PersonalPrivacy;
import 'platform/app_menu.dart';
import 'platform/form_factor.dart';
import 'profiles/profile_locks.dart';
import 'profiles/profile_picker.dart';
import 'screens/onboarding_screen.dart';
import 'screens/twitch/twitch_channel_screen.dart';
import 'services/app_services.dart';
import 'services/notification_taps.dart';
import 'services/smoke_play.dart';
import 'shell/app_shell.dart';
import 'sync/remote_input.dart';
import 'sync/remote_input_platform.dart';
import 'theme/kinolune_theme.dart';
import 'twitch/twitch_models.dart';
import 'widgets/focusable.dart';
import 'widgets/secret_storage_text.dart';

class KinoluneApp extends StatefulWidget {
  const KinoluneApp({
    super.key,
    required this.services,
    required this.formFactor,
    this.initialSection = Section.home,
    this.libmpvMissing = false,
    this.menuBar,
  });

  /// The macOS menu bar (UI-022); null = on macOS only (tests force it).
  final bool? menuBar;

  /// Shows a dismissible banner: libmpv could not be loaded (Linux).
  final bool libmpvMissing;

  final AppServices services;
  final FormFactor formFactor;
  final Section initialSection;

  @override
  State<KinoluneApp> createState() => _KinoluneAppState();
}

class _KinoluneAppState extends State<KinoluneApp> {
  late AppServices _services = widget.services;
  late bool _showLibmpvWarning = widget.libmpvMissing;

  /// UI-026: notice buttons act under the app's navigator.
  final _navigatorKey = GlobalKey<NavigatorState>();
  late NoticeFeeds _noticeFeeds = NoticeFeeds(_services);

  /// UI-027: "Type from your phone" while a phone app is in the background.
  late final RemoteInputNotifier? _remoteInputNotifier =
      platformRemoteInputNotifier(
        () => lookupAppLocalizations(
          _services.locale.value ?? _services.systemLocale(),
        ),
      );

  /// App Lock and Settings/Search locks follow the app to the background
  /// and back (UI-015).
  late final AppLifecycleListener _lifecycle = AppLifecycleListener(
    onHide: () {
      _services.profileControl.onBackground();
      // MUS-003: the queue and playhead survive the app being killed.
      _services.musicQueue.save().ignore();
    },
  );

  @override
  void initState() {
    super.initState();
    registerAppLicenses();
    _lifecycle;
    _noticeFeeds.start();
    // PKG-021: release smoke test, only with KINOLUNE_SMOKE_PLAY set.
    armSmokePlay(_navigatorKey, () => _services);
    // QA-015: a tap on "X is live" opens the channel.
    NotificationTaps.instance.listen(_onNotificationTap);
  }

  /// A tap on one of the app's notifications: `twitch/channel/<login>`
  /// opens that channel's page over whatever is on screen.
  void _onNotificationTap(String payload) {
    final login = twitchChannelOfPayload(payload);
    if (login == null || !mounted) return;
    final nav = _navigatorKey.currentState;
    if (nav == null) {
      // A tap that started the app comes before the first frame.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (_navigatorKey.currentState != null) _onNotificationTap(payload);
      });
      return;
    }
    final favorite = _services.twitch.favorites
        .where((f) => f.login == login)
        .firstOrNull;
    nav.push(
      MaterialPageRoute<void>(
        builder: (_) => TwitchChannelScreen(
          channel: TwitchChannel(
            login: login,
            displayName: favorite?.displayName ?? login,
            userId: favorite?.userId,
            avatarUrl: favorite?.avatarUrl,
          ),
        ),
      ),
    );
  }

  @override
  void dispose() {
    NotificationTaps.instance.unlisten(_onNotificationTap);
    _noticeFeeds.dispose();
    _lifecycle.dispose();
    super.dispose();
  }

  Future<void> _useDemo() async {
    final demo = AppServices(
      library: FakeLibrary.demo(),
      secureStore: _services.secureStore,
      preferences: _services.preferences,
      personal: FakePersonalMedia.demo(),
      reviewInbox: FakeReviewInbox.demo(),
    );
    await demo.init();
    _noticeFeeds.dispose();
    _noticeFeeds = NoticeFeeds(demo)..start();
    setState(() => _services = demo);
  }

  @override
  Widget build(BuildContext context) {
    final tv = widget.formFactor == FormFactor.tv;
    return FormFactorScope(
      formFactor: widget.formFactor,
      child: Services(
        services: _services,
        child: ValueListenableBuilder<Locale?>(
          valueListenable: _services.locale,
          builder: (context, locale, _) => _app(tv, locale),
        ),
      ),
    );
  }

  /// UI-026: in-app notice banners over everything but App Lock; UI-027:
  /// typing on the TV from a phone.
  Widget _notices(Widget child) => PersonalPrivacy(
    child: RemoteInputHost(
      sync: _services.sync,
      notices: _services.notices,
      navigatorKey: _navigatorKey,
      tv: widget.formFactor == FormFactor.tv,
      phone: widget.formFactor == FormFactor.phone,
      notifier: _remoteInputNotifier,
      child: InAppNoticeHost(
        notices: _services.notices,
        navigatorKey: _navigatorKey,
        child: child,
      ),
    ),
  );

  Widget _app(bool tv, Locale? locale) => MaterialApp(
    title: kAppName,
    navigatorKey: _navigatorKey,
    // null = system locale; unsupported system locales fall back to
    // English (the first supported locale).
    locale: locale,
    supportedLocales: AppLocalizations.supportedLocales,
    localizationsDelegates: kLocalizationsDelegates,
    debugShowCheckedModeBanner: false,
    theme: buildKinoluneTheme(widget.formFactor),
    darkTheme: buildKinoluneTheme(widget.formFactor),
    themeMode: ThemeMode.dark,
    shortcuts: {
      ...WidgetsApp.defaultShortcuts,
      // D-pad centre / remote OK.
      const SingleActivator(LogicalKeyboardKey.select): const ActivateIntent(),
    },
    // On TV, the focus highlight must always be visible.
    builder: (context, child) {
      if (tv) {
        FocusManager.instance.highlightStrategy =
            FocusHighlightStrategy.alwaysTraditional;
      }
      // UI-022: the macOS menu bar (nothing elsewhere).
      return AppMenuBar(
        enabled: widget.menuBar,
        child: KeepFocusVisible(
          enabled: !tv,
          child: ListenableBuilder(
            listenable: _services,
            builder: (context, _) {
              final l = context.l10n;
              // UI-015: App Lock covers everything until the PIN; the app
              // under it keeps its state but takes no focus.
              final profiles = _services.profileControl;
              if (_services.loaded && profiles.locked) {
                return Stack(
                  children: [
                    Positioned.fill(
                      child: ExcludeFocus(
                        child: ExcludeSemantics(
                          child: TickerMode(enabled: false, child: child!),
                        ),
                      ),
                    ),
                    Positioned.fill(
                      child: Overlay(
                        initialEntries: [
                          OverlayEntry(
                            builder: (_) => AppLockScreen(controller: profiles),
                          ),
                        ],
                      ),
                    ),
                  ],
                );
              }
              // CORE-007: once, after the first-run setup.
              final secretNotice =
                  _services.loaded &&
                  !_services.onboardingPending &&
                  _services.secretNoticePending;
              // PKG-018: always in the tree of full builds, empty until the
              // daily check finds a release.
              final updates = AppFlavor.selfUpdate ? _services.updates : null;
              if (!_showLibmpvWarning && !secretNotice && updates == null) {
                return _notices(child!);
              }
              return _notices(
                Column(
                  children: [
                    if (_showLibmpvWarning)
                      _TopBanner(
                        icon: Icons.warning_amber_rounded,
                        text: l.appLibmpvMissing(kAppName),
                        action: l.commonDismiss,
                        onAction: () =>
                            setState(() => _showLibmpvWarning = false),
                      ),
                    if (secretNotice)
                      _TopBanner(
                        key: const ValueKey('secret-fallback-notice'),
                        icon: Icons.lock_open_outlined,
                        text: secretFallbackNotice(l, _services.secretStorage),
                        action: l.secretsNoticeOk,
                        safeTop: !_showLibmpvWarning,
                        onAction: () =>
                            unawaited(_services.dismissSecretNotice()),
                      ),
                    if (updates != null)
                      // PKG-022: the strip announces a new release too
                      // (with its changelog), not a notice card.
                      UpdateBanner(
                        controller: updates,
                        safeTop: !_showLibmpvWarning && !secretNotice,
                      ),
                    Expanded(child: child!),
                  ],
                ),
              );
            },
          ),
        ),
      );
    },
    home: ListenableBuilder(
      listenable: _services,
      builder: (context, _) {
        if (!_services.loaded) {
          return const Scaffold(body: SizedBox.shrink());
        }
        if (_services.onboardingPending) {
          return OnboardingScreen(
            onTryDemo: _useDemo,
            onFinish: () => unawaited(_services.completeOnboarding()),
          );
        }
        final profiles = _services.profileControl;
        // UI-015: "who is watching?" at start (two or more profiles).
        if (profiles.pickerPending) {
          return const ProfilePickerScreen(atStart: true);
        }
        // A new profile starts with fresh navigation: nothing the previous
        // one had open stays on screen.
        return AppShell(
          key: ValueKey((_services, profiles.active?.id)),
          initialSection: widget.initialSection,
        );
      },
    ),
  );
}

/// A notice across the top of the app with one action.
class _TopBanner extends StatelessWidget {
  const _TopBanner({
    super.key,
    required this.icon,
    required this.text,
    required this.action,
    required this.onAction,
    this.safeTop = true,
  });

  final IconData icon;
  final String text;
  final String action;
  final VoidCallback onAction;

  /// Pads for the status bar; only the topmost banner does.
  final bool safeTop;

  @override
  Widget build(BuildContext context) => Material(
    color: KColors.bgElevated2,
    child: SafeArea(
      top: safeTop,
      bottom: false,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 10, 8, 10),
        child: Row(
          children: [
            Icon(icon, color: Colors.amber),
            const SizedBox(width: 12),
            Expanded(child: Text(text)),
            TextButton(onPressed: onAction, child: Text(action)),
          ],
        ),
      ),
    ),
  );
}
