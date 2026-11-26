// SPDX-License-Identifier: MPL-2.0
//
// LIB-028: the grey card of a file on its way into the library, on every
// device; the small «Это „…“?» card right after a drop (nothing to press
// when it is right, BUG-009) and «Не то» — search and pick by hand.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../files/file_size.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/artwork.dart';
import '../widgets/focusable.dart';
import 'live_uploads.dart';

/// «Копируется с …» and the line below it of a card in [p].
(String, String?) liveUploadStatus(
  AppLocalizations l,
  Locale locale,
  LiveUpload u,
  LiveUploadProgress p,
) {
  final info = u.info.value;
  final device = u.local
      ? l.liveUploadThisDevice
      : (info.deviceName.isEmpty ? '—' : info.deviceName);
  switch (p.state) {
    case LiveUploadState.waiting:
      return (l.liveUploadWaiting(device), _bytes(l, locale, p));
    case LiveUploadState.copying:
      final parts = [
        ?_bytes(l, locale, p),
        if (p.rate > 0) l.liveUploadSpeed(fileSize(l, locale, p.rate)),
        if (p.eta case final eta?) liveUploadEta(l, eta),
      ];
      return (
        l.liveUploadFrom(device, '${p.percent}'),
        parts.isEmpty ? null : parts.join(' · '),
      );
    case LiveUploadState.verifying:
      return (l.liveUploadVerifying, _bytes(l, locale, p));
    case LiveUploadState.paused:
      return (l.liveUploadPaused, _bytes(l, locale, p));
    case LiveUploadState.offline:
      return (l.liveUploadOffline(device), _bytes(l, locale, p));
    case LiveUploadState.done:
      return (l.liveUploadAdding, null);
    case LiveUploadState.failed:
      return (l.liveUploadFailed, null);
    case LiveUploadState.cancelled:
      return (l.liveUploadCancelled, null);
  }
}

String? _bytes(AppLocalizations l, Locale locale, LiveUploadProgress p) =>
    p.size <= 0
    ? null
    : l.liveUploadBytes(
        fileSize(l, locale, p.done),
        fileSize(l, locale, p.size),
      );

/// «ещё 20 с», «ещё 4 мин», «ещё 1 ч 5 мин».
String liveUploadEta(AppLocalizations l, Duration eta) {
  final s = eta.inSeconds;
  if (s < 60) return l.liveUploadEtaSeconds(s < 1 ? 1 : s);
  if (s < 3600) return l.liveUploadEtaMinutes((s / 60).ceil());
  return l.liveUploadEtaHours(s ~/ 3600, (s % 3600) ~/ 60);
}

/// What the card is called: the guess, else the file's name.
String liveUploadTitle(LiveUploadInfo info) =>
    info.guess.isEmpty ? info.name : info.guess.title;

/// The line under the title: the year, or the season and episode.
String liveUploadSubtitle(AppLocalizations l, LiveUploadInfo info) {
  final g = info.guess;
  if (info.kind == LiveUploadKind.series &&
      g.season != null &&
      g.episode != null) {
    return l.liveUploadEpisode(g.season!, g.episode!);
  }
  return g.year?.toString() ?? '';
}

/// Whether this device may cancel [u]: the device that copies, or an
/// administrator (any profile that is not a restricted kid's).
bool canCancelLiveUpload(BuildContext context, LiveUpload u) =>
    u.local || !Services.read(context).profileControl.restricted;

/// The grey card in a library grid: as wide as a poster, the art grey and
/// see-through, a live bar and the status; it cannot be opened until it is
/// copied. Repaints on its own notifiers only.
class LiveUploadCard extends StatelessWidget {
  const LiveUploadCard({super.key, required this.upload, required this.width});

  final LiveUpload upload;
  final double width;

