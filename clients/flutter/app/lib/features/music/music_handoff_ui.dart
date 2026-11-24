// SPDX-License-Identifier: MPL-2.0
//
// «Продолжить на…» and «Забрать сюда» (MUS-015): a button for Now Playing
// and the music screen, and the sheet with the family's online devices.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../../l10n/l10n.dart';
import '../../services/app_services.dart';
import '../../sync/device_sync.dart';
import 'music_handoff.dart';

IconData _icon(SyncDeviceKind k) => switch (k) {
  SyncDeviceKind.phone => Icons.smartphone,
  SyncDeviceKind.tablet => Icons.tablet_mac,
  SyncDeviceKind.tv => Icons.tv,
  SyncDeviceKind.desktop => Icons.computer,
  SyncDeviceKind.server => Icons.dns,
};

/// Opens the sheet of [MusicHandoffController] devices.
Future<void> showMusicHandoffSheet(BuildContext context) {
  final handoff = Services.read(context).musicHandoff;
  return showModalBottomSheet<void>(
    context: context,
    useRootNavigator: true,
    showDragHandle: true,
    builder: (_) => MusicHandoffSheet(handoff: handoff),
  );
}

/// The «devices» button: hidden without sync.
class MusicHandoffButton extends StatelessWidget {
  const MusicHandoffButton({super.key, this.color});

  final Color? color;

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    if (!services.sync.available || !services.sync.paired) {
      return const SizedBox.shrink();
    }
    return IconButton(
      key: const ValueKey('music-handoff-button'),
      icon: const Icon(Icons.devices_outlined),
      color: color,
      tooltip: context.l10n.musicHandoffTitle,
      onPressed: () => unawaited(showMusicHandoffSheet(context)),
    );
  }
}

/// Online devices of the family: continue the music there, or take it
/// from there.
class MusicHandoffSheet extends StatefulWidget {
  const MusicHandoffSheet({super.key, required this.handoff});

  final MusicHandoffController handoff;

  @override
  State<MusicHandoffSheet> createState() => _MusicHandoffSheetState();
}

class _MusicHandoffSheetState extends State<MusicHandoffSheet> {
  String? _busy;

  Future<void> _run(String key, Future<bool> Function() f, String done,
      String failed) async {
    setState(() => _busy = key);
    var ok = false;
    try {
      ok = await f();
    } catch (e) {
      debugPrint('Music handoff failed: ${e.runtimeType}');
    }
    if (!mounted) return;
    setState(() => _busy = null);
    final messenger = ScaffoldMessenger.maybeOf(context);
    if (ok) Navigator.of(context).maybePop();
    messenger?.showSnackBar(SnackBar(content: Text(ok ? done : failed)));
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final h = widget.handoff;
    final devices = h.targets();
    final canSend = h.player.hasQueue;
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(8, 0, 8, 16),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
              child: Text(
                l.musicHandoffTitle,
                style: Theme.of(context).textTheme.titleMedium,
              ),
            ),
            if (devices.isEmpty)
              Padding(
                key: const ValueKey('music-handoff-empty'),
                padding: const EdgeInsets.all(16),
                child: Text(l.musicHandoffNoDevices),
              ),
            for (final d in devices) ...[
              if (canSend)
                ListTile(
                  key: ValueKey('music-handoff-to-${d.id}'),
                  leading: Icon(_icon(d.kind)),
                  title: Text(l.musicHandoffContinueOn(d.name)),
                  trailing: _busy == 'to-${d.id}'
                      ? const SizedBox.square(
                          dimension: 20,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : null,
                  enabled: _busy == null,
                  onTap: () => _run(
                    'to-${d.id}',
                    () => h.continueOn(d),
                    l.musicHandoffSentTitle(d.name),
                    l.musicHandoffFailed,
                  ),
                ),
              ListTile(
                key: ValueKey('music-handoff-from-${d.id}'),
                leading: const Icon(Icons.download_for_offline_outlined),
                title: Text(l.musicHandoffTakeFrom(d.name)),
                trailing: _busy == 'from-${d.id}'
                    ? const SizedBox.square(
                        dimension: 20,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : null,
                enabled: _busy == null,
                onTap: () => _run(
                  'from-${d.id}',
                  () => h.pullFrom(d),
                  l.musicHandoffReceivedTitle(d.name),
                  l.musicHandoffNothingThere(d.name),
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}
