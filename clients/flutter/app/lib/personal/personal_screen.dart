// SPDX-License-Identifier: MPL-2.0
//
// Personal media (UI-025, ADR-0098): the «Личные медиа» section. The
// user's own videos — home videos, ambient loops, anything — in personal
// libraries, each with no lock, a PIN or a password. The core never
// recognizes them and never shows them in the main library; this section
// is the only way in, and a locked library shows its name only.
//
// * The section root lists the libraries; a locked one asks for its PIN
//   (the pad of UI-015) or password before it opens.
// * A library shows its videos with filters by category, tag and stars,
//   a title search and a sort; a video plays, gets stars, tags and
//   categories (LIB-022, the library's own label space), and a video moved
//   from the main library goes back.
// * Folders of any source join a library, picked by browsing the source
//   (UI-034); in the file browser of a source a folder (or the whole
//   source) moves to «Личное» and back (ADR-0099: a place, not a content
//   type to pick in the source forms).
// * A library keeps where each video was left (UI-033): its own
//   «Продолжить» row, never the shared one, never synced.
// * UI-035: «Файлы» opens the library's folder in the personal storage
//   (the file view: folders, move, trash) and «Загрузить» sends files from
//   this device there; they show up in the library unrecognized.
// * Every library locks again when the app goes to the background
//   ([PersonalPrivacy]); the screen is covered while the app is not in
//   front, so the app switcher shows no preview.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../files/personal_files.dart';
import '../files/uploads_panel.dart';
import '../integration/library.dart';
import '../integration/player.dart';
import '../l10n/l10n.dart';
import '../platform/form_factor.dart';
import '../profiles/pin_pad.dart';
import '../profiles/profile_controller.dart' show PinCheck;
import '../screens/playback.dart' show offerMusicResume;
import '../services/app_services.dart';
import '../sources/source_browser.dart';
import '../sources/source_manager.dart' show ManagedSource;
import '../theme/kinolune_theme.dart';
import '../widgets/cards.dart';
import '../widgets/format.dart';
import '../widgets/video_frame_image.dart';

/// The «Личные медиа» section root.
class PersonalScreen extends StatefulWidget {
  const PersonalScreen({super.key});

  @override
  State<PersonalScreen> createState() => _PersonalScreenState();
}

class _PersonalScreenState extends State<PersonalScreen> {
  PersonalMedia? _personal;
  StreamSubscription<void>? _sub;
  StreamSubscription<void>? _filesSub;
  List<PersonalLibrary>? _libraries;
  Object? _error;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final personal = Services.of(context).personal;
    if (!identical(personal, _personal)) {
      _sub?.cancel();
      _personal = personal;
      _sub = personal?.changes.listen((_) => _load());
      _load();
    }
    // UI-035: uploads change the counts of the libraries.
    _filesSub ??= Services.of(context).filesControl?.changes
        .listen((_) => _load());
  }

  @override
  void dispose() {
    _sub?.cancel();
    _filesSub?.cancel();
    super.dispose();
  }

  Future<void> _load() async {
    final personal = _personal;
    if (personal == null) return;
    try {
      final libraries = await personal.libraries();
      if (mounted) setState(() => _libraries = libraries);
    } catch (e) {
      if (mounted) setState(() => _error = e);
    }
  }

  Future<void> _create() async {
    final personal = _personal;
    if (personal == null) return;
    final created = await showCreatePersonalLibraryDialog(context, personal);
    if (created != null && mounted) {
      await openPersonalLibrary(context, created);
    }
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final d = KDensity.of(context);
    final personal = _personal;
    final libraries = _libraries;
    final Widget body;
    if (personal == null) {
      body = EmptyState(
        icon: Icons.lock_outline,
        title: l.personalTitle,
        message: l.personalUnavailable,
      );
    } else if (_error != null) {
      body = EmptyState(
        icon: Icons.error_outline,
        title: l.personalTitle,
        message: '$_error',
      );
    } else if (libraries == null) {
      body = const Center(child: CircularProgressIndicator());
    } else if (libraries.isEmpty) {
      body = EmptyState(
        icon: Icons.video_library_outlined,
        title: l.personalEmptyTitle,
        message: l.personalIntro,
        action: FilledButton.icon(
          key: const ValueKey('personal-create'),
          onPressed: _create,
          icon: const Icon(Icons.add),
          label: Text(l.personalCreate),
        ),
      );
    } else {
      body = ListView(
        key: const ValueKey('personal-libraries'),
        padding: EdgeInsets.fromLTRB(
          d.pageInsets.left,
          0,
          d.pageInsets.right,
          32,
        ),
        children: [
          Text(l.personalIntro, style: Theme.of(context).textTheme.bodySmall),
          const SizedBox(height: 16),
          for (final lib in libraries) ...[
            _LibraryTile(library: lib),
            const SizedBox(height: 10),
          ],
        ],
      );
    }
    return Scaffold(
      key: const ValueKey('personal-page'),
      backgroundColor: KColors.bgBase,
      body: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          PageTitle(
            l.personalTitle,
            trailing: personal == null || (libraries?.isEmpty ?? true)
                ? null
                : TextButton.icon(
                    key: const ValueKey('personal-create'),
                    onPressed: _create,
                    icon: const Icon(Icons.add),
                    label: Text(l.personalCreate),
                  ),
          ),
          Expanded(child: body),
        ],
      ),
    );
  }
}

IconData _lockIcon(PersonalLibrary l) => switch (l.lock) {
  PersonalLock.none => Icons.folder_outlined,
  _ when l.locked => Icons.lock_outline,
  _ => Icons.lock_open_outlined,
};

class _LibraryTile extends StatelessWidget {
  const _LibraryTile({required this.library});

  final PersonalLibrary library;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final count = library.itemCount;
    return Material(
      color: KColors.bgElevated,
      borderRadius: BorderRadius.circular(12),
      clipBehavior: Clip.antiAlias,
      child: ListTile(
        key: ValueKey('personal-library-${library.id}'),
        contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
        leading: Icon(_lockIcon(library), size: 30),
        title: Text(library.name),
        subtitle: Text(
          library.locked || count == null
              ? l.personalLocked
              : l.personalVideoCount(count),
        ),
        trailing: const Icon(Icons.chevron_right),
        onTap: () => openPersonalLibrary(context, library),
      ),
    );
  }
}

/// Unlocks [library] if it is locked (PIN pad or password), then opens it.
Future<void> openPersonalLibrary(
  BuildContext context,
  PersonalLibrary library,
) async {
  final personal = Services.read(context).personal;
  if (personal == null) return;
  if (library.locked && !await unlockPersonalLibrary(context, library)) {
    return;
  }
  if (!context.mounted) return;
  await Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) => PersonalLibraryScreen(library: library),
    ),
  );
}

