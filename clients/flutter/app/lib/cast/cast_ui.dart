// SPDX-License-Identifier: MPL-2.0
//
// PLAY-012 (ADR-NNNN): the cast button (player OSD, detail pages), the
// device list, the «this device can't play it» dialog and the
// mini-controller docked in the shell while casting.

import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import '../widgets/format.dart';
import 'cast_controller.dart';
import 'cast_plan.dart';
import 'cast_platform.dart';

/// Runs discovery while [child] is on screen and rebuilds it with the
/// controller.
class CastWatch extends StatefulWidget {
  const CastWatch({super.key, required this.builder, this.controller});

  final Widget Function(BuildContext context, CastController controller)
  builder;
  final CastController? controller;

  @override
  State<CastWatch> createState() => _CastWatchState();
}

class _CastWatchState extends State<CastWatch> {
  late final CastController _c = widget.controller ?? CastController.instance;

  @override
  void initState() {
    super.initState();
    _c.watch();
  }

  @override
  void dispose() {
    _c.unwatch();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: _c,
    builder: (context, _) => widget.builder(context, _c),
  );
}

/// The name of [device] for the user: the device's own, «AirPlay» for the
/// system picker's entry.
String castDeviceName(AppLocalizations l, CastDevice device) =>
    device.name.isEmpty && device.tech == CastTech.airPlay
    ? l.castAirPlay
    : device.name;

/// Global rectangle of [context]'s render box (the AirPlay picker opens
/// there).
Rect? anchorOf(BuildContext context) {
  final box = context.findRenderObject();
  if (box is! RenderBox || !box.hasSize) return null;
  return box.localToGlobal(Offset.zero) & box.size;
}

/// Asks where to cast: the devices found, or «Stop casting» when a session
/// runs. Null when the user closed the list; [stopKey] for stopping.
Future<CastDevice?> pickCastDevice(
  BuildContext context,
  CastController controller,
) {
  final l = context.l10n;
  return showDialog<CastDevice>(
    context: context,
    builder: (context) => ListenableBuilder(
      listenable: controller,
      builder: (context, _) {
        final session = controller.session;
        return SimpleDialog(
          key: const ValueKey('cast-picker'),
          title: Text(
            l.castPickerTitle,
            style: Theme.of(context).textTheme.titleLarge,
          ),
          children: [
            for (final d in controller.devices)
              SimpleDialogOption(
                key: ValueKey('cast-device-${d.id}'),
                onPressed: () => Navigator.pop(context, d),
                child: Row(
                  children: [
                    Icon(
                      d.tech == CastTech.airPlay
                          ? Icons.airplay_rounded
                          : Icons.cast_rounded,
                      color: session?.device == d
                          ? KColors.accent
                          : KColors.textSecondary,
                    ),
                    const SizedBox(width: 16),
                    Expanded(child: Text(castDeviceName(l, d))),
                  ],
                ),
              ),
            if (session != null)
              SimpleDialogOption(
                key: stopKey,
                onPressed: () {
                  Navigator.pop(context);
                  controller.stop();
                },
                child: Text(l.castStop),
              ),
          ],
        );
      },
    ),
  );
}

/// Key of «Stop casting» in the device list.
const stopKey = ValueKey('cast-stop');

/// The cast icon: hidden until a device is found. [onPick] casts to the
/// chosen device; [anchor] is this button's rectangle.
class CastButton extends StatelessWidget {
  const CastButton({
    super.key,
    required this.onPick,
    this.color = Colors.white,
    this.controller,
  });

  final Future<void> Function(CastDevice device, Rect? anchor) onPick;
  final Color color;
  final CastController? controller;

  @override
  Widget build(BuildContext context) {
    return CastWatch(
      controller: controller,
      builder: (context, c) {
        if (!c.hasDevices) return const SizedBox.shrink();
        final casting = c.session != null || c.connecting != null;
        return IconButton(
          key: const ValueKey('cast-button'),
          color: casting ? KColors.accent : color,
          tooltip: context.l10n.castButtonTooltip,
          icon: Icon(
            c.devices.every((d) => d.tech == CastTech.airPlay)
                ? Icons.airplay_rounded
                : (casting ? Icons.cast_connected_rounded : Icons.cast_rounded),
          ),
          onPressed: () => _press(context, c),
        );
      },
    );
  }

