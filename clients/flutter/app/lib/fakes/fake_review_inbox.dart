// SPDX-License-Identifier: MPL-2.0

import 'dart:async';

import '../integration/library.dart';

/// Frieren on TMDB, as a pasted TheTVDB link names it (UI-012 demo).
const demoFrierenTmdb = MatchCandidate(
  extKind: 'tv',
  extId: '209867',
  title: 'Провожающая в последний путь Фрирен',
  originalTitle: 'Sousou no Frieren',
  year: 2023,
);

/// Frieren on AniList, as a pasted MyAnimeList page names it.
const demoFrierenAniList = MatchCandidate(
  provider: 'anilist',
  extKind: '',
  extId: '154587',
  title: 'Sousou no Frieren',
  year: 2023,
);

/// In-memory [ReviewInbox] for the demo mode and widget tests (META-016).
class FakeReviewInbox implements ReviewInbox {
  FakeReviewInbox({
    List<ReviewEntry> entries = const [],
    this.catalog = const [],
    this.ids = const {},
    Map<String, FolderRules> folderRules = const {},
    Map<String, KnowledgePlan> knowledge = const {},
  }) : _entries = [...entries],
       _rules = {...folderRules},
       _knowledge = {...knowledge};

  /// The demo: a shown result to check, a weak one and a file nobody knows.
  factory FakeReviewInbox.demo() => FakeReviewInbox(
    entries: [
      const ReviewEntry(
        workId: 901,
        itemId: 's901',
        kind: 'anime',
        state: ReviewState.check,
        title: 'Хвост феи',
        localTitle: 'Fairy Tail OVA',
        confidence: 0.85,
        files: 9,
        samplePath:
            'Anime/Fairy Tail OVA/[Doki] Fairy Tail OVA - 01 (720p).mkv',
        signals: MatchSignals(
          title: 0.85,
          tier: 'exact',
          matchedTitle: 'Fairy Tail',
          fileKind: 'anime',
          kind: 'tv',
          year: 2009,
          lostMarker: 'ova',
          language: true,
        ),
        candidates: [
          MatchCandidate(
            extKind: 'tv',
            extId: '46261',
            title: 'Хвост феи',
            originalTitle: 'FAIRY TAIL',
            year: 2009,
            score: 0.85,
            current: true,
            signals: MatchSignals(
              title: 0.85,
              tier: 'exact',
              matchedTitle: 'Fairy Tail',
              fileKind: 'anime',
              kind: 'tv',
              year: 2009,
              lostMarker: 'ova',
              language: true,
            ),
          ),
          MatchCandidate(
            extKind: 'tv',
            extId: '60866',
            title: 'Хвост феи: Сто лет',
            originalTitle: 'FAIRY TAIL 100 YEARS QUEST',
            year: 2024,
            score: 0.61,
            signals: MatchSignals(
              title: 0.61,
              tier: 'fuzzy',
              fileKind: 'anime',
              kind: 'tv',
              year: 2024,
            ),
          ),
        ],
      ),
      const ReviewEntry(
        workId: 902,
        itemId: 'm902',
        state: ReviewState.manual,
        title: 'Steins Gate Load Region',
        year: 2013,
        samplePath: 'Movies/Steins Gate Load Region (2013).mkv',
        candidates: [
          MatchCandidate(
            extKind: 'movie',
            extId: '212162',
            title: 'Врата Штейна: Нагрузочная область дежавю',
            originalTitle: '劇場版 STEINS;GATE 負荷領域のデジャヴ',
            year: 2013,
            score: 0.64,
            signals: MatchSignals(
              title: 0.64,
              tier: 'fuzzy',
              fileYear: 2013,
              year: 2013,
              fileKind: 'movie',
              kind: 'movie',
              durationError: 0.03,
            ),
          ),
        ],
      ),
      const ReviewEntry(
        workId: 903,
        itemId: 'm903',
        state: ReviewState.manual,
        title: 'VID 20240612',
        samplePath: 'Movies/VID_20240612_181455.mp4',
      ),
    ],
    catalog: const [
      MatchCandidate(
        extKind: 'movie',
        extId: '27205',
        title: 'Начало',
        originalTitle: 'Inception',
        year: 2010,
      ),
    ],
    // UI-012: a pasted TheTVDB link or MyAnimeList page of Frieren.
    ids: const {'424536': demoFrierenTmdb, '52991': demoFrierenAniList},
    // META-035: the OVA folder already follows a fix.
    folderRules: const {
      's901': FolderRules(
        kind: 'anime',
        rules: [FolderRule(id: 1, folder: 'Anime/Fairy Tail OVA')],
      ),
    },
    // LIB-014: the OVA folder can get its NFO and the id in its name.
    knowledge: const {
      's901': KnowledgePlan(
        ids: ['tmdb 46261'],
        nfoPaths: ['/Anime/Fairy Tail OVA/tvshow.nfo'],
        renames: [('/Anime/Fairy Tail OVA', 'Fairy Tail OVA [tmdbid-46261]')],
      ),
    },
  );

