// SPDX-License-Identifier: MPL-2.0
//
// The file browser (UI-006, 01 §2.1): the second path to content. A source
// → its folders → a file → the same Detail page as from the library
// (movie, show at the episode's season) or, for a file the library has not
// recognized, straight to the player. Sort and "folders first" are saved
// per source (`folder:<id>` view, LIB-009); "file names" shows the names
// on disk instead of the recognized titles. Breadcrumbs jump up; Back (the
// remote's too) and Backspace go up one folder before leaving the screen.
// UI-034 (ADR-0099): a folder — or, at the root, the whole source — moves
// to «Личное» and back ("Перенести в Личное" / "Вернуть в медиатеку");
// moved before the first scan, its files are never recognized.
// LIB-025: a folder says what the core found in it («Мои видео»,
// «Аудиокниги»…) and has «Это не то»: one tap gives its files the other
// kind of their medium, and the folder remembers it; «Вернуть» undoes it.
// UI-035: a source that can be written gets «Файлы» — the file view of
// the current folder (upload, folders, move, trash).

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../files/files_screen.dart' show openFiles;
import '../integration/library.dart';
import '../integration/player.dart';
import '../l10n/l10n.dart';
import '../library/library_queries.dart';
import '../personal/personal_screen.dart'
    show moveFolderToPersonal, returnFolderToLibrary;
import '../personal/source_content.dart' show PersonalPlaces;
import '../services/app_services.dart';
import '../services/preferences.dart';
import '../sources/source_browser.dart';
import '../sources/source_manager.dart';
import '../sources/source_text.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import '../widgets/format.dart';
import 'detail_sections.dart' show formatSize;
import 'library_grid_screen.dart' show FilterPill;
import 'own_videos.dart' show contentKindsOf;
import 'playback.dart' show playTracks, showUnplayable;
import 'routes.dart';

/// View key of the sort and "folders first" of [sourceId] (LIB-009).
String browserView(String sourceId) => 'folder:$sourceId';

/// Sort fields of the browser's menu, with the direction picked first.
const _sortDefaults = <SortField, bool>{
  SortField.title: false,
  SortField.added: true,
  SortField.duration: false,
};

/// Opens the file browser of [source] in the current navigator.
Future<void> openFileBrowser(BuildContext context, ManagedSource source) =>
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => FileBrowserScreen(source: source),
      ),
    );

class FileBrowserScreen extends StatefulWidget {
  const FileBrowserScreen({super.key, required this.source, this.dir = ''});

  final ManagedSource source;

  /// Folder to start in, relative to the source root.
  final String dir;

  @override
  State<FileBrowserScreen> createState() => _FileBrowserScreenState();
}

class _FileBrowserScreenState extends State<FileBrowserScreen> {
  late AppServices _services;
  late SourceBrowser _browser;
  bool _started = false;

  late String _dir = widget.dir;
  LibrarySort _sort = const LibrarySort();
  bool _foldersFirst = true;
  bool _fileNames = false;

  BrowseListing? _listing;
  Object? _error;
  int _generation = 0;
  final Map<String, LibraryItem> _items = {};
  final Map<String, LibraryItem> _shows = {};

  final _focus = FocusNode(debugLabel: 'file-browser');

  /// LIB-025: what the subfolders of [_dir] hold, by path.
  Map<String, FolderContent> _kinds = const {};

