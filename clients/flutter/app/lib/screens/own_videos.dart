// SPDX-License-Identifier: MPL-2.0
//
// «Мои видео» (LIB-025, ADR-0099): footage from phones and cameras the core
// found among the user's files. They are never recognized and never sent
// to a provider; they are neither hidden nor locked («Личное» is where the
// user moves things, UI-034). Home shows them in a row ([OwnVideosRow]),
// «Все» opens the whole list ([OwnVideosScreen]); a tap plays the file.

import 'dart:async';

import 'package:intl/intl.dart';
import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../integration/player.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/cards.dart';
import '../widgets/format.dart';
import 'playback.dart' show offerMusicResume;

/// The content kinds of the app's library, if it has them (the Rust core).
LibraryContentKinds? contentKindsOf(BuildContext context) =>
    switch (Services.of(context).library) {
      final LibraryContentKinds k => k,
      _ => null,
    };

/// «12 июн. 2024 · 3 мин»: when it was shot and how long it is.
String ownVideoSubtitle(BuildContext context, OwnVideo v) {
  final l = context.l10n;
  final locale = Localizations.localeOf(context).toLanguageTag();
  return [
    DateFormat.yMMMd(locale).format(v.takenAt),
    formatRuntime(l, v.durationMs),
  ].where((s) => s.isNotEmpty).join(' · ');
}

/// Plays [video]; a snackbar if it cannot be played here.
Future<void> playOwnVideo(
  BuildContext context,
  LibraryContentKinds kinds,
  OwnVideo video,
) async {
  final services = Services.read(context);
  final l = context.l10n;
  final messenger = ScaffoldMessenger.maybeOf(context);
  final navigator = Navigator.of(context, rootNavigator: true);
  final uris = await kinds.ownVideoPlayUris(video);
  if (uris.isEmpty) {
    messenger?.showSnackBar(SnackBar(content: Text(l.browserCannotPlay)));
    return;
  }
  final musicInterrupted = services.music.pauseForVideo();
  await navigator.push(
    MaterialPageRoute<void>(
      fullscreenDialog: true,
      builder: (_) => PlayerScreen(
        request: PlayRequest(
          uri: uris.first,
          title: video.title,
          durationMs: video.durationMs,
        ),
        // Not an item of the library: no shared «Продолжить», no sync.
        onProgress: (_, _) {},
        engineFactory: services.playerEngineFactory,
        subtitleSettings: services.subtitleSettings,
        subtitleLanguages: services.subtitleLanguages,
        seekStep: Duration(seconds: services.skipSeconds.value),
      ),
    ),
  );
  if (musicInterrupted) offerMusicResume(services, messenger, l);
}

/// «Мои видео» of [kinds], reloaded when the library changes.
class _OwnVideosLoader extends StatefulWidget {
  const _OwnVideosLoader({required this.kinds, required this.builder});

  final LibraryContentKinds kinds;
  final Widget Function(BuildContext context, List<OwnVideo>? videos) builder;

  @override
  State<_OwnVideosLoader> createState() => _OwnVideosLoaderState();
}

class _OwnVideosLoaderState extends State<_OwnVideosLoader> {
  List<OwnVideo>? _videos;
  StreamSubscription<void>? _sub;

  @override
  void initState() {
    super.initState();
    _sub = widget.kinds.contentKindChanges.listen((_) => _load());
    _load();
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  Future<void> _load() async {
    try {
      final v = await widget.kinds.ownVideos();
      if (mounted) setState(() => _videos = v);
    } catch (_) {
      // The library closed meanwhile: keep what is shown.
    }
  }

  @override
  Widget build(BuildContext context) => widget.builder(context, _videos);
}

/// The «Мои видео» row of Home; nothing while there are none.
class OwnVideosRow extends StatelessWidget {
  const OwnVideosRow({super.key});

  @override
  Widget build(BuildContext context) {
    final kinds = contentKindsOf(context);
    if (kinds == null) return const SizedBox.shrink();
    return _OwnVideosLoader(
      kinds: kinds,
      builder: (context, videos) {
        if (videos == null || videos.isEmpty) return const SizedBox.shrink();
        final d = KDensity.of(context);
        final l = context.l10n;
        return Padding(
          padding: EdgeInsets.only(bottom: d.rowGap),
          child: MediaRow(
            key: const ValueKey('home-own-videos'),
            title: l.ownVideosTitle,
            itemCount: videos.length,
            height:
                d.landscapeWidth * 9 / 16 +
                cardCaptionHeight(d) +
                (d.isTv ? 12 : 6),
            onSeeAll: () => Navigator.of(context).push(
              MaterialPageRoute<void>(builder: (_) => const OwnVideosScreen()),
            ),
            itemBuilder: (context, i) {
              final v = videos[i];
              return LandscapeCard(
                key: ValueKey('own-video-${v.fileId}'),
                width: d.landscapeWidth,
                seed: 'own:${v.fileId}',
                title: v.title,
                subtitle: ownVideoSubtitle(context, v),
                onTap: () => playOwnVideo(context, kinds, v),
              );
            },
          ),
        );
      },
    );
  }
}

/// All of «Мои видео», newest first.
class OwnVideosScreen extends StatelessWidget {
  const OwnVideosScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final kinds = contentKindsOf(context);
    final d = KDensity.of(context);
    final l = context.l10n;
    return Scaffold(
      key: const ValueKey('own-videos-page'),
      backgroundColor: KColors.bgBase,
      appBar: AppBar(
        backgroundColor: KColors.bgBase,
        title: Text(
          l.ownVideosTitle,
          style: Theme.of(context).textTheme.titleLarge,
        ),
      ),
      body: kinds == null
          ? const SizedBox.shrink()
          : _OwnVideosLoader(
              kinds: kinds,
              builder: (context, videos) {
                if (videos == null) {
                  return const Center(child: CircularProgressIndicator());
                }
                if (videos.isEmpty) {
                  return EmptyState(
                    icon: Icons.videocam_outlined,
                    title: l.ownVideosEmptyTitle,
                    message: l.ownVideosEmptyMessage,
                  );
                }
                return GridView.builder(
                  key: const ValueKey('own-videos-grid'),
                  padding: EdgeInsets.fromLTRB(
                    d.pageInsets.left,
                    12,
                    d.pageInsets.right,
                    d.pageInsets.bottom + 24,
                  ),
                  gridDelegate: CardGridDelegate(
                    minItemWidth: d.landscapeWidth,
                    mainSpacing: 18,
                    crossSpacing: d.gap,
                    artAspect: 9 / 16,
                    extraHeight: cardCaptionHeight(d) + (d.isTv ? 12 : 6),
                  ),
                  itemCount: videos.length,
                  itemBuilder: (context, i) {
                    final v = videos[i];
                    return LayoutBuilder(
                      builder: (context, c) => LandscapeCard(
                        key: ValueKey('own-video-${v.fileId}'),
                        width: c.maxWidth,
                        seed: 'own:${v.fileId}',
                        title: v.title,
                        subtitle: ownVideoSubtitle(context, v),
                        autofocus: i == 0 && d.isTv,
                        onTap: () => playOwnVideo(context, kinds, v),
                      ),
                    );
                  },
                );
              },
            ),
    );
  }
}
