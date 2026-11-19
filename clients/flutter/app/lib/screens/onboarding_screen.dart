// SPDX-License-Identifier: MPL-2.0
//
// First run in three steps (UI-042, was UI-004's five):
// 1. hello and the interface language;
// 2. what Kinolune found on the network itself — NAS and shared folders,
//    Plex, Jellyfin, Emby, DLNA (NET-009/010/018) — as «Нашли: …» with a
//    one-tap «Добавить», plus «Папка на этом устройстве» and «Ввести
//    адрес» (the add flow of UI-005); no question about the content type
//    (ADR-0099); an added source is scanned at once and the home screen
//    fills while it runs;
// 3. done — the scan, and pairing with another device (UI-019) if wanted.
// "Skip" ends it on any step. Shown on the first start while there are no
// sources; the library (empty home and grids, Sources) and Settings →
// About open it again.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../branding.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../sources/discovery.dart';
import '../sources/source_manager.dart';
import '../sources/source_text.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/artwork.dart';
import '../widgets/focusable.dart';
import '../widgets/logo.dart';
import '../widgets/scan_status.dart';
import 'add_source_dialog.dart';
import 'devices_screen.dart';

/// Steps of the setup, in order.
enum OnboardingStep { welcome, sources, done }

/// Opens the setup again (Settings, the library). It closes itself when
/// done or skipped. [step]: where it starts — the library opens it at what
/// was found on the network.
Future<void> openOnboarding(
  BuildContext context, {
  OnboardingStep step = OnboardingStep.welcome,
}) => Navigator.of(context).push(
  MaterialPageRoute<void>(
    builder: (route) => OnboardingScreen(
      initialStep: step,
      onFinish: () {
        unawaited(Services.read(route).completeOnboarding());
        Navigator.of(route).pop();
      },
    ),
  ),
);

class OnboardingScreen extends StatefulWidget {
  const OnboardingScreen({
    super.key,
    required this.onFinish,
    this.onTryDemo,
    this.initialStep = OnboardingStep.welcome,
  });

  /// "Open home" on the last step or "Skip" on any.
  final VoidCallback onFinish;

  /// Loads the built-in demo library (fictional titles) to look around;
  /// null hides the button (setup opened again later).
  final VoidCallback? onTryDemo;

  final OnboardingStep initialStep;

  @override
  State<OnboardingScreen> createState() => _OnboardingScreenState();
}

class _OnboardingScreenState extends State<OnboardingScreen> {
  late var _step = widget.initialStep;

  bool get _last => _step == OnboardingStep.values.last;

