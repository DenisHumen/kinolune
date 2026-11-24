// SPDX-License-Identifier: MPL-2.0
//
// Search (UI-011, 01 §1.5; one search for everything — UI-045): as-you-type
// search over movies, shows, episodes, music (artists, albums, tracks),
// audiobooks, personal media of the libraries that are open now and Twitch
// channels, in groups with «Все»; recent searches per profile. Keyboard and
// remote: ↓ leaves the field for the results, the arrows move between them,
// OK / Enter opens one, Esc clears the field first and then goes back;
// Ctrl/Cmd+K or «/» anywhere opens Search (app_shell.dart). On a TV, a
// voice button (the system recognizer, voice_input.dart) and «Ввести с
// телефона» (UI-027) sit next to the field.

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../personal/personal_screen.dart' show PersonalLibraryScreen;
import '../platform/voice_input.dart';
import '../services/app_services.dart';
import '../shell/app_shell.dart' show AppShellState;
import '../shell/section_active.dart';
import '../theme/kinolune_theme.dart';
import '../twitch/twitch_models.dart' show TwitchChannel;
import '../widgets/cards.dart';
import '../widgets/focusable.dart';
import '../widgets/format.dart';
import 'audiobooks/audiobooks_screen.dart' show BookCard, openAudiobook;
import 'music/music_pages.dart' show openAlbum, openArtist;
import 'music/music_widgets.dart'
    show ArtistCard, TrackTile, albumArtist, albumRowHeight, albumTitle;
import 'playback.dart';
import 'routes.dart';
import 'search_model.dart';
import 'search_sources.dart';
import 'twitch_screen.dart' show ChannelTile;

/// The title of a result group.
String searchSectionTitle(AppLocalizations l, SearchSection s) => switch (s) {
  SearchSection.movies => l.navMovies,
  SearchSection.shows => l.navShows,
  SearchSection.episodes => l.searchEpisodes,
  SearchSection.artists => l.musicSectionArtists,
  SearchSection.albums => l.musicSectionAlbums,
  SearchSection.tracks => l.musicSectionTracks,
  SearchSection.audiobooks => l.searchGroupAudiobooks,
  SearchSection.personal => l.searchGroupPersonal,
  SearchSection.channels => l.searchGroupChannels,
};

class SearchScreen extends StatefulWidget {
  const SearchScreen({super.key, this.initialQuery = ''});

  final String initialQuery;

  @override
  State<SearchScreen> createState() => _SearchScreenState();
}

class _SearchScreenState extends State<SearchScreen> {
  late final _text = TextEditingController(text: widget.initialQuery);
  final _field = FocusNode(debugLabel: 'search-field');
  UnifiedSearch? _search;
  SearchHistory? _history;
  MusicSearchIndex? _music;
  ValueNotifier<int>? _focusRequests;

  /// The TV can listen (a system speech recognizer).
  bool _voice = false;