  Future<void> _press(BuildContext context, CastController c) async {
    final anchor = anchorOf(context);
    // AirPlay alone: the system picker is the device list.
    final device = c.session == null && c.devices.length == 1
        ? c.devices.single
        : await pickCastDevice(context, c);
    if (device == null) return;
    await onPick(device, anchor);
  }
}

/// The «На экран» button of a detail page: shown while a device is found.
class CastActionButton extends StatelessWidget {
  const CastActionButton({super.key, required this.onPick});

  final Future<void> Function(CastDevice device, Rect? anchor) onPick;

  @override
  Widget build(BuildContext context) {
    final c = CastController.instance;
    return Builder(
      builder: (context) => KButton(
        key: const ValueKey('detail-cast'),
        icon: c.session != null
            ? Icons.cast_connected_rounded
            : Icons.cast_rounded,
        label: context.l10n.castDetailButton,
        onPressed: () async {
          final anchor = anchorOf(context);
          final device = c.session == null && c.devices.length == 1
              ? c.devices.single
              : await pickCastDevice(context, c);
          if (device != null) await onPick(device, anchor);
        },
      ),
    );
  }
}

/// The reason [problem] for the «can't play» dialog.
String castProblemText(AppLocalizations l, CastUnsupported u) {
  final what = u.detail ?? '';
  return switch (u.problem) {
    CastProblem.container => l.castProblemContainer(what),
    CastProblem.videoCodec => l.castProblemVideo(what),
    CastProblem.audioCodec => l.castProblemAudio(what),
    CastProblem.picture => l.castProblemPicture(what),
    CastProblem.hdr => l.castProblemHdr,
    CastProblem.parts => l.castProblemParts,
    CastProblem.unknown => l.castProblemUnknown,
  };
}

/// «[device] can't play this file»: why, and how to watch it anyway. True
/// when the user wants to try anyway.
Future<bool> showCastUnsupported(
  BuildContext context,
  CastDevice device,
  CastUnsupported u,
) async {
  final l = context.l10n;
  final tryIt = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      key: const ValueKey('cast-unsupported'),
      title: Text(l.castUnsupportedTitle(castDeviceName(l, device))),
      content: Text('${castProblemText(l, u)}\n\n${l.castUnsupportedHint}'),
      actions: [
        TextButton(
          key: const ValueKey('cast-try-anyway'),
          onPressed: () => Navigator.pop(context, true),
          child: Text(l.castTryAnyway),
        ),
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: Text(l.castCancel),
        ),
      ],
    ),
  );
  return tryIt ?? false;
}

/// The mini-controller while casting: title, device, position, play/pause
/// and stop; a tap opens the full controls.
class CastMiniBar extends StatelessWidget {
  const CastMiniBar({super.key, this.controller});

  final CastController? controller;

