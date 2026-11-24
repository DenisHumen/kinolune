// SPDX-License-Identifier: MPL-2.0
//
// Settings → Devices (UI-019): pair this device with the user's other
// devices by QR code or 6-digit code, see which are online, unpair. Paired
// devices sync progress, "Continue watching" and watched marks over the LAN.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../branding.dart';
import '../l10n/l10n.dart';
import '../platform/form_factor.dart';
import '../services/app_services.dart';
import '../sources/source_manager.dart';
import '../sync/device_sync.dart';
import '../sync/new_device_setup.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/cards.dart';
import '../widgets/focusable.dart';
import '../widgets/qr_code.dart';
import 'new_device_screen.dart';
import 'qr_scan_screen.dart';

class DevicesScreen extends StatefulWidget {
  const DevicesScreen({super.key, this.scanQr});

  /// Opens the camera and returns a scanned code; tests replace it.
  final Future<String?> Function(BuildContext context)? scanQr;

  @override
  State<DevicesScreen> createState() => _DevicesScreenState();
}

class _DevicesScreenState extends State<DevicesScreen> {
  final _code = TextEditingController();
  SyncInvite? _invite;
  bool _busy = false;
  DeviceSync? _sync;
  StreamSubscription<SyncEvent>? _sub;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _sync ??= Services.read(context).sync;
    _sub ??= _sync!.events.listen(_onEvent);
  }

  @override
  void dispose() {
    unawaited(_sub?.cancel());
    // Leaving the screen closes an open invitation.
    if (_invite != null) unawaited(_sync?.cancelInvite());
    _code.dispose();
    super.dispose();
  }

  void _onEvent(SyncEvent e) {
    if (!mounted) return;
    final l = context.l10n;
    switch (e.kind) {
      case SyncEventKind.paired:
        _snack(l.devicesPaired(e.name ?? ''));
        setState(() => _invite = null);
      case SyncEventKind.inviteClosed:
        setState(() => _invite = null);
      case SyncEventKind.pairingFailed:
        _snack(l.devicesPairingFailed);
      case SyncEventKind.devicesChanged:
        setState(() {});
      case SyncEventKind.libraryChanged ||
          SyncEventKind.playback ||
          SyncEventKind.takeover ||
          SyncEventKind.input:
        break;
    }
  }

  void _snack(String text) => ScaffoldMessenger.of(context)
    ..removeCurrentSnackBar()
    ..showSnackBar(SnackBar(content: Text(text)));

  String _errorText(Object e) {
    final l = context.l10n;
    final code = e is SyncException ? e.code : SyncErrorCode.other;
    return switch (code) {
      SyncErrorCode.invalidCode => l.devicesErrorInvalidCode,
      SyncErrorCode.wrongCode => l.devicesErrorWrongCode,
      SyncErrorCode.expired => l.devicesErrorExpired,
      SyncErrorCode.notFound => l.devicesErrorNotFound,
      SyncErrorCode.alreadyPaired => l.devicesErrorAlreadyPaired,
      SyncErrorCode.refused => l.devicesErrorRefused,
      SyncErrorCode.unavailable => l.devicesUnavailable(kAppName),
      SyncErrorCode.other => l.devicesErrorOther,
    };
  }

  Future<void> _run(Future<void> Function() action) async {
    if (_busy) return;
    setState(() => _busy = true);
    try {
      await action();
    } catch (e) {
      if (mounted) _snack(_errorText(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _showCode() => _run(() async {
    final invite = await Services.read(context).sync.invite();
    if (mounted) setState(() => _invite = invite);
  });

  Future<void> _hideCode() async {
    setState(() => _invite = null);
    await Services.read(context).sync.cancelInvite();
  }

  Future<void> _join(String input) => _run(() async {
    final services = Services.read(context);
    final l = context.l10n;
    final host = await services.sync.join(input);
    _code.clear();
    if (mounted) _snack(l.devicesJoined(host.name));
  });

  Future<void> _scan() async {
    final scan = widget.scanQr;
    final text = scan != null
        ? await scan(context)
        : await Navigator.of(context).push<String>(
            MaterialPageRoute(builder: (_) => const QrScanScreen()),
          );
    if (text == null || text.isEmpty || !mounted) return;
    // UI-046: the QR code of a new device asks to be set up, not to join.
    if (SetupCode.isSetupQr(text)) {
      await openSendSetup(context, request: text, scanQr: widget.scanQr);
      return;
    }
    await _join(text);
  }

  Future<void> _unpair(SyncDevice d) async {
    final l = context.l10n;
    final ok = await showDialog<bool>(
      context: context,
      builder: (c) => AlertDialog(
        title: Text(
          d.thisDevice ? l.devicesLeaveTitle : l.devicesUnpairTitle(d.name),
        ),
        content: Text(
          d.thisDevice ? l.devicesLeaveBody : l.devicesUnpairBody(d.name),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(c, false),
            child: Text(l.commonCancel),
          ),
          TextButton(
            onPressed: () => Navigator.pop(c, true),
            child: Text(d.thisDevice ? l.devicesLeave : l.devicesUnpair),
          ),
        ],
      ),
    );
    if (ok == true && mounted) {
      await _run(() => Services.read(context).sync.unpair(d.id));
    }
  }

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final sync = services.sync;
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final devices = sync.devices();
    final me = devices.where((x) => x.thisDevice).firstOrNull;
    final others = devices.where((x) => !x.thisDevice).toList();
    final formFactor = FormFactorScope.of(context);
    return Scaffold(
      body: SafeArea(
        child: ListView(
          padding: EdgeInsets.only(bottom: d.pageInsets.bottom + 90),
          children: [
            if (!d.isTv)
              Padding(
                padding: EdgeInsets.only(left: d.pageInsets.left - 8, top: 8),
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: IconButton(
                    tooltip: l.commonBack,
                    onPressed: () => Navigator.of(context).maybePop(),
                    icon: const Icon(Icons.arrow_back),
                  ),
                ),
              ),
            PageTitle(l.devicesTitle),
            Padding(
              padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
              child: Align(
                alignment: Alignment.topLeft,
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 760),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Padding(
                        padding: const EdgeInsets.only(bottom: 20),
                        child: Text(
                          l.devicesSyncNote,
                          style: theme.textTheme.bodyMedium,
                        ),
                      ),
                      if (!sync.available)
                        _Panel(
                          title: l.devicesTitle,
                          children: [Text(l.devicesUnavailable(kAppName))],
                        )
                      else ...[
                        _Panel(
                          title: l.devicesPairedTitle,
                          children: [
                            if (others.isEmpty)
                              Padding(
                                padding: const EdgeInsets.only(bottom: 8),
                                child: Text(
                                  l.devicesNone,
                                  style: theme.textTheme.bodySmall,
                                ),
                              ),
                            if (me != null && others.isNotEmpty)
                              _DeviceTile(
                                device: me,
                                onUnpair: _busy ? null : () => _unpair(me),
                              ),
                            for (final x in others)
                              _DeviceTile(
                                device: x,
                                onUnpair: _busy ? null : () => _unpair(x),
                              ),
                          ],
                        ),
                        _Panel(
                          title: l.newDeviceSendPanelTitle,
                          subtitle: l.newDeviceSendPanelBody(kAppName),
                          children: [
                            Align(
                              alignment: Alignment.centerLeft,
                              child: KButton(
                                key: const ValueKey('devices-setup-new'),
                                label: l.newDeviceSendAction,
                                icon: Icons.tv,
                                onPressed: _busy
                                    ? null
                                    : () => openSendSetup(
                                        context,
                                        scanQr: widget.scanQr,
                                      ),
                              ),
                            ),
                          ],
                        ),
                        _Panel(
                          title: l.devicesAddTitle,
                          subtitle: l.devicesAddBody,
                          children: [
                            if (_invite case final invite?)
                              _InviteView(invite: invite, onHide: _hideCode)
                            else
                              Align(
                                alignment: Alignment.centerLeft,
                                child: KButton(
                                  label: l.devicesShowCode,
                                  icon: Icons.qr_code_2,
                                  primary: others.isEmpty,
                                  onPressed: _busy ? null : _showCode,
                                ),
                              ),
                          ],
                        ),
                        _Panel(
                          title: l.devicesJoinTitle,
                          subtitle: l.devicesJoinBody,
                          children: [
                            Wrap(
                              spacing: 12,
                              runSpacing: 12,
                              crossAxisAlignment: WrapCrossAlignment.center,
                              children: [
                                SizedBox(
                                  width: 220,
                                  child: TextField(
                                    key: const ValueKey('devices-code'),
                                    controller: _code,
                                    keyboardType: TextInputType.number,
                                    maxLength: 7,
                                    decoration: InputDecoration(
                                      labelText: l.devicesCodeField,
                                      counterText: '',
                                    ),
                                    onSubmitted: (v) => _join(v),
                                  ),
                                ),
                                KButton(
                                  label: l.devicesConnect,
                                  icon: Icons.link,
                                  onPressed: _busy
                                      ? null
                                      : () => _join(_code.text),
                                ),
                                if (canScanQr(formFactor) ||
                                    widget.scanQr != null)
                                  KButton(
                                    label: l.devicesScanQr,
                                    icon: Icons.qr_code_scanner,
                                    onPressed: _busy ? null : _scan,
                                  ),
                              ],
                            ),
                          ],
                        ),
                        const _FolderSyncPanel(),
                        if (me != null || sync.fingerprint.isNotEmpty)
                          Text(
                            l.devicesFingerprint(sync.fingerprint),
                            style: theme.textTheme.labelSmall,
                          ),
                      ],
                    ],
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// Sync through a shared folder (SYNC-007, SYNC-015). The folder picks
/// itself: the tile only says which one and how it went. «Изменить»
/// (advanced) picks another folder or turns it off; «Автоматически» gives
/// the choice back to the app.
class _FolderSyncPanel extends StatefulWidget {
  const _FolderSyncPanel();

  @override
  State<_FolderSyncPanel> createState() => _FolderSyncPanelState();
}

class _FolderSyncPanelState extends State<_FolderSyncPanel> {
  FolderSyncState _state = FolderSyncState.none;
  List<ManagedSource> _sources = const [];
  bool _loaded = false;
  bool _busy = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (!_loaded) {
      _loaded = true;
      unawaited(_load());
    }
  }

  Future<void> _load() async {
    final services = Services.read(context);
    var sources = const <ManagedSource>[];
    try {
      sources = (await services.sourceManager.list())
          .where((s) => s.type.folderSyncable)
          .toList();
    } catch (_) {}
    var state = FolderSyncState.none;
    try {
      state = await services.sync.folderSync();
    } catch (_) {}
    if (mounted) {
      setState(() {
        _sources = sources;
        _state = state;
      });
    }
  }

  Future<void> _run(Future<FolderSyncState> Function() action) async {
    if (_busy) return;
    setState(() => _busy = true);
    try {
      final state = await action();
      if (mounted) setState(() => _state = state);
    } catch (_) {
      if (mounted) {
        ScaffoldMessenger.of(context)
          ..removeCurrentSnackBar()
          ..showSnackBar(
            SnackBar(content: Text(context.l10n.devicesFolderErrorOther)),
          );
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  String? _error(FolderSyncError? e) {
    final l = context.l10n;
    return switch (e) {
      null => null,
      FolderSyncError.unavailable => l.devicesFolderErrorUnavailable,
      FolderSyncError.denied => l.devicesFolderErrorDenied,
      FolderSyncError.full => l.devicesFolderErrorFull,
      FolderSyncError.other => l.devicesFolderErrorOther,
    };
  }

  /// "just now", "5 min ago", "at 21:40" or "not synced yet".
  String _when(DateTime? lastSync, DateTime now) {
    final l = context.l10n;
    if (lastSync == null) return l.devicesFolderNotYet;
    final minutes = now.difference(lastSync).inMinutes;
    if (minutes < 1) return l.devicesFolderJustNow;
    if (minutes < 60) return l.devicesFolderMinutesAgo(minutes);
    return l.devicesFolderAt(
      MaterialLocalizations.of(context)
          .formatTimeOfDay(TimeOfDay.fromDateTime(lastSync)),
    );
  }

  String _status(FolderSyncState state, DateTime now) {
    final l = context.l10n;
    if (state.mode == FolderSyncMode.off) return l.devicesFolderOffStatus;
    final id = state.sourceId;
    if (id == null) return l.devicesFolderSearching;
    final name = _sources.where((s) => s.id == id).firstOrNull?.name;
    return l.devicesFolderVia(
      name ?? l.devicesFolderUnnamed,
      state.devices == 0
          ? l.devicesFolderOnlyThisDevice
          : l.devicesFolderDeviceCount(state.devices + 1),
      _when(state.lastSync, now),
    );
  }

  /// Advanced: automatic (recommended), one of the folders, or off.
  Future<void> _change() async {
    final l = context.l10n;
    final sync = Services.read(context).sync;
    final state = _state;
    final current = switch (state.mode) {
      FolderSyncMode.auto => 'auto',
      FolderSyncMode.off => 'off',
      FolderSyncMode.manual => 'source:${state.sourceId}',
    };
    final options = <(String, String, String?)>[
      ('auto', l.devicesFolderAuto, l.devicesFolderAutoHint),
      for (final s in _sources) ('source:${s.id}', s.name, null),
      ('off', l.devicesFolderOff, null),
    ];
    final picked = await showDialog<String>(
      context: context,
      builder: (context) => SimpleDialog(
        title: Text(
          l.devicesFolderChangeTitle,
          style: Theme.of(context).textTheme.titleLarge,
        ),
        children: [
          for (final (value, label, hint) in options)
            SimpleDialogOption(
              key: ValueKey('devices-folder-option-$value'),
              onPressed: () => Navigator.of(context).pop(value),
              child: Semantics(
                selected: value == current,
                inMutuallyExclusiveGroup: true,
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Icon(
                      value == current
                          ? Icons.radio_button_checked
                          : Icons.radio_button_unchecked,
                      size: 20,
                    ),
                    const SizedBox(width: 12),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(label),
                          if (hint != null)
                            Text(
                              hint,
                              style: Theme.of(context).textTheme.bodySmall,
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
    if (picked == null || picked == current) return;
    await _run(switch (picked) {
      'auto' => sync.setFolderSyncAuto,
      'off' => () => sync.setFolderSync(null),
      _ => () => sync.setFolderSync(picked.substring('source:'.length)),
    });
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final services = Services.of(context);
    final sync = services.sync;
    final state = _state;
    final error = state.enabled ? _error(state.error) : null;
    return _Panel(
      title: l.devicesFolderTitle,
      subtitle: l.devicesFolderBody,
      children: [
        Text(
          _status(state, services.clock()),
          key: const ValueKey('devices-folder-status'),
          style: theme.textTheme.bodyLarge,
        ),
        if (state.enabled)
          Padding(
            padding: const EdgeInsets.only(top: 2),
            child: Text(
              state.mode == FolderSyncMode.manual
                  ? l.devicesFolderPickedByYou
                  : l.devicesFolderPickedAuto,
              style: theme.textTheme.bodySmall,
            ),
          ),
        if (error != null)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(
              error,
              key: const ValueKey('devices-folder-error'),
              style: theme.textTheme.bodyMedium?.copyWith(
                color: theme.colorScheme.error,
              ),
            ),
          ),
        if (state.enabled && state.readOnly)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(
              l.devicesFolderReadOnly,
              style: theme.textTheme.bodySmall,
            ),
          ),
        const SizedBox(height: 12),
        Wrap(
          spacing: 12,
          runSpacing: 12,
          children: [
            KButton(
              key: const ValueKey('devices-folder-change'),
              label: l.devicesFolderChange,
              icon: Icons.tune,
              onPressed: _busy || !sync.available ? null : _change,
            ),
            if (state.mode != FolderSyncMode.off)
              KButton(
                key: const ValueKey('devices-folder-now'),
                label: l.devicesFolderSyncNow,
                icon: Icons.sync,
                onPressed: _busy || !sync.available
                    ? null
                    : () => _run(sync.folderSyncNow),
              ),
          ],
        ),
      ],
    );
  }
}

class _Panel extends StatelessWidget {
  const _Panel({required this.title, required this.children, this.subtitle});

  final String title;
  final String? subtitle;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final d = KDensity.of(context);
    return Padding(
      padding: EdgeInsets.only(bottom: d.isTv ? 18 : 24),
      child: FocusTraversalGroup(
        child: Container(
          padding: const EdgeInsets.all(18),
          decoration: BoxDecoration(
            color: KColors.bgElevated,
            borderRadius: BorderRadius.circular(d.panelRadius * .75),
            border: Border.all(color: KColors.stroke),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(title, style: theme.textTheme.titleLarge),
              if (subtitle != null) ...[
                const SizedBox(height: 2),
                Text(subtitle!, style: theme.textTheme.bodySmall),
              ],
              const SizedBox(height: 12),
              ...children,
            ],
          ),
        ),
      ),
    );
  }
}

class _InviteView extends StatelessWidget {
  const _InviteView({required this.invite, required this.onHide});

  final SyncInvite invite;
  final VoidCallback onHide;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final until = MaterialLocalizations.of(context)
        .formatTimeOfDay(TimeOfDay.fromDateTime(invite.expiresAt));
    final code = '${invite.code.substring(0, 3)} ${invite.code.substring(3)}';
    final details = Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          code,
          key: const ValueKey('devices-invite-code'),
          style: theme.textTheme.displaySmall?.copyWith(
            letterSpacing: 4,
            fontFeatures: const [FontFeature.tabularFigures()],
          ),
        ),
        const SizedBox(height: 6),
        Text(l.devicesCodeHint(until), style: theme.textTheme.bodySmall),
        const SizedBox(height: 14),
        KButton(label: l.devicesHideCode, icon: Icons.close, onPressed: onHide),
      ],
    );
    return LayoutBuilder(
      builder: (context, c) {
        final qr = QrCodeView(
          data: invite.qr,
          size: c.maxWidth < 420 ? 180 : 220,
        );
        if (c.maxWidth < 420) {
          return Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [qr, const SizedBox(height: 16), details],
          );
        }
        return Row(
          crossAxisAlignment: CrossAxisAlignment.center,
          children: [
            qr,
            const SizedBox(width: 24),
            Expanded(child: details),
          ],
        );
      },
    );
  }
}

class _DeviceTile extends StatelessWidget {
  const _DeviceTile({required this.device, this.onUnpair});

  final SyncDevice device;
  final VoidCallback? onUnpair;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final icon = switch (device.kind) {
      SyncDeviceKind.phone => Icons.smartphone,
      SyncDeviceKind.tablet => Icons.tablet_mac,
      SyncDeviceKind.tv => Icons.tv,
      SyncDeviceKind.desktop => Icons.computer,
      SyncDeviceKind.server => Icons.dns,
    };
    final status = device.thisDevice
        ? l.devicesThisDeviceTag
        : (device.online ? l.devicesOnline : l.devicesOffline);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Row(
        children: [
          Icon(icon, color: KColors.textSecondary),
          const SizedBox(width: 14),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  device.name,
                  style: theme.textTheme.titleMedium,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
                Row(
                  children: [
                    Container(
                      width: 8,
                      height: 8,
                      decoration: BoxDecoration(
                        shape: BoxShape.circle,
                        color: device.online || device.thisDevice
                            ? const Color(0xFF3FB950)
                            : KColors.textTertiary,
                      ),
                    ),
                    const SizedBox(width: 6),
                    Flexible(
                      child: Text(
                        status,
                        style: theme.textTheme.labelSmall,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
          KButton(
            label: device.thisDevice ? l.devicesLeave : l.devicesUnpair,
            onPressed: onUnpair,
          ),
        ],
      ),
    );
  }
}
