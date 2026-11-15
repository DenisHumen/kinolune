// SPDX-License-Identifier: MPL-2.0
//
// First-run setup (UI-004): language, how the user watches (STRATEGY modes
// A/B/C), a local profile, the first source (found on the network or added
// by hand, the UI-005 forms) and, optionally, pairing with another device
// (UI-019). Every step can be passed with "Next", the whole setup with
// "Skip". Shown on the first start while there are no sources; Settings →
// About opens it again.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../branding.dart';
import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../services/local_profile.dart';
import '../sources/discovery.dart';
import '../sources/source_manager.dart';
import '../sources/source_text.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/artwork.dart';
import '../widgets/focusable.dart';
import '../widgets/logo.dart';
import 'add_source_dialog.dart';
import 'devices_screen.dart';

/// Steps of the setup, in order.
enum OnboardingStep { welcome, mode, profile, source, devices }

/// Opens the setup again (Settings). It closes itself when done or skipped.
Future<void> openOnboarding(BuildContext context) => Navigator.of(context).push(
  MaterialPageRoute<void>(
    builder: (route) => OnboardingScreen(
      onFinish: () {
        unawaited(Services.read(route).completeOnboarding());
        Navigator.of(route).pop();
      },
    ),
  ),
);

class OnboardingScreen extends StatefulWidget {
  const OnboardingScreen({super.key, required this.onFinish, this.onTryDemo});

  /// "Done" on the last step or "Skip" on any.
  final VoidCallback onFinish;

  /// Loads the built-in demo library (fictional titles) to look around;
  /// null hides the button (setup opened from Settings).
  final VoidCallback? onTryDemo;

  @override
  State<OnboardingScreen> createState() => _OnboardingScreenState();
}

