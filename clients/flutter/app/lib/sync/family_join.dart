// SPDX-License-Identifier: MPL-2.0
//
// Joining the family over the LAN without a code (SYNC-015, ADR-0159).
//
// A new device sees the user's devices on the LAN ([FamilyJoinOffer]:
// «Присоединиться к вашим устройствам?»). One tap asks to join; the new
// device shows a short code. Every trusted device in the LAN shows the
// request ([FamilyJoinHost] → [FamilyJoinDialog]) with the device's name,
// kind and the same code; one «Разрешить» on any of them lets it in, and
// profiles and progress come over sync. Without it nothing is shared.
// «Отклонить» has the focus on a TV: a stray OK does not let a stranger in.

import 'dart:async';

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import 'device_sync.dart';

/// The kind of a device in a sentence.
String familyJoinKindName(AppLocalizations l, SyncDeviceKind kind) =>
    switch (kind) {
      SyncDeviceKind.phone => l.familyJoinKindPhone,
      SyncDeviceKind.tablet => l.familyJoinKindTablet,
      SyncDeviceKind.tv => l.familyJoinKindTv,
      SyncDeviceKind.desktop => l.familyJoinKindDesktop,
      SyncDeviceKind.server => l.familyJoinKindServer,
    };

IconData _kindIcon(SyncDeviceKind kind) => switch (kind) {
  SyncDeviceKind.phone => Icons.smartphone,
  SyncDeviceKind.tablet => Icons.tablet_mac,
  SyncDeviceKind.tv => Icons.tv,
  SyncDeviceKind.desktop => Icons.computer,
  SyncDeviceKind.server => Icons.dns,
};

TextStyle? _codeStyle(ThemeData theme) =>
    theme.textTheme.headlineSmall?.copyWith(
      fontFeatures: const [FontFeature.tabularFigures()],
      letterSpacing: 2,
    );

// ------------------------------------------------------- trusted device

/// Shows a [FamilyJoinDialog] on this trusted device for each new device
/// that asks to join, one at a time. Nothing while sync cannot run.
class FamilyJoinHost extends StatefulWidget {
  const FamilyJoinHost({
    super.key,
    required this.sync,
    required this.navigatorKey,
    required this.child,
  });

  final DeviceSync sync;

  /// The app's navigator: the dialog opens on it.
  final GlobalKey<NavigatorState> navigatorKey;
  final Widget child;

  @override
  State<FamilyJoinHost> createState() => _FamilyJoinHostState();
}

class _FamilyJoinHostState extends State<FamilyJoinHost> {
  StreamSubscription<SyncEvent>? _sub;

  /// Requests shown once (answered or dismissed) are not shown again.
  final Set<String> _seen = {};

  /// The request whose dialog is up, and the live list it follows.
  String? _shown;
  final _requests = ValueNotifier<List<FamilyJoinRequest>>(const []);

  @override
  void initState() {
    super.initState();
    _attach();
  }

  @override
  void didUpdateWidget(FamilyJoinHost old) {
    super.didUpdateWidget(old);
    if (old.sync != widget.sync) {
      unawaited(_sub?.cancel());
      _sub = null;
      _attach();
    }
  }

  @override
  void dispose() {
    unawaited(_sub?.cancel());
    _requests.dispose();
    super.dispose();
  }

  void _attach() {
    if (!widget.sync.available) return;
    _sub = widget.sync.events.listen((e) {
      if (e.kind == SyncEventKind.joinRequests) unawaited(_refresh());
    }, onError: (_) {});
    unawaited(_refresh());
  }

  Future<void> _refresh() async {
    List<FamilyJoinRequest> list;
    try {
      list = await widget.sync.joinRequests();
    } catch (_) {
      list = const [];
    }
    if (!mounted) return;
    _requests.value = list;
    if (_shown != null) return;
    final next = list.where((r) => !_seen.contains(r.id)).firstOrNull;
    if (next != null) _show(next);
  }

  void _show(FamilyJoinRequest request) {
    final ctx = widget.navigatorKey.currentContext;
    if (ctx == null) return;
    _shown = request.id;
    _seen.add(request.id);
    unawaited(
      showDialog<void>(
        context: ctx,
        builder: (_) => FamilyJoinDialog(
          request: request,
          requests: _requests,
          onAllow: () => widget.sync.joinAllow(request.id),
          onDecline: () => widget.sync.joinDecline(request.id),
        ),
      ).whenComplete(() {
        _shown = null;
        if (mounted) unawaited(_refresh());
      }),
    );
  }

  @override
  Widget build(BuildContext context) => widget.child;
}

/// «“TV” wants to join»: the device's name, kind and code, «Отклонить»
/// (focused) and «Разрешить». Closes itself when the request goes away
/// (it ran out, or another device let it in).
class FamilyJoinDialog extends StatefulWidget {
  const FamilyJoinDialog({
    super.key,
    required this.request,
    required this.requests,
    required this.onAllow,
    required this.onDecline,
  });

