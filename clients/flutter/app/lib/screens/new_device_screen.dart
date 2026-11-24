// SPDX-License-Identifier: MPL-2.0
//
// A new device set up from a phone (UI-046, ADR-NNNN).
//
// On the new device (a TV first of all) the first run shows a QR code
// «Перенести настройки с телефона» as the main way in ([NewDeviceQrPanel]);
// under it the same code as 6 digits for a phone without a camera. A phone
// of the user scans it (Settings → Devices → «Настроить новое устройство»,
// [SendSetupScreen]); if the phone has no family yet, it founds one in the
// same step. The new device joins the phone's family and gets its sources
// with their passwords (sealed end to end, into its own secret store),
// then its settings; profiles with their PIN locks follow over sync. The
// new device scans its sources at once, the first title plays without a
// single question.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../branding.dart';
import '../l10n/l10n.dart';
import '../platform/form_factor.dart';
import '../services/app_services.dart';
import '../services/preferences.dart';
import '../sync/device_sync.dart';
import '../sync/new_device_setup.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import '../widgets/qr_code.dart';
import 'qr_scan_screen.dart';

/// `123456` as `123 456`.
String _spaced(String code) =>
    code.length == 6 ? '${code.substring(0, 3)} ${code.substring(3)}' : code;

class _Card extends StatelessWidget {
  const _Card({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) => DecoratedBox(
    decoration: BoxDecoration(
      color: KColors.bgElevated.withValues(alpha: .92),
      borderRadius: BorderRadius.circular(16),
      border: Border.all(color: KColors.stroke),
    ),
    child: Padding(padding: const EdgeInsets.all(18), child: child),
  );
}

// ------------------------------------------------------------ new device

/// The QR code a new device shows on its first run (UI-046). It needs no
/// focus: the remote stays where it is («Начать» sets up by hand) until a
/// phone answers; then «Смотреть» takes it. Hidden where sync cannot run
/// and once this device is in a family.
class NewDeviceQrPanel extends StatefulWidget {
  const NewDeviceQrPanel({super.key, this.onDone, this.gapAfter = 0});

  /// «Смотреть» after the transfer: ends the first run.
  final VoidCallback? onDone;

  /// Space under the panel while it shows (none while hidden).
  final double gapAfter;

  @override
  State<NewDeviceQrPanel> createState() => _NewDeviceQrPanelState();
}

class _NewDeviceQrPanelState extends State<NewDeviceQrPanel> {
  DeviceSync? _sync;
  SetupOffer? _offer;
  SetupReceived? _received;
  bool _hidden = false;
  bool _disposed = false;
  final _open = FocusNode(debugLabel: 'new-device-open');

