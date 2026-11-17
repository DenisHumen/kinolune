// SPDX-License-Identifier: MPL-2.0
//
// In-memory personal media (UI-025) for the demo mode, screenshots and
// widget tests: the same rules as the core — a locked library answers
// nothing, PINs and passwords are checked, items move in and back, and
// positions (UI-033) stay in their library.

import 'dart:async';

import '../integration/library.dart';

class FakePersonalMedia implements PersonalMedia {
  FakePersonalMedia({this.onMove});

  /// The demo: a family library with a PIN (2468) and an open one.
  factory FakePersonalMedia.demo({
    void Function(String itemId, bool toPersonal)? onMove,
  }) {
    final f = FakePersonalMedia(onMove: onMove);
    final family = f._add('Семья', PersonalLock.pin, '2468');
    final home = f._add('Домашнее видео', PersonalLock.none, null);
    var file = 1;
    PersonalVideo v(
      String title,
      int minutes, {
      int? stars,
      List<String> tags = const [],
      List<int> categories = const [],
      int daysAgo = 1,
      int? watchedMinutes,
    }) {
      final id = file++;
      return PersonalVideo(
        fileId: id,
        labelId: 'file:${id.toRadixString(16).padLeft(16, '0')}',
        sourceId: 7,
        relPath: 'Видео/$title.mp4',
        title: title,
        sizeBytes: minutes * 60 * 1200000,
        durationMs: minutes * 60000,
        addedAt: DateTime(2026, 11, 16).subtract(Duration(days: daysAgo)),
        stars: stars,
        tags: tags,
        categories: categories,
        progress: watchedMinutes == null
            ? null
            : PersonalProgress(
                positionMs: watchedMinutes * 60000,
                durationMs: minutes * 60000,
                updatedAt: DateTime(
                  2026,
                  11,
                  16,
                  20,
                ).subtract(Duration(minutes: watchedMinutes)),
              ),
      );
    }

    final summer = f._category(family, 'Лето');
    final birthdays = f._category(family, 'Дни рождения');
    f._videos[family.id]!.addAll([
      v(
        'Дача, июль',
        14,
        stars: 5,
        tags: ['дача'],
        categories: [summer],
        daysAgo: 2,
      ),
      v(
        'Море 2025',
        38,
        stars: 4,
        categories: [summer],
        daysAgo: 30,
        watchedMinutes: 12,
      ),
      v(
        'Аня — 5 лет',
        22,
        stars: 5,
        tags: ['аня'],
        categories: [birthdays],
        daysAgo: 60,
      ),
      v('Первый снег', 6, stars: 3, daysAgo: 90),
      v('Выпускной', 95, daysAgo: 120, watchedMinutes: 41),
    ]);
    f._videos[home.id]!.addAll([
      v('Камин, 1 час', 60, tags: ['фон'], daysAgo: 5),
      v('Аквариум', 45, tags: ['фон'], daysAgo: 6),
    ]);
    f._folders[home.id]!.add(const PersonalFolder(7, '/Фон'));
    f.lockAllNow();
    return f;
  }

  /// Called when an item of the main library moves in or back.
  final void Function(String itemId, bool toPersonal)? onMove;

  final _changes = StreamController<void>.broadcast();
  final _libraries = <int, PersonalLibrary>{};
  final _secrets = <int, String?>{};
  final _unlocked = <int>{};
  final _videos = <int, List<PersonalVideo>>{};
  final _folders = <int, List<PersonalFolder>>{};
  final _categories = <int, List<MediaCategory>>{};
  final _content = <int, SourceContent>{};

  /// Main-library item ids by the file id their video got.
  final _moved = <int, String>{};
  var _nextId = 1;
  var _nextFile = 1000;
  var _failures = 0;

  /// Source content calls, for tests.
  final contentCalls = <(int, SourceContent, int?)>[];

  PersonalLibrary _add(String name, PersonalLock lock, String? secret) {
    final id = _nextId++;
    final l = PersonalLibrary(id: id, name: name, lock: lock);
    _libraries[id] = l;
    _secrets[id] = secret;
    _videos[id] = [];
    _folders[id] = [];
    _categories[id] = [];
    _unlocked.add(id);
    return l;
  }

  int _category(PersonalLibrary l, String name) {
    final id = _nextId++;
    _categories[l.id]!.add(MediaCategory(id: '$id', name: name));
    return id;
  }

  void _changed() {
    if (!_changes.isClosed) _changes.add(null);
  }

  void lockAllNow() =>
      _unlocked.removeWhere((id) => _libraries[id]?.lock != PersonalLock.none);

  bool _isOpen(int id) =>
      _libraries[id]?.lock == PersonalLock.none || _unlocked.contains(id);

  void _require(int id) {
    if (!_libraries.containsKey(id)) throw StateError('no library $id');
    if (!_isOpen(id)) throw PersonalLocked('library $id is locked');
  }

