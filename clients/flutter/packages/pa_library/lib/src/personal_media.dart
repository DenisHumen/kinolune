// SPDX-License-Identifier: MPL-2.0

// Personal media (UI-025, «Личные медиа»): the user's own videos in
// personal libraries of the active profile, each with no lock, a PIN or a
// password. Their files are never recognized and never sent to TMDB or
// any provider, and no list of the main library shows them. Items of the
// main library can be moved into a personal library and back.
//
// [RustPersonalMedia] implements it on the Rust core (pa-catalog).

import 'dart:async';
import 'dart:typed_data' show Uint8List;

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show Int64List;
import 'package:pa_core/pa_core.dart';

import 'library_labels.dart' show MediaCategory, MediaTag;
import 'library_profiles.dart' show PinAttempts, PinResult;
import 'rust_library.dart';

/// How a personal library is locked.
enum PersonalLock { none, pin, password }

/// What a source holds, chosen when it is added.
enum SourceContent {
  /// Movies and shows: recognized, with metadata from TMDB.
  video,

  /// Music.
  music,

  /// The user's own videos: never recognized, only in personal media.
  personal,
}

/// Order of the videos of a personal library.
enum PersonalSort { added, title, rating, duration, size }

/// A folder of a personal library.
class PersonalFolder {
  final int sourceId;

  /// Path inside the source, `/` for all of it.
  final String path;

  const PersonalFolder(this.sourceId, this.path);

  @override
  bool operator ==(Object other) =>
      other is PersonalFolder &&
      other.sourceId == sourceId &&
      other.path == path;

  @override
  int get hashCode => Object.hash(sourceId, path);
}

/// A personal library of the active profile.
class PersonalLibrary {
  final int id;
  final String name;
  final PersonalLock lock;

  /// Its contents cannot be read until it is unlocked.
  final bool locked;

  /// Videos in it; null while locked.
  final int? itemCount;

  /// Empty while locked.
  final List<PersonalFolder> folders;

  const PersonalLibrary({
    required this.id,
    required this.name,
    this.lock = PersonalLock.none,
    this.locked = false,
    this.itemCount,
    this.folders = const [],
  });

  PersonalLibrary copyWith({
    String? name,
    PersonalLock? lock,
    bool? locked,
    int? itemCount,
    List<PersonalFolder>? folders,
  }) => PersonalLibrary(
    id: id,
    name: name ?? this.name,
    lock: lock ?? this.lock,
    locked: locked ?? this.locked,
    itemCount: itemCount ?? this.itemCount,
    folders: folders ?? this.folders,
  );

  @override
  String toString() => 'PersonalLibrary($id "$name" $lock locked=$locked)';
}

/// Where a video of a personal library was left (UI-033). Kept in the
/// library: never in the shared "Continue watching", never synced.
class PersonalProgress {
  final int positionMs;

  /// Null if unknown.
  final int? durationMs;
  final bool watched;
  final DateTime updatedAt;

  const PersonalProgress({
    required this.positionMs,
    this.durationMs,
    this.watched = false,
    required this.updatedAt,
  });

  /// 0–1 of a known duration, else null.
  double? get fraction {
    final d = durationMs;
    if (d == null || d <= 0) return null;
    return (positionMs / d).clamp(0.0, 1.0);
  }
}

/// A video of a personal library.
class PersonalVideo {
  final int fileId;

  /// What the label calls take: null until the file is fingerprinted.
  final String? labelId;
  final int sourceId;

  /// Path relative to the source root.
  final String relPath;

  /// The recognized title of a video moved from the main library, else
  /// the file name.
  final String title;
  final int sizeBytes;
  final int? durationMs;
  final DateTime addedAt;

  /// Moved from the main library: it can go back.
  final bool moved;

  /// 1–5.
  final int? stars;
  final List<String> tags;
  final List<int> categories;

  /// Where it was left in this library; null if never played here.
  final PersonalProgress? progress;

  const PersonalVideo({
    required this.fileId,
    this.labelId,
    required this.sourceId,
    required this.relPath,
    required this.title,
    this.sizeBytes = 0,
    this.durationMs,
    required this.addedAt,
    this.moved = false,
    this.stars,
    this.tags = const [],
    this.categories = const [],
    this.progress,
  });

  PersonalVideo copyWith({
    int? Function()? stars,
    List<String>? tags,
    List<int>? categories,
    PersonalProgress? Function()? progress,
  }) => PersonalVideo(
    fileId: fileId,
    labelId: labelId,
    sourceId: sourceId,
    relPath: relPath,
    title: title,
    sizeBytes: sizeBytes,
    durationMs: durationMs,
    addedAt: addedAt,
    moved: moved,
    stars: stars == null ? this.stars : stars(),
    tags: tags ?? this.tags,
    categories: categories ?? this.categories,
    progress: progress == null ? this.progress : progress(),
  );