  bool _shown = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    // The shell keeps Search alive while hidden, so `autofocus` fired once,
    // before the tab was opened: focus the field each time it is shown.
    final shown = SectionActive.of(context);
    if (shown && !_shown && KDensity.of(context).isDesktop) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted && SectionActive.of(context)) _field.requestFocus();
      });
    }
    _shown = shown;
    // Ctrl/Cmd+K, «/» and the menu ask for the field (UI-045).
    final requests = context
        .findAncestorStateOfType<AppShellState>()
        ?.searchFocusRequests;
    if (!identical(requests, _focusRequests)) {
      _focusRequests?.removeListener(_onFocusRequest);
      _focusRequests = requests?..addListener(_onFocusRequest);
    }
    if (_search != null) return;
    final services = Services.of(context);
    _music = MusicSearchIndex(libraryMusic(services.library));
    _search = UnifiedSearch(searchSources(services, music: _music));
    _history = SearchHistory(
      services.preferences,
      key: services.profileControl.searchHistoryKey,
    )..load();
    if (widget.initialQuery.trim().isNotEmpty) {
      _search!.submit(widget.initialQuery);
    }
    if (KDensity.of(context).isTv) {
      VoiceInput.instance.available().then((v) {
        if (mounted && v != _voice) setState(() => _voice = v);
      });
    }
  }

  @override
  void dispose() {
    _focusRequests?.removeListener(_onFocusRequest);
    _search?.dispose();
    _history?.dispose();
    _music?.dispose();
    _text.dispose();
    _field.dispose();
    super.dispose();
  }

  void _onFocusRequest() {
    if (!mounted || !(ModalRoute.of(context)?.isCurrent ?? true)) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _field.requestFocus();
    });
  }

  void _setText(String q) {
    _text.value = TextEditingValue(
      text: q,
      selection: TextSelection.collapsed(offset: q.length),
    );
  }

  void _clear() {
    _setText('');
    _search!.update('');
    _field.requestFocus();
  }

  void _submit(String q) {
    _search!.submit(q);
    _history!.add(q);
  }

  void _fromHistory(String q) {
    _setText(q);
    _submit(q);
  }

  /// Opens a result and remembers the query that found it.
  void _open(VoidCallback open) {
    _history!.add(_text.text);
    open();
  }

  /// Voice search on the TV: the system listening screen, then the text
  /// goes into the field and is searched.
  Future<void> _listen() async {
    final l = context.l10n;
    final language = Localizations.localeOf(context).toLanguageTag();
    final text = await VoiceInput.instance.listen(
      language: language,
      prompt: l.searchVoicePrompt,
    );
    if (!mounted || text == null) return;
    _setText(text);
    _submit(text);
  }

  /// «Ввести с телефона» (UI-027): the focused field asks the paired
  /// phones for the text.
  void _fromPhone() => _field.requestFocus();

  bool _phoneInput(AppServices services) =>
      services.sync.available && services.sync.paired;

  void _seeAll(SearchSection section) {
    final query = _search!.query;
    _history!.add(query);
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => SearchAllScreen(
          section: section,
          query: query,
          load: () => _search!.all(section, query),
        ),
      ),
    );
  }

  KeyEventResult _fieldKey(FocusNode _, KeyEvent event) {
    if (event is! KeyDownEvent) return KeyEventResult.ignored;
    final key = event.logicalKey;
    if (key == LogicalKeyboardKey.escape && _text.text.isNotEmpty) {
      _clear();
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.arrowDown) {
      // The field would move the caret; leave it for the first result.
      _field.focusInDirection(TraversalDirection.down);
      return KeyEventResult.handled;
    }
    final sel = _text.selection;
    if (key == LogicalKeyboardKey.arrowRight &&
        KDensity.of(context).isTv &&
        sel.isCollapsed &&
        sel.baseOffset >= _text.text.length) {
      // The caret is at the end: on to the voice and phone buttons.
      return _field.focusInDirection(TraversalDirection.right)
          ? KeyEventResult.handled
          : KeyEventResult.ignored;
    }
    return KeyEventResult.ignored;
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final search = _search!;
    final services = Services.of(context);
    final phone = d.isTv && _phoneInput(services);

    final field = Focus(
      canRequestFocus: false,
      skipTraversal: true,
      onKeyEvent: _fieldKey,
      child: ListenableBuilder(
        listenable: _text,
        builder: (context, _) => TextField(
          controller: _text,
          focusNode: _field,
          autofocus: d.isDesktop,
          onChanged: search.update,
          onSubmitted: _submit,
          textInputAction: TextInputAction.search,
          style: Theme.of(context).textTheme.bodyLarge,
          decoration: InputDecoration(
            prefixIcon: const Icon(Icons.search),
            hintText: l.searchHint,
            suffixIcon: _text.text.isEmpty
                ? null
                : IconButton(
                    tooltip: l.commonClear,
                    onPressed: _clear,
                    icon: const Icon(Icons.close),
                  ),
          ),
        ),
      ),
    );

    return ListView(
      clipBehavior: Clip.none,
      padding: EdgeInsets.only(bottom: d.pageInsets.bottom + 90),
      children: [
        PageTitle(l.navSearch),
        Padding(
          padding: EdgeInsets.fromLTRB(
            d.pageInsets.left,
            0,
            d.pageInsets.right,
            24,
          ),
          child: ConstrainedBox(
            constraints: BoxConstraints(maxWidth: d.isTv ? 960 : 640),
            child: !(_voice && d.isTv) && !phone
                ? field
                : Row(
                    children: [
                      Expanded(child: field),
                      if (_voice && d.isTv) ...[
                        const SizedBox(width: 12),
                        KButton(
                          key: const ValueKey('search-voice'),
                          label: l.searchVoice,
                          icon: Icons.mic,
                          onPressed: _listen,
                        ),
                      ],
                      if (phone) ...[
                        const SizedBox(width: 12),
                        KButton(
                          key: const ValueKey('search-phone'),
                          label: l.searchFromPhone,
                          icon: Icons.smartphone,
                          onPressed: _fromPhone,
                        ),
                      ],
                    ],
                  ),
          ),
        ),
        ListenableBuilder(
          listenable: Listenable.merge([search, _history!]),
          builder: (context, _) => _body(context, search),
        ),
      ],
    );
  }

  Widget _body(BuildContext context, UnifiedSearch search) {
    final l = context.l10n;
    if (_text.text.trim().isEmpty) {
      final history = _history!.value;
      if (history.isEmpty) {
        return SizedBox(
          height: 260,
          child: EmptyState(
            icon: Icons.search,
            title: l.searchPromptTitle,
            message: l.searchPromptMessage,
          ),
        );
      }
      return _HistoryList(
        entries: history,
        onPick: _fromHistory,
        onRemove: _history!.remove,
        onClear: _history!.clear,
      );
    }
    if (search.error != null) {
      return SizedBox(
        height: 260,
        child: EmptyState(icon: Icons.error_outline, title: l.searchFailed),
      );
    }
    if (search.isEmptyResult) {
      return SizedBox(
        height: 260,
        child: EmptyState(
          icon: Icons.search_off,
          title: l.searchNoResults(search.query),
        ),
      );
    }
    final d = KDensity.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        for (final g in search.groups)
          Padding(
            key: ValueKey('search-${g.section.name}'),
            padding: EdgeInsets.only(bottom: d.rowGap),
            child: SearchGroupView(
              hits: g,
              remember: _open,
              onSeeAll: g.more ? () => _seeAll(g.section) : null,
            ),
          ),
      ],
    );
  }
}

