// SPDX-License-Identifier: MPL-2.0

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import 'branding.dart';
import 'fakes/fake_library.dart';
import 'l10n/l10n.dart';
import 'licenses.dart';
import 'platform/form_factor.dart';
import 'profiles/profile_locks.dart';
import 'profiles/profile_picker.dart';
import 'screens/onboarding_screen.dart';
import 'services/app_services.dart';
import 'shell/app_shell.dart';
import 'theme/kinolune_theme.dart';
import 'widgets/secret_storage_text.dart';

class KinoluneApp extends StatefulWidget {
  const KinoluneApp({
    super.key,
    required this.services,
    required this.formFactor,
    this.initialSection = Section.home,
    this.libmpvMissing = false,
  });

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

  /// App Lock and Settings/Search locks follow the app to the background
  /// and back (UI-015).
  late final AppLifecycleListener _lifecycle = AppLifecycleListener(
    onHide: () => _services.profileControl.onBackground(),
  );

  @override
  void initState() {
    super.initState();
    registerAppLicenses();
    _lifecycle;
  }

  @override
  void dispose() {
    _lifecycle.dispose();
    super.dispose();
  }

  Future<void> _useDemo() async {
    final demo = AppServices(
      library: FakeLibrary.demo(),
      secureStore: _services.secureStore,
      preferences: _services.preferences,
    );
    await demo.init();
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

  Widget _app(bool tv, Locale? locale) => MaterialApp(
    title: kAppName,
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
      return ListenableBuilder(
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
                    child: ExcludeSemantics(child: TickerMode(enabled: false, child: child!)),
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
          if (!_showLibmpvWarning && !secretNotice) return child!;
          return Column(
            children: [
              if (_showLibmpvWarning)
                _TopBanner(
                  icon: Icons.warning_amber_rounded,
                  text: l.appLibmpvMissing(kAppName),
                  action: l.commonDismiss,
                  onAction: () => setState(() => _showLibmpvWarning = false),
                ),
              if (secretNotice)
                _TopBanner(
                  key: const ValueKey('secret-fallback-notice'),
                  icon: Icons.lock_open_outlined,
                  text: secretFallbackNotice(l, _services.secretStorage),
                  action: l.secretsNoticeOk,
                  safeTop: !_showLibmpvWarning,
                  onAction: () => unawaited(_services.dismissSecretNotice()),
                ),
              Expanded(child: child!),
            ],
          );
        },
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