class _OnboardingScreenState extends State<OnboardingScreen> {
  var _step = OnboardingStep.welcome;
  AppMode _mode = AppMode.app;
  final _name = TextEditingController();
  int _avatar = 0;
  bool _loaded = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_loaded) return;
    _loaded = true;
    final services = Services.read(context);
    _mode = services.appMode.value ?? AppMode.app;
    final profile = services.profile.value;
    if (profile != null) {
      _name.text = profile.name;
      _avatar = profile.avatar;
    }
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  bool get _last => _step == OnboardingStep.values.last;

  Future<void> _next() async {
    final services = Services.read(context);
    switch (_step) {
      case OnboardingStep.mode:
        await services.setAppMode(_mode);
      case OnboardingStep.profile:
        final name = _name.text.trim();
        if (name.isNotEmpty) {
          await services.setProfile(LocalProfile(name: name, avatar: _avatar));
        }
      case OnboardingStep.welcome ||
          OnboardingStep.source ||
          OnboardingStep.devices:
        break;
    }
    if (!mounted) return;
    if (_last) {
      widget.onFinish();
    } else {
      setState(() => _step = OnboardingStep.values[_step.index + 1]);
    }
  }

  void _back() =>
      setState(() => _step = OnboardingStep.values[_step.index - 1]);

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final total = OnboardingStep.values.length;
    final Widget content = switch (_step) {
      OnboardingStep.welcome => _WelcomeStep(onTryDemo: widget.onTryDemo),
      OnboardingStep.mode => _ModeStep(
        mode: _mode,
        onChanged: (m) => setState(() => _mode = m),
      ),
      OnboardingStep.profile => _ProfileStep(
        name: _name,
        avatar: _avatar,
        onAvatar: (i) => setState(() => _avatar = i),
      ),
      OnboardingStep.source => const _SourceStep(),
      OnboardingStep.devices => const _DevicesStep(),
    };
    return Scaffold(
      body: Stack(
        fit: StackFit.expand,
        children: [
          const BrandPlaceholder(style: ArtStyle.backdrop),
          const DecoratedBox(
            decoration: BoxDecoration(
              gradient: LinearGradient(
                begin: Alignment.centerLeft,
                end: Alignment.centerRight,
                colors: [Color(0xF20B0B0F), Color(0xB30B0B0F)],
              ),
            ),
          ),
          SafeArea(
            child: FocusTraversalGroup(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Padding(
                    padding: EdgeInsets.fromLTRB(
                      d.pageInsets.left,
                      12,
                      d.pageInsets.right - 8,
                      0,
                    ),
                    child: Row(
                      children: [
                        // Aligned rather than stretched: the text's own box
                        // for screen readers and the contrast audit.
                        Expanded(
                          child: Align(
                            alignment: AlignmentDirectional.centerStart,
                            child: Text(
                              l.onboardingStep(_step.index + 1, total),
                              key: const ValueKey('onboarding-step'),
                              style: theme.textTheme.labelLarge?.copyWith(
                                color: KColors.textSecondary,
                              ),
                            ),
                          ),
                        ),
                        TextButton(
                          key: const ValueKey('onboarding-skip'),
                          onPressed: widget.onFinish,
                          child: Text(l.onboardingSkip),
                        ),
                      ],
                    ),
                  ),
                  Expanded(
                    child: Align(
                      alignment: d.isPhone
                          ? Alignment.topLeft
                          : Alignment.centerLeft,
                      child: SingleChildScrollView(
                        key: ValueKey(_step),
                        padding: EdgeInsets.symmetric(
                          horizontal: d.pageInsets.left,
                          vertical: 16,
                        ),
                        child: ConstrainedBox(
                          constraints: const BoxConstraints(maxWidth: 640),
                          child: content,
                        ),
                      ),
                    ),
                  ),
                  Padding(
                    padding: EdgeInsets.fromLTRB(
                      d.pageInsets.left,
                      8,
                      d.pageInsets.right,
                      d.isTv ? d.pageInsets.bottom : 16,
                    ),
                    child: Row(
                      children: [
                        if (_step.index > 0)
                          KButton(
                            key: const ValueKey('onboarding-back'),
                            icon: Icons.arrow_back,
                            label: l.onboardingBack,
                            onPressed: _back,
                          ),
                        const Spacer(),
                        KButton(
                          key: const ValueKey('onboarding-next'),
                          autofocus: !d.isPhone,
                          primary: true,
                          icon: _last ? Icons.check : Icons.arrow_forward,
                          label: switch (_step) {
                            OnboardingStep.welcome => l.onboardingStart,
                            OnboardingStep.devices => l.onboardingFinish,
                            _ => l.onboardingNext,
                          },
                          onPressed: _next,
                        ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _StepTitle extends StatelessWidget {
  const _StepTitle(this.title, this.body);

  final String title;
  final String body;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(title, style: theme.textTheme.headlineMedium),
        const SizedBox(height: 10),
        Text(
          body,
          style: theme.textTheme.bodyLarge?.copyWith(
            color: KColors.textSecondary,
          ),
        ),
        const SizedBox(height: 24),
      ],
    );
  }
}

// ----------------------------------------------------------------- welcome

/// The pitch and the interface language (Settings → Language, UI-016).
class _WelcomeStep extends StatelessWidget {
  const _WelcomeStep({this.onTryDemo});

  final VoidCallback? onTryDemo;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    final l = context.l10n;
    final services = Services.of(context);
    final demo = onTryDemo;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        KinoluneLogo(size: d.isTv ? 48 : 56),
        SizedBox(height: d.isTv ? 20 : 28),
        Text(
          l.onboardingHeadline,
          style: d.isPhone
              ? theme.textTheme.headlineMedium
              : theme.textTheme.displayMedium,
        ),
        const SizedBox(height: 14),
        Text(
          l.onboardingBody(kAppName),
          style: theme.textTheme.bodyLarge?.copyWith(
            color: KColors.textSecondary,
          ),
        ),
        const SizedBox(height: 28),
        Text(l.onboardingLanguageTitle, style: theme.textTheme.titleMedium),
        const SizedBox(height: 10),
        ValueListenableBuilder<Locale?>(
          valueListenable: services.locale,
          builder: (context, current, _) => Wrap(
            spacing: 8,
            runSpacing: 8,
            children: [
              for (final (locale, name) in <(Locale?, String)>[
                (null, l.settingsLanguageSystem),
                (const Locale('en'), l.settingsLanguageEnglish),
                (const Locale('ru'), l.settingsLanguageRussian),
              ])
                ChoiceChip(
                  key: ValueKey('onboarding-language-${locale ?? 'system'}'),
                  label: Text(
                    name,
                    style: Theme.of(context).textTheme.labelLarge?.copyWith(
                      color: current == locale
                          ? KColors.focusLabel
                          : KColors.textPrimary,
                    ),
                  ),
                  selected: current == locale,
                  onSelected: (_) => services.setLocale(locale),
                ),
            ],
          ),
        ),
        if (demo != null) ...[
          const SizedBox(height: 28),
          KButton(
            icon: Icons.auto_awesome_outlined,
            label: l.onboardingTryDemo,
            onPressed: demo,
          ),
        ],
      ],
    );
  }
}

// -------------------------------------------------------------------- mode

/// Modes A/B/C of STRATEGY §1. B and C are announced, not wired yet.
class _ModeStep extends StatefulWidget {
  const _ModeStep({required this.mode, required this.onChanged});

  final AppMode mode;
  final ValueChanged<AppMode> onChanged;

  @override
  State<_ModeStep> createState() => _ModeStepState();
}

class _ModeStepState extends State<_ModeStep> {
  /// One subscription while the step is shown: browsing runs as long as
  /// someone listens (NET-020).
  Stream<List<DiscoveredServer>>? _servers;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final services = Services.read(context);
    final discovery = services.discovery;
    final servers = _servers ??= discovery.watchServers();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _StepTitle(l.onboardingModeTitle, l.onboardingModeBody),
        _OptionCard(
          key: const ValueKey('onboarding-mode-app'),
          icon: Icons.devices_outlined,
          title: l.onboardingModeApp,
          body: l.onboardingModeAppBody,
          selected: widget.mode == AppMode.app,
          onTap: () => widget.onChanged(AppMode.app),
        ),
        const SizedBox(height: 12),
        _OptionCard(
          key: const ValueKey('onboarding-mode-server'),
          icon: Icons.dns_outlined,
          title: l.onboardingModeServer(kAppName),
          body: l.onboardingModeServerBody,
          badge: l.onboardingSoon,
          selected: widget.mode == AppMode.server,
          onTap: () => widget.onChanged(AppMode.server),
          below: widget.mode == AppMode.server
              ? _ServerList(stream: servers, supported: discovery.supported)
              : null,
        ),
        const SizedBox(height: 12),
        _OptionCard(
          key: const ValueKey('onboarding-mode-media-server'),
          icon: Icons.cast_connected_outlined,
          title: l.onboardingModeMediaServer,
          body: l.onboardingModeMediaServerBody,
          badge: l.onboardingSoon,
          selected: false,
          // Until NET-012..015 (Plex, Jellyfin, Emby clients).
          onTap: null,
        ),
      ],
    );
  }
}