PinCheck _check(PinResult r) =>
    r.ok ? PinCheck.ok : (r.locked ? PinCheck.locked : PinCheck.wrong);

/// Asks for the PIN or password of [library]; true once it is unlocked.
Future<bool> unlockPersonalLibrary(
  BuildContext context,
  PersonalLibrary library,
) async {
  final personal = Services.read(context).personal;
  if (personal == null) return false;
  final l = context.l10n;
  switch (library.lock) {
    case PersonalLock.none:
      return true;
    case PersonalLock.pin:
      return showPinDialog(
        context,
        title: l.personalUnlockTitle(library.name),
        check: (pin) async => _check(await personal.unlock(library.id, pin)),
      );
    case PersonalLock.password:
      final ok = await showDialog<bool>(
        context: context,
        builder: (_) => _PasswordDialog(library: library, personal: personal),
      );
      return ok ?? false;
  }
}

class _PasswordDialog extends StatefulWidget {
  const _PasswordDialog({required this.library, required this.personal});

  final PersonalLibrary library;
  final PersonalMedia personal;

  @override
  State<_PasswordDialog> createState() => _PasswordDialogState();
}

class _PasswordDialogState extends State<_PasswordDialog> {
  final _field = TextEditingController();
  String? _error;
  bool _busy = false;

  @override
  void dispose() {
    _field.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    final l = context.l10n;
    setState(() => _busy = true);
    final r = await widget.personal.unlock(widget.library.id, _field.text);
    if (!mounted) return;
    if (r.ok) {
      Navigator.pop(context, true);
      return;
    }
    final until = r.attempts.lockedUntil;
    setState(() {
      _busy = false;
      _field.clear();
      _error = r.locked && until != null
          ? l.personalWait(formatPinWait(until.difference(DateTime.now())))
          : l.personalPasswordWrong;
    });
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return AlertDialog(
      titleTextStyle: Theme.of(context).textTheme.titleLarge,
      title: Text(l.personalUnlockTitle(widget.library.name)),
      content: TextField(
        key: const ValueKey('personal-password'),
        controller: _field,
        autofocus: true,
        obscureText: true,
        enableSuggestions: false,
        autocorrect: false,
        decoration: InputDecoration(
          labelText: l.personalPassword,
          errorText: _error,
        ),
        onSubmitted: (_) => _busy ? null : _submit(),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: Text(l.commonCancel),
        ),
        FilledButton(
          key: const ValueKey('personal-password-ok'),
          onPressed: _busy ? null : _submit,
          child: Text(l.personalUnlock),
        ),
      ],
    );
  }
}

/// Asks for a name and a lock and creates a library; null if cancelled.
Future<PersonalLibrary?> showCreatePersonalLibraryDialog(
  BuildContext context,
  PersonalMedia personal,
) async {
  final choice = await showDialog<(String, PersonalLock, String?)>(
    context: context,
    builder: (_) => const _LockDialog(askName: true),
  );
  if (choice == null || !context.mounted) return null;
  var (name, lock, secret) = choice;
  if (lock == PersonalLock.pin) {
    secret = await showNewPinDialog(
      context,
      title: context.l10n.personalLockPin,
    );
    if (secret == null) return null;
  }
  return personal.createLibrary(name, lock: lock, secret: secret);
}

/// The name (on create) and lock of a library: none, PIN (asked next on
/// the pad) or a password typed twice.
class _LockDialog extends StatefulWidget {
  const _LockDialog({required this.askName});

  final bool askName;

  @override
  State<_LockDialog> createState() => _LockDialogState();
}

class _LockDialogState extends State<_LockDialog> {
  final _name = TextEditingController();
  final _password = TextEditingController();
  final _repeat = TextEditingController();
  PersonalLock _lock = PersonalLock.pin;
  bool _submitted = false;

  @override
  void dispose() {
    _name.dispose();
    _password.dispose();
    _repeat.dispose();
    super.dispose();
  }

  String? _passwordError(AppLocalizations l) {
    if (!_submitted || _lock != PersonalLock.password) return null;
    if (_password.text.length < 6) return l.personalPasswordShort;
    return null;
  }

  String? _repeatError(AppLocalizations l) {
    if (!_submitted || _lock != PersonalLock.password) return null;
    if (_repeat.text != _password.text) return l.personalPasswordMismatch;
    return null;
  }

  void _submit() {
    final l = context.l10n;
    setState(() => _submitted = true);
    if (widget.askName && _name.text.trim().isEmpty) return;
    if (_passwordError(l) != null || _repeatError(l) != null) return;
    Navigator.pop(context, (
      _name.text.trim(),
      _lock,
      _lock == PersonalLock.password ? _password.text : null,
    ));
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return AlertDialog(
      titleTextStyle: Theme.of(context).textTheme.titleLarge,
      title: Text(widget.askName ? l.personalCreate : l.personalChangeLock),
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (widget.askName) ...[
                TextField(
                  key: const ValueKey('personal-name'),
                  controller: _name,
                  autofocus: true,
                  maxLength: 100,
                  decoration: InputDecoration(
                    labelText: l.personalName,
                    errorText: _submitted && _name.text.trim().isEmpty
                        ? l.personalName
                        : null,
                  ),
                ),
                const SizedBox(height: 8),
              ],
              Text(l.personalLockLabel),
              const SizedBox(height: 8),
              SegmentedButton<PersonalLock>(
                key: const ValueKey('personal-lock'),
                segments: [
                  ButtonSegment(
                    value: PersonalLock.none,
                    label: Text(l.personalLockNone),
                  ),
                  ButtonSegment(
                    value: PersonalLock.pin,
                    label: Text(l.personalLockPin),
                  ),
                  ButtonSegment(
                    value: PersonalLock.password,
                    label: Text(l.personalLockPassword),
                  ),
                ],
                selected: {_lock},
                onSelectionChanged: (s) => setState(() => _lock = s.first),
              ),
              if (_lock == PersonalLock.password) ...[
                const SizedBox(height: 12),
                TextField(
                  key: const ValueKey('personal-new-password'),
                  controller: _password,
                  obscureText: true,
                  enableSuggestions: false,
                  autocorrect: false,
                  decoration: InputDecoration(
                    labelText: l.personalPassword,
                    helperText: l.personalPasswordShort,
                    errorText: _passwordError(l),
                  ),
                ),
                TextField(
                  key: const ValueKey('personal-repeat-password'),
                  controller: _repeat,
                  obscureText: true,
                  enableSuggestions: false,
                  autocorrect: false,
                  decoration: InputDecoration(
                    labelText: l.personalPasswordRepeat,
                    errorText: _repeatError(l),
                  ),
                  onSubmitted: (_) => _submit(),
                ),
              ],
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: Text(l.commonCancel),
        ),
        FilledButton(
          key: const ValueKey('personal-lock-ok'),
          onPressed: _submit,
          child: Text(l.commonSave),
        ),
      ],
    );
  }
}

