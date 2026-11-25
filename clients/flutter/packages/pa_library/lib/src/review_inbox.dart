// SPDX-License-Identifier: MPL-2.0

// The review inbox («На проверку», META-016): works whose recognition the
// user should check (a result is shown with a «проверьте» mark) or make
// (no result was good enough), the provider results to choose from with
// the signals behind each score, and the user's answers. Answers are
// final: no later scan or metadata run decides those works again.
//
// [RustReviewInbox] implements it on the Rust core (pa-catalog review).

import 'dart:async';

import 'package:pa_core/pa_core.dart';

import 'rust_library.dart';

/// Why a work is in the inbox.
enum ReviewState {
  /// A result is shown; the user should check it.
  check,

  /// No result was good enough; the user decides.
  manual,
}

/// What made a candidate score what it did.
class MatchSignals {
  const MatchSignals({
    this.provider = 'tmdb',
    this.title = 0,
    this.titleSimilarity,
    this.yearAdjust,
    this.remakeAdjust,
    this.kindAdjust,
    this.tier = '',
    this.matchedTitle,
    this.fileYear,
    this.year,
    this.fileKind = '',
    this.kind = '',
    this.durationError,
    this.episodesOk = 0,
    this.episodesBad = 0,
    this.language,
    this.tied = false,
    this.lostMarker,
  });

  /// `tmdb`, `anime_offline_database`, `anilist`.
  final String provider;

  /// Title score with the year, remake and type adjustments, 0..1.
  final double title;

  /// Similarity of the titles alone, 0..1 (META-036); null for matches
  /// stored before it, where [title] is all there is.
  final double? titleSimilarity;

  /// What the years (±1 or further apart), the remake rule (a movie of
  /// another year) and the type added to [title], e.g. −0.08 (META-036).
  final double? yearAdjust;
  final double? remakeAdjust;
  final double? kindAdjust;

  /// `exact`, `loose`, `compact`, `base`, `head`, `fuzzy`.
  final String tier;
  final String? matchedTitle;

  /// Year in the file names, and of the candidate.
  final int? fileYear;
  final int? year;

  /// `movie`, `show`, `anime`; the candidate: `movie` or `tv`.
  final String fileKind;
  final String kind;

  /// |duration − runtime| / runtime.
  final double? durationError;
  final int episodesOk;
  final int episodesBad;

  /// An audio track is in the candidate's original language.
  final bool? language;

  /// Another work scores the same.
  final bool tied;

  /// A word of the file names (`ova`, `movie`) the candidate lacks.
  final String? lostMarker;
}

/// A provider result offered for a work.
class MatchCandidate {
  const MatchCandidate({
    this.provider = 'tmdb',
    required this.extKind,
    required this.extId,
    required this.title,
    this.originalTitle,
    this.year,
    this.posterUrl,
    this.overview,
    this.score = 0,
    this.signals = const MatchSignals(),
    this.current = false,
  });

  final String provider;

  /// TMDB `movie` / `tv`.
  final String extKind;
  final String extId;
  final String title;
  final String? originalTitle;
  final int? year;
  final String? posterUrl;
  final String? overview;

  /// 0..1.
  final double score;
  final MatchSignals signals;

  /// The work shows this result now.
  final bool current;

  /// The same provider result.
  bool sameAs(MatchCandidate o) =>
      o.provider == provider && o.extKind == extKind && o.extId == extId;

  @override
  String toString() => 'MatchCandidate($provider:$extKind:$extId "$title")';
}

/// A work in the inbox (or, for «Исправить распознавание», any work).
class ReviewEntry {
  const ReviewEntry({
    required this.workId,
    required this.itemId,
    this.kind = 'movie',
    required this.state,
    required this.title,
    String? localTitle,
    this.year,
    this.posterUrl,
    this.confidence,
    this.signals,
    this.files = 1,
    this.samplePath = '',
    this.candidates = const [],
  }) : localTitle = localTitle ?? title;