  final List<ReviewEntry> _entries;

  /// Type and folder rules by item id (META-035); a work without an
  /// entry here is a series of its entry's kind without rules.
  final Map<String, FolderRules> _rules;

  /// What the knowledge export finds, by item id (LIB-014); an item
  /// without an entry has no ids yet.
  final Map<String, KnowledgePlan> _knowledge;

  /// What [search] finds (by a word of the title or original title).
  final List<MatchCandidate> catalog;

  /// What [lookup] finds by the number in a pasted link or id (UI-012).
  final Map<String, MatchCandidate> ids;

  /// Decisions in order: `confirm:<work>`, `choose:<work>:<ext id>`,
  /// `not_media:<work>`, `search:<work>:<title>`,
  /// `numbering:<item>:<season>:<offset>:<abs|ep>`, `kind:<item>:<kind>`,
  /// `forget:<item>`, `nfo:<item>:<keep|replace>`, `rename_ids:<item>`,
  /// `lookup:<work>:<text>`.
  final List<String> log = [];

  final _changes = StreamController<void>.broadcast();

  @override
  Stream<void> get changes => _changes.stream;

  void _decided(int workId, String entry) {
    log.add(entry);
    _entries.removeWhere((e) => e.workId == workId);
    _changes.add(null);
  }

  @override
  Future<ReviewCount> count() async => ReviewCount(
    check: _entries.where((e) => e.state == ReviewState.check).length,
    manual: _entries.where((e) => e.state == ReviewState.manual).length,
  );

  @override
  Future<List<ReviewEntry>> entries({int limit = 500}) async =>
      _entries.take(limit).toList();

  @override
  Future<ReviewEntry?> entryFor(String itemId) async =>
      _entries.where((e) => e.itemId == itemId).firstOrNull;

  @override
  Future<ReviewEntry?> inboxEntryFor(String itemId) => entryFor(itemId);

  /// What a fix of [workId] covers: the entry's files and its folder (the
  /// folder rule's, else the folder of its sample file).
  FixReport _report(int workId, {String? kind}) {
    final e = _entries.where((e) => e.workId == workId).firstOrNull;
    if (e == null) return FixReport(workId: workId, kind: kind ?? 'movie');
    final rule = _rules[e.itemId]?.rules.firstOrNull?.folder;
    final slash = e.samplePath.lastIndexOf('/');
    final folder =
        rule ?? (slash > 0 ? e.samplePath.substring(0, slash) : null);
    return FixReport(
      workId: workId,
      kind: kind ?? e.kind,
      files: e.files,
      folders: [?folder],
    );
  }

  @override
  Future<FixReport> confirm(int workId) async {
    final r = _report(workId);
    _decided(workId, 'confirm:$workId');
    return r;
  }

  @override
  Future<FixReport> choose(int workId, MatchCandidate candidate) async {
    final e = _entries.where((e) => e.workId == workId).firstOrNull;
    final kind = switch ((candidate.provider, candidate.extKind)) {
      ('anilist', _) => 'anime',
      (_, 'movie') => 'movie',
      _ when e?.kind == 'movie' => 'show',
      _ => e?.kind,
    };
    final r = _report(workId, kind: kind);
    _decided(workId, 'choose:$workId:${candidate.extId}');
    return r;
  }

  /// What [lookup] reads: a link of a known site or a prefixed id; its
  /// number is looked up in [catalog] and the entries' candidates.
  static final _pasted = RegExp(
    r'(themoviedb|thetvdb|imdb|anidb|myanimelist|shikimori|anilist|'
    r'^tmdb|^tvdb|^mal\b|^shiki|^\d+$)',
  );

