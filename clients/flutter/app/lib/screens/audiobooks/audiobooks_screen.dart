// SPDX-License-Identifier: MPL-2.0
//
// MUS-018: the «Аудиокниги» section — its own place in the navigation,
// shown only while the library has books (the core finds them; the user
// never says what is a book). «Продолжить слушать» first, then every book;
// a book page with its chapters; the mini bar of the open book.

import 'package:material_ui/material_ui.dart';

import '../../features/audiobooks/audiobook_service.dart';
import '../../integration/library.dart';
import '../../l10n/l10n.dart';
import '../../services/app_services.dart';
import '../../theme/kinolune_theme.dart';
import '../../widgets/artwork.dart';
import '../../widgets/cards.dart';
import '../../widgets/focusable.dart';
import '../../widgets/format.dart';
import '../music/music_widgets.dart';
import 'book_player_screen.dart';

/// Author, else nothing.
String bookByline(Audiobook b) => b.author ?? '';

/// What the card says under the title: progress, else the author.
String bookCardSubtitle(AppLocalizations l, Audiobook b) {
  final p = b.progress;
  if (p != null && p.finished) return l.audiobookFinished;
  if (b.inProgress && b.durationMs > 0) {
    return l.audiobookLeft(formatRuntime(l, b.durationMs - p!.positionMs));
  }
  return [
    bookByline(b),
    if (b.durationMs > 0) formatRuntime(l, b.durationMs),
  ].where((s) => s.isNotEmpty).join(' · ');
}

/// The cover of a book.
Widget bookArt(Audiobook b) =>
    Artwork(url: b.coverUri, seed: b.id, style: ArtStyle.album, title: b.title);

class AudiobooksScreen extends StatelessWidget {
  const AudiobooksScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final service = Services.of(context).audiobooks;
    final d = KDensity.of(context);
    final l = context.l10n;
    return ListenableBuilder(
      listenable: service,
      builder: (context, _) {
        final books = service.books;
        final started = [
          for (final b in books)
            if (b.inProgress) b,
        ]..sort((a, b) => b.progress!.updatedAt.compareTo(a.progress!.updatedAt));
        return CustomScrollView(
          key: const ValueKey('audiobooks-screen'),
          clipBehavior: Clip.none,
          slivers: [
            SliverToBoxAdapter(child: PageTitle(l.audiobookTitle)),
            if (books.isEmpty)
              SliverFillRemaining(
                hasScrollBody: false,
                child: EmptyState(
                  icon: Icons.menu_book_outlined,
                  title: l.audiobookEmptyTitle,
                  message: l.audiobookEmptyMessage,
                ),
              )
            else ...[
              if (started.isNotEmpty)
                SliverToBoxAdapter(
                  child: Padding(
                    padding: EdgeInsets.only(bottom: d.rowGap),
                    child: MediaRow(
                      key: const ValueKey('audiobooks-continue'),
                      title: l.audiobookContinue,
                      itemCount: started.length,
                      height: albumRowHeight(d),
                      itemBuilder: (context, i) => BookCard(
                        book: started[i],
                        width: d.albumWidth,
                        onTap: () => openAudiobook(context, started[i]),
                      ),
                    ),
                  ),
                ),
              SliverPadding(
                padding: musicPagePadding(d),
                sliver: SliverToBoxAdapter(
                  child: MusicSectionTitle(l.audiobookAll),
                ),
              ),
              SliverPadding(
                padding: musicPagePadding(d),
                sliver: SliverGrid(
                  key: const ValueKey('audiobooks-grid'),
                  gridDelegate: CardGridDelegate(
                    minItemWidth: d.albumWidth,
                    mainSpacing: 18,
                    crossSpacing: d.gap,
                    artAspect: 1,
                    extraHeight: cardCaptionHeight(d),
                  ),
                  delegate: SliverChildBuilderDelegate(
                    (context, i) => LayoutBuilder(
                      builder: (context, c) => BookCard(
                        key: ValueKey('audiobook-card-$i'),
                        book: books[i],
                        width: c.maxWidth,
                        onTap: () => openAudiobook(context, books[i]),
                      ),
                    ),
                    childCount: books.length,
                  ),
                ),
              ),
            ],
            SliverToBoxAdapter(
              child: SizedBox(height: d.pageInsets.bottom + 90),
            ),
          ],
        );
      },
    );
  }
}

