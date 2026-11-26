// SPDX-License-Identifier: MPL-2.0
//
// UI-057: when the next episode of an ongoing show or anime airs.
//
// - The show page: «Серия 8 — чт, 4 дек.»; anime (an exact moment from
//   AniList or Shikimori) count down to the hour: «Серия 8 — сб, 28 нояб.,
//   18:30 · через 2 д 5 ч». Offline the last value stays with «нет сети ·
//   данные от …».
// - The library posters: «Новая серия в чт», «Сериал завершён».
// - The episode row: a card of the episode still to come, without a frame,
//   at the end of its season (show_detail_screen.dart).
//
// Times are shown in the device's time zone. The labels of all posters
// come from one batch read ([ShowAiringLabels]); the daily background
// refresh runs in the library (the core asks only the shows that are due).

import 'dart:async';

import 'package:intl/intl.dart';
import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';

/// Key of the airing line on the show page.
const showAiringKey = ValueKey('show-airing');

/// Key of the card of the episode still to come.
const showUpcomingKey = ValueKey('show-upcoming-episode');

/// Key of the label on the poster of [itemId].
ValueKey<String> airingBadgeKey(String itemId) =>
    ValueKey('badge-airing-$itemId');

String _tag(BuildContext context) =>
    Localizations.localeOf(context).toLanguageTag();

DateTime _day(DateTime t) => DateTime(t.year, t.month, t.day);

/// «сегодня», «завтра» or «чт, 4 дек.» (the device's calendar); anime with
/// the local time: «сб, 28 нояб., 18:30».
String airingWhen(
  AppLocalizations l,
  String locale,
  NextAiring next,
  DateTime now,
) {
  final day = next.day;
  if (day == null) return '';
  final today = _day(now);
  final days = day.difference(today).inDays;
  final at = next.at?.toLocal();
  final time = at == null ? '' : DateFormat.Hm(locale).format(at);
  final String date;
  if (days == 0) {
    date = l.airingToday;
  } else if (days == 1) {
    date = l.airingTomorrow;
  } else {
    date = DateFormat.MMMEd(locale).format(day);
  }
  return time.isEmpty ? date : '$date, $time';
}

/// «2 д 5 ч», «5 ч», «меньше часа» until [at].
String airingCountdown(AppLocalizations l, DateTime at, DateTime now) {
  final left = at.difference(now);
  final hours = left.inHours;
  if (hours < 1) return l.airingLeftSoon;
  if (hours < 24) return l.airingLeftHours(hours);
  return l.airingLeftDaysHours(hours ~/ 24, hours % 24);
}

/// The line of the show page; null when there is nothing to say (no data,
/// or the next episode already aired and is waiting for a refresh).
String? airingLine(
  AppLocalizations l,
  String locale,
  ShowAiring a,
  DateTime now,
) {
  final next = a.next;
  final String? text;
  if (a.status == AiringStatus.ended) {
    text = l.airingEnded;
  } else if (next != null && next.day != null) {
    final when = airingWhen(l, locale, next, now);
    final at = next.at;
    text = at == null
        ? l.airingNextOn(next.episode, when)
        : l.airingNextIn(next.episode, when, airingCountdown(l, at, now));
  } else if (a.status == AiringStatus.hiatus) {
    text = l.airingNoDate;
  } else {
    text = null;
  }
  if (text == null) return null;
  if (!a.offline) return text;
  final checked = DateFormat.MMMd(locale).format(a.checkedAt.toLocal());
  return '$text  ·  ${l.airingOffline(checked)}';
}

/// The poster label: «Новая серия в чт», «Сериал завершён»; null otherwise.
String? airingBadge(
  AppLocalizations l,
  String locale,
  ShowAiring a,
  DateTime now,
) {
  if (a.status == AiringStatus.ended) return l.airingEnded;
  final day = a.next?.day;
  if (day == null) return null;
  final days = day.difference(_day(now)).inDays;
  if (days < 0) return null;
  if (days == 0) return l.airingBadgeToday;
  if (days == 1) return l.airingBadgeTomorrow;
  if (days < 7) return l.airingBadgeNext(_weekdays[day.weekday - 1]);
  return l.airingBadgeOn(DateFormat.MMMd(locale).format(day));
}

const _weekdays = [
  'mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun', // l10n-ignore
];

/// The airing state of every show with one, read in one batch and again
/// after each refresh (the daily background refresh runs in the library:
/// [RustLibrary.airingRefresh]).
class ShowAiringLabels extends ValueNotifier<Map<String, ShowAiring>> {
  ShowAiringLabels._(this.library) : super(const {}) {
    _sub = library.airingChanges.listen((_) => unawaited(_load()));
    unawaited(_load());
  }

  final LibraryAiring library;
  StreamSubscription<void>? _sub;

  static final _byLibrary = Expando<ShowAiringLabels>('show airing labels');

  /// The shared labels of [library]; they live as long as the library.
  static ShowAiringLabels of(LibraryAiring library) =>
      _byLibrary[library] ??= ShowAiringLabels._(library);