  final int workId;

  /// Library item id (`m<work>`, `s<work>`).
  final String itemId;

  /// `movie`, `show`, `anime`.
  final String kind;
  final ReviewState state;

  /// Shown now.
  final String title;

  /// What the files are named.
  final String localTitle;
  final int? year;
  final String? posterUrl;
  final double? confidence;
  final MatchSignals? signals;
  final int files;

  /// One file, relative to its source.
  final String samplePath;

  /// Best first; the shown one is [MatchCandidate.current].
  final List<MatchCandidate> candidates;

  MatchCandidate? get current => candidates.where((c) => c.current).firstOrNull;

  @override
  String toString() => 'ReviewEntry($workId "$title" $state)';
}

/// What a fix did (UI-012): every file of the work follows it, and the
/// folders whose rule names the work take their new files too.
class FixReport {
  const FixReport({
    required this.workId,
    this.kind = 'movie',
    this.files = 0,
    this.folders = const [],
  });

  /// The work that carries the result now.
  final int workId;

  /// `movie`, `show`, `anime` after the fix.
  final String kind;

  /// Video files that follow the fix.
  final int files;

  /// Folders with a rule (relative to their source; `''` the whole
  /// source).
  final List<String> folders;

  @override
  String toString() => 'FixReport($workId $kind files=$files $folders)';
}

/// What a pasted id or link names (UI-012).
class IdLookup {
  const IdLookup({this.source, this.candidates = const []});

  /// `tmdb`, `tvdb`, `imdb`, `anidb`, `anilist`, `mal`, `shikimori`;
  /// null: the text is not an id or link the app reads.
  final String? source;

  /// Results of the work's kind first; empty: nobody knows the id.
  final List<MatchCandidate> candidates;

  bool get understood => source != null;
}

/// How many works wait.
class ReviewCount {
  const ReviewCount({this.check = 0, this.manual = 0});
  final int check;
  final int manual;
  int get total => check + manual;
}

/// A folder rule a fix taught (META-017): new files of [folder] follow the
/// fix of the work, with this numbering.
class FolderRule {
  const FolderRule({
    required this.id,
    this.folder = '',
    this.kind,
    this.absolute = false,
    this.season,
    this.offset = 0,
  });

  final int id;

  /// Relative to the source root; `''` the whole source.
  final String folder;

  /// `movie`, `show`, `anime` if the user set the type.
  final String? kind;

  /// The number in a file name is a running count, not an episode of a
  /// season.
  final bool absolute;

  /// Season of the episodes; null: the season written in the name.
  final int? season;

  /// Added to the number in the name.
  final int offset;

  @override
  String toString() =>
      'FolderRule($id "$folder" abs=$absolute s=$season +$offset)';
}

/// The type of a work and its folder rules (META-035).
class FolderRules {
  const FolderRules({required this.kind, this.rules = const []});

  /// `movie`, `show`, `anime`.
  final String kind;
  final List<FolderRule> rules;

  /// The rule whose numbering the work follows now (the first one).
  FolderRule? get numbering => rules.firstOrNull;
}

/// What «Записать NFO» and «Добавить ID в имя папки» would do for a work
/// (LIB-014, ADR-0122).
class KnowledgePlan {
  const KnowledgePlan({
    this.ids = const [],
    this.nfoPaths = const [],
    this.existingNfo = const [],
    this.renames = const [],
    this.readOnly = false,
  });

  /// `tmdb 438631`: the ids an NFO gets; empty — nothing to write yet.
  final List<String> ids;

  /// NFO files the export writes (paths in their sources).
  final List<String> nfoPaths;

  /// Those of [nfoPaths] that exist already: ask before replacing.
  final List<String> existingNfo;

  /// `(folder now, its new name)`.
  final List<(String, String)> renames;

  /// Some files are on a source that cannot be written now.
  final bool readOnly;