/// A square book card with a progress line on the cover.
class BookCard extends StatelessWidget {
  const BookCard({
    super.key,
    required this.book,
    required this.width,
    this.onTap,
  });

  final Audiobook book;
  final double width;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final fraction = book.inProgress ? book.progress!.fraction : null;
    return Stack(
      children: [
        AlbumCard(
          width: width,
          seed: book.id,
          artUrl: book.coverUri,
          title: book.title,
          subtitle: bookCardSubtitle(l, book),
          onTap: onTap,
        ),
        if (fraction != null)
          Positioned(
            left: 8,
            right: 8,
            top: width - 12,
            child: IgnorePointer(child: KProgressBar(value: fraction)),
          ),
      ],
    );
  }
}

/// Opens the page of [book].
void openAudiobook(BuildContext context, Audiobook book) =>
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => AudiobookScreen(bookId: book.id, initial: book),
      ),
    );

/// The page of a book: what it is, «Слушать» / «Продолжить с …» and the
/// chapters.
class AudiobookScreen extends StatelessWidget {
  const AudiobookScreen({super.key, required this.bookId, required this.initial});

  final String bookId;

  /// The book as it was when opened (the list may change meanwhile).
  final Audiobook initial;

  @override
  Widget build(BuildContext context) {
    final service = Services.of(context).audiobooks;
    return ListenableBuilder(
      listenable: Listenable.merge([service, service.player]),
      builder: (context, _) {
        final book =
            service.books.where((b) => b.id == bookId).firstOrNull ?? initial;
        return _page(context, service, book);
      },
    );
  }

  Widget _page(BuildContext context, AudiobookService service, Audiobook book) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final open = service.player.book != null && service.player.book!.key == book.key;
    final current = open ? service.player.chapterIndex : -1;
    final p = book.progress;
    final label = p == null || p.positionMs <= 0
        ? l.audiobookListen
        : p.finished
        ? l.audiobookListenAgain
        : l.audiobookContinueFrom(formatTimecode(p.positionMs));
    final series = book.series;
    return MusicPage(
      slivers: [
        SliverPadding(
          padding: musicPagePadding(d, bottom: 24),
          sliver: SliverToBoxAdapter(
            child: MusicPageHeader(
              art: bookArt(book),
              overline: l.audiobookLabel,
              title: book.title,
              facts: [
                bookByline(book),
                if (book.year != null) '${book.year}',
                if (book.chapters.isNotEmpty)
                  l.audiobookChapterCount(book.chapters.length),
                formatRuntime(l, book.durationMs),
              ].where((s) => s.isNotEmpty).join('  ·  '),
              details: [
                if (book.narrator != null) l.audiobookNarrator(book.narrator!),
                if (series != null)
                  book.seriesNumberLabel == null
                      ? series
                      : l.audiobookSeriesNumber(series, book.seriesNumberLabel!),
              ].join('  ·  '),
              buttons: [
                KButton(
                  key: const ValueKey('audiobook-listen'),
                  label: label,
                  icon: Icons.play_arrow_rounded,
                  primary: true,
                  autofocus: !d.isPhone,
                  onPressed: () async {
                    await service.play(
                      book,
                      startMs: p != null && p.finished ? 0 : null,
                    );
                    if (context.mounted) openBookPlayer(context);
                  },
                ),
              ],
            ),
          ),
        ),
        SliverPadding(
          padding: musicPagePadding(d),
          sliver: SliverToBoxAdapter(child: MusicSectionTitle(l.audiobookChapters)),
        ),
        SliverPadding(
          padding: musicPagePadding(d),
          sliver: SliverList.builder(
            itemCount: book.chapters.length,
            itemBuilder: (context, i) {
              final c = book.chapters[i];
              final playing = i == current;
              return ListTile(
                key: ValueKey('audiobook-chapter-$i'),
                contentPadding: const EdgeInsets.symmetric(horizontal: 10),
                leading: SizedBox(
                  width: 32,
                  child: playing
                      ? const Icon(Icons.graphic_eq_rounded, color: KColors.accent)
                      : Text(
                          '${i + 1}',
                          textAlign: TextAlign.center,
                          style: theme.textTheme.labelMedium,
                        ),
                ),
                title: Text(
                  c.title ?? l.audiobookChapter(i + 1),
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: playing
                      ? theme.textTheme.bodyLarge?.copyWith(color: KColors.accent)
                      : theme.textTheme.bodyLarge,
                ),
                trailing: c.durationMs > 0
                    ? Text(
                        formatTimecode(c.durationMs),
                        style: theme.textTheme.labelMedium,
                      )
                    : null,
                onTap: () => service.playChapter(book, i),
              );
            },
          ),
        ),
      ],
    );
  }
}