/// Whether a group is a list of rows rather than a row of cards.
bool _isList(SearchSection s) =>
    s == SearchSection.tracks ||
    s == SearchSection.personal ||
    s == SearchSection.channels;

Future<void> _openEpisode(BuildContext context, LibraryItem e) async {
  final library = Services.of(context).library;
  final showId = e.showId;
  final show = showId == null ? null : await library.item(showId);
  if (!context.mounted) return;
  openItem(context, e, show: show);
}

/// The card of result [i] of [h] (sections that are rows of cards).
Widget _card(
  BuildContext context,
  SearchHits h,
  int i,
  void Function(VoidCallback open) remember, {
  double? width,
}) {
  final d = KDensity.of(context);
  final l = context.l10n;
  final item = h.items[i];
  final key = ValueKey('search-${h.section.name}-$i');
  switch (h.section) {
    case SearchSection.movies || SearchSection.shows:
      final it = item as LibraryItem;
      return PosterCard(
        key: ValueKey('search-${it.id}'),
        item: it,
        width: width ?? d.posterWidth,
        onTap: () => remember(() => openItem(context, it)),
      );
    case SearchSection.episodes:
      final e = item as LibraryItem;
      return LandscapeCard(
        key: ValueKey('search-${e.id}'),
        width: width ?? d.landscapeWidth,
        seed: e.id,
        imageUrl: e.backdropUrl ?? e.posterUrl,
        title: e.title,
        subtitle: episodeCode(l, e),
        onTap: () => remember(() => _openEpisode(context, e)),
      );
    case SearchSection.artists:
      final a = item as MusicArtist;
      return ArtistCard(
        key: key,
        width: width ?? d.albumWidth,
        artist: a,
        onTap: () => remember(() => openArtist(context, h.music!, a)),
      );
    case SearchSection.albums:
      final a = item as MusicAlbum;
      return AlbumCard(
        key: key,
        width: width ?? d.albumWidth,
        seed: a.artSeed,
        artUrl: a.artUrl,
        title: albumTitle(l, a),
        subtitle: albumArtist(l, a),
        onTap: () => remember(() => openAlbum(context, h.music!, a)),
      );
    case SearchSection.audiobooks:
      final b = item as Audiobook;
      return BookCard(
        key: key,
        book: b,
        width: width ?? d.albumWidth,
        onTap: () => remember(() => openAudiobook(context, b)),
      );
    case SearchSection.tracks ||
        SearchSection.personal ||
        SearchSection.channels:
      return _tile(context, h, i, remember);
  }
}