  final FamilyJoinRequest request;

  /// The current requests: the dialog closes when [request] leaves.
  final ValueListenable<List<FamilyJoinRequest>> requests;
  final Future<void> Function() onAllow;
  final Future<void> Function() onDecline;

  @override
  State<FamilyJoinDialog> createState() => _FamilyJoinDialogState();
}

class _FamilyJoinDialogState extends State<FamilyJoinDialog> {
  bool _busy = false;
  Timer? _expiry;

  @override
  void initState() {
    super.initState();
    widget.requests.addListener(_onRequests);
    final left = widget.request.expiresAt.difference(DateTime.now());
    _expiry = Timer(left.isNegative ? Duration.zero : left, _close);
  }

  @override
  void dispose() {
    widget.requests.removeListener(_onRequests);
    _expiry?.cancel();
    super.dispose();
  }

  void _onRequests() {
    if (_busy) return;
    if (!widget.requests.value.any((r) => r.id == widget.request.id)) {
      _close();
    }
  }

  void _close() {
    if (mounted) Navigator.of(context).pop();
  }

  Future<void> _answer(Future<void> Function() f) async {
    setState(() => _busy = true);
    try {
      await f();
    } catch (_) {
      // Ran out meanwhile: nothing to do, the dialog closes.
    }
    _close();
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final r = widget.request;
    return AlertDialog(
      key: const ValueKey('family-join-dialog'),
      icon: Icon(_kindIcon(r.kind)),
      title: Text(l.familyJoinRequestTitle(r.name)),
      content: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 480),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              '${familyJoinKindName(l, r.kind)} · ${l.familyJoinRequestBody}',
              style: theme.textTheme.bodyMedium,
            ),
            const SizedBox(height: 14),
            Center(
              child: Text(
                r.shortCode,
                key: const ValueKey('family-join-code'),
                style: _codeStyle(theme),
              ),
            ),
            if (r.duplicateName) ...[
              const SizedBox(height: 12),
              Text(
                l.familyJoinRequestDuplicate,
                style: theme.textTheme.bodySmall?.copyWith(
                  color: KColors.statusWarning,
                ),
              ),
            ],
          ],
        ),
      ),
      actions: [
        KButton(
          key: const ValueKey('family-join-decline'),
          label: l.familyJoinDecline,
          autofocus: true,
          onPressed: _busy ? null : () => _answer(widget.onDecline),
        ),
        KButton(
          key: const ValueKey('family-join-allow'),
          label: l.familyJoinAllow,
          primary: true,
          icon: Icons.check,
          onPressed: _busy ? null : () => _answer(widget.onAllow),
        ),
      ],
    );
  }
}

// ----------------------------------------------------------- new device

enum _Stage { offer, waiting, expired, failed, done }

/// The new device's card: «Присоединиться к вашим устройствам?» while it
/// sees a family on the LAN, then the code to compare while it waits for
/// «Разрешить», then «Готово». Hidden where sync cannot run, once this
/// device is paired, and while no family is in sight.
class FamilyJoinOffer extends StatefulWidget {
  const FamilyJoinOffer({
    super.key,
    this.gapAfter = 0,
    this.onJoined,
    this.poll = const Duration(seconds: 3),
  });

  /// Space under the card while it shows.
  final double gapAfter;

  /// After joining.
  final VoidCallback? onJoined;

  /// How often it looks for families.
  final Duration poll;

  @override
  State<FamilyJoinOffer> createState() => _FamilyJoinOfferState();
}

class _FamilyJoinOfferState extends State<FamilyJoinOffer> {
  DeviceSync? _sync;
  Timer? _timer;
  List<NearbyFamily> _families = const [];
  FamilyJoinAsk? _ask;
  String? _joinedName;
  var _stage = _Stage.offer;
  bool _disposed = false;