  @override
  String toString() => 'PersonalVideo($fileId "$title")';
}

/// Which videos of a personal library to list: any value within a field,
/// every field.
class PersonalFilter {
  final List<int> categories;
  final List<String> tags;

  /// Stars; 0 = not rated.
  final List<int> ratings;
  final String? text;

  const PersonalFilter({
    this.categories = const [],
    this.tags = const [],
    this.ratings = const [],
    this.text,
  });

  bool get isEmpty =>
      categories.isEmpty &&
      tags.isEmpty &&
      ratings.isEmpty &&
      (text == null || text!.trim().isEmpty);

  /// Whether [v] passes (the fakes filter with it; the core filters
  /// itself).
  bool matches(PersonalVideo v) {
    final words = (text ?? '').toLowerCase().split(RegExp(r'\s+'))
      ..removeWhere((w) => w.isEmpty);
    final title = v.title.toLowerCase();
    final keys = tags.map((t) => t.trim().toLowerCase()).toSet();
    return (categories.isEmpty || v.categories.any(categories.contains)) &&
        (keys.isEmpty || v.tags.any((t) => keys.contains(t.toLowerCase()))) &&
        (ratings.isEmpty || ratings.contains(v.stars ?? 0)) &&
        words.every(title.contains);
  }
}

/// The library is locked: unlock it first.
class PersonalLocked implements Exception {
  const PersonalLocked([this.message = '']);
  final String message;

  @override
  String toString() => 'PersonalLocked($message)';
}

/// Personal media of the active profile.
abstract interface class PersonalMedia {
  /// Something changed (a library, its videos or labels).
  Stream<void> get changes;

  /// Libraries in the user's order; names are shown while locked.
  Future<List<PersonalLibrary>> libraries();

  /// Creates a library; it starts unlocked. A PIN is 4–8 digits, a
  /// password 6–128 characters.
  Future<PersonalLibrary> createLibrary(
    String name, {
    PersonalLock lock = PersonalLock.none,
    String? secret,
  });
  Future<void> renameLibrary(int id, String name);
  Future<void> setLock(int id, PersonalLock lock, {String? secret});

  /// Deletes an unlocked library; moved videos return to the main library.
  Future<void> deleteLibrary(int id);

  /// Checks the PIN or password; a right one unlocks the library until
  /// [lockLibrary], [lockAll] or the next start.
  Future<PinResult> unlock(int id, String secret);
  Future<void> lockLibrary(int id);

  /// Locks every library (the app went to the background, the profile
  /// changed).
  Future<void> lockAll();

  /// What a source holds.
  Future<SourceContent> sourceContent(int sourceId);

  /// Sets what a source holds; [SourceContent.personal] needs an unlocked
  /// [libraryId]. Call it before the first scan of a new source.
  Future<void> setSourceContent(
    int sourceId,
    SourceContent content, {
    int? libraryId,
  });

  /// Adds a folder of a source (`''` for all of it) to an unlocked library.
  Future<void> addFolder(int libraryId, int sourceId, String path);
  Future<void> removeFolder(int libraryId, int sourceId, String path);

  /// Moves a movie, show or episode of the main library ([itemId] as in
  /// `LibraryItem.id`) into an unlocked library.
  Future<void> moveToPersonal(String itemId, int libraryId);

  /// Moves a video that came from the main library back there.
  Future<void> moveToMain(int libraryId, int fileId);

  /// Videos of an unlocked library.
  Future<List<PersonalVideo>> videos(
    int libraryId, {
    PersonalFilter filter = const PersonalFilter(),
    PersonalSort sort = PersonalSort.added,
  });

  /// What the player opens for a video (every part of a split file), local
  /// paths or stream URLs; empty if it cannot be played here.
  Future<List<String>> playUris(int libraryId, PersonalVideo video);

  /// UI-033: records where [video] of an unlocked library was left
  /// ("watched" when asked or past 95 %). The position stays in the
  /// library. False if the video has no key yet (nothing stored).
  Future<bool> saveProgress(
    int libraryId,
    PersonalVideo video,
    int positionMs,
    int durationMs, {
    bool watched = false,
  });

  /// Forgets where [video] was left: it leaves «Продолжить».
  Future<void> clearProgress(int libraryId, PersonalVideo video);

  /// «Продолжить» of an unlocked library: started, unfinished videos,
  /// latest first.
  Future<List<PersonalVideo>> continueWatching(int libraryId, {int limit});