  void _tap(BuildContext context) {
    final l = context.l10n;
    ScaffoldMessenger.maybeOf(context)
      ?..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(l.liveUploadUnavailable)));
  }

  Future<void> _menu(BuildContext context, Offset at) async {
    if (!canCancelLiveUpload(context, upload)) return;
    final l = context.l10n;
    final live = Services.read(context).liveUploads;
    final overlay = Overlay.of(context).context.findRenderObject() as RenderBox;
    final pick = await showMenu<bool>(
      context: context,
      position: RelativeRect.fromRect(
        at & const Size(1, 1),
        Offset.zero & overlay.size,
      ),
      items: [
        PopupMenuItem(
          key: ValueKey('live-upload-cancel-${upload.id}'),
          value: true,
          child: Text(l.liveUploadCancel),
        ),
      ],
    );
    if (pick == true) await live.cancel(upload);
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    final l = context.l10n;
    final locale = Localizations.localeOf(context);
    return ValueListenableBuilder<LiveUploadInfo>(
      valueListenable: upload.info,
      builder: (context, info, _) {
        final title = liveUploadTitle(info);
        return SizedBox(
          width: width,
          child: Column(
            key: ValueKey('live-upload-${upload.id}'),
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              KFocusable(
                onActivate: () => _tap(context),
                onContextMenu: (at) => unawaited(_menu(context, at)),
                focusScale: d.focusScalePoster,
                semanticLabel: title,
                builder: (context, _) => AspectRatio(
                  aspectRatio: 2 / 3,
                  child: ValueListenableBuilder<LiveUploadProgress>(
                    valueListenable: upload.progress,
                    builder: (context, p, _) {
                      final done = p.state == LiveUploadState.done;
                      final (status, detail) = liveUploadStatus(
                        l,
                        locale,
                        upload,
                        p,
                      );
                      return Stack(
                        fit: StackFit.expand,
                        children: [
                          _GreyArt(
                            grey: !done,
                            child: Artwork(
                              url: info.guess.posterUrl,
                              seed: upload.id,
                              style: ArtStyle.poster,
                              title: title,
                              subtitle: info.guess.year?.toString(),
                            ),
                          ),
                          if (!done)
                            Center(
                              child: Icon(
                                p.state == LiveUploadState.offline ||
                                        p.state == LiveUploadState.paused
                                    ? Icons.pause_circle_outline
                                    : Icons.cloud_upload_outlined,
                                size: d.isTv ? 44 : 32,
                                color: Colors.white70,
                              ),
                            ),
                          Positioned(
                            left: 0,
                            right: 0,
                            bottom: 0,
                            child: Container(
                              padding: EdgeInsets.fromLTRB(8, 18, 8, 8),
                              decoration: const BoxDecoration(
                                gradient: LinearGradient(
                                  begin: Alignment.topCenter,
                                  end: Alignment.bottomCenter,
                                  colors: [
                                    Color(0x00000000),
                                    Color(0xD9000000),
                                  ],
                                ),
                              ),
                              child: Column(
                                crossAxisAlignment: CrossAxisAlignment.start,
                                mainAxisSize: MainAxisSize.min,
                                children: [
                                  Text(
                                    status,
                                    key: ValueKey(
                                      'live-upload-status-${upload.id}',
                                    ),
                                    maxLines: 2,
                                    overflow: TextOverflow.ellipsis,
                                    style: theme.textTheme.labelSmall?.copyWith(
                                      color: Colors.white,
                                    ),
                                  ),
                                  if (detail != null)
                                    Text(
                                      detail,
                                      key: ValueKey(
                                        'live-upload-detail-${upload.id}',
                                      ),
                                      maxLines: 2,
                                      overflow: TextOverflow.ellipsis,
                                      style: theme.textTheme.labelSmall
                                          ?.copyWith(
                                            color: KColors.textSecondaryOnArt,
                                            fontSize:
                                                (theme
                                                        .textTheme
                                                        .labelSmall
                                                        ?.fontSize ??
                                                    11) -
                                                1,
                                          ),
                                    ),
                                  const SizedBox(height: 6),
                                  LiveUploadBar(progress: p),
                                ],
                              ),
                            ),
                          ),
                        ],
                      );
                    },
                  ),
                ),
              ),
              SizedBox(height: d.isTv ? 10 : 8),
              Text(
                title,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: theme.textTheme.titleSmall?.copyWith(
                  color: KColors.textSecondary,
                ),
              ),
              Text(
                liveUploadSubtitle(l, info),
                maxLines: 1,
                style: theme.textTheme.labelSmall,
              ),
            ],
          ),
        );
      },
    );
  }
}

/// Grey and see-through while it is copied; as is once it is done.
class _GreyArt extends StatelessWidget {
  const _GreyArt({required this.grey, required this.child});

  final bool grey;
  final Widget child;

  static const _greyscale = ColorFilter.matrix([
    0.2126, 0.7152, 0.0722, 0, 0, //
    0.2126, 0.7152, 0.0722, 0, 0,
    0.2126, 0.7152, 0.0722, 0, 0,
    0, 0, 0, 1, 0,
  ]);

  @override
  Widget build(BuildContext context) => AnimatedOpacity(
    opacity: grey ? .45 : 1,
    duration: KMotion.crossfadeArt,
    child: grey ? ColorFiltered(colorFilter: _greyscale, child: child) : child,
  );
}