  PersonalLibrary _info(PersonalLibrary l) {
    final open = _isOpen(l.id);
    return l.copyWith(
      locked: !open,
      itemCount: open ? _videos[l.id]!.length : null,
      folders: open ? List.of(_folders[l.id]!) : const [],
    );
  }

  @override
  Stream<void> get changes => _changes.stream;

  @override
  Future<List<PersonalLibrary>> libraries() async => [
    for (final l in _libraries.values) _info(l),
  ];

  @override
  Future<PersonalLibrary> createLibrary(
    String name, {
    PersonalLock lock = PersonalLock.none,
    String? secret,
  }) async {
    if (name.trim().isEmpty) throw ArgumentError('name');
    if (lock == PersonalLock.pin &&
        !RegExp(r'^\d{4,8}$').hasMatch(secret ?? '')) {
      throw ArgumentError('PIN');
    }
    if (lock == PersonalLock.password && (secret ?? '').length < 6) {
      throw ArgumentError('password');
    }
    final l = _add(name.trim(), lock, secret);
    _changed();
    return _info(l);
  }

  @override
  Future<void> renameLibrary(int id, String name) async {
    _require(id);
    _libraries[id] = _libraries[id]!.copyWith(name: name.trim());
    _changed();
  }

  @override
  Future<void> setLock(int id, PersonalLock lock, {String? secret}) async {
    _require(id);
    _libraries[id] = _libraries[id]!.copyWith(lock: lock);
    _secrets[id] = secret;
    _unlocked.add(id);
    _changed();
  }

  @override
  Future<void> deleteLibrary(int id) async {
    _require(id);
    for (final v in _videos[id]!) {
      if (_moved.remove(v.fileId) case final item?) onMove?.call(item, false);
    }
    _libraries.remove(id);
    _videos.remove(id);
    _changed();
  }

  @override
  Future<PinResult> unlock(int id, String secret) async {
    final l = _libraries[id];
    if (l == null) throw StateError('no library $id');
    if (l.lock == PersonalLock.none || _secrets[id] == secret) {
      _failures = 0;
      _unlocked.add(id);
      _changed();
      return const PinResult(ok: true);
    }
    _failures++;
    return PinResult(ok: false, attempts: PinAttempts(failures: _failures));
  }

  @override
  Future<void> lockLibrary(int id) async {
    _unlocked.remove(id);
    _changed();
  }

  @override
  Future<void> lockAll() async {
    lockAllNow();
    _changed();
  }

  @override
  Future<SourceContent> sourceContent(int sourceId) async =>
      _content[sourceId] ?? SourceContent.video;

  @override
  Future<void> setSourceContent(
    int sourceId,
    SourceContent content, {
    int? libraryId,
  }) async {
    if (content == SourceContent.personal) {
      if (libraryId == null) throw ArgumentError('library');
      _require(libraryId);
      _folders[libraryId]!.add(PersonalFolder(sourceId, '/'));
    }
    _content[sourceId] = content;
    contentCalls.add((sourceId, content, libraryId));
    _changed();
  }

  @override
  Future<void> addFolder(int libraryId, int sourceId, String path) async {
    _require(libraryId);
    final clean = '/${path.replaceAll(RegExp(r'^/+|/+$'), '')}';
    _folders[libraryId]!.add(PersonalFolder(sourceId, clean));
    _changed();
  }

  @override
  Future<void> removeFolder(int libraryId, int sourceId, String path) async {
    _require(libraryId);
    _folders[libraryId]!.removeWhere((f) => f.sourceId == sourceId);
    _changed();
  }

  /// Titles of the items of the main library, for [moveToPersonal].
  String Function(String itemId) titleOf = (id) => id;

  @override
  Future<void> moveToPersonal(String itemId, int libraryId) async {
    _require(libraryId);
    final file = _nextFile++;
    _moved[file] = itemId;
    _videos[libraryId]!.add(
      PersonalVideo(
        fileId: file,
        labelId: itemId,
        sourceId: 1,
        relPath: '$itemId.mkv',
        title: titleOf(itemId),
        addedAt: DateTime(2026, 11, 17),
        moved: true,
      ),
    );
    onMove?.call(itemId, true);
    _changed();
  }

  @override
  Future<void> moveToMain(int libraryId, int fileId) async {
    _require(libraryId);
    final item = _moved.remove(fileId);
    if (item == null) throw StateError('file $fileId was not moved');
    _videos[libraryId]!.removeWhere((v) => v.fileId == fileId);
    onMove?.call(item, false);
    _changed();
  }