  void _next() {
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
      OnboardingStep.sources => const _SourcesStep(),
      OnboardingStep.done => const _DoneStep(),
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
                            OnboardingStep.sources => l.onboardingNext,
                            OnboardingStep.done => l.onboardingOpenHome,
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

// ----------------------------------------------------------------- sources

/// Whether [found] is already one of the user's sources: same kind, same
/// host.
bool _alreadyAdded(List<ManagedSource> sources, DiscoveredSource found) {
  final host = Uri.tryParse(found.url ?? '')?.host;
  final hosts = {found.host, if (host != null && host.isNotEmpty) host};
  return sources.any(
    (s) =>
        s.type == found.type && hosts.contains(Uri.tryParse(s.address)?.host),
  );
}

/// What was found on the network with a one-tap «Добавить», the two other
/// ways (a folder on this device, an address), and the sources added so
/// far with their scan.
class _SourcesStep extends StatefulWidget {
  const _SourcesStep();

  @override
  State<_SourcesStep> createState() => _SourcesStepState();
}

class _SourcesStepState extends State<_SourcesStep> {
  /// One subscription while the step is shown: LAN browsing runs as long
  /// as someone listens (NET-020), so rebuilds must not restart it.
  Stream<List<DiscoveredSource>>? _found;

  /// Rows whose «Добавить» is running.
  final Set<String> _adding = {};

  /// After a while without results "searching" becomes "nothing yet".
  Timer? _quiet;
  bool _searchedAWhile = false;

  /// TV: the first «Добавить» takes the remote's focus once, when the
  /// first result shows up — one OK adds it.
  /// It stays with that row, also once it says «Добавлено», so the focus
  /// is not lost.
  final _firstAdd = FocusNode(debugLabel: 'onboarding-first-add');
  String? _firstAddRow;

  @override
  void initState() {
    super.initState();
    _quiet = Timer(const Duration(seconds: 8), () {
      if (mounted) setState(() => _searchedAWhile = true);
    });
  }

  @override
  void dispose() {
    _quiet?.cancel();
    _firstAdd.dispose();
    super.dispose();
  }

  static String _id(DiscoveredSource d) => '${d.type.name}-${d.host}';

  Future<void> _add(DiscoveredSource d) async {
    final id = _id(d);
    if (_adding.contains(id)) return;
    setState(() => _adding.add(id));
    try {
      await addDiscoveredSource(context, d);
    } finally {
      if (mounted) setState(() => _adding.remove(id));
    }
  }

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final discovery = services.discovery;
    final found = _found ??= discovery.watch();
    final localBadge = sourceKindBadge(l, services, SourceType.local);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _StepTitle(l.onboardingFindTitle, l.onboardingFindBody(kAppName)),
        if (discovery.supported)
          StreamBuilder<List<DiscoveredSource>>(
            stream: found,
            builder: (context, snap) {
              // Kinds this build cannot add are not offered.
              final list = [
                for (final d in snap.data ?? const <DiscoveredSource>[])
                  if (sourceKindBadge(l, services, d.type) == null) d,
              ];
              final pending = [
                for (final d in list)
                  if (!_alreadyAdded(services.managedSources, d)) d,
              ];
              if (_firstAddRow == null && pending.isNotEmpty) {
                _firstAddRow = _id(pending.first);
                if (KDensity.of(context).isTv) {
                  WidgetsBinding.instance.addPostFrameCallback((_) {
                    if (mounted && _firstAdd.context != null) {
                      _firstAdd.requestFocus();
                    }
                  });
                }
              }
              if (list.isEmpty) {
                final quiet =
                    _searchedAWhile ||
                    snap.connectionState == ConnectionState.done ||
                    snap.hasError;
                return Row(
                  key: const ValueKey('onboarding-searching'),
                  children: [
                    if (!quiet) ...[
                      const SizedBox.square(
                        dimension: 16,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      ),
                      const SizedBox(width: 10),
                    ],
                    // Flexible, not Expanded: the text's own box for screen
                    // readers and the contrast audit.
                    Flexible(
                      child: Text(
                        quiet
                            ? l.onboardingFindNone
                            : l.onboardingFindSearching,
                        style: theme.textTheme.bodyMedium?.copyWith(
                          color: KColors.textSecondary,
                        ),
                      ),
                    ),
                  ],
                );
              }
              return Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Align(
                    alignment: AlignmentDirectional.centerStart,
                    child: Text(
                      l.onboardingFindFound,
                      style: theme.textTheme.titleMedium,
                    ),
                  ),
                  const SizedBox(height: 8),
                  _Panel(
                    child: Column(
                      children: [
                        for (final d in list)
                          _FoundRow(
                            key: ValueKey('onboarding-found-${_id(d)}'),
                            source: d,
                            addKey: ValueKey('onboarding-add-${_id(d)}'),
                            focusNode: _id(d) == _firstAddRow
                                ? _firstAdd
                                : null,
                            added: _alreadyAdded(services.managedSources, d),
                            busy: _adding.contains(_id(d)),
                            onAdd: () => _add(d),
                          ),
                      ],
                    ),
                  ),
                ],
              );
            },
          ),
        const SizedBox(height: 20),
        Wrap(
          spacing: 12,
          runSpacing: 12,
          children: [
            KButton(
              key: const ValueKey('onboarding-local'),
              icon: Icons.folder_outlined,
              label: l.sourcesTypeLocal,
              onPressed: localBadge != null
                  ? null
                  : () => showAddSourceDialog(context, type: SourceType.local),
            ),
            KButton(
              key: const ValueKey('onboarding-address'),
              icon: Icons.link,
              label: l.onboardingFindAddress,
              onPressed: () => showAddSourceDialog(context),
            ),
          ],
        ),
        if (services.sources.isNotEmpty) ...[
          const SizedBox(height: 24),
          const _AddedSources(),
        ],
      ],
    );
  }
}

class _Panel extends StatelessWidget {
  const _Panel({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) => DecoratedBox(
    decoration: BoxDecoration(
      color: KColors.bgElevated.withValues(alpha: .92),
      borderRadius: BorderRadius.circular(16),
      border: Border.all(color: KColors.stroke),
    ),
    child: Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: child,
    ),
  );
}