/// The bar: moves smoothly between ticks (they come every 100 ms while
/// watched, every second otherwise).
class LiveUploadBar extends StatelessWidget {
  const LiveUploadBar({super.key, required this.progress});

  final LiveUploadProgress progress;

  @override
  Widget build(BuildContext context) {
    final f = progress.fraction;
    final color = switch (progress.state) {
      LiveUploadState.offline ||
      LiveUploadState.paused => KColors.statusWarning,
      LiveUploadState.failed => KColors.statusProblem,
      LiveUploadState.done => KColors.statusOk,
      _ => KColors.accent,
    };
    return ClipRRect(
      borderRadius: BorderRadius.circular(2),
      child: f == null
          ? LinearProgressIndicator(minHeight: 4, color: color)
          : TweenAnimationBuilder<double>(
              tween: Tween(end: f),
              duration: const Duration(milliseconds: 300),
              builder: (context, v, _) => LinearProgressIndicator(
                value: v,
                minHeight: 4,
                color: color,
                backgroundColor: Colors.white24,
              ),
            ),
    );
  }
}

/// The grey cards of one library page, in the page's grid (before its
/// items: new things first). While it shows cards this app watches, so the
/// devices that copy send 10 updates a second.
class LiveUploadsSliver extends StatefulWidget {
  const LiveUploadsSliver({
    super.key,
    required this.shows,
    required this.gridDelegate,
    required this.padding,
  });

  /// The TV shows page (else movies).
  final bool shows;
  final SliverGridDelegate gridDelegate;
  final EdgeInsets padding;

  @override
  State<LiveUploadsSliver> createState() => _LiveUploadsSliverState();
}

class _LiveUploadsSliverState extends State<LiveUploadsSliver> {
  LiveUploads? _live;
  var _watching = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final live = Services.of(context).liveUploads;
    if (!identical(live, _live)) {
      _watch(false);
      _live = live;
    }
  }

  void _watch(bool on) {
    if (on == _watching) return;
    _watching = on;
    if (on) {
      _live?.addWatcher();
    } else {
      _live?.removeWatcher();
    }
  }

  @override
  void dispose() {
    _watch(false);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final live = _live!;
    return ListenableBuilder(
      listenable: live,
      builder: (context, _) {
        final cards = live.forPage(shows: widget.shows);
        _watch(cards.isNotEmpty);
        if (cards.isEmpty) return const SliverToBoxAdapter(child: SizedBox());
        return SliverPadding(
          padding: widget.padding,
          sliver: SliverGrid(
            gridDelegate: widget.gridDelegate,
            delegate: SliverChildBuilderDelegate(
              (context, i) => LayoutBuilder(
                builder: (context, c) => LiveUploadCard(
                  key: ValueKey(cards[i].id),
                  upload: cards[i],
                  width: c.maxWidth,
                ),
              ),
              childCount: cards.length,
            ),
          ),
        );
      },
    );
  }
}

// --- the guess right after a drop -------------------------------------------

/// «Это „Дюна: Часть вторая“ (2024)?» with its poster: shown for a few
/// seconds after a drop; nothing to press when it is right. [extra] is
/// the line of LIB-027 (where it goes).
void showLiveUploadGuess(BuildContext context, LiveUpload u, String extra) {
  final l = context.l10n;
  ScaffoldMessenger.maybeOf(context)
    ?..hideCurrentSnackBar()
    ..showSnackBar(
      SnackBar(
        key: const ValueKey('live-upload-guess'),
        duration: const Duration(seconds: 8),
        // Goes away by itself: no answer is an answer (BUG-009).
        persist: false,
        content: _GuessContent(upload: u, extra: extra),
        action: SnackBarAction(
          key: const ValueKey('live-upload-not-this'),
          label: l.liveUploadNotThis,
          onPressed: () {
            if (context.mounted) {
              unawaited(showLiveUploadPicker(context, u));
            }
          },
        ),
      ),
    );
}

class _GuessContent extends StatelessWidget {
  const _GuessContent({required this.upload, required this.extra});