/// The row of result [i] of [h] (sections that are lists).
Widget _tile(
  BuildContext context,
  SearchHits h,
  int i,
  void Function(VoidCallback open) remember,
) {
  final item = h.items[i];
  switch (h.section) {
    case SearchSection.tracks:
      final tracks = h.items.cast<LibraryItem>();
      return TrackTile(
        key: ValueKey('search-${tracks[i].id}'),
        track: tracks[i],
        showArt: true,
        onTap: () => remember(() => playTracks(context, tracks, i)),
      );
    case SearchSection.personal:
      final hit = item as PersonalHit;
      return _PersonalTile(
        key: ValueKey('search-personal-${hit.library.id}-${hit.video.fileId}'),
        hit: hit,
        onTap: () => remember(
          () => Navigator.of(context).push(
            MaterialPageRoute<void>(
              builder: (_) => PersonalLibraryScreen(library: hit.library),
            ),
          ),
        ),
      );
    case SearchSection.channels:
      final c = item as TwitchChannel;
      return ChannelTile(
        key: ValueKey('search-channel-${c.login}'),
        channel: c,
      );
    case _:
      return _card(context, h, i, remember);
  }
}

/// Height of a row of cards of [s].
double _rowHeight(KDensity d, SearchSection s) {
  final extra = d.isTv ? 12.0 : 6.0;
  return switch (s) {
    SearchSection.movies ||
    SearchSection.shows => d.posterWidth * 1.5 + cardCaptionHeight(d) + extra,
    SearchSection.episodes =>
      d.landscapeWidth * 9 / 16 + cardCaptionHeight(d) + extra,
    _ => albumRowHeight(d),
  };
}

/// Width of a card of [s].
double _cardWidth(KDensity d, SearchSection s) => switch (s) {
  SearchSection.movies || SearchSection.shows => d.posterWidth,
  SearchSection.episodes => d.landscapeWidth,
  _ => d.albumWidth,
};

/// One group of results: a row of cards, or a titled list (tracks,
/// personal media, channels). [onSeeAll] when the group has more.
class SearchGroupView extends StatelessWidget {
  const SearchGroupView({
    super.key,
    required this.hits,
    required this.remember,
    this.onSeeAll,
  });

  final SearchHits hits;

  /// Wraps the opening of a result (remembers the query).
  final void Function(VoidCallback open) remember;
  final VoidCallback? onSeeAll;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final title = searchSectionTitle(l, hits.section);
    if (!_isList(hits.section)) {
      return MediaRow(
        title: title,
        itemCount: hits.items.length,
        height: _rowHeight(d, hits.section),
        onSeeAll: onSeeAll,
        seeAllTileWidth: _cardWidth(d, hits.section),
        itemBuilder: (context, i) => _card(context, hits, i, remember),
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: EdgeInsets.fromLTRB(
            d.pageInsets.left,
            0,
            d.pageInsets.right,
            8,
          ),
          child: Row(
            children: [
              Expanded(
                child: Text(
                  title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: Theme.of(context).textTheme.titleLarge,
                ),
              ),
              if (onSeeAll != null && !d.isTv)
                TextButton(onPressed: onSeeAll, child: Text(l.commonSeeAll)),
            ],
          ),
        ),
        FocusTraversalGroup(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              for (var i = 0; i < hits.items.length; i++)
                hits.section == SearchSection.tracks
                    ? Padding(
                        padding: EdgeInsets.symmetric(
                          horizontal: d.pageInsets.left,
                        ),
                        child: _tile(context, hits, i, remember),
                      )
                    : _tile(context, hits, i, remember),
              // On TV the header's «Все» is not shown: a row ends the list.
              if (onSeeAll != null && d.isTv)
                Padding(
                  padding: EdgeInsets.fromLTRB(
                    d.pageInsets.left,
                    8,
                    d.pageInsets.right,
                    0,
                  ),
                  child: KButton(
                    key: ValueKey('search-see-all-${hits.section.name}'),
                    label: l.commonSeeAll,
                    icon: Icons.chevron_right,
                    onPressed: onSeeAll,
                  ),
                ),
            ],
          ),
        ),
      ],
    );
  }
}

/// A video of a personal library in the results.
class _PersonalTile extends StatelessWidget {
  const _PersonalTile({super.key, required this.hit, required this.onTap});