// ------------------------------------------------------------------ library

/// The videos of one unlocked personal library.
class PersonalLibraryScreen extends StatefulWidget {
  const PersonalLibraryScreen({super.key, required this.library});

  final PersonalLibrary library;

  @override
  State<PersonalLibraryScreen> createState() => _PersonalLibraryScreenState();
}

class _PersonalLibraryScreenState extends State<PersonalLibraryScreen> {
  late PersonalLibrary _library = widget.library;
  PersonalMedia? _personal;
  StreamSubscription<void>? _sub;
  StreamSubscription<void>? _filesSub;
  List<PersonalVideo>? _videos;

  /// UI-033: «Продолжить» of this library (only while it is open).
  List<PersonalVideo> _continue = const [];
  List<MediaCategory> _categories = const [];
  List<MediaTag> _tags = const [];
  PersonalFilter _filter = const PersonalFilter();
  PersonalSort _sort = PersonalSort.added;
  final _search = TextEditingController();

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final personal = Services.of(context).personal;
    if (!identical(personal, _personal)) {
      _sub?.cancel();
      _personal = personal;
      _sub = personal?.changes.listen((_) => _load());
      _load();
    }
    // UI-035: an upload or a move landed: the library may have changed.
    _filesSub ??= Services.of(context).filesControl?.changes
        .listen((_) => _load());
  }

  @override
  void dispose() {
    _sub?.cancel();
    _filesSub?.cancel();
    _search.dispose();
    super.dispose();
  }

  /// Leaves the page (locked, deleted); safe to call twice.
  void _close() {
    if (!mounted) return;
    final route = ModalRoute.of(context);
    if (route != null && route.isActive) {
      Navigator.of(context).removeRoute(route);
    }
  }

  Future<void> _load() async {
    final personal = _personal;
    if (personal == null) return;
    final id = _library.id;
    try {
      final libraries = await personal.libraries();
      final current = libraries.where((l) => l.id == id).firstOrNull;
      if (current == null || current.locked) {
        // Deleted, or locked meanwhile (background, profile change).
        _close();
        return;
      }
      final videos = await personal.videos(id, filter: _filter, sort: _sort);
      final resume = await personal.continueWatching(id);
      final categories = await personal.categories(id);
      final tags = await personal.tags(id);
      if (!mounted) return;
      setState(() {
        _library = current;
        _videos = videos;
        _continue = resume;
        _categories = categories;
        _tags = tags;
      });
    } on PersonalLocked {
      _close();
    }
  }

  /// UI-032: the preview frame of [v]; refused once the library locks.
  Future<Uint8List?> _frame(PersonalVideo v) async {
    final personal = _personal;
    if (personal == null) return null;
    return personal.frame(_library.id, v);
  }

  void _setFilter(PersonalFilter f) {
    setState(() => _filter = f);
    _load();
  }

  List<T> _toggle<T>(List<T> list, T v) =>
      list.contains(v) ? ([...list]..remove(v)) : [...list, v];

  Future<void> _play(PersonalVideo v) async {
    final services = Services.read(context);
    final personal = _personal!;
    final l = context.l10n;
    final messenger = ScaffoldMessenger.maybeOf(context);
    final uris = await personal.playUris(_library.id, v);
    if (!mounted) return;
    if (uris.isEmpty) {
      messenger?.showSnackBar(SnackBar(content: Text(l.personalCannotPlay)));
      return;
    }
    final musicInterrupted = services.music.pauseForVideo();
    final id = _library.id;
    await Navigator.of(context, rootNavigator: true).push(
      MaterialPageRoute<void>(
        fullscreenDialog: true,
        builder: (_) => PlayerScreen(
          request: PlayRequest(
            uri: uris.first,
            title: v.title,
            durationMs: v.durationMs,
            startPositionMs: personalResumeMs(v),
          ),
          // UI-033: the position stays in this library — no shared
          // "Continue", no Watch Next, no sync.
          onProgress: (pos, dur) => unawaited(
            personal
                .saveProgress(id, v, pos, dur)
                .then<void>(
                  (_) {},
                  // Locked meanwhile (the app went to the background).
                  onError: (Object _) {},
                ),
          ),
          engineFactory: services.playerEngineFactory,
          subtitleSettings: services.subtitleSettings,
          subtitleLanguages: services.subtitleLanguages,
          seekStep: Duration(seconds: services.skipSeconds.value),
        ),
      ),
    );
    if (musicInterrupted && mounted) offerMusicResume(services, messenger, l);
    // The row and the tiles show where it was left.
    unawaited(_load());
  }

  Future<void> _menu(String action) async {
    final personal = _personal!;
    final l = context.l10n;
    switch (action) {
      case 'lock':
        await personal.lockLibrary(_library.id);
        _close();
      case 'folder':
        await showAddPersonalFolderDialog(context, personal, _library);
      case 'rename':
        final name = await _askText(
          context,
          title: l.personalRename,
          label: l.personalName,
          initial: _library.name,
        );
        if (name != null && name.trim().isNotEmpty) {
          await personal.renameLibrary(_library.id, name.trim());
        }
      case 'lock-change':
        final choice = await showDialog<(String, PersonalLock, String?)>(
          context: context,
          builder: (_) => const _LockDialog(askName: false),
        );
        if (choice == null || !mounted) return;
        var (_, lock, secret) = choice;
        if (lock == PersonalLock.pin) {
          secret = await showNewPinDialog(context, title: l.personalLockPin);
          if (secret == null) return;
        }
        await personal.setLock(_library.id, lock, secret: secret);
      case 'delete':
        final ok = await showDialog<bool>(
          context: context,
          builder: (c) => AlertDialog(
            titleTextStyle: Theme.of(context).textTheme.titleLarge,
            title: Text(l.personalDelete),
            content: Text(l.personalDeleteConfirm(_library.name)),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(c, false),
                child: Text(l.commonCancel),
              ),
              FilledButton(
                key: const ValueKey('personal-delete-ok'),
                onPressed: () => Navigator.pop(c, true),
                child: Text(l.personalDelete),
              ),
            ],
          ),
        );
        if (ok == true) {
          await personal.deleteLibrary(_library.id);
          _close();
        }
    }
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final d = KDensity.of(context);
    final videos = _videos;
    final f = _filter;
    final chipStyle = Theme.of(context).textTheme.labelLarge;
    final chips = <Widget>[
      for (final c in _categories)
        FilterChip(
          labelStyle: chipStyle,
          key: ValueKey('personal-category-${c.id}'),
          label: Text(c.name),
          selected: f.categories.contains(int.parse(c.id)),
          onSelected: (_) => _setFilter(
            PersonalFilter(
              categories: _toggle(f.categories, int.parse(c.id)),
              tags: f.tags,
              ratings: f.ratings,
              text: f.text,
            ),
          ),
        ),
      for (final t in _tags)
        FilterChip(
          labelStyle: chipStyle,
          key: ValueKey('personal-tag-${t.name}'),
          label: Text('#${t.name}'),
          selected: f.tags.contains(t.name),
          onSelected: (_) => _setFilter(
            PersonalFilter(
              categories: f.categories,
              tags: _toggle(f.tags, t.name),
              ratings: f.ratings,
              text: f.text,
            ),
          ),
        ),
      for (final s in const [5, 4, 3, 2, 1, 0])
        FilterChip(
          labelStyle: chipStyle,
          key: ValueKey('personal-stars-$s'),
          label: Text(s == 0 ? l.personalUnrated : l.personalStars(s)),
          selected: f.ratings.contains(s),
          onSelected: (_) => _setFilter(
            PersonalFilter(
              categories: f.categories,
              tags: f.tags,
              ratings: _toggle(f.ratings, s),
              text: f.text,
            ),
          ),
        ),
    ];
    final Widget content;
    if (videos == null) {
      content = const Center(child: CircularProgressIndicator());
    } else if (videos.isEmpty) {
      content = EmptyState(
        icon: Icons.video_library_outlined,
        title: f.isEmpty ? l.personalEmptyLibrary : l.personalNoMatches,
        message: f.isEmpty ? l.personalEmptyLibraryHint : null,
        action: f.isEmpty
            ? OutlinedButton.icon(
                onPressed: () => _menu('folder'),
                icon: const Icon(Icons.create_new_folder_outlined),
                label: Text(l.personalAddFolder),
              )
            : null,
      );
    } else {
      final resume = f.isEmpty ? _continue : const <PersonalVideo>[];
      final tileWidth = d.isTv ? 300.0 : 240.0;
      final heading = Theme.of(context).textTheme.titleMedium;
      content = CustomScrollView(
        key: const ValueKey('personal-videos'),
        slivers: [
          // UI-033: «Продолжить» of this library, above its videos.
          if (resume.isNotEmpty)
            SliverToBoxAdapter(
              child: Column(
                key: const ValueKey('personal-continue'),
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Padding(
                    padding: EdgeInsets.fromLTRB(
                      d.pageInsets.left,
                      12,
                      d.pageInsets.right,
                      8,
                    ),
                    child: Text(l.personalContinue, style: heading),
                  ),
                  SizedBox(
                    height: tileWidth * 13 / 16,
                    child: ListView.separated(
                      scrollDirection: Axis.horizontal,
                      padding: EdgeInsets.symmetric(
                        horizontal: d.pageInsets.left,
                      ),
                      itemCount: resume.length,
                      separatorBuilder: (_, _) => const SizedBox(width: 16),
                      itemBuilder: (_, i) => SizedBox(
                        width: tileWidth,
                        child: _VideoTile(
                          video: resume[i],
                          frame: () => _frame(resume[i]),
                          keyPrefix: 'personal-continue',
                          onPlay: () => _play(resume[i]),
                          onMore: () => _videoSheet(resume[i]),
                        ),
                      ),
                    ),
                  ),
                  Padding(
                    padding: EdgeInsets.fromLTRB(
                      d.pageInsets.left,
                      20,
                      d.pageInsets.right,
                      0,
                    ),
                    child: Text(l.personalAllVideos, style: heading),
                  ),
                ],
              ),
            ),
          SliverPadding(
            padding: EdgeInsets.fromLTRB(
              d.pageInsets.left,
              8,
              d.pageInsets.right,
              32,
            ),
            sliver: SliverGrid.builder(
              gridDelegate: SliverGridDelegateWithMaxCrossAxisExtent(
                maxCrossAxisExtent: d.isTv ? 360 : 280,
                mainAxisSpacing: 16,
                crossAxisSpacing: 16,
                childAspectRatio: 16 / 13,
              ),
              itemCount: videos.length,
              itemBuilder: (_, i) => _VideoTile(
                video: videos[i],
                frame: () => _frame(videos[i]),
                onPlay: () => _play(videos[i]),
                onMore: () => _videoSheet(videos[i]),
              ),
            ),
          ),
        ],
      );
    }
    return PersonalSecureScope(
      child: Scaffold(
        key: ValueKey('personal-library-page-${_library.id}'),
        backgroundColor: KColors.bgBase,
        appBar: AppBar(
          backgroundColor: KColors.bgBase,
          title: Text(
            _library.name,
            style: Theme.of(context).textTheme.titleLarge,
          ),
          actions: [
            PopupMenuButton<PersonalSort>(
              key: const ValueKey('personal-sort'),
              tooltip: l.personalSort,
              icon: const Icon(Icons.sort),
              initialValue: _sort,
              onSelected: (s) {
                setState(() => _sort = s);
                _load();
              },
              itemBuilder: (_) => [
                for (final s in PersonalSort.values)
                  PopupMenuItem(value: s, child: Text(_sortLabel(l, s))),
              ],
            ),
            if (Services.of(context).filesControl != null)
              IconButton(
                key: const ValueKey('personal-files'),
                tooltip: l.filesTitle,
                icon: const Icon(Icons.folder_open_outlined),
                onPressed: () => openPersonalFiles(context, _library),
              ),
            if (_library.lock != PersonalLock.none)
              IconButton(
                key: const ValueKey('personal-lock-now'),
                tooltip: l.personalLockNow,
                icon: const Icon(Icons.lock_outline),
                onPressed: () => _menu('lock'),
              ),
            PopupMenuButton<String>(
              key: const ValueKey('personal-menu'),
              onSelected: _menu,
              itemBuilder: (_) => [
                PopupMenuItem(
                  value: 'folder',
                  child: Text(l.personalAddFolder),
                ),
                PopupMenuItem(value: 'rename', child: Text(l.personalRename)),
                PopupMenuItem(
                  value: 'lock-change',
                  child: Text(l.personalChangeLock),
                ),
                PopupMenuItem(value: 'delete', child: Text(l.personalDelete)),
              ],
            ),
          ],
        ),
        body: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: EdgeInsets.fromLTRB(
                d.pageInsets.left,
                4,
                d.pageInsets.right,
                4,
              ),
              child: TextField(
                key: const ValueKey('personal-search'),
                controller: _search,
                decoration: InputDecoration(
                  prefixIcon: const Icon(Icons.search),
                  hintText: l.personalSearch,
                  isDense: true,
                ),
                onChanged: (t) => _setFilter(
                  PersonalFilter(
                    categories: f.categories,
                    tags: f.tags,
                    ratings: f.ratings,
                    text: t,
                  ),
                ),
              ),
            ),
            SizedBox(
              height: 48,
              child: ListView.separated(
                key: const ValueKey('personal-filters'),
                scrollDirection: Axis.horizontal,
                padding: EdgeInsets.symmetric(horizontal: d.pageInsets.left),
                itemCount: chips.length,
                separatorBuilder: (_, _) => const SizedBox(width: 8),
                itemBuilder: (_, i) => Center(child: chips[i]),
              ),
            ),
            Expanded(child: content),
          ],
        ),
        bottomNavigationBar: switch (Services.of(context).filesControl) {
          final f? => UploadsPanel(controller: f),
          null => null,
        },
        floatingActionButton: Services.of(context).filesControl == null
            ? null
            : FloatingActionButton.extended(
                key: const ValueKey('personal-upload'),
                onPressed: _upload,
                icon: const Icon(Icons.upload),
                label: Text(l.filesUpload),
              ),
      ),
    );
  }

  /// UI-035: gallery or files (any file on desktop) into this library.
  Future<void> _upload() async {
    final l = context.l10n;
    final media = FormFactorScope.of(context) == FormFactor.desktop
        ? false
        : await showModalBottomSheet<bool>(
            context: context,
            builder: (c) => SafeArea(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  ListTile(
                    key: const ValueKey('personal-upload-gallery'),
                    leading: const Icon(Icons.photo_library_outlined),
                    title: Text(l.filesFromGallery),
                    onTap: () => Navigator.pop(c, true),
                  ),
                  ListTile(
                    key: const ValueKey('personal-upload-files'),
                    leading: const Icon(Icons.folder_open_outlined),
                    title: Text(l.filesFromFiles),
                    onTap: () => Navigator.pop(c, false),
                  ),
                ],
              ),
            ),
          );
    if (media == null || !mounted) return;
    await pickAndUploadToLibrary(context, _library, media: media);
  }

  Future<void> _videoSheet(PersonalVideo v) async {
    final personal = _personal!;
    final l = context.l10n;
    final messenger = ScaffoldMessenger.maybeOf(context);
    final labelId = v.labelId;
    final action = await showModalBottomSheet<String>(
      context: context,
      backgroundColor: KColors.bgElevated,
      builder: (c) => SafeArea(
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              ListTile(
                title: Text(v.title, style: Theme.of(c).textTheme.titleMedium),
              ),
              ListTile(
                key: const ValueKey('personal-video-play'),
                leading: const Icon(Icons.play_arrow_rounded),
                title: Text(l.commonPlay),
                onTap: () => Navigator.pop(c, 'play'),
              ),
              if (labelId != null) ...[
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 16),
                  child: Wrap(
                    spacing: 4,
                    crossAxisAlignment: WrapCrossAlignment.center,
                    children: [
                      Text(l.personalRating),
                      for (var s = 1; s <= 5; s++)
                        IconButton(
                          key: ValueKey('personal-rate-$s'),
                          tooltip: l.personalStars(s),
                          icon: Icon(
                            s <= (v.stars ?? 0)
                                ? Icons.star_rounded
                                : Icons.star_outline_rounded,
                            color: KColors.accent,
                          ),
                          onPressed: () => Navigator.pop(c, 'rate:$s'),
                        ),
                      TextButton(
                        onPressed: () => Navigator.pop(c, 'rate:0'),
                        child: Text(l.personalClearRating),
                      ),
                    ],
                  ),
                ),
                ListTile(
                  key: const ValueKey('personal-video-tags'),
                  leading: const Icon(Icons.sell_outlined),
                  title: Text(l.personalTags),
                  subtitle: v.tags.isEmpty ? null : Text(v.tags.join(', ')),
                  onTap: () => Navigator.pop(c, 'tags'),
                ),
                ListTile(
                  key: const ValueKey('personal-video-categories'),
                  leading: const Icon(Icons.category_outlined),
                  title: Text(l.personalCategories),
                  onTap: () => Navigator.pop(c, 'categories'),
                ),
              ],
              if (v.progress != null)
                ListTile(
                  key: const ValueKey('personal-video-clear-progress'),
                  leading: const Icon(Icons.restart_alt_rounded),
                  title: Text(l.personalClearProgress),
                  onTap: () => Navigator.pop(c, 'clear-progress'),
                ),
              if (v.moved)
                ListTile(
                  key: const ValueKey('personal-video-move-back'),
                  leading: const Icon(Icons.undo_rounded),
                  title: Text(l.personalMoveBack),
                  onTap: () => Navigator.pop(c, 'back'),
                ),
            ],
          ),
        ),
      ),
    );
    if (action == null || !mounted) return;
    final id = _library.id;
    if (action == 'play') return _play(v);
    if (action == 'clear-progress') return personal.clearProgress(id, v);
    if (action.startsWith('rate:') && labelId != null) {
      final s = int.parse(action.substring(5));
      return personal.setRating(id, labelId, s == 0 ? null : s);
    }
    if (action == 'tags' && labelId != null) {
      final text = await _askText(
        context,
        title: l.personalTags,
        label: l.personalTagsHint,
        initial: v.tags.join(', '),
      );
      if (text == null) return;
      final tags = [
        for (final t in text.split(','))
          if (t.trim().isNotEmpty) t.trim(),
      ];
      return personal.setTags(id, labelId, tags);
    }
    if (action == 'categories' && labelId != null) {
      if (!mounted) return;
      return showDialog<void>(
        context: context,
        builder: (_) =>
            _CategoriesDialog(personal: personal, libraryId: id, video: v),
      );
    }
    if (action == 'back') {
      await personal.moveToMain(id, v.fileId);
      messenger?.showSnackBar(SnackBar(content: Text(l.personalMovedBack)));
    }
  }
}

