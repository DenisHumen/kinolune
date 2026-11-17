// SPDX-License-Identifier: MPL-2.0

import 'dart:async';

import '../integration/library.dart';

/// In-memory [ReviewInbox] for the demo mode and widget tests (META-016).
class FakeReviewInbox implements ReviewInbox {
  FakeReviewInbox({
    List<ReviewEntry> entries = const [],
    this.catalog = const [],
  }) : _entries = [...entries];

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
  );

  final List<ReviewEntry> _entries;

  /// What [search] finds (by a word of the title or original title).
  final List<MatchCandidate> catalog;

  /// Decisions in order: `confirm:<work>`, `choose:<work>:<ext id>`,
  /// `not_media:<work>`, `search:<work>:<title>`.
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

  @override
  Future<void> confirm(int workId) async => _decided(workId, 'confirm:$workId');

  @override
  Future<void> choose(int workId, MatchCandidate candidate) async =>
      _decided(workId, 'choose:$workId:${candidate.extId}');

  @override
  Future<void> notMedia(int workId) async =>
      _decided(workId, 'not_media:$workId');

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
