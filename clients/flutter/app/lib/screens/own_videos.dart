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
import '../personal/personal_screen.dart' show moveFolderToPersonal;
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
  StreamSubscription<void>? _personalSub;
  Object? _personal;

  @override
  void initState() {
    super.initState();
    _sub = widget.kinds.contentKindChanges.listen((_) => _load());
    _load();
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    // A clip moved to «Личное» leaves the list.
    final personal = Services.of(context).personal;
    if (!identical(personal, _personal)) {
      _personal = personal;
      _personalSub?.cancel();
      _personalSub = personal?.changes.listen((_) => _load());
    }
  }

  @override
  void dispose() {
    _sub?.cancel();
    _personalSub?.cancel();
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

/// The folder of [video] relative to its source root ('' at the root).
String _folderOf(OwnVideo video) {
  final i = video.relPath.lastIndexOf('/');
  return i <= 0 ? '' : video.relPath.substring(0, i);
}

/// Long press or right click on a clip: «Скрыть в Личное» moves the clip's
/// folder to a personal library (a folder at the source root only, never
/// the whole source).
class _OwnVideoActions extends StatelessWidget {
  const _OwnVideoActions({required this.video, required this.child});

  final OwnVideo video;
  final Widget child;

  Future<void> _menu(BuildContext context) async {
    final folder = _folderOf(video);
    if (folder.isEmpty || Services.read(context).personal == null) return;
    final l = context.l10n;
    final hide = await showModalBottomSheet<bool>(
      context: context,
      builder: (context) => SafeArea(
        child: ListTile(
          key: const ValueKey('own-video-to-personal'),
          leading: const Icon(Icons.lock_outline),
          title: Text(l.homeOwnVideosToPersonal),
          subtitle: Text(folder),
          onTap: () => Navigator.of(context).pop(true),
        ),
      ),
    );
    if (hide == true && context.mounted) {
      await moveFolderToPersonal(context, '${video.sourceId}', folder);
    }
  }

  @override
  Widget build(BuildContext context) => GestureDetector(
    onLongPress: () => _menu(context),
    onSecondaryTap: () => _menu(context),
    child: child,
  );
}

/// Home when the library has nothing but the user's own clips: the
/// «Мои видео» row instead of the empty state.
class HomeOwnVideosOrEmpty extends StatelessWidget {
  const HomeOwnVideosOrEmpty({super.key, required this.empty});

  final Widget empty;

  @override
  Widget build(BuildContext context) {
    final kinds = contentKindsOf(context);
    if (kinds == null) return empty;
    return _OwnVideosLoader(
      kinds: kinds,
      builder: (context, videos) {
        if (videos == null || videos.isEmpty) return empty;
        final d = KDensity.of(context);
        return ListView(
          key: const ValueKey('home-only-own-videos'),
          padding: EdgeInsets.only(
            top: d.safeTop(context) + 24,
            bottom: d.pageInsets.bottom + 24,
          ),
          clipBehavior: Clip.none,
          children: const [OwnVideosRow()],
        );
      },
    );
  }
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
            title: l.homeOwnVideosTitle,
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
              return _OwnVideoActions(
                video: v,
                child: LandscapeCard(
                  key: ValueKey('own-video-${v.fileId}'),
                  width: d.landscapeWidth,
                  seed: 'own:${v.fileId}',
                  title: v.title,
                  subtitle: ownVideoSubtitle(context, v),
                  onTap: () => playOwnVideo(context, kinds, v),
                ),
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
          l.homeOwnVideosTitle,
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
                    title: l.homeOwnVideosEmptyTitle,
                    message: l.homeOwnVideosEmptyMessage,
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
                      builder: (context, c) => _OwnVideoActions(
                        video: v,
                        child: LandscapeCard(
                          key: ValueKey('own-video-${v.fileId}'),
                          width: c.maxWidth,
                          seed: 'own:${v.fileId}',
                          title: v.title,
                          subtitle: ownVideoSubtitle(context, v),
                          autofocus: i == 0 && d.isTv,
                          onTap: () => playOwnVideo(context, kinds, v),
                        ),
                      ),
                    );
                  },
                );
              },
            ),
    );
  }
}