/// Where the player starts [v]: where it was left, unless it was watched
/// to the end (UI-033).
int personalResumeMs(PersonalVideo v) {
  final p = v.progress;
  if (p == null || p.watched) return 0;
  final f = p.fraction;
  if (f != null && f > 0.95) return 0;
  return p.positionMs;
}

String _sortLabel(AppLocalizations l, PersonalSort s) => switch (s) {
  PersonalSort.added => l.personalSortAdded,
  PersonalSort.title => l.personalSortTitle,
  PersonalSort.rating => l.personalSortRating,
  PersonalSort.duration => l.personalSortDuration,
  PersonalSort.size => l.personalSortSize,
};

/// A video: a frame-like tile with the length, the title and the stars.
class _VideoTile extends StatelessWidget {
  const _VideoTile({
    required this.video,
    required this.frame,
    required this.onPlay,
    required this.onMore,
    this.keyPrefix = 'personal-video',
  });

  final PersonalVideo video;

  /// UI-032: its preview frame from the core (only while the library is
  /// open); the tint stays until it comes or when there is none.
  final Future<Uint8List?> Function() frame;
  final VoidCallback onPlay;
  final VoidCallback onMore;

  /// Keys of the tile (`<prefix>-<file id>`): the «Продолжить» row shows
  /// the same video as the grid.
  final String keyPrefix;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final ms = video.durationMs;
    final stars = video.stars;
    final progress = video.progress;
    // A stable tint per file: under the frame until it comes (UI-032),
    // and for a file without one.
    final hue = (video.fileId * 47) % 360;
    return Material(
      color: KColors.bgElevated,
      borderRadius: BorderRadius.circular(12),
      clipBehavior: Clip.antiAlias,
      child: InkWell(
        key: ValueKey('$keyPrefix-${video.fileId}'),
        onTap: onPlay,
        onLongPress: onMore,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(
              child: DecoratedBox(
                decoration: BoxDecoration(
                  gradient: LinearGradient(
                    begin: Alignment.topLeft,
                    end: Alignment.bottomRight,
                    colors: [
                      HSLColor.fromAHSL(1, hue.toDouble(), .35, .28).toColor(),
                      HSLColor.fromAHSL(
                        1,
                        (hue + 40) % 360,
                        .30,
                        .14,
                      ).toColor(),
                    ],
                  ),
                ),
                child: Stack(
                  children: [
                    Positioned.fill(
                      child: VideoFrameImage(
                        key: ValueKey('$keyPrefix-frame-box-${video.fileId}'),
                        load: frame,
                        imageKey: ValueKey('$keyPrefix-frame-${video.fileId}'),
                        fallback: const SizedBox.expand(),
                      ),
                    ),
                    const Center(
                      child: Icon(
                        Icons.play_circle_outline_rounded,
                        size: 44,
                        color: Color(0xCCFFFFFF),
                      ),
                    ),
                    if (ms != null && ms > 0)
                      Positioned(
                        right: 8,
                        bottom: 8,
                        child: DecoratedBox(
                          decoration: BoxDecoration(
                            color: const Color(0xB3000000),
                            borderRadius: BorderRadius.circular(4),
                          ),
                          child: Padding(
                            padding: const EdgeInsets.symmetric(
                              horizontal: 6,
                              vertical: 2,
                            ),
                            child: Text(
                              formatClock(ms),
                              style: theme.textTheme.labelSmall,
                            ),
                          ),
                        ),
                      ),
                    if (video.moved)
                      const Positioned(
                        left: 8,
                        top: 8,
                        child: Icon(
                          Icons.movie_outlined,
                          size: 18,
                          color: Color(0xCCFFFFFF),
                        ),
                      ),
                    // UI-033: where it was left, or watched.
                    if (progress != null && progress.watched)
                      Positioned(
                        right: 8,
                        top: 8,
                        child: Icon(
                          Icons.check_circle_rounded,
                          key: ValueKey('$keyPrefix-watched-${video.fileId}'),
                          size: 18,
                          color: const Color(0xCCFFFFFF),
                        ),
                      )
                    else if (progress?.fraction case final f?)
                      Positioned(
                        left: 0,
                        right: 0,
                        bottom: 0,
                        child: LinearProgressIndicator(
                          key: ValueKey('$keyPrefix-progress-${video.fileId}'),
                          value: f,
                          minHeight: 3,
                          color: KColors.accent,
                          backgroundColor: const Color(0x66000000),
                        ),
                      ),
                  ],
                ),
              ),
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(12, 8, 4, 8),
              child: Row(
                children: [
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          video.title,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: theme.textTheme.titleSmall,
                        ),
                        Text(
                          stars == null ? ' ' : '★' * stars,
                          style: theme.textTheme.labelSmall?.copyWith(
                            color: KColors.accent,
                          ),
                        ),
                      ],
                    ),
                  ),
                  IconButton(
                    key: ValueKey('$keyPrefix-more-${video.fileId}'),
                    tooltip: context.l10n.commonMore,
                    icon: const Icon(Icons.more_vert),
                    onPressed: onMore,
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _CategoriesDialog extends StatefulWidget {
  const _CategoriesDialog({
    required this.personal,
    required this.libraryId,
    required this.video,
  });

  final PersonalMedia personal;
  final int libraryId;
  final PersonalVideo video;

  @override
  State<_CategoriesDialog> createState() => _CategoriesDialogState();
}

class _CategoriesDialogState extends State<_CategoriesDialog> {
  List<MediaCategory> _all = const [];
  late final Set<int> _in = {...widget.video.categories};
  final _name = TextEditingController();

  @override
  void initState() {
    super.initState();
    _reload();
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _reload() async {
    final all = await widget.personal.categories(widget.libraryId);
    if (mounted) setState(() => _all = all);
  }

  Future<void> _toggle(int id, bool on) async {
    final label = widget.video.labelId!;
    await widget.personal.setInCategory(widget.libraryId, id, label, on);
    setState(() => on ? _in.add(id) : _in.remove(id));
  }

  Future<void> _add() async {
    final name = _name.text.trim();
    if (name.isEmpty) return;
    final c = await widget.personal.createCategory(widget.libraryId, name);
    _name.clear();
    await _toggle(int.parse(c.id), true);
    await _reload();
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return AlertDialog(
      titleTextStyle: Theme.of(context).textTheme.titleLarge,
      title: Text(l.personalCategories),
      content: SizedBox(
        width: 380,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (final c in _all)
              CheckboxListTile(
                key: ValueKey('personal-in-category-${c.id}'),
                value: _in.contains(int.parse(c.id)),
                title: Text(c.name),
                onChanged: (v) => _toggle(int.parse(c.id), v ?? false),
              ),
            TextField(
              key: const ValueKey('personal-new-category'),
              controller: _name,
              decoration: InputDecoration(
                labelText: l.personalNewCategory,
                suffixIcon: IconButton(
                  icon: const Icon(Icons.add),
                  onPressed: _add,
                ),
              ),
              onSubmitted: (_) => _add(),
            ),
          ],
        ),
      ),
      actions: [
        FilledButton(
          onPressed: () => Navigator.pop(context),
          child: Text(l.commonBack),
        ),
      ],
    );
  }
}

Future<String?> _askText(
  BuildContext context, {
  required String title,
  required String label,
  String initial = '',
}) {
  final controller = TextEditingController(text: initial);
  return showDialog<String>(
    context: context,
    builder: (c) => AlertDialog(
      titleTextStyle: Theme.of(context).textTheme.titleLarge,
      title: Text(title),
      content: TextField(
        key: const ValueKey('personal-text'),
        controller: controller,
        autofocus: true,
        decoration: InputDecoration(labelText: label),
        onSubmitted: (t) => Navigator.pop(c, t),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(c),
          child: Text(context.l10n.commonCancel),
        ),
        FilledButton(
          key: const ValueKey('personal-text-ok'),
          onPressed: () => Navigator.pop(c, controller.text),
          child: Text(context.l10n.commonSave),
        ),
      ],
    ),
  ).whenComplete(controller.dispose);
}

/// Adds a folder of one of the sources to [library]: the user browses the
/// source's folders (UI-034) instead of typing a path.
Future<void> showAddPersonalFolderDialog(
  BuildContext context,
  PersonalMedia personal,
  PersonalLibrary library,
) async {
  final services = Services.read(context);
  final l = context.l10n;
  final sources = [
    for (final s in services.managedSources)
      if (int.tryParse(s.id) != null) s,
  ];
  if (sources.isEmpty) {
    ScaffoldMessenger.maybeOf(context)
        ?.showSnackBar(SnackBar(content: Text(l.personalNoSources)));
    return;
  }
  final result = await showDialog<(int, String)>(
    context: context,
    builder: (_) => PersonalFolderPicker(sources: sources),
  );
  if (result == null) return;
  await personal.addFolder(library.id, result.$1, result.$2);
}

/// Picks a source, then a folder in it, by browsing (UI-034): the source's
/// folders as the file browser reads them (live, else what the library
/// knows). Pops `(source id, folder)`; `''` is the whole source.
class PersonalFolderPicker extends StatefulWidget {
  const PersonalFolderPicker({super.key, required this.sources});