  /// Something can be written.
  bool get canExport => ids.isNotEmpty && nfoPaths.isNotEmpty;
}

/// What «Записать NFO» did.
class NfoExportReport {
  const NfoExportReport({
    this.written = 0,
    this.replaced = 0,
    this.skipped = 0,
  });

  final int written;

  /// Old files moved to the source's trash.
  final int replaced;

  /// Existing files left alone.
  final int skipped;
}

/// The review inbox of the library.
abstract interface class ReviewInbox {
  /// The inbox changed (a decision, a metadata run).
  Stream<void> get changes;

  Future<ReviewCount> count();

  /// Shown results to check first, then the manual queue.
  Future<List<ReviewEntry>> entries({int limit = 500});

  /// The inbox entry of an item's work; for a work not in the inbox, its
  /// shown result and the results its last search weighed («Исправить
  /// распознавание»). Null for music and unknown items.
  Future<ReviewEntry?> entryFor(String itemId);

  /// The inbox entry of an item's work, or null when the work is not in
  /// the inbox (its recognition is sure or already decided). The media
  /// card shows its candidates right away (META-034).
  Future<ReviewEntry?> inboxEntryFor(String itemId);

  /// «Верно»: the shown result stays; the folder learns it.
  Future<FixReport> confirm(int workId);

  /// Another result: a candidate, a [search] or a [lookup] result. A
  /// result of another kind switches the work's type; every file of the
  /// work takes it and its folder learns the rule (UI-012).
  Future<FixReport> choose(int workId, MatchCandidate candidate);

  /// Search by title and year: TMDB, ranked with what the work's files
  /// say, and the anime database on the device.
  Future<List<MatchCandidate>> search(int workId, String title, {int? year});

  /// The results a pasted id or page link names: TMDB, TheTVDB, IMDb,
  /// AniDB, MyAnimeList, Shikimori, AniList (UI-012).
  Future<IdLookup> lookup(int workId, String text);

  /// «Это не фильм»: the files stay under their own names and are never
  /// recognized again.
  Future<void> notMedia(int workId);

  /// The type of an item's work and the folder rules its fixes taught
  /// (META-035). Null for music and unknown items.
  Future<FolderRules?> folderRules(String itemId);

  /// «Нумерация» for the whole folder of an item's work: the number in
  /// each file name plus [offset] is the episode of [season] (null: the
  /// season written in the name), or with [absolute] the running count.
  Future<void> setNumbering(
    String itemId, {
    int? season,
    int offset = 0,
    bool absolute = false,
  });

  /// «Тип»: `movie`, `show` or `anime` for the whole folder.
  Future<void> setKind(String itemId, String kind);

  /// «Забыть правило папки»: new files are recognized as before;
  /// decisions already made stay. Returns how many rules went.
  Future<int> forgetFolderRules(String itemId);

  /// What the knowledge export would do for an item's work (LIB-014).
  /// Null for music and unknown items.
  Future<KnowledgePlan?> knowledgePlan(String itemId);

  /// «Записать NFO»: Kodi/Jellyfin NFO next to the work's files. An
  /// existing file stays unless [replace]; with [replace] it goes to the
  /// source's trash first.
  Future<NfoExportReport> exportNfo(String itemId, {bool replace = false});

  /// «Добавить ID в имя папки»: the work's own folders get ` [tmdbid-…]`.
  /// Returns how many were renamed.
  Future<int> renameWithIds(String itemId);
}

/// [ReviewInbox] on the Rust core of [library].
class RustReviewInbox implements ReviewInbox {
  RustReviewInbox(this.library);

  final RustLibrary library;
  final _changes = StreamController<void>.broadcast();

  CoreLibrary get _core => library.core;