/// `_projectalpha._tcp` servers on the LAN, and why they cannot be used yet.
class _ServerList extends StatelessWidget {
  const _ServerList({required this.stream, required this.supported});

  final Stream<List<DiscoveredServer>> stream;
  final bool supported;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final small = theme.textTheme.bodySmall;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (supported)
          StreamBuilder<List<DiscoveredServer>>(
            stream: stream,
            builder: (context, snap) {
              final found = snap.data ?? const <DiscoveredServer>[];
              if (found.isEmpty) {
                return Text(
                  snap.connectionState == ConnectionState.done || snap.hasError
                      ? l.onboardingServerNone
                      : l.onboardingServerSearching,
                  style: small,
                );
              }
              return Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    l.onboardingServerFound,
                    style: theme.textTheme.labelLarge,
                  ),
                  for (final s in found)
                    Padding(
                      padding: const EdgeInsets.only(top: 6),
                      child: Row(
                        children: [
                          const Icon(Icons.dns, size: 18),
                          const SizedBox(width: 8),
                          Expanded(
                            child: Text(
                              s.port == null
                                  ? '${s.name} · ${s.host}'
                                  : '${s.name} · ${s.host}:${s.port}',
                              style: theme.textTheme.bodyMedium,
                            ),
                          ),
                        ],
                      ),
                    ),
                ],
              );
            },
          )
        else
          Text(l.onboardingServerNone, style: small),
        const SizedBox(height: 8),
        Text(l.onboardingServerSoonNote, style: small),
      ],
    );
  }
}