  final List<ManagedSource> sources;

  @override
  State<PersonalFolderPicker> createState() => _PersonalFolderPickerState();
}

class _PersonalFolderPickerState extends State<PersonalFolderPicker> {
  ManagedSource? _source;
  String _dir = '';
  List<BrowseEntry>? _entries;
  Object? _error;
  SourceBrowser? _browser;

  @override
  void initState() {
    super.initState();
    // One source: straight into it.
    if (widget.sources.length == 1) {
      WidgetsBinding.instance.addPostFrameCallback(
        (_) => _open(widget.sources.single, ''),
      );
    }
  }

  Future<void> _open(ManagedSource source, String dir) async {
    if (!mounted) return;
    final services = Services.read(context);
    final browser = _browser ??= SourceBrowser.of(
      services.library,
      services.sourceManager,
    );
    setState(() {
      _source = source;
      _dir = dir;
      _entries = null;
      _error = null;
    });
    try {
      final listing = await browser.list(source, dir);
      if (!mounted || _source != source || _dir != dir) return;
      final folders = [
        for (final e in listing.entries)
          if (e.isFolder && !isHiddenName(e.name)) e,
      ]..sort((a, b) => naturalKey(a.name).compareTo(naturalKey(b.name)));
      setState(() => _entries = folders);
    } catch (e) {
      if (mounted) setState(() => _error = e);
    }
  }