  Future<List<MediaCategory>> categories(int libraryId);
  Future<MediaCategory> createCategory(int libraryId, String name);
  Future<void> deleteCategory(int libraryId, int categoryId);
  Future<void> setInCategory(
    int libraryId,
    int categoryId,
    String labelId,
    bool on,
  );
  Future<List<MediaTag>> tags(int libraryId);
  Future<void> setTags(int libraryId, String labelId, List<String> tags);
  Future<void> setRating(int libraryId, String labelId, int? stars);
}

/// [PersonalMedia] on the Rust core of [library].
class RustPersonalMedia implements PersonalMedia {
  RustPersonalMedia(this.library);

  final RustLibrary library;
  final _changes = StreamController<void>.broadcast();
  String? _streamBase;

  CoreLibrary get _core => library.core;

  @override
  Stream<void> get changes => _changes.stream;

  void _changed({bool mainLibrary = false}) {
    if (!_changes.isClosed) _changes.add(null);
    if (mainLibrary) library.notifyExternalChange();
  }

  /// Runs a core call; `denied:` errors become [PersonalLocked].
  Future<T> _call<T>(Future<T> Function() f) async {
    try {
      return await f();
    } on LibraryError catch (e) {
      if (e.message.startsWith('denied')) throw PersonalLocked(e.message);
      rethrow;
    }
  }

  static PersonalLock _lock(String s) => switch (s) {
    'pin' => PersonalLock.pin,
    'password' => PersonalLock.password,
    _ => PersonalLock.none,
  };

  static PersonalLibrary _library(CorePersonalLibrary l) => PersonalLibrary(
    id: l.id,
    name: l.name,
    lock: _lock(l.lock),
    locked: l.locked,
    itemCount: l.itemCount,
    folders: [for (final f in l.folders) PersonalFolder(f.sourceId, f.path)],
  );

  static PersonalVideo _video(CorePersonalItem i) => PersonalVideo(
    fileId: i.fileId,
    labelId: i.labelId,
    sourceId: i.sourceId,
    relPath: i.relPath,
    title: i.title,
    sizeBytes: i.sizeBytes,
    durationMs: i.durationMs,
    addedAt: DateTime.fromMillisecondsSinceEpoch(i.addedAtMs),
    moved: i.moved,
    stars: i.stars,
    tags: i.tags,
    categories: [for (final c in i.categories) c.toInt()],
    progress: switch (i.progress) {
      final p? => PersonalProgress(
        positionMs: p.positionMs,
        durationMs: p.durationMs,
        watched: p.watched,
        updatedAt: DateTime.fromMillisecondsSinceEpoch(p.updatedAtMs),
      ),
      null => null,
    },
  );

  static MediaCategory _category(CoreCategory c) =>
      MediaCategory(id: '${c.id}', name: c.name, itemCount: c.itemCount);

  @override
  Future<List<PersonalLibrary>> libraries() async => [
    for (final l in await _core.personalLibraries()) _library(l),
  ];

  @override
  Future<PersonalLibrary> createLibrary(
    String name, {
    PersonalLock lock = PersonalLock.none,
    String? secret,
  }) async {
    final l = await _core.createPersonalLibrary(
      name: name,
      lock: lock.name,
      secret: secret,
    );
    _changed();
    return _library(l);
  }

  @override
  Future<void> renameLibrary(int id, String name) async {
    await _call(() => _core.renamePersonalLibrary(id: id, name: name));
    _changed();
  }

  @override
  Future<void> setLock(int id, PersonalLock lock, {String? secret}) async {
    await _call(
      () => _core.setPersonalLock(id: id, lock: lock.name, secret: secret),
    );
    _changed();
  }

  @override
  Future<void> deleteLibrary(int id) async {
    await _call(() => _core.deletePersonalLibrary(id: id));
    _changed(mainLibrary: true);
  }

  @override
  Future<PinResult> unlock(int id, String secret) async {
    final v = await _core.unlockPersonalLibrary(id: id, secret: secret);
    if (v.ok) _changed();
    return PinResult(
      ok: v.ok,
      locked: v.locked,
      attempts: PinAttempts(
        failures: v.attempts.failures,
        lockedUntil: switch (v.attempts.lockedUntilMs) {
          final ms? => DateTime.fromMillisecondsSinceEpoch(ms),
          null => null,
        },
      ),
    );
  }

  @override
  Future<void> lockLibrary(int id) async {
    await _core.lockPersonalLibrary(id: id);
    _changed();
  }

  @override
  Future<void> lockAll() async {
    await _core.lockPersonalLibraries();
    _changed();
  }

  @override
  Future<SourceContent> sourceContent(int sourceId) async =>
      switch (await _core.sourceContent(sourceId: sourceId)) {
        'music' => SourceContent.music,
        'personal' => SourceContent.personal,
        _ => SourceContent.video,
      };