/// The mini bar of the open book, docked like the music one.
class BookMiniBar extends StatelessWidget {
  const BookMiniBar({super.key});

  @override
  Widget build(BuildContext context) {
    final service = Services.of(context).audiobooks;
    final player = service.player;
    final l = context.l10n;
    return ListenableBuilder(
      listenable: player,
      builder: (context, _) {
        final b = player.book;
        if (b == null) return const SizedBox.shrink();
        final d = player.duration.inMilliseconds;
        final f = d <= 0
            ? 0.0
            : (player.position.inMilliseconds / d).clamp(0.0, 1.0);
        final chapters = player.chapters;
        final i = player.chapterIndex;
        final chapter = chapters.isEmpty
            ? null
            : chapters[i].title ?? l.audiobookChapter(i + 1);
        return Material(
          color: KColors.bgElevated,
          child: InkWell(
            key: const ValueKey('book-mini-bar'),
            onTap: () => openBookPlayer(context),
            child: SizedBox(
              height: 68,
              child: Column(
                children: [
                  LinearProgressIndicator(
                    value: f,
                    minHeight: 2,
                    backgroundColor: KColors.stroke,
                    color: KColors.textPrimary,
                  ),
                  Expanded(
                    child: Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 12),
                      child: Row(
                        children: [
                          ClipRRect(
                            borderRadius: BorderRadius.circular(6),
                            child: SizedBox.square(
                              dimension: 48,
                              child: Artwork(
                                url: b.artworkUri,
                                seed: b.key ?? b.title,
                                style: ArtStyle.album,
                              ),
                            ),
                          ),
                          const SizedBox(width: 12),
                          Expanded(
                            child: Column(
                              mainAxisAlignment: MainAxisAlignment.center,
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Text(
                                  b.title,
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  style: Theme.of(context).textTheme.titleSmall,
                                ),
                                if (chapter != null)
                                  Text(
                                    chapter,
                                    maxLines: 1,
                                    overflow: TextOverflow.ellipsis,
                                    style: Theme.of(context).textTheme.labelSmall,
                                  ),
                              ],
                            ),
                          ),
                          IconButton(
                            key: const ValueKey('book-bar-back30'),
                            tooltip: l.audiobookBack30,
                            icon: const Icon(Icons.replay_30_rounded),
                            onPressed: player.back,
                          ),
                          IconButton(
                            key: const ValueKey('book-bar-play'),
                            tooltip: player.playing
                                ? l.audiobookPause
                                : l.audiobookListen,
                            icon: Icon(
                              player.playing
                                  ? Icons.pause_rounded
                                  : Icons.play_arrow_rounded,
                            ),
                            iconSize: 32,
                            onPressed: player.togglePlay,
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