  void _up() {
    final source = _source;
    if (source == null) return;
    if (_dir.isEmpty) {
      if (widget.sources.length > 1) setState(() => _source = null);
      return;
    }
    final i = _dir.lastIndexOf('/');
    unawaited(_open(source, i < 0 ? '' : _dir.substring(0, i)));
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final source = _source;
    final List<Widget> rows;
    if (source == null) {
      rows = [
        for (final s in widget.sources)
          ListTile(
            key: ValueKey('personal-folder-source-${s.id}'),
            leading: const Icon(Icons.storage_outlined),
            title: Text(s.name),
            trailing: const Icon(Icons.chevron_right),
            onTap: () => _open(s, ''),
          ),
      ];
    } else if (_error != null) {
      rows = [
        Padding(
          padding: const EdgeInsets.all(16),
          child: Text(
            l.personalFolderListFailed,
            key: const ValueKey('personal-folder-error'),
          ),
        ),
      ];
    } else if (_entries == null) {
      rows = const [
        Padding(
          padding: EdgeInsets.all(24),
          child: Center(child: CircularProgressIndicator()),
        ),
      ];
    } else if (_entries!.isEmpty) {
      rows = [
        Padding(
          padding: const EdgeInsets.all(16),
          child: Text(
            l.personalFolderNoSubfolders,
            style: Theme.of(context).textTheme.bodySmall,
          ),
        ),
      ];
    } else {
      rows = [
        for (final e in _entries!)
          ListTile(
            key: ValueKey('personal-folder-entry-${e.relPath}'),
            leading: const Icon(Icons.folder_outlined),
            title: Text(e.name),
            trailing: const Icon(Icons.chevron_right),
            onTap: () => _open(source, e.relPath),
          ),
      ];
    }
    final canGoUp =
        source != null && (_dir.isNotEmpty || widget.sources.length > 1);
    return AlertDialog(
      titleTextStyle: Theme.of(context).textTheme.titleLarge,
      title: Text(
        source == null
            ? l.personalAddFolder
            : (_dir.isEmpty ? source.name : '${source.name} / $_dir'),
        key: const ValueKey('personal-folder-title'),
      ),
      content: SizedBox(
        width: 480,
        height: 360,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              source == null
                  ? l.personalFolderPickSource
                  : (_dir.isEmpty
                        ? l.personalFolderWholeSource
                        : l.personalFolderPickHint),
              style: Theme.of(context).textTheme.bodySmall,
            ),
            const SizedBox(height: 8),
            Expanded(child: ListView(children: rows)),
          ],
        ),
      ),
      actions: [
        if (canGoUp)
          TextButton(
            key: const ValueKey('personal-folder-up'),
            onPressed: _up,
            child: Text(l.personalFolderUp),
          ),
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: Text(l.commonCancel),
        ),
        if (source != null)
          FilledButton(
            key: const ValueKey('personal-folder-ok'),
            onPressed: () =>
                Navigator.pop(context, (int.parse(source.id), _dir)),
            child: Text(l.personalFolderChoose),
          ),
      ],
    );
  }
}