  @override
  Future<void> setSourceContent(
    int sourceId,
    SourceContent content, {
    int? libraryId,
  }) async {
    await _call(
      () => _core.setSourceContent(
        sourceId: sourceId,
        content: content.name,
        libraryId: libraryId,
      ),
    );
    _changed(mainLibrary: true);
  }

  @override
  Future<void> addFolder(int libraryId, int sourceId, String path) async {
    await _call(
      () => _core.addPersonalFolder(
        id: libraryId,
        sourceId: sourceId,
        path: path,
      ),
    );
    _changed(mainLibrary: true);
  }

  @override
  Future<void> removeFolder(int libraryId, int sourceId, String path) async {
    await _call(
      () => _core.removePersonalFolder(
        id: libraryId,
        sourceId: sourceId,
        path: path,
      ),
    );
    _changed(mainLibrary: true);
  }

  @override
  Future<void> moveToPersonal(String itemId, int libraryId) async {
    await _call(
      () => _core.moveToPersonal(itemId: itemId, libraryId: libraryId),
    );
    _changed(mainLibrary: true);
  }

  @override
  Future<void> moveToMain(int libraryId, int fileId) async {
    await _call(() => _core.moveToMain(libraryId: libraryId, fileId: fileId));
    _changed(mainLibrary: true);
  }

  @override
  Future<List<PersonalVideo>> videos(
    int libraryId, {
    PersonalFilter filter = const PersonalFilter(),
    PersonalSort sort = PersonalSort.added,
  }) => _call(() async {
    final items = await _core.personalItems(
      id: libraryId,
      filter: CorePersonalFilter(
        categories: Int64List.fromList(filter.categories),
        tags: filter.tags,
        ratings: Uint8List.fromList(filter.ratings),
        text: filter.text,
      ),
      sort: sort.name,
    );
    return [for (final i in items) _video(i)];
  });

  @override
  Future<bool> saveProgress(
    int libraryId,
    PersonalVideo video,
    int positionMs,
    int durationMs, {
    bool watched = false,
  }) => _call(
    () => _core.savePersonalProgress(
      id: libraryId,
      fileId: video.fileId,
      positionMs: positionMs,
      durationMs: durationMs,
      updatedAtMs: DateTime.now().millisecondsSinceEpoch,
      watched: watched,
    ),
  );

  @override
  Future<void> clearProgress(int libraryId, PersonalVideo video) async {
    await _call(
      () => _core.clearPersonalProgress(id: libraryId, fileId: video.fileId),
    );
    _changed();
  }

  @override
  Future<List<PersonalVideo>> continueWatching(
    int libraryId, {
    int limit = 20,
  }) => _call(() async {
    final items = await _core.personalContinue(id: libraryId, limit: limit);
    return [for (final i in items) _video(i)];
  });

  @override
  Future<List<String>> playUris(int libraryId, PersonalVideo video) =>
      _call(() async {
        final parts = await _core.personalPlayParts(
          id: libraryId,
          fileId: video.fileId,
        );
        String? base;
        if (parts.any((p) => p.localPath == null)) {
          base = _streamBase ??= await _core.streamBaseUrl();
        }
        return [
          for (final part in parts)
            if (part.localPath case final local?)
              local
            else if (base != null)
              RustLibrary.streamUrl(base, video.sourceId, part.relPath),
        ];
      });

  @override
  Future<List<MediaCategory>> categories(int libraryId) => _call(
    () async => [
      for (final c in await _core.personalCategories(id: libraryId))
        _category(c),
    ],
  );

  @override
  Future<MediaCategory> createCategory(int libraryId, String name) =>
      _call(() async {
        final c = await _core.personalCreateCategory(id: libraryId, name: name);
        _changed();
        return _category(c);
      });

  @override
  Future<void> deleteCategory(int libraryId, int categoryId) => _call(() async {
    await _core.personalDeleteCategory(id: libraryId, category: categoryId);
    _changed();
  });

  @override
  Future<void> setInCategory(
    int libraryId,
    int categoryId,
    String labelId,
    bool on,
  ) => _call(() async {
    await _core.personalSetInCategory(
      id: libraryId,
      category: categoryId,
      labelId: labelId,
      on_: on,
    );
    _changed();
  });

  @override
  Future<List<MediaTag>> tags(int libraryId) => _call(
    () async => [
      for (final t in await _core.personalTags(id: libraryId))
        MediaTag(t.name, t.itemCount),
    ],
  );

  @override
  Future<void> setTags(int libraryId, String labelId, List<String> tags) =>
      _call(() async {
        await _core.personalSetTags(
          id: libraryId,
          labelId: labelId,
          tags: tags,
        );
        _changed();
      });

  @override
  Future<void> setRating(int libraryId, String labelId, int? stars) =>
      _call(() async {
        await _core.personalSetRating(
          id: libraryId,
          labelId: labelId,
          stars: stars,
        );
        _changed();
      });
}
