// SPDX-License-Identifier: MPL-2.0
import 'package:flutter/material.dart';

import '../ui/focus_button.dart';
import '../ui/tokens.dart';
import 'sleep_timer.dart';
import 'sleep_timer_strings.dart';

/// "14:32" / "1:05:00" of the time left.
String formatSleepRemaining(Duration d) {
  final s = (d.inMilliseconds / 1000).ceil();
  final h = s ~/ 3600, m = (s % 3600) ~/ 60, sec = s % 60;
  final ss = sec.toString().padLeft(2, '0');
  if (h > 0) return '$h:${m.toString().padLeft(2, '0')}:$ss';
  return '$m:$ss';
}

/// The choices of the sleep timer (PLAY-022) as focusable rows, the chosen
/// one with the time left, and the «Вы ещё смотрите?» stepper. The body of
/// the OSD panel and of the Twitch embed's dialog.
class SleepTimerMenu extends StatelessWidget {
  const SleepTimerMenu({
    super.key,
    required this.controller,
    this.endOfItem = true,
    this.stillWatching = true,
    this.onPicked,
  });

  final SleepTimerController controller;

  /// «В конце этой серии или фильма» is offered (not on Twitch).
  final bool endOfItem;

  /// The «Вы ещё смотрите?» setting is shown (players with Up Next).
  final bool stillWatching;

  /// A choice was made (the panel closes).
  final VoidCallback? onPicked;

  @override
  Widget build(BuildContext context) {
    final l = SleepTimerStrings.of(context);
    return ListenableBuilder(
      listenable: controller,
      builder: (context, _) {
        final current = controller.option;
        final left = controller.remaining;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final o in SleepTimerOption.values)
              if (o != SleepTimerOption.endOfItem || endOfItem)
                FocusListRow(
                  key: ValueKey('sleep-${o.name}'),
                  title: l.option(o),
                  subtitle: o == current && o != SleepTimerOption.off
                      ? (left == null
                            ? null
                            : l.pausesIn(formatSleepRemaining(left)))
                      : null,
                  leading: o == SleepTimerOption.off
                      ? const Icon(Icons.timer_off_outlined)
                      : o == SleepTimerOption.endOfItem
                      ? const Icon(Icons.last_page_rounded)
                      : const Icon(Icons.bedtime_outlined),
                  selected: o == current,
                  autofocus: o == current,
                  onPressed: () {
                    controller.set(o);
                    onPicked?.call();
                  },
                ),
            if (stillWatching) ...[
              Padding(
                padding: const EdgeInsets.fromLTRB(14, 16, 14, 0),
                child: Text(
                  l.askSection,
                  style: const TextStyle(
                    color: PaTokens.textSecondary,
                    fontSize: 13,
                  ),
                ),
              ),
              _AskStepper(controller: controller, strings: l),
            ],
          ],
        );
      },
    );
  }
}

class _AskStepper extends StatelessWidget {
  const _AskStepper({required this.controller, required this.strings});

  final SleepTimerController controller;
  final SleepTimerStrings strings;

  @override
  Widget build(BuildContext context) {
    final n = controller.stillWatchingAfter;
    final l = strings;
    return Padding(
      padding: const EdgeInsets.fromLTRB(14, 2, 4, 2),
      child: Row(
        children: [
          Expanded(
            child: Text(
              l.askAfter,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(color: PaTokens.text, fontSize: 15),
            ),
          ),
          FocusIconButton(
            key: const ValueKey('sleep-ask-less'),
            icon: Icons.remove_rounded,
            tooltip: l.fewer,
            size: 40,
            onPressed: n > 0
                ? () => controller.stillWatchingAfter = n - 1
                : null,
          ),
          SizedBox(
            width: 96,
            child: Text(
              l.episodes(n),
              key: const ValueKey('sleep-ask-value'),
              textAlign: TextAlign.center,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(color: PaTokens.text, fontSize: 15),
            ),
          ),
          FocusIconButton(
            key: const ValueKey('sleep-ask-more'),
            icon: Icons.add_rounded,
            tooltip: l.more,
            size: 40,
            onPressed: n < SleepTimerController.kMaxStillWatchingAfter
                ? () => controller.stillWatchingAfter = n + 1
                : null,
          ),
        ],
      ),
    );
  }
}

