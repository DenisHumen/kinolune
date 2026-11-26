// SPDX-License-Identifier: MPL-2.0
//
// TWI-035 (full builds only): «● Записать» on the live players (native and
// embed) and the channel page. The first press shows the ToS warning once
// (TWI-019), then the core starts recording the channel from now
// (`recordNow`, the profile's rule settings if any). While it records the
// button becomes «Идёт запись 1:23:45 · 2,1 ГБ» with «Остановить»; the
// native player also keeps a small REC badge on the video, which the OSD
// does not hide. The recording is a job of the core's queue: it goes on
// when the player closes, as long as the app runs (the snack bar says so),
// and when the broadcast ends it is finalised and announced (notice
// feeds). Store builds have none of this (ADR-0007): callers check
// `AppFlavor.twitchArchive`, and the widgets render nothing without it.
//
// «Записывать на сервере» (SRV-016) is not offered yet: the family server
// has no Twitch tasks.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../../flavor.dart';
import '../../l10n/l10n.dart';
import '../../offline/offline_ui.dart' show formatOfflineSize;
import '../../services/app_services.dart';
import '../../theme/kinolune_theme.dart';
import '../../twitch/twitch_archive.dart';
import '../../twitch/twitch_downloads.dart';
import '../../twitch/twitch_live_recordings.dart';
import 'twitch_archive_screens.dart' show confirmRecordWarning;
import 'twitch_widgets.dart' show formatLength;

/// Time recorded so far: since the job was queued, else the media length.
Duration recordingElapsed(TwitchRecordingItem r, DateTime now) {
  final at = r.createdAt;
  if (at == null) return r.media;
  final d = now.difference(at);
  return d.isNegative ? Duration.zero : d;
}

/// Watches the recording of one channel and ticks once a second while it
/// runs.
mixin _RecordingWatch<T extends StatefulWidget> on State<T> {
  TwitchLiveRecordings? model;
  Timer? _tick;

  String get login;
  String? get userId;

  TwitchRecordingItem? get recording =>
      model?.activeFor(userId: userId, login: login);

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final archive = AppFlavor.twitchArchive
        ? Services.of(context).visibleArchive
        : null;
    final next = archive == null ? null : TwitchLiveRecordings.of(archive);
    if (identical(next, model)) return;
    model?.removeListener(_changed);
    model = next?..addListener(_changed);
    // A build follows anyway.
    _syncTick();
  }

  void _changed() {
    _syncTick();
    if (mounted) setState(() {});
  }

  /// The one-second tick of the elapsed time, only while recording.
  void _syncTick() {
    final running = recording?.activity == RecordingActivity.active;
    if (running && _tick == null) {
      _tick = Timer.periodic(const Duration(seconds: 1), (_) {
        if (mounted) setState(() {});
      });
    } else if (!running) {
      _tick?.cancel();
      _tick = null;
    }
  }

  @override
  void dispose() {
    _tick?.cancel();
    model?.removeListener(_changed);
    super.dispose();
  }

  String status(BuildContext context, TwitchRecordingItem r) {
    final l = context.l10n;
    if (r.activity == RecordingActivity.paused) {
      return r.state == TwitchDownloadState.waiting ||
              r.waitingFor == 'resources'
          ? l.twitchRecordingWaiting
          : l.twitchRecordingPaused;
    }
    return l.twitchRecordLiveStatus(
      formatLength(recordingElapsed(r, DateTime.now())),
      formatOfflineSize(l, Localizations.localeOf(context), r.bytesDone),
    );
  }
}

/// «● Записать» / «Идёт запись 1:23:45 · 2,1 ГБ · Остановить».
class TwitchRecordLiveButton extends StatefulWidget {
  const TwitchRecordLiveButton({
    super.key,
    required this.login,
    this.userId,
    this.compact = false,
    this.onDark = true,
  });

  /// The channel; [userId] when known (else the core looks it up).
  final String login;
  final String? userId;

  /// An icon instead of the label (a phone's top bar).
  final bool compact;

  /// White on the video or the black bar of the embed.
  final bool onDark;

  @override
  State<TwitchRecordLiveButton> createState() => _TwitchRecordLiveButtonState();
}