  /// Its own decisions and every change of the library: a metadata run
  /// that puts works into the inbox (or takes them out) changes no
  /// decision here, but the home line and the Settings count must follow
  /// it (BUG-012).
  @override
  Stream<void> get changes => Stream.multi((c) {
    final own = _changes.stream.listen(c.add);
    final library = this.library.dataChanges.listen(c.add);
    c.onCancel = () async {
      await own.cancel();
      await library.cancel();
    };
  });

  void _changed() {
    if (!_changes.isClosed) _changes.add(null);
  }

  /// After a decision: the views refresh and the chosen result is fetched.
  void _decided({bool fetch = false}) {
    _changed();
    library.notifyExternalChange();
    if (fetch) library.fetchPendingMetadata();
  }

  static MatchSignals _signals(CoreMatchSignals s) => MatchSignals(
    provider: s.provider,
    title: s.title,
    titleSimilarity: s.titleSimilarity,
    yearAdjust: s.yearAdjust,
    remakeAdjust: s.remakeAdjust,
    kindAdjust: s.kindAdjust,
    tier: s.tier,
    matchedTitle: s.matchedTitle,
    fileYear: s.fileYear,
    year: s.year,
    fileKind: s.fileKind,
    kind: s.kind,
    durationError: s.durationError,
    episodesOk: s.episodesOk,
    episodesBad: s.episodesBad,
    language: s.language,
    tied: s.tied,
    lostMarker: s.lostMarker,
  );

  static MatchCandidate _candidate(CoreMatchCandidate c) => MatchCandidate(
    provider: c.provider,
    extKind: c.extKind,
    extId: c.extId,
    title: c.title,
    originalTitle: c.originalTitle,
    year: c.year,
    posterUrl: c.posterUrl,
    overview: c.overview,
    score: c.score,
    signals: _signals(c.signals),
    current: c.current,
  );

  static ReviewEntry _entry(CoreReviewItem i) => ReviewEntry(
    workId: i.workId,
    itemId: i.itemId,
    kind: i.kind,
    state: i.state == 'check' ? ReviewState.check : ReviewState.manual,
    title: i.title,
    localTitle: i.localTitle,
    year: i.year,
    posterUrl: i.posterUrl,
    confidence: i.confidence,
    signals: switch (i.signals) {
      final s? => _signals(s),
      null => null,
    },
    files: i.files,
    samplePath: i.samplePath,
    candidates: [for (final c in i.candidates) _candidate(c)],
  );

  @override
  Future<ReviewCount> count() async {
    final n = await _core.reviewCount();
    return ReviewCount(check: n.check, manual: n.manual);
  }

  @override
  Future<List<ReviewEntry>> entries({int limit = 500}) async => [
    for (final i in await _core.reviewItems(limit: limit)) _entry(i),
  ];

  @override
  Future<ReviewEntry?> inboxEntryFor(String itemId) async {
    final i = await _core.reviewItem(itemId: itemId);
    return i == null ? null : _entry(i);
  }

  @override
  Future<ReviewEntry?> entryFor(String itemId) async {
    final inbox = await _core.reviewItem(itemId: itemId);
    if (inbox != null) return _entry(inbox);
    final item = await library.item(itemId);
    if (item == null) return null;
    final workItem = item.showId ?? item.id;
    final workId = int.tryParse(workItem.substring(1));
    if (workId == null ||
        !(workItem.startsWith('m') || workItem.startsWith('s'))) {
      return null;
    }
    final work = workItem == item.id ? item : await library.item(workItem);
    final candidates = [
      for (final c in await _core.matchCandidates(itemId: itemId))
        _candidate(c),
    ];
    return ReviewEntry(
      workId: workId,
      itemId: workItem,
      kind: workItem.startsWith('m') ? 'movie' : 'show',
      state: candidates.any((c) => c.current)
          ? ReviewState.check
          : ReviewState.manual,
      title: work?.title ?? item.title,
      year: work?.year,
      posterUrl: work?.posterUrl,
      confidence: work?.matchConfidence,
      samplePath: item.filePath,
      candidates: candidates,
    );
  }