  @override
  Future<IdLookup> lookup(int workId, String text) async {
    log.add('lookup:$workId:${text.trim()}');
    final t = text.trim().toLowerCase();
    final site = _pasted.firstMatch(t)?.group(1);
    if (site == null) return const IdLookup();
    final source = switch (site) {
      'themoviedb' || 'tmdb' => 'tmdb',
      'thetvdb' || 'tvdb' => 'tvdb',
      'myanimelist' || 'mal' => 'mal',
      'shiki' || 'shikimori' => 'shikimori',
      _ when RegExp(r'^\d+$').hasMatch(site) => 'tmdb',
      _ => site,
    };
    final id = RegExp(r'\d+').allMatches(t).lastOrNull?.group(0);
    final known = [
      ?ids[id],
      for (final c in [...catalog, for (final e in _entries) ...e.candidates])
        if (c.extId == id) c,
    ];
    return IdLookup(
      source: source,
      candidates: [
        for (final c in known)
          MatchCandidate(
            provider: c.provider,
            extKind: c.extKind,
            extId: c.extId,
            title: c.title,
            originalTitle: c.originalTitle,
            year: c.year,
            posterUrl: c.posterUrl,
            overview: c.overview,
            score: 1,
            signals: MatchSignals(provider: source, title: 1, tier: 'id'),
          ),
      ].take(1).toList(),
    );
  }

  @override
  Future<void> notMedia(int workId) async =>
      _decided(workId, 'not_media:$workId');

  FolderRules _of(String itemId) =>
      _rules[itemId] ??
      FolderRules(
        kind:
            _entries.where((e) => e.itemId == itemId).firstOrNull?.kind ??
            (itemId.startsWith('m') ? 'movie' : 'show'),
      );

  @override
  Future<FolderRules?> folderRules(String itemId) async =>
      itemId.startsWith('t') ? null : _of(itemId);

  @override
  Future<void> setNumbering(
    String itemId, {
    int? season,
    int offset = 0,
    bool absolute = false,
  }) async {
    log.add(
      'numbering:$itemId:${season ?? '-'}:$offset:${absolute ? 'abs' : 'ep'}',
    );
    final r = _of(itemId);
    final old = r.numbering;
    _rules[itemId] = FolderRules(
      kind: r.kind,
      rules: [
        FolderRule(
          id: old?.id ?? 1,
          folder: old?.folder ?? '',
          kind: old?.kind,
          absolute: absolute,
          season: season,
          offset: offset,
        ),
        ...r.rules.skip(1),
      ],
    );
    _changes.add(null);
  }

  @override
  Future<void> setKind(String itemId, String kind) async {
    log.add('kind:$itemId:$kind');
    final r = _of(itemId);
    final old = r.numbering;
    _rules[itemId] = FolderRules(
      kind: kind,
      rules: [
        FolderRule(
          id: old?.id ?? 1,
          folder: old?.folder ?? '',
          kind: kind,
          absolute: old?.absolute ?? false,
          season: old?.season,
          offset: old?.offset ?? 0,
        ),
        ...r.rules.skip(1),
      ],
    );
    _changes.add(null);
  }

  @override
  Future<int> forgetFolderRules(String itemId) async {
    log.add('forget:$itemId');
    final r = _of(itemId);
    _rules[itemId] = FolderRules(kind: r.kind);
    _changes.add(null);
    return r.rules.length;
  }

  @override
  Future<KnowledgePlan?> knowledgePlan(String itemId) async =>
      itemId.startsWith('t')
      ? null
      : _knowledge[itemId] ?? const KnowledgePlan();

  @override
  Future<NfoExportReport> exportNfo(
    String itemId, {
    bool replace = false,
  }) async {
    log.add('nfo:$itemId:${replace ? 'replace' : 'keep'}');
    final p = _knowledge[itemId] ?? const KnowledgePlan();
    final existing = p.existingNfo.length;
    _knowledge[itemId] = KnowledgePlan(
      ids: p.ids,
      nfoPaths: p.nfoPaths,
      existingNfo: p.nfoPaths,
      renames: p.renames,
      readOnly: p.readOnly,
    );
    return NfoExportReport(
      written: p.nfoPaths.length - (replace ? 0 : existing),
      replaced: replace ? existing : 0,
      skipped: replace ? 0 : existing,
    );
  }

  @override
  Future<int> renameWithIds(String itemId) async {
    log.add('rename_ids:$itemId');
    final p = _knowledge[itemId] ?? const KnowledgePlan();
    _knowledge[itemId] = KnowledgePlan(
      ids: p.ids,
      nfoPaths: p.nfoPaths,
      existingNfo: p.existingNfo,
      readOnly: p.readOnly,
    );
    return p.renames.length;
  }

  @override
  Future<List<MatchCandidate>> search(
    int workId,
    String title, {
    int? year,
  }) async {
    log.add('search:$workId:$title');
    final q = title.trim().toLowerCase();
    return [
      for (final c in catalog)
        if (q.isNotEmpty &&
            (c.title.toLowerCase().contains(q) ||
                (c.originalTitle ?? '').toLowerCase().contains(q)) &&
            (year == null || c.year == null || c.year == year))
          c,
    ];
  }
}