// ------------------------------------------------------------- main library

/// Asks which personal library (unlocking it if needed) receives an item
/// or a folder; null if there is none or the user cancelled.
Future<PersonalLibrary?> pickPersonalLibrary(BuildContext context) async {
  final personal = Services.read(context).personal;
  if (personal == null) return null;
  final l = context.l10n;
  final messenger = ScaffoldMessenger.maybeOf(context);
  final libraries = await personal.libraries();
  if (!context.mounted) return null;
  if (libraries.isEmpty) {
    messenger?.showSnackBar(SnackBar(content: Text(l.personalNoOpenLibrary)));
    return null;
  }
  final target = libraries.length == 1
      ? libraries.single
      : await showDialog<PersonalLibrary>(
          context: context,
          builder: (c) => SimpleDialog(
            titleTextStyle: Theme.of(context).textTheme.titleLarge,
            title: Text(l.personalChooseLibrary),
            children: [
              for (final lib in libraries)
                SimpleDialogOption(
                  key: ValueKey('personal-target-${lib.id}'),
                  onPressed: () => Navigator.pop(c, lib),
                  child: Row(
                    children: [
                      Icon(_lockIcon(lib)),
                      const SizedBox(width: 12),
                      Expanded(child: Text(lib.name)),
                    ],
                  ),
                ),
            ],
          ),
        );
  if (target == null || !context.mounted) return null;
  if (target.locked && !await unlockPersonalLibrary(context, target)) {
    return null;
  }
  return target;
}