  static FixReport _report(CoreFixReport r) => FixReport(
    workId: r.workId,
    kind: r.kind,
    files: r.files,
    folders: r.folders,
  );

  @override
  Future<FixReport> confirm(int workId) async {
    await _core.reviewConfirm(workId: workId);
    _decided();
    return _report(await _core.reviewFixReport(workId: workId));
  }

  @override
  Future<FixReport> choose(int workId, MatchCandidate candidate) async {
    final r = await _core.reviewApply(
      workId: workId,
      provider: candidate.provider,
      extKind: candidate.extKind,
      extId: candidate.extId,
    );
    _decided(fetch: true);
    return _report(r);
  }

  @override
  Future<IdLookup> lookup(int workId, String text) async {
    final r = await _core.reviewLookup(
      workId: workId,
      settings: library.metadataSettings(),
      text: text,
    );
    return IdLookup(
      source: r.source,
      candidates: [for (final c in r.candidates) _candidate(c)],
    );
  }

  @override
  Future<List<MatchCandidate>> search(
    int workId,
    String title, {
    int? year,
  }) async => [
    for (final c in await _core.reviewSearch(
      workId: workId,
      settings: library.metadataSettings(),
      title: title,
      year: year,
    ))
      _candidate(c),
  ];

  @override
  Future<void> notMedia(int workId) async {
    await _core.reviewNotMedia(workId: workId);
    _decided();
  }

  @override
  Future<FolderRules?> folderRules(String itemId) async {
    if (!(itemId.startsWith('m') ||
        itemId.startsWith('s') ||
        itemId.startsWith('e'))) {
      return null;
    }
    final r = await _core.folderRules(itemId: itemId);
    return FolderRules(
      kind: r.kind,
      rules: [
        for (final x in r.rules)
          FolderRule(
            id: x.id,
            folder: x.folder,
            kind: x.kind,
            absolute: x.absolute,
            season: x.season,
            offset: x.offset,
          ),
      ],
    );
  }

  @override
  Future<void> setNumbering(
    String itemId, {
    int? season,
    int offset = 0,
    bool absolute = false,
  }) async {
    await _core.setNumbering(
      itemId: itemId,
      season: season,
      offset: offset,
      absolute: absolute,
    );
    _decided(fetch: true);
  }

  @override
  Future<void> setKind(String itemId, String kind) async {
    await _core.setWorkKind(itemId: itemId, kind: kind);
    _decided(fetch: true);
  }

  @override
  Future<int> forgetFolderRules(String itemId) async {
    final n = await _core.forgetFolderRules(itemId: itemId);
    _changed();
    return n;
  }

  @override
  Future<KnowledgePlan?> knowledgePlan(String itemId) async {
    if (!(itemId.startsWith('m') ||
        itemId.startsWith('s') ||
        itemId.startsWith('e'))) {
      return null;
    }
    final p = await _core.knowledgePlan(itemId: itemId);
    return KnowledgePlan(
      ids: p.ids,
      nfoPaths: [for (final t in p.nfo) t.path],
      existingNfo: [
        for (final t in p.nfo)
          if (t.exists) t.path,
      ],
      renames: [for (final t in p.rename) (t.path, t.newName)],
      readOnly: p.readOnly,
    );
  }

  @override
  Future<NfoExportReport> exportNfo(
    String itemId, {
    bool replace = false,
  }) async {
    final r = await _core.exportNfo(itemId: itemId, replace: replace);
    library.notifyExternalChange();
    return NfoExportReport(
      written: r.written,
      replaced: r.replaced,
      skipped: r.skipped,
    );
  }

  @override
  Future<int> renameWithIds(String itemId) async {
    final n = await _core.renameWithIds(itemId: itemId);
    library.notifyExternalChange();
    return n;
  }

  Future<void> close() => _changes.close();
}