  final PersonalHit hit;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    return Padding(
      padding: EdgeInsets.symmetric(
        horizontal: d.pageInsets.left - 8,
        vertical: 2,
      ),
      child: KFocusable(
        onActivate: onTap,
        light: false,
        radius: 12,
        semanticLabel: hit.video.title,
        builder: (context, focused) => Padding(
          padding: const EdgeInsets.all(8),
          child: Row(
            children: [
              Icon(
                Icons.video_library_outlined,
                size: d.isTv ? 32 : 28,
                color: KColors.textTertiary,
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      hit.video.title,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.titleSmall,
                    ),
                    Text(
                      context.l10n.searchPersonalIn(hit.library.name),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.labelSmall,
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// «Все» of one group: every result of the query in that group.
class SearchAllScreen extends StatefulWidget {
  const SearchAllScreen({
    super.key,
    required this.section,
    required this.query,
    required this.load,
  });

  final SearchSection section;
  final String query;
  final Future<SearchHits?> Function() load;

  @override
  State<SearchAllScreen> createState() => _SearchAllScreenState();
}

class _SearchAllScreenState extends State<SearchAllScreen> {
  late final Future<SearchHits?> _hits = widget.load();

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final title = l.searchAllTitle(
      searchSectionTitle(l, widget.section),
      widget.query,
    );
    void remember(VoidCallback open) => open();
    return Scaffold(
      key: ValueKey('search-all-${widget.section.name}'),
      backgroundColor: KColors.bgBase,
      appBar: AppBar(
        backgroundColor: KColors.bgBase,
        title: Text(
          title,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: Theme.of(context).textTheme.titleLarge,
        ),
      ),
      body: FutureBuilder<SearchHits?>(
        future: _hits,
        builder: (context, snap) {
          final hits = snap.data;
          final Widget body;
          if (snap.connectionState != ConnectionState.done) {
            body = const SizedBox(
              height: 200,
              child: Center(child: CircularProgressIndicator()),
            );
          } else if (hits == null || hits.items.isEmpty) {
            body = SizedBox(
              height: 260,
              child: EmptyState(
                icon: Icons.search_off,
                title: l.searchNoResults(widget.query),
              ),
            );
          } else if (_isList(widget.section)) {
            body = SearchGroupView(hits: hits, remember: remember);
          } else {
            body = Padding(
              padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
              child: FocusTraversalGroup(
                child: Wrap(
                  spacing: d.gap,
                  runSpacing: 18,
                  children: [
                    for (var i = 0; i < hits.items.length; i++)
                      _card(context, hits, i, remember),
                  ],
                ),
              ),
            );
          }
          return ListView(
            clipBehavior: Clip.none,
            padding: EdgeInsets.only(top: 8, bottom: d.pageInsets.bottom + 90),
            children: [body],
          );
        },
      ),
    );
  }
}

/// Recent searches under the empty field.
class _HistoryList extends StatelessWidget {
  const _HistoryList({
    required this.entries,
    required this.onPick,
    required this.onRemove,
    required this.onClear,
  });

  final List<String> entries;
  final ValueChanged<String> onPick;
  final ValueChanged<String> onRemove;
  final VoidCallback onClear;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    return Padding(
      padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 640),
        child: FocusTraversalGroup(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  Expanded(
                    child: Text(
                      l.searchHistoryTitle,
                      style: theme.textTheme.titleLarge,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  TextButton(
                    onPressed: onClear,
                    child: Text(l.searchHistoryClear),
                  ),
                ],
              ),
              const SizedBox(height: 4),
              for (final q in entries)
                Row(
                  key: ValueKey('search-history-$q'),
                  children: [
                    Expanded(
                      child: KFocusable(
                        onActivate: () => onPick(q),
                        focusScale: 1.0,
                        light: false,
                        semanticLabel: q,
                        builder: (context, focused) => AnimatedContainer(
                          duration: KMotion.focus,
                          padding: const EdgeInsets.symmetric(
                            horizontal: 10,
                            vertical: 10,
                          ),
                          color: focused
                              ? Colors.white.withValues(alpha: .09)
                              : Colors.transparent,
                          child: Row(
                            children: [
                              Icon(
                                Icons.history,
                                size: 20,
                                color: KColors.textTertiary,
                              ),
                              const SizedBox(width: 12),
                              Expanded(
                                child: Text(
                                  q,
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  style: theme.textTheme.bodyLarge,
                                ),
                              ),
                            ],
                          ),
                        ),
                      ),
                    ),
                    IconButton(
                      tooltip: l.searchHistoryRemove(q),
                      onPressed: () => onRemove(q),
                      icon: const Icon(Icons.close, size: 18),
                    ),
                  ],
                ),
            ],
          ),
        ),
      ),
    );
  }
}