  @override
  Widget build(BuildContext context) {
    final c = controller ?? CastController.instance;
    if (!c.offered) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: c,
      builder: (context, _) {
        final s = c.session;
        if (s == null) return const SizedBox.shrink();
        final l = context.l10n;
        final dur = s.duration.inMilliseconds;
        final pos = s.position.inMilliseconds;
        return Material(
          color: KColors.bgElevated2,
          child: InkWell(
            key: const ValueKey('cast-mini-bar'),
            onTap: () => showCastControls(context, c),
            child: SizedBox(
              height: 68,
              child: Column(
                children: [
                  LinearProgressIndicator(
                    value: dur <= 0 ? 0 : (pos / dur).clamp(0.0, 1.0),
                    minHeight: 2,
                    backgroundColor: KColors.stroke,
                    color: KColors.accent,
                  ),
                  Expanded(
                    child: Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 12),
                      child: Row(
                        children: [
                          Icon(
                            s.device.tech == CastTech.airPlay
                                ? Icons.airplay_rounded
                                : Icons.cast_connected_rounded,
                            color: KColors.accent,
                          ),
                          const SizedBox(width: 12),
                          Expanded(
                            child: Column(
                              mainAxisAlignment: MainAxisAlignment.center,
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Text(
                                  s.request.title,
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  style: Theme.of(context).textTheme.titleSmall,
                                ),
                                Text(
                                  l.castOnDevice(castDeviceName(l, s.device)),
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  style: Theme.of(context).textTheme.labelSmall
                                      ?.copyWith(color: KColors.textSecondary),
                                ),
                              ],
                            ),
                          ),
                          IconButton(
                            key: const ValueKey('cast-mini-play'),
                            tooltip: s.playing ? l.castPause : l.castPlay,
                            icon: Icon(
                              s.playing
                                  ? Icons.pause_rounded
                                  : Icons.play_arrow_rounded,
                            ),
                            onPressed: c.togglePause,
                          ),
                          IconButton(
                            key: const ValueKey('cast-mini-stop'),
                            tooltip: l.castStop,
                            icon: const Icon(Icons.stop_rounded),
                            onPressed: c.stop,
                          ),
                        ],
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        );
      },
    );
  }
}

/// Full controls of the cast session: seek bar, ±10 s, play/pause, stop.
Future<void> showCastControls(BuildContext context, CastController c) {
  return showModalBottomSheet<void>(
    context: context,
    builder: (context) => ListenableBuilder(
      listenable: c,
      builder: (context, _) {
        final s = c.session;
        if (s == null) return const SizedBox(height: 120);
        final l = context.l10n;
        final dur = s.duration;
        final pos = s.position > dur && dur > Duration.zero ? dur : s.position;
        const step = Duration(seconds: 10);
        return SafeArea(
          child: Padding(
            key: const ValueKey('cast-controls'),
            padding: const EdgeInsets.fromLTRB(24, 20, 24, 16),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  s.request.title,
                  style: Theme.of(context).textTheme.titleMedium,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                ),
                Text(
                  l.castOnDevice(castDeviceName(l, s.device)),
                  style: Theme.of(
                    context,
                  ).textTheme.labelMedium?.copyWith(color: KColors.textSecondary),
                ),
                Slider(
                  key: const ValueKey('cast-seek'),
                  value: dur > Duration.zero
                      ? pos.inMilliseconds / dur.inMilliseconds
                      : 0,
                  onChanged: dur > Duration.zero ? (_) {} : null,
                  onChangeEnd: (v) => c.seek(dur * v),
                ),
                Row(
                  children: [
                    Text(formatTimecode(pos.inMilliseconds)),
                    const Spacer(),
                    Text(formatTimecode(dur.inMilliseconds)),
                  ],
                ),
                const SizedBox(height: 8),
                Row(
                  mainAxisAlignment: MainAxisAlignment.center,
                  children: [
                    IconButton(
                      tooltip: l.castBack(step.inSeconds),
                      icon: const Icon(Icons.replay_10_rounded),
                      onPressed: () => c.seek(pos - step),
                    ),
                    const SizedBox(width: 16),
                    IconButton.filled(
                      tooltip: s.playing ? l.castPause : l.castPlay,
                      iconSize: 36,
                      icon: Icon(
                        s.playing
                            ? Icons.pause_rounded
                            : Icons.play_arrow_rounded,
                      ),
                      onPressed: c.togglePause,
                    ),
                    const SizedBox(width: 16),
                    IconButton(
                      tooltip: l.castForward(step.inSeconds),
                      icon: const Icon(Icons.forward_10_rounded),
                      onPressed: () => c.seek(pos + step),
                    ),
                  ],
                ),
                const SizedBox(height: 8),
                Center(
                  child: TextButton.icon(
                    icon: const Icon(Icons.stop_rounded),
                    label: Text(l.castStop),
                    onPressed: () {
                      Navigator.pop(context);
                      c.stop();
                    },
                  ),
                ),
              ],
            ),
          ),
        );
      },
    ),
  );
}