class _OptionCard extends StatelessWidget {
  const _OptionCard({
    super.key,
    required this.icon,
    required this.title,
    required this.body,
    required this.selected,
    required this.onTap,
    this.badge,
    this.below,
  });

  final IconData icon;
  final String title;
  final String body;
  final bool selected;

  /// Null disables the option.
  final VoidCallback? onTap;
  final String? badge;

  /// Extra content inside the card (the server list).
  final Widget? below;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final enabled = onTap != null;
    final b = badge;
    final extra = below;
    return Opacity(
      opacity: enabled ? 1 : .55,
      child: KFocusable(
        onActivate: onTap,
        radius: 16,
        focusScale: 1.02,
        light: false,
        semanticLabel: title,
        builder: (context, focused) => AnimatedContainer(
          duration: KMotion.focusOut,
          padding: const EdgeInsets.all(16),
          decoration: BoxDecoration(
            color: KColors.bgElevated.withValues(alpha: .92),
            borderRadius: BorderRadius.circular(16),
            border: Border.all(
              color: selected || focused ? KColors.accent : KColors.stroke,
              width: selected ? 2 : 1,
            ),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Icon(icon, size: 28),
                  const SizedBox(width: 14),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Wrap(
                          spacing: 8,
                          runSpacing: 4,
                          crossAxisAlignment: WrapCrossAlignment.center,
                          children: [
                            Text(title, style: theme.textTheme.titleMedium),
                            if (b != null) SourceBadge(b),
                          ],
                        ),
                        const SizedBox(height: 4),
                        Text(
                          body,
                          style: theme.textTheme.bodyMedium?.copyWith(
                            color: KColors.textSecondary,
                          ),
                        ),
                      ],
                    ),
                  ),
                  const SizedBox(width: 8),
                  Icon(
                    selected
                        ? Icons.radio_button_checked
                        : Icons.radio_button_unchecked,
                    color: selected ? KColors.accent : KColors.textTertiary,
                  ),
                ],
              ),
              if (extra != null) ...[
                const SizedBox(height: 12),
                Padding(padding: const EdgeInsets.only(left: 42), child: extra),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

// ----------------------------------------------------------------- profile

/// A minimal local profile: a name and an avatar colour (until LIB-012).
class _ProfileStep extends StatelessWidget {
  const _ProfileStep({
    required this.name,
    required this.avatar,
    required this.onAvatar,
  });

  final TextEditingController name;
  final int avatar;
  final ValueChanged<int> onAvatar;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _StepTitle(l.onboardingProfileTitle, l.onboardingProfileBody),
        ListenableBuilder(
          listenable: name,
          builder: (context, _) => Row(
            children: [
              _Avatar(
                profile: LocalProfile(name: name.text, avatar: avatar),
                size: 64,
              ),
              const SizedBox(width: 16),
              Expanded(
                child: TextField(
                  key: const ValueKey('onboarding-profile-name'),
                  controller: name,
                  textCapitalization: TextCapitalization.words,
                  decoration: InputDecoration(
                    labelText: l.onboardingProfileName,
                    hintText: l.onboardingProfileNameHint,
                  ),
                ),
              ),
            ],
          ),
        ),
        const SizedBox(height: 24),
        Text(l.onboardingProfileAvatar, style: theme.textTheme.titleMedium),
        const SizedBox(height: 12),
        Wrap(
          spacing: 12,
          runSpacing: 12,
          children: [
            for (var i = 0; i < kProfileAvatarColors.length; i++)
              KFocusable(
                key: ValueKey('onboarding-avatar-$i'),
                onActivate: () => onAvatar(i),
                radius: 999,
                light: false,
                semanticLabel: l.onboardingProfileAvatarItem(i + 1),
                builder: (context, focused) => Container(
                  padding: const EdgeInsets.all(3),
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    border: Border.all(
                      color: i == avatar || focused
                          ? KColors.textPrimary
                          : Colors.transparent,
                      width: 2,
                    ),
                  ),
                  child: CircleAvatar(
                    radius: 18,
                    backgroundColor: kProfileAvatarColors[i],
                  ),
                ),
              ),
          ],
        ),
      ],
    );
  }
}

class _Avatar extends StatelessWidget {
  const _Avatar({required this.profile, required this.size});