  /// The labels of the app's library, or null when it cannot tell.
  static ShowAiringLabels? maybeOf(BuildContext context) {
    final library = context
        .getInheritedWidgetOfExactType<Services>()
        ?.notifier
        ?.library;
    return library is LibraryAiring ? of(library as LibraryAiring) : null;
  }

  int _generation = 0;

  Future<void> _load() async {
    final g = ++_generation;
    try {
      final all = await library.airingAll();
      if (g != _generation) return;
      value = {for (final a in all) a.showId: a};
    } catch (e) {
      debugPrint('Airing not read: $e');
    }
  }

  @override
  void dispose() {
    unawaited(_sub?.cancel());
    super.dispose();
  }
}

/// «Новая серия в чт» / «Сериал завершён» at the bottom of a show poster
/// in a [Stack]; nothing for other items and without data.
class AiringPosterLabel extends StatelessWidget {
  const AiringPosterLabel({super.key, required this.item});

  final LibraryItem item;

  @override
  Widget build(BuildContext context) {
    if (item.kind != ItemKind.show) return const SizedBox.shrink();
    final labels = ShowAiringLabels.maybeOf(context);
    if (labels == null) return const SizedBox.shrink();
    return ValueListenableBuilder<Map<String, ShowAiring>>(
      valueListenable: labels,
      builder: (context, all, _) {
        final a = all[item.id];
        final now = Services.read(context).clock();
        final text = a == null
            ? null
            : airingBadge(context.l10n, _tag(context), a, now);
        if (text == null) return const SizedBox.shrink();
        final ended = a!.status == AiringStatus.ended;
        return Positioned(
          left: 6,
          right: 6,
          bottom: 18,
          child: IgnorePointer(
            child: Align(
              alignment: Alignment.bottomLeft,
              child: Container(
                key: airingBadgeKey(item.id),
                padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                decoration: BoxDecoration(
                  color: ended
                      ? Colors.black.withValues(alpha: .62)
                      : KColors.accent,
                  borderRadius: BorderRadius.circular(4),
                ),
                child: Text(
                  text,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: Theme.of(context).textTheme.labelSmall?.copyWith(
                    color: Colors.white,
                    fontWeight: FontWeight.w700,
                  ),
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

/// The card of the episode still to come at the end of the episode row: no
/// frame (no spoiler), the date instead, and it does not open. The remote
/// can focus it: the focused card shows the date large.
class UpcomingEpisodeCard extends StatelessWidget {
  const UpcomingEpisodeCard({
    super.key,
    required this.airing,
    required this.next,
    required this.code,
    required this.width,
    this.focusNode,
  });

  final ShowAiring airing;
  final NextAiring next;

  /// «С1 · Э8» or «Э8».
  final String code;
  final double width;
  final FocusNode? focusNode;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final now = Services.read(context).clock();
    final when = airingWhen(l, _tag(context), next, now);
    final at = next.at;
    final countdown = at == null ? null : airingCountdown(l, at, now);
    final radius = d.cardRadius * .75;
    // No onActivate: OK, Enter and a tap do nothing.
    return KeyedSubtree(
      key: showUpcomingKey,
      child: KFocusable(
        focusNode: focusNode,
        focusScale: d.isTv ? 1.05 : 1.03,
        light: false,
        semanticLabel: l.airingCardLabel(code, when),
        builder: (context, focused) => SizedBox(
          width: width,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              AspectRatio(
                aspectRatio: 16 / 9,
                child: AnimatedContainer(
                  duration: KMotion.focus,
                  decoration: BoxDecoration(
                    borderRadius: BorderRadius.circular(radius),
                    color: Colors.white.withValues(alpha: focused ? .12 : .05),
                    border: Border.all(
                      color: focused
                          ? Colors.white.withValues(alpha: .9)
                          : Colors.white.withValues(alpha: .18),
                      width: focused ? 2 : 1,
                    ),
                  ),
                  padding: const EdgeInsets.all(12),
                  child: Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    children: [
                      Icon(
                        Icons.event_outlined,
                        size: focused ? 30 : 26,
                        color: KColors.textSecondary,
                      ),
                      const SizedBox(height: 8),
                      Text(
                        when,
                        key: const ValueKey('show-upcoming-date'),
                        textAlign: TextAlign.center,
                        maxLines: 2,
                        overflow: TextOverflow.ellipsis,
                        style:
                            (focused
                                    ? theme.textTheme.titleMedium
                                    : theme.textTheme.titleSmall)
                                ?.copyWith(fontWeight: FontWeight.w700),
                      ),
                      if (countdown != null) ...[
                        const SizedBox(height: 2),
                        Text(
                          countdown,
                          maxLines: 1,
                          style: theme.textTheme.labelSmall?.copyWith(
                            color: KColors.textSecondary,
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
              ),
              const SizedBox(height: 10),
              Text(
                code,
                style: theme.textTheme.labelSmall,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
              const SizedBox(height: 2),
              Text(
                l.airingCardTitle,
                style: theme.textTheme.titleMedium?.copyWith(
                  color: KColors.textSecondary,
                ),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ],
          ),
        ),
      ),
    );
  }
}