/// One source found on the network: icon, name, kind and host, and
/// «Добавить» («Добавлено» once it is one of the user's sources).
class _FoundRow extends StatelessWidget {
  const _FoundRow({
    super.key,
    required this.source,
    required this.addKey,
    required this.added,
    required this.busy,
    required this.onAdd,
    this.focusNode,
  });

  final DiscoveredSource source;
  final Key addKey;

  /// The remote's first stop on TV.
  final FocusNode? focusNode;
  final bool added;
  final bool busy;
  final VoidCallback onAdd;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    // One button that stays in place while adding and once added, so the
    // remote's focus is never left on a widget that went away (TV).
    final trailing = KButton(
      key: addKey,
      focusNode: focusNode,
      primary: !added,
      icon: added
          ? Icons.check_circle
          : (busy ? Icons.hourglass_top : Icons.add),
      label: added ? l.onboardingFindAdded : l.onboardingFindAdd,
      onPressed: added || busy ? null : onAdd,
    );
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
      child: Row(
        children: [
          Icon(sourceTypeIcon(source.type), size: 26),
          const SizedBox(width: 14),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  source.name,
                  style: theme.textTheme.titleMedium,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
                Text(
                  '${sourceTypeLabel(l, source.type)} · ${source.host}',
                  style: theme.textTheme.bodySmall?.copyWith(
                    color: KColors.textSecondary,
                  ),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ],
            ),
          ),
          const SizedBox(width: 12),
          trailing,
        ],
      ),
    );
  }
}

/// The user's sources with a spinner while each is scanned, and the scan
/// in one line under them.
class _AddedSources extends StatelessWidget {
  const _AddedSources();

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
      status = scanStatusText(l, p);
    } else if (p.error != null) {
      status = l.settingsScanFailed(p.error!);
    } else {
      status = l.settingsScanUpToDate(p.filesFound);
    }
    // Start, not stretch: each text keeps its own box (contrast audit).
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
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
            child: SizedBox(
              width: double.infinity,
              child: LinearProgressIndicator(
              value: (p.total ?? 0) == 0 ? null : p.processed / p.total!,
                minHeight: 4,
              ),
            ),
          ),
        ],
      ],
    );
  }
}

// -------------------------------------------------------------------- done

/// The home screen fills while the scan runs; pairing with another device
/// (UI-019) is offered, not required.
class _DoneStep extends StatelessWidget {
  const _DoneStep();

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final sync = services.sync;
    final any = services.sources.isNotEmpty;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        _StepTitle(
          any ? l.onboardingDoneTitle : l.onboardingDoneLaterTitle,
          any ? l.onboardingDoneBody(kAppName) : l.onboardingDoneLaterBody,
        ),
        if (any) ...[const _AddedSources(), const SizedBox(height: 28)],
        Text(
          l.onboardingDevicesTitle(kAppName),
          style: theme.textTheme.titleMedium,
        ),
        const SizedBox(height: 6),
        Text(
          l.onboardingDevicesBody,
          style: theme.textTheme.bodyMedium?.copyWith(
            color: KColors.textSecondary,
          ),
        ),
        const SizedBox(height: 12),
        if (!sync.available)
          Text(l.devicesUnavailable(kAppName), style: theme.textTheme.bodySmall)
        else ...[
          Align(
            alignment: AlignmentDirectional.centerStart,
            child: KButton(
              key: const ValueKey('onboarding-pair'),
              icon: Icons.devices,
              label: l.onboardingDevicesPair,
              onPressed: () => Navigator.of(context).push(
                MaterialPageRoute<void>(builder: (_) => const DevicesScreen()),
              ),
            ),
          ),
          const SizedBox(height: 8),
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

// ------------------------------------------------------------------- again

/// «Найти в сети»: the setup again at what was found on the network
/// (UI-042), from the empty library and the list of sources.
class FindSourcesButton extends StatelessWidget {
  const FindSourcesButton({super.key, this.primary = true});

  final bool primary;

  @override
  Widget build(BuildContext context) => FittedBox(
    // A narrow phone: smaller rather than cut off.
    fit: BoxFit.scaleDown,
    child: KButton(
      icon: Icons.travel_explore,
      label: context.l10n.onboardingFindAgain,
      primary: primary,
      onPressed: () => openOnboarding(context, step: OnboardingStep.sources),
    ),
  );
}