  final LocalProfile profile;
  final double size;

  @override
  Widget build(BuildContext context) => CircleAvatar(
    radius: size / 2,
    backgroundColor: profile.color,
    child: profile.initial.isEmpty
        ? Icon(Icons.person, size: size * .5, color: Colors.white)
        : Text(
            profile.initial,
            style: TextStyle(
              fontSize: size * .42,
              fontWeight: FontWeight.w600,
              color: Colors.white,
            ),
          ),
  );
}

// ------------------------------------------------------------------ source

/// The first source: shares found on the network first, then the kinds
/// that can be added by hand — the same list and forms as Settings →
/// Add source (UI-005, NET-020). An added source is scanned at once.
class _SourceStep extends StatelessWidget {
  const _SourceStep();

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final p = services.scanProgress;
    final String? status;
    if (services.sources.isEmpty || p == null) {
      status = null;
    } else if (services.scanning) {
      status = p.phase == ScanPhase.connecting
          ? l.settingsScanConnecting
          : l.settingsScanFiles(p.processed, p.total ?? p.filesFound);
    } else if (p.error != null) {
      status = l.settingsScanFailed(p.error!);
    } else {
      status = l.settingsScanUpToDate(p.filesFound);
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _StepTitle(l.onboardingSourceTitle, l.onboardingSourceBody(kAppName)),
        if (services.sources.isNotEmpty) ...[
          Text(l.onboardingSourceAdded, style: theme.textTheme.titleMedium),
          const SizedBox(height: 8),
          for (final s in services.sources)
            Padding(
              padding: const EdgeInsets.only(bottom: 6),
              child: Row(
                children: [
                  Icon(
                    sourceTypeIcon(
                      services.managedSource(s.id)?.type ?? SourceType.local,
                    ),
                    size: 20,
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text(
                      s.name,
                      key: ValueKey('onboarding-added-${s.id}'),
                      style: theme.textTheme.bodyLarge,
                    ),
                  ),
                  if (services.scanningSources.contains(s.id))
                    const SizedBox.square(
                      dimension: 16,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  else
                    const Icon(Icons.check_circle_outline, size: 20),
                ],
              ),
            ),
          if (status != null) ...[
            const SizedBox(height: 4),
            Text(
              status,
              key: const ValueKey('onboarding-scan-status'),
              style: theme.textTheme.bodySmall,
            ),
          ],
          if (services.scanning && p != null) ...[
            const SizedBox(height: 8),
            ClipRRect(
              borderRadius: BorderRadius.circular(4),
              child: LinearProgressIndicator(
                value: (p.total ?? 0) == 0 ? null : p.processed / p.total!,
                minHeight: 4,
              ),
            ),
          ],
          const SizedBox(height: 20),
        ],
        DecoratedBox(
          decoration: BoxDecoration(
            color: KColors.bgElevated.withValues(alpha: .92),
            borderRadius: BorderRadius.circular(16),
            border: Border.all(color: KColors.stroke),
          ),
          child: Padding(
            padding: const EdgeInsets.symmetric(vertical: 8),
            child: SourceKindList(
              onPick: (type, [found]) =>
                  showAddSourceDialog(context, type: type, discovered: found),
            ),
          ),
        ),
      ],
    );
  }
}

// ----------------------------------------------------------------- devices

/// Optional: pair with the user's other devices (UI-019).
class _DevicesStep extends StatelessWidget {
  const _DevicesStep();

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final sync = services.sync;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _StepTitle(l.onboardingDevicesTitle(kAppName), l.onboardingDevicesBody),
        if (!sync.available)
          Text(l.devicesUnavailable(kAppName), style: theme.textTheme.bodySmall)
        else ...[
          KButton(
            key: const ValueKey('onboarding-pair'),
            icon: Icons.devices,
            label: l.onboardingDevicesPair,
            onPressed: () => Navigator.of(context).push(
              MaterialPageRoute<void>(builder: (_) => const DevicesScreen()),
            ),
          ),
          const SizedBox(height: 12),
          Text(
            l.settingsDevicesCount(
              sync.devices().where((d) => !d.thisDevice).length,
            ),
            style: theme.textTheme.bodySmall,
          ),
        ],
      ],
    );
  }
}