  final LiveUpload upload;
  final String extra;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return ValueListenableBuilder<LiveUploadInfo>(
      valueListenable: upload.info,
      builder: (context, info, _) {
        final g = info.guess;
        final question = g.year == null
            ? l.liveUploadGuess(liveUploadTitle(info))
            : l.liveUploadGuessYear(liveUploadTitle(info), '${g.year}');
        return Row(
          children: [
            SizedBox(
              width: 36,
              height: 54,
              child: ClipRRect(
                borderRadius: BorderRadius.circular(4),
                child: Artwork(
                  url: g.posterUrl,
                  seed: upload.id,
                  style: ArtStyle.poster,
                  title: liveUploadTitle(info),
                ),
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    question,
                    key: const ValueKey('live-upload-guess-text'),
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                  ),
                  if (extra.isNotEmpty)
                    Text(
                      extra,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: Theme.of(context).textTheme.bodySmall,
                    ),
                ],
              ),
            ),
          ],
        );
      },
    );
  }
}

/// «Не то»: search by title and year and pick what it is.
Future<void> showLiveUploadPicker(BuildContext context, LiveUpload u) async {
  final live = Services.read(context).liveUploads;
  final picked = await showDialog<LiveUploadCandidate>(
    context: context,
    builder: (_) => _PickerDialog(upload: u, live: live),
  );
  if (picked != null) await live.correct(u, picked);
}

class _PickerDialog extends StatefulWidget {
  const _PickerDialog({required this.upload, required this.live});

  final LiveUpload upload;
  final LiveUploads live;

  @override
  State<_PickerDialog> createState() => _PickerDialogState();
}

class _PickerDialogState extends State<_PickerDialog> {
  late final _title = TextEditingController(
    text: liveUploadTitle(widget.upload.info.value),
  );
  late final _year = TextEditingController(
    text: widget.upload.info.value.guess.year?.toString() ?? '',
  );
  List<LiveUploadCandidate>? _found;
  var _busy = false;

  @override
  void initState() {
    super.initState();
    unawaited(_search());
  }

  @override
  void dispose() {
    _title.dispose();
    _year.dispose();
    super.dispose();
  }

  Future<void> _search() async {
    setState(() => _busy = true);
    List<LiveUploadCandidate> found;
    try {
      found = await widget.live.link.search(
        widget.upload.info.value.kind,
        _title.text.trim(),
        int.tryParse(_year.text.trim()),
      );
    } catch (_) {
      found = const [];
    }
    if (mounted) {
      setState(() {
        _found = found;
        _busy = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final found = _found;
    return AlertDialog(
      key: const ValueKey('live-upload-picker'),
      title: Text(l.liveUploadPickTitle),
      content: SizedBox(
        width: 460,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              widget.upload.info.value.name,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: Theme.of(context).textTheme.labelSmall,
            ),
            const SizedBox(height: 12),
            Row(
              children: [
                Expanded(
                  child: TextField(
                    key: const ValueKey('live-upload-picker-title'),
                    controller: _title,
                    autofocus: true,
                    decoration: InputDecoration(
                      labelText: l.liveUploadSearchLabel,
                    ),
                    onSubmitted: (_) => unawaited(_search()),
                  ),
                ),
                const SizedBox(width: 12),
                SizedBox(
                  width: 88,
                  child: TextField(
                    controller: _year,
                    keyboardType: TextInputType.number,
                    decoration: InputDecoration(
                      labelText: l.liveUploadSearchYear,
                    ),
                    onSubmitted: (_) => unawaited(_search()),
                  ),
                ),
                IconButton(
                  key: const ValueKey('live-upload-picker-search'),
                  tooltip: l.liveUploadSearch,
                  icon: const Icon(Icons.search),
                  onPressed: _busy ? null : () => unawaited(_search()),
                ),
              ],
            ),
            const SizedBox(height: 12),
            ConstrainedBox(
              constraints: const BoxConstraints(maxHeight: 320),
              child: switch (found) {
                null => const Center(
                  child: Padding(
                    padding: EdgeInsets.all(24),
                    child: CircularProgressIndicator(),
                  ),
                ),
                [] => Padding(
                  padding: const EdgeInsets.all(12),
                  child: Text(l.liveUploadNoResults),
                ),
                _ => ListView(
                  shrinkWrap: true,
                  children: [
                    for (final (i, c) in found.indexed)
                      ListTile(
                        key: ValueKey('live-upload-candidate-$i'),
                        autofocus: i == 0,
                        leading: SizedBox(
                          width: 32,
                          height: 48,
                          child: Artwork(
                            url: c.posterUrl,
                            seed: c.tmdbId ?? c.title,
                            style: ArtStyle.poster,
                            title: c.title,
                          ),
                        ),
                        title: Text(c.title),
                        subtitle: Text(c.year?.toString() ?? ''),
                        onTap: () => Navigator.of(context).pop(c),
                      ),
                  ],
                ),
              },
            ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.commonCancel),
        ),
      ],
    );
  }
}