  /// How long one wait for a phone lasts before it starts again.
  static const _round = Duration(seconds: 4);

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_sync != null) return;
    final sync = _sync = Services.read(context).sync;
    if (!sync.available || sync.paired) {
      _hidden = true;
      return;
    }
    unawaited(_start(sync));
  }

  @override
  void dispose() {
    _disposed = true;
    if (_offer != null && _received == null) unawaited(_sync?.setupCancel());
    _open.dispose();
    super.dispose();
  }

  Future<void> _start(DeviceSync sync) async {
    try {
      final offer = await sync.setupOffer();
      if (_disposed) {
        await sync.setupCancel();
        return;
      }
      setState(() => _offer = offer);
    } on SyncException {
      if (!_disposed) setState(() => _hidden = true);
      return;
    }
    while (!_disposed) {
      final SetupReceived? received;
      try {
        received = await sync.setupWait(_round);
      } on SyncException {
        // Cancelled, or another way paired this device meanwhile.
        if (!_disposed) setState(() => _hidden = true);
        return;
      }
      if (_disposed) return;
      if (received != null) {
        await _apply(received);
        return;
      }
    }
  }

  /// The core has added the sources already: settings, then a scan.
  Future<void> _apply(SetupReceived received) async {
    final services = Services.read(context);
    final written = await applySetupSettings(
      services.preferences,
      received.settings,
    );
    if (written.contains(Preferences.locale)) {
      await services.setLocale(
        parseLocaleTag(received.settings[Preferences.locale]),
      );
    }
    await services.refreshSources();
    unawaited(services.scan());
    if (_disposed || !mounted) return;
    setState(() => _received = received);
    if (KDensity.of(context).isTv) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!_disposed && _open.context != null) _open.requestFocus();
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final Widget? card = _hidden
        ? null
        : _received != null
        ? _done(context, _received!)
        : _offer != null
        ? _qr(context, _offer!)
        : null;
    if (card == null) return const SizedBox.shrink();
    return Padding(
      padding: EdgeInsets.only(bottom: widget.gapAfter),
      child: card,
    );
  }

  Widget _qr(BuildContext context, SetupOffer offer) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final d = KDensity.of(context);
    final text = Column(
      key: const ValueKey('new-device-qr-text'),
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(l.newDeviceQrTitle, style: theme.textTheme.titleLarge),
        const SizedBox(height: 8),
        Text(
          l.newDeviceQrBody(kAppName),
          style: theme.textTheme.bodyMedium?.copyWith(
            color: KColors.textSecondary,
          ),
        ),
        const SizedBox(height: 12),
        Text(
          l.newDeviceQrCode(_spaced(offer.code)),
          key: const ValueKey('new-device-code'),
          style: theme.textTheme.titleMedium?.copyWith(
            fontFeatures: const [FontFeature.tabularFigures()],
          ),
        ),
        const SizedBox(height: 10),
        Row(
          children: [
            const SizedBox.square(
              dimension: 14,
              child: CircularProgressIndicator(strokeWidth: 2),
            ),
            const SizedBox(width: 10),
            Flexible(
              child: Text(
                l.newDeviceQrWaiting,
                style: theme.textTheme.bodySmall?.copyWith(
                  color: KColors.textSecondary,
                ),
              ),
            ),
          ],
        ),
      ],
    );
    return _Card(
      child: LayoutBuilder(
        builder: (context, c) {
          final narrow = c.maxWidth < 440;
          final qr = QrCodeView(
            key: const ValueKey('new-device-qr'),
            data: offer.qr,
            size: narrow ? 168 : (d.isTv ? 200 : 188),
          );
          if (narrow) {
            return Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [qr, const SizedBox(height: 14), text],
            );
          }
          return Row(
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              qr,
              const SizedBox(width: 20),
              Expanded(child: text),
            ],
          );
        },
      ),
    );
  }

  Widget _done(BuildContext context, SetupReceived received) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final muted = theme.textTheme.bodyMedium?.copyWith(
      color: KColors.textSecondary,
    );
    final onDone = widget.onDone;
    return _Card(
      child: Column(
        key: const ValueKey('new-device-done'),
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            l.newDeviceDoneTitle(received.deviceName),
            style: theme.textTheme.titleLarge,
          ),
          const SizedBox(height: 8),
          if (received.bundle) ...[
            Text(
              l.newDeviceDoneSources(
                received.sourcesAdded + received.sourcesUpdated,
              ),
              style: muted,
            ),
            const SizedBox(height: 4),
            Text(l.newDeviceDoneProfiles, style: muted),
          ] else
            Text(l.newDeviceDoneNoBundle, style: muted),
          if (onDone != null) ...[
            const SizedBox(height: 14),
            KButton(
              key: const ValueKey('new-device-open'),
              focusNode: _open,
              primary: true,
              icon: Icons.play_arrow,
              label: l.newDeviceDoneOpen,
              onPressed: onDone,
            ),
          ],
        ],
      ),
    );
  }
}

// ----------------------------------------------------------------- phone

/// Opens the transfer to a new device: the camera first where there is
/// one, the 6 digits otherwise. [request]: a setup QR code already
/// scanned. [scanQr] replaces the camera in tests.
Future<void> openSendSetup(
  BuildContext context, {
  String? request,
  Future<String?> Function(BuildContext context)? scanQr,
}) => Navigator.of(context).push(
  MaterialPageRoute<void>(
    builder: (_) => SendSetupScreen(request: request, scanQr: scanQr),
  ),
);

/// The phone's side of UI-046: which device, what goes over, «Перенести»,
/// then waiting until the new device joined.
class SendSetupScreen extends StatefulWidget {
  const SendSetupScreen({super.key, this.request, this.scanQr});

  /// The new device's QR code or digits, if known already.
  final String? request;

  /// Opens the camera and returns a scanned code; tests replace it.
  final Future<String?> Function(BuildContext context)? scanQr;

  @override
  State<SendSetupScreen> createState() => _SendSetupScreenState();
}

enum _Stage { code, confirm, waiting, done }

class _SendSetupScreenState extends State<SendSetupScreen> {
  final _field = TextEditingController();
  String? _request;
  SetupCode? _code;
  var _stage = _Stage.code;
  bool _busy = false;
  String? _error;
  String? _joinedName;
  DeviceSync? _sync;
  StreamSubscription<SyncEvent>? _sub;

  @override
  void initState() {
    super.initState();
    final request = widget.request;
    if (request != null) _take(request);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _sync ??= Services.read(context).sync;
    _sub ??= _sync!.events.listen(_onEvent);
  }

  @override
  void dispose() {
    unawaited(_sub?.cancel());
    if (_stage == _Stage.waiting) unawaited(_sync?.cancelInvite());
    _field.dispose();
    super.dispose();
  }

  void _onEvent(SyncEvent e) {
    if (!mounted || _stage != _Stage.waiting) return;
    if (e.kind == SyncEventKind.paired) {
      setState(() {
        _joinedName = e.name;
        _stage = _Stage.done;
      });
    } else if (e.kind == SyncEventKind.inviteClosed) {
      // Expired or used up without the device: back to the start.
      setState(() {
        _stage = _code == null ? _Stage.code : _Stage.confirm;
        _error = context.l10n.devicesErrorExpired;
      });
    }
  }

  /// A scanned or typed code; false if it is not one.
  bool _take(String text) {
    final code = SetupCode.parse(text);
    if (code == null) return false;
    _request = text.trim();
    _code = code;
    _stage = _Stage.confirm;
    _error = null;
    return true;
  }