  /// One wait for «Разрешить» before it starts again.
  static const _round = Duration(seconds: 4);

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_sync != null) return;
    final sync = _sync = Services.read(context).sync;
    if (!sync.available || sync.paired) return;
    unawaited(_look());
    _timer = Timer.periodic(widget.poll, (_) => unawaited(_look()));
  }

  @override
  void dispose() {
    _disposed = true;
    _timer?.cancel();
    if (_stage == _Stage.waiting) unawaited(_sync?.joinAskCancel());
    super.dispose();
  }

  Future<void> _look() async {
    final sync = _sync;
    if (sync == null || _stage != _Stage.offer) return;
    List<NearbyFamily> list;
    try {
      list = await sync.familiesNearby();
    } catch (_) {
      list = const [];
    }
    if (_disposed || _stage != _Stage.offer) return;
    if (sync.paired) {
      _timer?.cancel();
      list = const [];
    }
    String key(List<NearbyFamily> l) =>
        l.map((f) => '${f.familyId}:${f.devices.join(',')}').join(';');
    if (key(list) == key(_families)) return;
    setState(() => _families = list);
  }

  Future<void> _join(NearbyFamily family) async {
    final sync = _sync!;
    try {
      final ask = await sync.joinAsk(family.familyId);
      if (_disposed) return;
      setState(() {
        _ask = ask;
        _stage = _Stage.waiting;
      });
    } on SyncException {
      if (!_disposed) setState(() => _stage = _Stage.failed);
      return;
    }
    while (!_disposed && _stage == _Stage.waiting) {
      final SyncDevice? device;
      try {
        device = await sync.joinAskWait(_round);
      } on SyncException catch (e) {
        if (_disposed) return;
        setState(
          () => _stage = e.code == SyncErrorCode.expired
              ? _Stage.expired
              : _Stage.failed,
        );
        return;
      }
      if (_disposed) return;
      if (device != null) {
        setState(() {
          _joinedName = device!.name;
          _stage = _Stage.done;
        });
        _timer?.cancel();
        widget.onJoined?.call();
        return;
      }
    }
  }

  Future<void> _cancel() async {
    await _sync?.joinAskCancel();
    if (_disposed) return;
    setState(() {
      _ask = null;
      _stage = _Stage.offer;
    });
    unawaited(_look());
  }

  @override
  Widget build(BuildContext context) {
    final card = switch (_stage) {
      _Stage.offer => _families.isEmpty ? null : _offer(context),
      _Stage.waiting => _waiting(context),
      _Stage.expired || _Stage.failed => _retry(context),
      _Stage.done => _done(context),
    };
    if (card == null) return const SizedBox.shrink();
    return Padding(
      padding: EdgeInsets.only(bottom: widget.gapAfter),
      child: DecoratedBox(
        key: const ValueKey('family-join-offer'),
        decoration: BoxDecoration(
          color: KColors.bgElevated.withValues(alpha: .92),
          borderRadius: BorderRadius.circular(16),
          border: Border.all(color: KColors.stroke),
        ),
        child: Padding(padding: const EdgeInsets.all(18), child: card),
      ),
    );
  }

  String _names(List<String> names) => names.map((n) => '«$n»').join(', ');

  Widget _offer(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(l.familyJoinOfferTitle, style: theme.textTheme.titleLarge),
        for (final f in _families) ...[
          const SizedBox(height: 8),
          Text(
            l.familyJoinOfferBody(_names(f.devices)),
            style: theme.textTheme.bodyMedium?.copyWith(
              color: KColors.textSecondary,
            ),
          ),
          const SizedBox(height: 12),
          KButton(
            key: ValueKey('family-join-${f.familyId}'),
            label: l.familyJoinOfferAction,
            icon: Icons.group_add,
            primary: true,
            onPressed: () => unawaited(_join(f)),
          ),
        ],
      ],
    );
  }

  Widget _waiting(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final ask = _ask!;
    final family = _families
        .where((f) => f.familyId == ask.familyId)
        .firstOrNull;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(l.familyJoinWaitingTitle, style: theme.textTheme.titleLarge),
        const SizedBox(height: 8),
        Text(
          l.familyJoinWaitingBody(family?.devices.join(', ') ?? ''),
          style: theme.textTheme.bodyMedium?.copyWith(
            color: KColors.textSecondary,
          ),
        ),
        const SizedBox(height: 10),
        Text(
          l.familyJoinCode(ask.shortCode),
          key: const ValueKey('family-join-my-code'),
          style: _codeStyle(theme),
        ),
        const SizedBox(height: 12),
        Row(
          children: [
            const SizedBox.square(
              dimension: 14,
              child: CircularProgressIndicator(strokeWidth: 2),
            ),
            const SizedBox(width: 12),
            KButton(label: l.commonCancel, onPressed: _cancel),
          ],
        ),
      ],
    );
  }

  Widget _retry(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          _stage == _Stage.expired ? l.familyJoinExpired : l.familyJoinFailed,
          style: theme.textTheme.bodyMedium,
        ),
        const SizedBox(height: 12),
        KButton(
          key: const ValueKey('family-join-retry'),
          label: l.familyJoinRetry,
          onPressed: _cancel,
        ),
      ],
    );
  }

  Widget _done(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    return Column(
      key: const ValueKey('family-join-done'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          l.familyJoinDoneTitle(_joinedName ?? ''),
          style: theme.textTheme.titleLarge,
        ),
        const SizedBox(height: 8),
        Text(
          l.familyJoinDoneBody,
          style: theme.textTheme.bodyMedium?.copyWith(
            color: KColors.textSecondary,
          ),
        ),
      ],
    );
  }
}