class _TwitchRecordLiveButtonState extends State<TwitchRecordLiveButton>
    with _RecordingWatch {
  bool _busy = false;

  @override
  String get login => widget.login;

  @override
  String? get userId => widget.userId;

  void _snack(String text) =>
      ScaffoldMessenger.maybeOf(context)
          ?.showSnackBar(SnackBar(content: Text(text)));

  Future<void> _start() async {
    final m = model;
    if (m == null || _busy) return;
    final services = Services.read(context);
    if (!await confirmRecordWarning(context)) return;
    if (!mounted) return;
    final l = context.l10n;
    setState(() => _busy = true);
    try {
      await m.start(
        profileId: services.archiveProfileId,
        login: widget.login,
        userId: widget.userId,
      );
      if (mounted) _snack(l.twitchRecordLiveKeepsGoing);
    } on TwitchRecordError catch (e) {
      if (!mounted) return;
      _snack(switch (e.code) {
        'no_space' => l.twitchRecordLiveNoSpace,
        'archive_disabled' ||
        'native_off' ||
        'store_build' => l.twitchRecordLiveOff,
        _ => l.twitchRecordLiveError(e.message),
      });
    } catch (e) {
      if (mounted) _snack(l.twitchRecordLiveError('$e'));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _stop(TwitchRecordingItem r) async {
    final m = model;
    if (m == null) return;
    final l = context.l10n;
    final ok = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        key: const ValueKey('twitch-record-live-stop-dialog'),
        title: Text(l.twitchRecordLiveStopTitle),
        content: Text(l.twitchRecordLiveStopBody),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: Text(l.commonCancel),
          ),
          FilledButton(
            key: const ValueKey('twitch-record-live-stop-confirm'),
            onPressed: () => Navigator.of(context).pop(true),
            child: Text(l.twitchRecordingStop),
          ),
        ],
      ),
    );
    if (ok != true || !mounted) return;
    try {
      await m.stop(r.jobId);
      if (mounted) _snack(l.twitchRecordLiveStopped);
    } catch (e) {
      if (mounted) _snack('$e');
    }
  }

  @override
  Widget build(BuildContext context) {
    if (!AppFlavor.twitchArchive || model == null) {
      return const SizedBox.shrink();
    }
    final l = context.l10n;
    final fg = widget.onDark
        ? Colors.white
        : Theme.of(context).colorScheme.onSurface;
    final r = recording;
    if (r == null) {
      if (_busy) {
        return Padding(
          key: const ValueKey('twitch-record-live-busy'),
          padding: const EdgeInsets.symmetric(horizontal: 12),
          child: SizedBox.square(
            dimension: 18,
            child: CircularProgressIndicator(strokeWidth: 2, color: fg),
          ),
        );
      }
      const dot = Icon(Icons.fiber_manual_record, color: KColors.live);
      if (widget.compact) {
        return IconButton(
          key: const ValueKey('twitch-record-live'),
          tooltip: l.twitchRecordLiveTooltip,
          onPressed: () => unawaited(_start()),
          icon: dot,
        );
      }
      return Tooltip(
        message: l.twitchRecordLiveTooltip,
        child: TextButton.icon(
          key: const ValueKey('twitch-record-live'),
          style: TextButton.styleFrom(foregroundColor: fg),
          onPressed: () => unawaited(_start()),
          icon: dot,
          label: Text(l.twitchRecordLive),
        ),
      );
    }
    final text = status(context, r);
    final paused = r.activity == RecordingActivity.paused;
    return Row(
      key: const ValueKey('twitch-record-live-recording'),
      mainAxisSize: MainAxisSize.min,
      children: [
        Tooltip(
          message: l.twitchRecordLiveKeepsGoing,
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                paused ? Icons.pause_circle_outline : Icons.fiber_manual_record,
                size: 14,
                color: paused ? fg : KColors.live,
              ),
              const SizedBox(width: 6),
              Text(
                widget.compact && !paused ? _short(text) : text,
                key: const ValueKey('twitch-record-live-status'),
                style: Theme.of(context).textTheme.labelLarge?.copyWith(
                  color: fg,
                  fontFeatures: const [FontFeature.tabularFigures()],
                ),
              ),
            ],
          ),
        ),
        IconButton(
          key: const ValueKey('twitch-record-live-stop'),
          tooltip: l.twitchRecordingStop,
          color: fg,
          onPressed: () => unawaited(_stop(r)),
          icon: const Icon(Icons.stop_rounded),
        ),
      ],
    );
  }

  /// On a phone only the numbers: «1:23:45 · 2,1 ГБ».
  String _short(String status) {
    final r = recording;
    if (r == null) return status;
    final l = context.l10n;
    return '${formatLength(recordingElapsed(r, DateTime.now()))} · '
        '${formatOfflineSize(l, Localizations.localeOf(context), r.bytesDone)}';
  }
}

/// A small «● REC 1:23:45» on the video of the native player, visible with
/// the OSD hidden; nothing while the channel is not recorded.
class TwitchRecordingBadge extends StatefulWidget {
  const TwitchRecordingBadge({super.key, required this.login, this.userId});

  final String login;
  final String? userId;

  @override
  State<TwitchRecordingBadge> createState() => _TwitchRecordingBadgeState();
}

class _TwitchRecordingBadgeState extends State<TwitchRecordingBadge>
    with _RecordingWatch {
  @override
  String get login => widget.login;

  @override
  String? get userId => widget.userId;

  @override
  Widget build(BuildContext context) {
    final r = recording;
    if (!AppFlavor.twitchArchive || r == null) {
      return const SizedBox.shrink();
    }
    final paused = r.activity == RecordingActivity.paused;
    return IgnorePointer(
      child: SafeArea(
        child: Align(
          alignment: Alignment.topRight,
          child: Container(
            key: const ValueKey('twitch-recording-badge'),
            margin: const EdgeInsets.fromLTRB(16, 72, 16, 16),
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
            decoration: BoxDecoration(
              color: Colors.black.withValues(alpha: .6),
              borderRadius: BorderRadius.circular(4),
            ),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Icon(
                  paused
                      ? Icons.pause_circle_outline
                      : Icons.fiber_manual_record,
                  size: 12,
                  color: paused ? Colors.white : KColors.live,
                ),
                const SizedBox(width: 4),
                // The universal recording mark, like the dot.
                Text(
                  'REC ${formatLength(recordingElapsed(r, DateTime.now()))}', // l10n-ignore
                  style: const TextStyle(
                    color: Colors.white,
                    fontSize: 12,
                    fontWeight: FontWeight.w700,
                    fontFeatures: [FontFeature.tabularFigures()],
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