  @override
  Future<List<PersonalVideo>> videos(
    int libraryId, {
    PersonalFilter filter = const PersonalFilter(),
    PersonalSort sort = PersonalSort.added,
  }) async {
    _require(libraryId);
    final out = [
      for (final v in _videos[libraryId]!)
        if (filter.matches(v)) v,
    ];
    int byTitle(PersonalVideo a, PersonalVideo b) =>
        a.title.toLowerCase().compareTo(b.title.toLowerCase());
    out.sort(switch (sort) {
      PersonalSort.added => (a, b) => b.addedAt.compareTo(a.addedAt),
      PersonalSort.title => byTitle,
      PersonalSort.rating => (a, b) => (b.stars ?? 0).compareTo(a.stars ?? 0),
      PersonalSort.duration => (a, b) => (b.durationMs ?? 0).compareTo(
        a.durationMs ?? 0,
      ),
      PersonalSort.size => (a, b) => b.sizeBytes.compareTo(a.sizeBytes),
    });
    return out;
  }

  /// Position reports, for tests: (library, file, position).
  final progressCalls = <(int, int, int)>[];

  @override
  Future<bool> saveProgress(
    int libraryId,
    PersonalVideo video,
    int positionMs,
    int durationMs, {
    bool watched = false,
  }) async {
    _require(libraryId);
    progressCalls.add((libraryId, video.fileId, positionMs));
    final complete = durationMs > 0 && positionMs > durationMs * 0.95;
    final list = _videos[libraryId]!;
    for (var i = 0; i < list.length; i++) {
      if (list[i].fileId != video.fileId) continue;
      list[i] = list[i].copyWith(
        progress: () => PersonalProgress(
          positionMs: positionMs,
          durationMs: durationMs > 0 ? durationMs : list[i].durationMs,
          watched: watched || complete,
          updatedAt: DateTime.now(),
        ),
      );
    }
    return true;
  }

  @override
  Future<void> clearProgress(int libraryId, PersonalVideo video) async {
    _require(libraryId);
    _update(
      libraryId,
      (v) => v.fileId == video.fileId,
      (v) => v.copyWith(progress: () => null),
    );
  }

  @override
  Future<List<PersonalVideo>> continueWatching(
    int libraryId, {
    int limit = 20,
  }) async {
    _require(libraryId);
    final started = [
      for (final v in _videos[libraryId]!)
        if (v.progress case final p?)
          if (!p.watched && (p.fraction ?? 0) >= 0.02 && p.fraction! <= 0.95) v,
    ]..sort((a, b) => b.progress!.updatedAt.compareTo(a.progress!.updatedAt));
    return started.take(limit).toList();
  }

  @override
  Future<List<String>> playUris(int libraryId, PersonalVideo video) async {
    _require(libraryId);
    return ['/media/${video.relPath}'];
  }

  @override
  Future<List<MediaCategory>> categories(int libraryId) async {
    _require(libraryId);
    return [
      for (final c in _categories[libraryId]!)
        MediaCategory(
          id: c.id,
          name: c.name,
          itemCount: _videos[libraryId]!
              .where((v) => v.categories.contains(int.parse(c.id)))
              .length,
        ),
    ];
  }

  @override
  Future<MediaCategory> createCategory(int libraryId, String name) async {
    _require(libraryId);
    final id = _category(_libraries[libraryId]!, name.trim());
    _changed();
    return MediaCategory(id: '$id', name: name.trim());
  }

  @override
  Future<void> deleteCategory(int libraryId, int categoryId) async {
    _require(libraryId);
    _categories[libraryId]!.removeWhere((c) => c.id == '$categoryId');
    _update(
      libraryId,
      (v) => true,
      (v) => v.copyWith(categories: [...v.categories]..remove(categoryId)),
    );
  }

  void _update(
    int libraryId,
    bool Function(PersonalVideo) which,
    PersonalVideo Function(PersonalVideo) change,
  ) {
    final list = _videos[libraryId]!;
    for (var i = 0; i < list.length; i++) {
      if (which(list[i])) list[i] = change(list[i]);
    }
    _changed();
  }

  @override
  Future<void> setInCategory(
    int libraryId,
    int categoryId,
    String labelId,
    bool on,
  ) async {
    _require(libraryId);
    _update(
      libraryId,
      (v) => v.labelId == labelId,
      (v) => v.copyWith(
        categories: on
            ? {...v.categories, categoryId}.toList()
            : ([...v.categories]..remove(categoryId)),
      ),
    );
  }

  @override
  Future<List<MediaTag>> tags(int libraryId) async {
    _require(libraryId);
    final counts = <String, int>{};
    for (final v in _videos[libraryId]!) {
      for (final t in v.tags) {
        counts[t] = (counts[t] ?? 0) + 1;
      }
    }
    final names = counts.keys.toList()..sort();
    return [for (final n in names) MediaTag(n, counts[n]!)];
  }

  @override
  Future<void> setTags(int libraryId, String labelId, List<String> tags) async {
    _require(libraryId);
    _update(
      libraryId,
      (v) => v.labelId == labelId,
      (v) => v.copyWith(tags: List.of(tags)),
    );
  }

  @override
  Future<void> setRating(int libraryId, String labelId, int? stars) async {
    _require(libraryId);
    _update(
      libraryId,
      (v) => v.labelId == labelId,
      (v) => v.copyWith(stars: () => stars),
    );
  }
}