/// «Пауза через 30 с — Продлить»: the row of the fade before the pause.
/// Empty while there is no [SleepTimerController.warning].
class SleepTimerWarning extends StatelessWidget {
  const SleepTimerWarning({super.key, required this.controller});

  final SleepTimerController controller;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: controller,
      builder: (context, _) {
        if (!controller.warning) return const SizedBox.shrink();
        final l = SleepTimerStrings.of(context);
        final left = controller.remaining ?? Duration.zero;
        final seconds = (left.inMilliseconds / 1000).ceil();
        return _Pill(
          key: const ValueKey('sleep-warning'),
          children: [
            const Icon(Icons.bedtime_outlined, color: PaTokens.text, size: 20),
            const SizedBox(width: 10),
            Flexible(
              child: Text(
                l.pausesInSeconds(seconds),
                key: const ValueKey('sleep-warning-text'),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(color: PaTokens.text, fontSize: 15),
              ),
            ),
            const SizedBox(width: 8),
            Tooltip(
              message: l.extendBy(controller.extension.inMinutes),
              child: TextButton(
                key: const ValueKey('sleep-extend'),
                onPressed: controller.extend,
                child: Text(l.extend),
              ),
            ),
          ],
        );
      },
    );
  }
}

/// «Вы ещё смотрите?» with «Продолжить просмотр» and «Пауза». Empty while
/// the controller does not ask.
class StillWatchingPrompt extends StatelessWidget {
  const StillWatchingPrompt({
    super.key,
    required this.controller,
    required this.onPause,
  });

  final SleepTimerController controller;

  /// «Пауза»: the host pauses now.
  final VoidCallback onPause;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: controller,
      builder: (context, _) {
        if (!controller.asking) return const SizedBox.shrink();
        final l = SleepTimerStrings.of(context);
        final left = controller.askRemaining ?? Duration.zero;
        final seconds = (left.inMilliseconds / 1000).ceil();
        final width = MediaQuery.sizeOf(context).width;
        return Container(
          key: const ValueKey('still-watching'),
          width: (width - 32).clamp(0.0, 420.0),
          padding: const EdgeInsets.all(20),
          decoration: BoxDecoration(
            color: PaTokens.glassStrong,
            borderRadius: BorderRadius.circular(PaTokens.radius),
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                l.stillWatching,
                style: const TextStyle(
                  color: PaTokens.text,
                  fontSize: 20,
                  fontWeight: FontWeight.w600,
                ),
              ),
              const SizedBox(height: 6),
              Text(
                l.askDetail(l.episodes(controller.autoplaysInRow), seconds),
                key: const ValueKey('still-watching-detail'),
                style: const TextStyle(
                  color: PaTokens.textSecondary,
                  fontSize: 14,
                ),
              ),
              const SizedBox(height: 14),
              Wrap(
                spacing: 8,
                runSpacing: 8,
                children: [
                  FilledButton.icon(
                    key: const ValueKey('still-watching-yes'),
                    autofocus: true,
                    onPressed: controller.stillWatching,
                    icon: const Icon(Icons.play_arrow_rounded),
                    label: Text(l.keepWatching),
                  ),
                  TextButton(
                    key: const ValueKey('still-watching-pause'),
                    onPressed: () {
                      controller.stillWatching();
                      onPause();
                    },
                    child: Text(l.pauseNow),
                  ),
                ],
              ),
            ],
          ),
        );
      },
    );
  }
}

class _Pill extends StatelessWidget {
  const _Pill({super.key, required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.fromLTRB(16, 4, 6, 4),
    decoration: BoxDecoration(
      color: PaTokens.glassStrong,
      borderRadius: BorderRadius.circular(999),
    ),
    child: Row(mainAxisSize: MainAxisSize.min, children: children),
  );
}