  Future<void> _scan() async {
    final scan = widget.scanQr;
    final text = scan != null
        ? await scan(context)
        : await Navigator.of(context).push<String>(
            MaterialPageRoute(builder: (_) => const QrScanScreen()),
          );
    if (!mounted || text == null || text.isEmpty) return;
    setState(() {
      if (!_take(text)) _error = context.l10n.devicesErrorInvalidCode;
    });
  }

  void _typed() => setState(() {
    if (!_take(_field.text)) _error = context.l10n.devicesErrorInvalidCode;
  });

  Future<void> _send() async {
    final request = _request;
    if (request == null || _busy) return;
    final services = Services.read(context);
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final settings = await collectSetupSettings(services.preferences);
      if (!mounted) return;
      setState(() => _stage = _Stage.waiting);
      await services.sync.setupSend(request, settings);
    } on SyncException catch (e) {
      if (!mounted) return;
      final l = context.l10n;
      setState(() {
        _stage = _Stage.confirm;
        _error = switch (e.code) {
          SyncErrorCode.invalidCode => l.devicesErrorInvalidCode,
          SyncErrorCode.unavailable => l.devicesUnavailable(kAppName),
          _ => l.devicesErrorOther,
        };
      });
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  String _target(BuildContext context) =>
      _code?.deviceName ?? context.l10n.newDeviceUnnamed;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final d = KDensity.of(context);
    final camera =
        canScanQr(FormFactorScope.of(context)) || widget.scanQr != null;
    final Widget body = switch (_stage) {
      _Stage.code => Column(
        key: const ValueKey('send-setup-code'),
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(l.newDeviceSendCodeHint, style: theme.textTheme.bodyLarge),
          const SizedBox(height: 16),
          if (camera) ...[
            KButton(
              key: const ValueKey('send-setup-scan'),
              autofocus: true,
              primary: true,
              icon: Icons.qr_code_scanner,
              label: l.devicesScanQr,
              onPressed: _scan,
            ),
            const SizedBox(height: 16),
          ],
          Wrap(
            spacing: 12,
            runSpacing: 12,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              SizedBox(
                width: 220,
                child: TextField(
                  key: const ValueKey('send-setup-field'),
                  controller: _field,
                  autofocus: !camera,
                  keyboardType: TextInputType.number,
                  maxLength: 7,
                  decoration: InputDecoration(
                    labelText: l.newDeviceSendCodeField,
                    counterText: '',
                  ),
                  onSubmitted: (_) => _typed(),
                ),
              ),
              KButton(
                key: const ValueKey('send-setup-next'),
                icon: Icons.arrow_forward,
                label: l.newDeviceSendNext,
                onPressed: _typed,
              ),
            ],
          ),
        ],
      ),
      _Stage.confirm => Column(
        key: const ValueKey('send-setup-confirm'),
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            _code?.deviceName != null
                ? l.newDeviceSendTarget(_code!.deviceName!)
                : l.newDeviceSendTargetUnnamed,
            style: theme.textTheme.headlineSmall,
          ),
          const SizedBox(height: 10),
          Text(
            l.newDeviceSendWhat,
            style: theme.textTheme.bodyMedium?.copyWith(
              color: KColors.textSecondary,
            ),
          ),
          const SizedBox(height: 18),
          KButton(
            key: const ValueKey('send-setup-send'),
            autofocus: true,
            primary: true,
            icon: Icons.send,
            label: l.newDeviceSendButton,
            onPressed: _busy ? null : _send,
          ),
        ],
      ),
      _Stage.waiting => Row(
        key: const ValueKey('send-setup-waiting'),
        children: [
          const SizedBox.square(
            dimension: 18,
            child: CircularProgressIndicator(strokeWidth: 2),
          ),
          const SizedBox(width: 12),
          Flexible(
            child: Text(
              l.newDeviceSendWaiting(_target(context)),
              style: theme.textTheme.bodyLarge,
            ),
          ),
        ],
      ),
      _Stage.done => Column(
        key: const ValueKey('send-setup-done'),
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            l.newDeviceSendDone(_joinedName ?? _target(context)),
            style: theme.textTheme.titleLarge,
          ),
          const SizedBox(height: 18),
          KButton(
            key: const ValueKey('send-setup-close'),
            autofocus: true,
            primary: true,
            icon: Icons.check,
            label: l.newDeviceSendClose,
            onPressed: () => Navigator.of(context).maybePop(),
          ),
        ],
      ),
    };
    return Scaffold(
      appBar: AppBar(title: Text(l.newDeviceSendTitle)),
      body: SafeArea(
        child: ListView(
          padding: EdgeInsets.fromLTRB(
            d.pageInsets.left,
            16,
            d.pageInsets.right,
            d.pageInsets.bottom + 24,
          ),
          children: [
            Align(
              alignment: Alignment.topLeft,
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 560),
                child: _Card(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      body,
                      if (_error case final error?) ...[
                        const SizedBox(height: 12),
                        Text(
                          error,
                          key: const ValueKey('send-setup-error'),
                          style: theme.textTheme.bodyMedium?.copyWith(
                            color: theme.colorScheme.error,
                          ),
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