  /// UI-034: which folders of this source are personal media.
  PersonalPlaces _places = const PersonalPlaces();
  StreamSubscription<void>? _personalSub;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _services = Services.of(context);
    if (_started) return;
    _started = true;
    _browser = SourceBrowser.of(_services.library, _services.sourceManager);
    unawaited(_loadPrefs());
    _personalSub = _services.personal?.changes.listen((_) => _readPlaces());
    unawaited(_readPlaces());
    unawaited(_readWritable());
  }

  /// UI-035: whether the source can be written (the «Файлы» pill).
  bool _writable = false;

  Future<void> _readWritable() async {
    final id = int.tryParse(widget.source.id);
    final files = _services.filesControl;
    if (id == null || files == null) return;
    final w = await files.writable(id);
    if (mounted && w != _writable) setState(() => _writable = w);
  }

  @override
  void dispose() {
    _personalSub?.cancel();
    _focus.dispose();
    super.dispose();
  }

  Future<void> _readPlaces() async {
    final personal = _services.personal;
    if (personal == null) return;
    try {
      final places = await PersonalPlaces.read(personal, widget.source.id);
      if (mounted) setState(() => _places = places);
    } catch (e) {
      debugPrint('Personal folders of ${widget.source.id} not read: $e');
    }
  }

  /// Personal media works on the core's sources (numeric ids).
  bool get _canMovePlaces =>
      _services.personal != null && int.tryParse(widget.source.id) != null;

  Future<void> _toPersonal(String folder) async {
    if (await moveFolderToPersonal(context, widget.source.id, folder)) {
      await _readPlaces();
    }
  }

  Future<void> _backToLibrary(String folder) async {
    await returnFolderToLibrary(
      context,
      widget.source.id,
      folder,
      libraryId: _places.libraryOf(folder),
    );
    await _readPlaces();
  }

  Future<void> _loadPrefs() async {
    final view = browserView(widget.source.id);
    ViewPrefs prefs;
    try {
      prefs = await _services.queries.viewPrefs(view);
    } catch (_) {
      prefs = ViewPrefs(view);
    }
    final names = await _services.preferences.read(
      Preferences.browserShowFileNames,
    );
    if (!mounted) return;
    setState(() {
      _sort = _sortDefaults.containsKey(prefs.sort.field)
          ? prefs.sort
          : const LibrarySort();
      _foldersFirst = prefs.foldersFirst;
      _fileNames = names == 'true';
    });
    await _load();
  }

  Future<void> _savePrefs() async {
    try {
      await _services.queries.setViewPrefs(
        ViewPrefs(
          browserView(widget.source.id),
          sort: _sort,
          foldersFirst: _foldersFirst,
        ),
      );
    } catch (e) {
      debugPrint('File browser prefs not saved: $e');
    }
  }

  Future<void> _load() async {
    final generation = ++_generation;
    setState(() {
      _listing = null;
      _error = null;
    });
    try {
      final listing = await _browser.list(widget.source, _dir);
      await _resolveItems(listing.entries);
      final kinds = await _readKinds();
      if (!mounted || generation != _generation) return;
      setState(() {
        _listing = listing;
        _kinds = kinds;
      });
      // The row that had focus may be gone with the old folder: keep keys
      // (Backspace) working. On TV the new first row autofocuses.
      if (!_focus.hasFocus && !KDensity.of(context).isTv) {
        _focus.requestFocus();
      }
    } catch (e) {
      if (!mounted || generation != _generation) return;
      setState(() => _error = e);
    }
  }

  /// LIB-025: what the subfolders hold; nothing without the core.
  Future<Map<String, FolderContent>> _readKinds() async {
    final kinds = contentKindsOf(context);
    final id = int.tryParse(widget.source.id);
    if (kinds == null || id == null) return const {};
    try {
      return await kinds.folderKinds(id, _dir);
    } catch (e) {
      debugPrint('Folder kinds of ${widget.source.id} not read: $e');
      return const {};
    }
  }

  /// «Это не то» on [folder]; the snackbar offers to undo it.
  Future<void> _notThis(String folder, {bool undo = false}) async {
    final kinds = contentKindsOf(context);
    final id = int.tryParse(widget.source.id);
    if (kinds == null || id == null) return;
    final l = context.l10n;
    final messenger = ScaffoldMessenger.maybeOf(context);
    final kind = await kinds.notThis(id, folder);
    final read = await _readKinds();
    if (!mounted) return;
    setState(() => _kinds = read);
    if (kind == null || undo) return;
    messenger?.hideCurrentSnackBar();
    messenger?.showSnackBar(
      SnackBar(
        content: Text(
          l.browserNotThisDone(contentKindLabel(l, kind)),
          style: Theme.of(context).textTheme.bodyMedium,
        ),
        action: SnackBarAction(
          label: l.browserNotThisUndo,
          onPressed: () => unawaited(_notThis(folder, undo: true)),
        ),
      ),
    );
  }

  /// Library items of the recognized files, and the shows of episodes.
  Future<void> _resolveItems(List<BrowseEntry> entries) async {
    final library = _services.library;
    final ids = {
      for (final e in entries)
        if (e.itemId case final id? when !_items.containsKey(id)) id,
    };
    for (final item in await Future.wait(ids.map(library.item))) {
      if (item != null) _items[item.id] = item;
    }
    final shows = {
      for (final id in ids)
        if (_items[id]?.showId case final s? when !_shows.containsKey(s)) s,
    };
    for (final show in await Future.wait(shows.map(library.item))) {
      if (show != null) _shows[show.id] = show;
    }
  }

  void _go(String dir) {
    if (dir == _dir) return;
    setState(() => _dir = dir);
    unawaited(_load());
  }

  /// Up one folder; false at the root.
  bool _up() {
    if (_dir.isEmpty) return false;
    final i = _dir.lastIndexOf('/');
    _go(i < 0 ? '' : _dir.substring(0, i));
    return true;
  }

  void _setSort(LibrarySort s) {
    setState(() => _sort = s);
    unawaited(_savePrefs());
  }

  void _setFoldersFirst(bool v) {
    setState(() => _foldersFirst = v);
    unawaited(_savePrefs());
  }

  void _setFileNames(bool v) {
    setState(() => _fileNames = v);
    unawaited(
      _services.preferences.write(
        Preferences.browserShowFileNames,
        v ? 'true' : null,
      ),
    );
  }

  LibraryItem? _itemOf(BrowseEntry e) => _items[e.itemId];

  /// What the row says: the recognized title, or the file name.
  String _label(AppLocalizations l, BrowseEntry e) {
    final item = _itemOf(e);
    if (_fileNames || item == null) return e.name;
    return switch (item.kind) {
      ItemKind.movie when item.year != null => '${item.title} (${item.year})',
      ItemKind.episode => [
        ?_shows[item.showId]?.title,
        episodeCode(l, item),
        item.title,
      ].join(' · '),
      _ => item.title,
    };
  }

  Future<void> _open(BrowseEntry e) async {
    if (e.isFolder) {
      _go(e.relPath);
      return;
    }
    // The same pages as from the library: the movie, or the episode's show
    // at its season (routes.dart).
    final item = _itemOf(e);
    if (item != null) {
      switch (item.kind) {
        case ItemKind.track:
          playTracks(context, [item], 0);
        case ItemKind.episode:
          openItem(context, item, show: _shows[item.showId]);
        case _:
          openItem(context, item);
      }
      return;
    }
    if (e.isDiscImage) return showUnplayable(context, Unplayable.discImage);
    final uri = _browser.fileUri(widget.source, e.relPath);
    if (uri == null) {
      ScaffoldMessenger.maybeOf(
        context,
      )?.showSnackBar(SnackBar(content: Text(context.l10n.browserCannotPlay)));
      return;
    }
    final request = PlayRequest(
      uri: uri,
      title: e.name,
      durationMs: e.durationMs,
    );
    if (e.mediaType == MediaType.audio) {
      _services.music.play([request]);
      return;
    }
    _services.music.pause();
    await Navigator.of(context, rootNavigator: true).push(
      MaterialPageRoute<void>(
        fullscreenDialog: true,
        builder: (_) => PlayerScreen(
          request: request,
          // A file outside the library has no progress to keep.
          onProgress: (_, _) {},
          engineFactory: _services.playerEngineFactory,
          subtitleSettings: _services.subtitleSettings,
          subtitleLanguages: _services.subtitleLanguages,
          seekStep: Duration(seconds: _services.skipSeconds.value),
        ),
      ),
    );
  }

  KeyEventResult _onKey(FocusNode _, KeyEvent event) {
    if (event is! KeyDownEvent) return KeyEventResult.ignored;
    if (event.logicalKey == LogicalKeyboardKey.backspace && _up()) {
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final listing = _listing;
    final entries = listing == null
        ? const <BrowseEntry>[]
        : sortBrowseEntries(
            listing.entries,
            sort: _sort,
            foldersFirst: _foldersFirst,
            labelOf: (e) => _label(l, e),
          );
    final liveError = listing?.liveError;
    final error = _error;
    return PopScope(
      canPop: _dir.isEmpty,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) _up();
      },
      child: Focus(
        focusNode: _focus,
        // TV: the first row takes focus instead (it autofocuses).
        autofocus: !d.isTv,
        onKeyEvent: _onKey,
        child: Scaffold(
          body: SafeArea(
            child: CustomScrollView(
              key: const ValueKey('browser-scroll'),
              slivers: [
                SliverToBoxAdapter(
                  child: _Header(
                    source: widget.source,
                    dir: _dir,
                    onGo: _go,
                    onBack: () => Navigator.of(context).maybePop(),
                  ),
                ),
                SliverToBoxAdapter(
                  child: Padding(
                    padding: EdgeInsets.fromLTRB(
                      d.pageInsets.left,
                      0,
                      d.pageInsets.right,
                      12,
                    ),
                    child: _Toolbar(
                      sort: _sort,
                      onSort: _setSort,
                      foldersFirst: _foldersFirst,
                      onFoldersFirst: _setFoldersFirst,
                      fileNames: _fileNames,
                      onFileNames: _setFileNames,
                      extra: [
                        if (_writable)
                          FilterPill(
                            key: const ValueKey('browser-files'),
                            icon: Icons.drive_folder_upload_outlined,
                            label: l.filesTitle,
                            selected: false,
                            onTap: () => openFiles(
                              context,
                              sourceId: int.parse(widget.source.id),
                              sourceName: widget.source.name,
                              dir: _dir,
                            ),
                          ),
                        // UI-034: the whole source, at its root.
                        if (_canMovePlaces && _dir.isEmpty)
                          _places.wholeSource
                              ? FilterPill(
                                  key: const ValueKey('browser-source-back'),
                                  icon: Icons.undo_rounded,
                                  label: l.browserSourceBackToLibrary,
                                  selected: false,
                                  onTap: () => _backToLibrary(''),
                                )
                              : FilterPill(
                                  key: const ValueKey(
                                    'browser-source-to-personal',
                                  ),
                                  icon: Icons.lock_outline,
                                  label: l.browserSourceToPersonal,
                                  selected: false,
                                  onTap: () => _toPersonal(''),
                                ),
                      ],
                    ),
                  ),
                ),
                if (liveError != null)
                  SliverToBoxAdapter(
                    child: _Notice(
                      key: const ValueKey('browser-offline'),
                      text: l.browserOffline,
                      detail: sourceErrorText(l, liveError.code),
                    ),
                  ),
                if (error != null)
                  SliverToBoxAdapter(
                    child: _Notice(
                      key: const ValueKey('browser-error'),
                      text: l.browserError,
                      detail: error is SourceException
                          ? sourceErrorText(l, error.code)
                          : '$error',
                    ),
                  )
                else if (listing == null)
                  const SliverToBoxAdapter(
                    child: Padding(
                      padding: EdgeInsets.all(32),
                      child: Center(child: CircularProgressIndicator()),
                    ),
                  )
                else if (entries.isEmpty)
                  SliverToBoxAdapter(
                    child: Padding(
                      padding: EdgeInsets.symmetric(
                        horizontal: d.pageInsets.left,
                        vertical: 16,
                      ),
                      child: Text(
                        l.browserEmpty,
                        key: const ValueKey('browser-empty'),
                        style: Theme.of(context).textTheme.bodyMedium,
                      ),
                    ),
                  )
                else
                  SliverPadding(
                    padding: EdgeInsets.fromLTRB(
                      d.pageInsets.left,
                      0,
                      d.pageInsets.right,
                      d.pageInsets.bottom + 90,
                    ),
                    sliver: SliverList.builder(
                      itemCount: entries.length,
                      itemBuilder: (context, i) {
                        final e = entries[i];
                        final personal =
                            e.isFolder && _places.libraryOf(e.relPath) != null;
                        final kind = e.isFolder ? _kinds[e.relPath] : null;
                        return _EntryRow(
                          key: ValueKey('browser-entry-${e.relPath}'),
                          entry: e,
                          kind: kind,
                          onNotThis: kind == null || personal
                              ? null
                              : () => _notThis(e.relPath),
                          item: _itemOf(e),
                          label: _label(l, e),
                          fileNames: _fileNames,
                          autofocus: i == 0 && d.isTv,
                          onTap: () => _open(e),
                          personal: personal,
                          // UI-034: a folder's own action; inside a
                          // personal source every folder is already there.
                          onPlaceAction:
                              e.isFolder &&
                                  _canMovePlaces &&
                                  !_places.wholeSource
                              ? () => personal
                                    ? _backToLibrary(e.relPath)
                                    : _toPersonal(e.relPath)
                              : null,
                        );
                      },
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

/// Back button, the source name and the breadcrumbs of [dir].
class _Header extends StatelessWidget {
  const _Header({
    required this.source,
    required this.dir,
    required this.onGo,
    required this.onBack,
  });

  final ManagedSource source;
  final String dir;
  final ValueChanged<String> onGo;
  final VoidCallback onBack;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final parts = dir.isEmpty ? const <String>[] : dir.split('/');
    final crumbs = <(String, String)>[
      (source.name, ''),
      for (var i = 0; i < parts.length; i++)
        (parts[i], parts.take(i + 1).join('/')),
    ];
    return Padding(
      padding: EdgeInsets.fromLTRB(
        d.pageInsets.left,
        d.isTv ? d.safeTop(context) : 8,
        d.pageInsets.right,
        d.isTv ? 12 : 16,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (!d.isTv)
            Padding(
              padding: const EdgeInsets.only(left: 0, bottom: 4),
              child: IconButton(
                key: const ValueKey('browser-back'),
                tooltip: dir.isEmpty ? l.commonBack : l.browserUp,
                onPressed: onBack,
                icon: const Icon(Icons.arrow_back),
              ),
            ),
          Text(
            crumbs.last.$1,
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: theme.textTheme.headlineLarge,
          ),
          const SizedBox(height: 8),
          Semantics(
            label: l.browserPath,
            container: true,
            child: FocusTraversalGroup(
              child: Wrap(
                key: const ValueKey('browser-breadcrumbs'),
                crossAxisAlignment: WrapCrossAlignment.center,
                spacing: 2,
                runSpacing: 4,
                children: [
                  for (final (i, (name, path)) in crumbs.indexed) ...[
                    if (i > 0)
                      const Icon(
                        Icons.chevron_right,
                        size: 18,
                        color: Colors.white54,
                      ),
                    _Crumb(
                      key: ValueKey('browser-crumb-$path'),
                      label: name,
                      icon: i == 0 ? sourceTypeIcon(source.type) : null,
                      current: i == crumbs.length - 1,
                      onTap: () => onGo(path),
                    ),
                  ],
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _Crumb extends StatelessWidget {
  const _Crumb({
    super.key,
    required this.label,
    required this.current,
    required this.onTap,
    this.icon,
  });

  final String label;
  final bool current;
  final VoidCallback onTap;
  final IconData? icon;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return KFocusable(
      onActivate: current ? null : onTap,
      radius: 8,
      light: false,
      semanticLabel: label,
      builder: (context, focused) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
        constraints: const BoxConstraints(maxWidth: 260),
        decoration: BoxDecoration(
          color: focused ? Colors.white : null,
          borderRadius: BorderRadius.circular(8),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (icon != null) ...[
              Icon(
                icon,
                size: 16,
                color: focused ? Colors.black : KColors.textSecondary,
              ),
              const SizedBox(width: 6),
            ],
            Flexible(
              child: Text(
                label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: theme.textTheme.labelLarge?.copyWith(
                  color: focused
                      ? Colors.black
                      : current
                      ? Colors.white
                      : KColors.textSecondary,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _Toolbar extends StatelessWidget {
  const _Toolbar({
    required this.sort,
    required this.onSort,
    required this.foldersFirst,
    required this.onFoldersFirst,
    required this.fileNames,
    required this.onFileNames,
    this.extra = const [],
  });

  /// More pills after the view options (UI-034: the whole source to
  /// «Личное» and back).
  final List<Widget> extra;

  final LibrarySort sort;
  final ValueChanged<LibrarySort> onSort;
  final bool foldersFirst;
  final ValueChanged<bool> onFoldersFirst;
  final bool fileNames;
  final ValueChanged<bool> onFileNames;

  static String sortLabel(AppLocalizations l, SortField f) => switch (f) {
    SortField.added => l.browserSortDate,
    SortField.duration => l.browserSortDuration,
    _ => l.browserSortName,
  };

  Future<void> _openSort(BuildContext context) async {
    final l = context.l10n;
    final box = context.findRenderObject()! as RenderBox;
    final overlay =
        Overlay.of(context).context.findRenderObject()! as RenderBox;
    final rect = Rect.fromPoints(
      box.localToGlobal(box.size.bottomLeft(Offset.zero), ancestor: overlay),
      box.localToGlobal(box.size.bottomRight(Offset.zero), ancestor: overlay),
    );
    final field = await showMenu<SortField>(
      context: context,
      position: RelativeRect.fromRect(rect, Offset.zero & overlay.size),
      items: [
        for (final f in _sortDefaults.keys)
          PopupMenuItem(
            key: ValueKey('browser-sort-${f.name}'),
            value: f,
            child: Row(
              children: [
                SizedBox(
                  width: 28,
                  child: f == sort.field
                      ? Icon(
                          sort.descending
                              ? Icons.arrow_downward
                              : Icons.arrow_upward,
                          size: 18,
                        )
                      : null,
                ),
                Text(sortLabel(l, f)),
              ],
            ),
          ),
      ],
    );
    if (field == null) return;
    onSort(
      field == sort.field
          ? LibrarySort(field: field, descending: !sort.descending)
          : LibrarySort(field: field, descending: _sortDefaults[field]!),
    );
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return FocusTraversalGroup(
      child: Wrap(
        spacing: 8,
        runSpacing: 8,
        children: [
          Builder(
            builder: (context) => FilterPill(
              key: const ValueKey('browser-sort'),
              icon: sort.descending ? Icons.arrow_downward : Icons.arrow_upward,
              label: l.librarySort(sortLabel(l, sort.field)),
              semanticLabel: l.librarySort(
                '${sortLabel(l, sort.field)}, '
                '${sort.descending ? l.librarySortDescending : l.librarySortAscending}',
              ),
              selected: true,
              onTap: () => _openSort(context),
            ),
          ),
          FilterPill(
            key: const ValueKey('browser-folders-first'),
            icon: foldersFirst ? Icons.check : Icons.folder_outlined,
            label: l.browserFoldersFirst,
            selected: foldersFirst,
            onTap: () => onFoldersFirst(!foldersFirst),
          ),
          FilterPill(
            key: const ValueKey('browser-file-names'),
            icon: fileNames ? Icons.check : Icons.description_outlined,
            label: l.browserShowFileNames,
            selected: fileNames,
            onTap: () => onFileNames(!fileNames),
          ),
          ...extra,
        ],
      ),
    );
  }
}

class _Notice extends StatelessWidget {
  const _Notice({super.key, required this.text, required this.detail});

  final String text;
  final String detail;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    return Container(
      margin: EdgeInsets.fromLTRB(d.pageInsets.left, 0, d.pageInsets.right, 12),
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: KColors.bgElevated,
        borderRadius: BorderRadius.circular(12),
        border: Border.all(color: KColors.stroke),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Icon(Icons.cloud_off_outlined, color: Colors.white70),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(text, style: theme.textTheme.bodyMedium),
                const SizedBox(height: 4),
                Text(detail, style: theme.textTheme.bodySmall),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// The name of a content kind in the browser and its snackbars.
String contentKindLabel(AppLocalizations l, ContentKind k) => switch (k) {
  ContentKind.video => l.browserKindVideo,
  ContentKind.ownVideo => l.browserKindOwnVideo,
  ContentKind.music => l.browserKindMusic,
  ContentKind.audiobook => l.browserKindAudiobook,
};

class _EntryRow extends StatelessWidget {
  const _EntryRow({
    super.key,
    required this.entry,
    required this.item,
    required this.label,
    required this.fileNames,
    required this.onTap,
    this.autofocus = false,
    this.personal = false,
    this.onPlaceAction,
    this.kind,
    this.onNotThis,
  });

  final BrowseEntry entry;

  /// LIB-025: what a folder holds, as the core decided.
  final FolderContent? kind;

  /// «Это не то» on the folder.
  final VoidCallback? onNotThis;
  final LibraryItem? item;
  final String label;
  final bool fileNames;
  final VoidCallback onTap;
  final bool autofocus;

  /// UI-034: a folder of an open personal library.
  final bool personal;

  /// «Перенести в Личное» (or «Вернуть в медиатеку» when [personal]).
  final VoidCallback? onPlaceAction;

  IconData get _icon {
    final e = entry;
    if (e.isFolder) {
      return personal ? Icons.folder_special_rounded : Icons.folder_rounded;
    }
    if (e.isDiscImage) return Icons.album_outlined;
    return switch (item?.kind) {
      ItemKind.movie => Icons.movie_outlined,
      ItemKind.episode || ItemKind.show => Icons.tv_outlined,
      ItemKind.track => Icons.music_note_outlined,
      null =>
        e.mediaType == MediaType.audio
            ? Icons.audio_file_outlined
            : Icons.video_file_outlined,
    };
  }

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    final locale = Localizations.localeOf(context);
    final e = entry;
    final i = item;
    final meta = <String>[
      // The other name: the file's when titles show, the title's when names
      // show.
      if (i != null && !e.isFolder) fileNames ? i.title : e.name,
      if (personal) l.browserInPersonal,
      // Movies and music are what a folder usually holds: only the other
      // kinds, and the user's own choice, are named.
      if (kind case final k? when !personal)
        if (k.overridden)
          l.browserKindChanged(contentKindLabel(l, k.kind))
        else if (k.kind == ContentKind.ownVideo ||
            k.kind == ContentKind.audiobook)
          contentKindLabel(l, k.kind),
      if (e.isFolder && e.fileCount != null) l.browserFileCount(e.fileCount!),
      if (!e.isFolder) formatRuntime(l, e.durationMs),
      if (!e.isFolder && (e.sizeBytes ?? 0) > 0)
        formatSize(l, locale, e.sizeBytes!),
      if (!e.isFolder && i == null) l.browserNotRecognized,
    ].where((s) => s.isNotEmpty).join('  ·  ');
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 2),
      child: KFocusable(
        autofocus: autofocus,
        onActivate: onTap,
        focusScale: 1.02,
        light: false,
        radius: 10,
        semanticLabel: label,
        builder: (context, focused) => AnimatedContainer(
          duration: KMotion.focus,
          padding: EdgeInsets.symmetric(
            horizontal: 12,
            vertical: d.isTv ? 10 : 8,
          ),
          decoration: BoxDecoration(
            color: focused
                ? Colors.white.withValues(alpha: .14)
                : Colors.white.withValues(alpha: .03),
            borderRadius: BorderRadius.circular(10),
          ),
          child: Row(
            children: [
              Icon(
                _icon,
                color: e.isFolder ? KColors.accent : KColors.textSecondary,
                size: d.isTv ? 30 : 26,
              ),
              const SizedBox(width: 14),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(
                      label,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.titleSmall,
                    ),
                    if (meta.isNotEmpty)
                      Text(
                        meta,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: theme.textTheme.labelSmall,
                      ),
                  ],
                ),
              ),
              if (onNotThis case final action?)
                IconButton(
                  key: ValueKey('browser-folder-not-this-${e.relPath}'),
                  tooltip: l.browserNotThis,
                  icon: const Icon(
                    Icons.swap_horiz_rounded,
                    color: Colors.white70,
                  ),
                  onPressed: action,
                ),
              if (onPlaceAction case final action?)
                IconButton(
                  key: ValueKey(
                    personal
                        ? 'browser-folder-back-${e.relPath}'
                        : 'browser-folder-to-personal-${e.relPath}',
                  ),
                  tooltip: personal
                      ? l.browserFolderBackToLibrary
                      : l.browserFolderToPersonal,
                  icon: Icon(
                    personal ? Icons.undo_rounded : Icons.lock_outline,
                    color: Colors.white70,
                  ),
                  onPressed: action,
                ),
              if (e.isFolder)
                const Icon(Icons.chevron_right, color: Colors.white38),
            ],
          ),
        ),
      ),
    );
  }
}