/// «Перенести в личные медиа» for an item of the main library: picks an
/// open library (or unlocks one) and moves the item there.
Future<void> moveItemToPersonal(BuildContext context, LibraryItem item) async {
  final personal = Services.read(context).personal;
  if (personal == null) return;
  final l = context.l10n;
  final messenger = ScaffoldMessenger.maybeOf(context);
  final target = await pickPersonalLibrary(context);
  if (target == null) return;
  await personal.moveToPersonal(item.id, target.id);
  messenger?.showSnackBar(
    SnackBar(content: Text(l.personalMovedTo(target.name))),
  );
}

/// «Перенести в Личное» for a folder of a source in its file browser
/// (UI-034, ADR-0099): [folder] relative to the source root, `''` for the
/// whole source. The folder leaves the main library at once; its files are
/// never recognized unless they already were. False if nothing moved.
Future<bool> moveFolderToPersonal(
  BuildContext context,
  String sourceId,
  String folder,
) async {
  final services = Services.read(context);
  final personal = services.personal;
  final id = int.tryParse(sourceId);
  if (personal == null || id == null) return false;
  final l = context.l10n;
  final messenger = ScaffoldMessenger.maybeOf(context);
  final target = await pickPersonalLibrary(context);
  if (target == null) return false;
  if (folder.isEmpty) {
    // The whole source: marked personal, so it also leaves other devices.
    await personal.setSourceContent(
      id,
      SourceContent.personal,
      libraryId: target.id,
    );
  } else {
    await personal.addFolder(target.id, id, folder);
  }
  messenger?.showSnackBar(
    SnackBar(content: Text(l.personalMovedTo(target.name))),
  );
  return true;
}

/// «Вернуть в медиатеку» for a folder (or, with `''`, the whole source)
/// of an open personal library: its files return to the main library and
/// the source is scanned so they are recognized.
Future<void> returnFolderToLibrary(
  BuildContext context,
  String sourceId,
  String folder, {
  int? libraryId,
}) async {
  final services = Services.read(context);
  final personal = services.personal;
  final id = int.tryParse(sourceId);
  if (personal == null || id == null) return;
  final l = context.l10n;
  final messenger = ScaffoldMessenger.maybeOf(context);
  if (folder.isEmpty) {
    await personal.setSourceContent(id, SourceContent.video);
  } else if (libraryId != null) {
    await personal.removeFolder(libraryId, id, folder);
  } else {
    return;
  }
  unawaited(services.scan(sourceId: sourceId));
  messenger?.showSnackBar(SnackBar(content: Text(l.personalMovedBack)));
}

// ------------------------------------------------------------------ privacy

/// Locks every personal library when the app leaves the foreground and
/// covers the app while it is not in front, so the app switcher shows no
/// preview of a personal library (UI-025). Wraps the whole app.
class PersonalPrivacy extends StatefulWidget {
  const PersonalPrivacy({super.key, required this.child});

  final Widget child;

  @override
  State<PersonalPrivacy> createState() => _PersonalPrivacyState();
}

class _PersonalPrivacyState extends State<PersonalPrivacy>
    with WidgetsBindingObserver {
  bool _covered = false;

  /// Whether a personal library is open on screen (the cover is only
  /// needed then).
  static final active = ValueNotifier<bool>(false);

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    final personal = Services.read(context).personal;
    switch (state) {
      case AppLifecycleState.resumed:
        if (_covered) setState(() => _covered = false);
      case AppLifecycleState.inactive:
        if (active.value && !_covered) setState(() => _covered = true);
      case AppLifecycleState.hidden ||
          AppLifecycleState.paused ||
          AppLifecycleState.detached:
        if (active.value && !_covered) setState(() => _covered = true);
        personal?.lockAll();
    }
  }

  @override
  Widget build(BuildContext context) => Stack(
    textDirection: TextDirection.ltr,
    children: [
      widget.child,
      if (_covered)
        Positioned.fill(
          child: ColoredBox(
            key: const ValueKey('personal-privacy-cover'),
            color: KColors.bgBase,
            child: const Center(
              child: Icon(Icons.lock_outline, size: 64, color: Colors.white54),
            ),
          ),
        ),
    ],
  );
}

/// Marks a personal library as open on screen while [child] is shown, and
/// asks the platform to keep it out of screenshots and the app switcher
/// (Android FLAG_SECURE).
class PersonalSecureScope extends StatefulWidget {
  const PersonalSecureScope({super.key, required this.child});

  final Widget child;

  @override
  State<PersonalSecureScope> createState() => _PersonalSecureScopeState();
}

const _deviceChannel = MethodChannel('app.kinolune/device');

int _secureScopes = 0;

Future<void> _setSecure(bool on) async {
  _PersonalPrivacyState.active.value = on;
  try {
    await _deviceChannel.invokeMethod<void>('secureWindow', on);
  } on MissingPluginException {
    // Not Android, or an older shell: the cover alone.
  } on PlatformException {
    // Same.
  }
}

class _PersonalSecureScopeState extends State<PersonalSecureScope> {
  @override
  void initState() {
    super.initState();
    if (_secureScopes++ == 0) unawaited(_setSecure(true));
  }

  @override
  void dispose() {
    if (--_secureScopes == 0) unawaited(_setSecure(false));
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => widget.child;
}
