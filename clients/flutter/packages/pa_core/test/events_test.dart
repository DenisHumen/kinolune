// SPDX-License-Identifier: MPL-2.0
// CORE-009 through the Dart bindings: the core event stream carries scan
// progress and library changes, and core errors carry stable codes.
//
// The bus is one per process and `dart test` runs the other test files in
// the same process: every test looks only at the events of its own
// library (`libraryPath`).
import 'dart:async';
import 'dart:io';

import 'package:pa_core/pa_core.dart';
import 'package:test/test.dart';

import 'native_library.dart';

/// A fake media file with content unique to [seed].
void _media(String root, String rel, int seed) {
  final f = File('$root/$rel')..createSync(recursive: true);
  f.writeAsBytesSync([for (var i = 0; i < 150000; i++) (i * 31 + seed) & 0xff]);
}

Future<void> _until(String what, bool Function() ok) async {
  final deadline = DateTime.now().add(const Duration(seconds: 15));
  while (!ok()) {
    if (DateTime.now().isAfter(deadline)) fail('timed out: $what');
    await Future<void>.delayed(const Duration(milliseconds: 50));
  }
}

/// Cancels [sub]; fails the test instead of hanging when the stream does
/// not end (BUG-007: a cancel waited for the next event of the bus).
Future<void> _cancel(StreamSubscription<CoreEvent> sub) => sub.cancel().timeout(
  const Duration(seconds: 5),
  onTimeout: () => fail('cancelling a coreEvents stream did not complete'),
);

/// Events of the library at [db] from now on, until the test ends.
List<CoreEvent> _listen(String db) {
  final events = <CoreEvent>[];
  final sub = coreEvents().where((e) => e.libraryPath == db).listen(events.add);
  addTearDown(() => _cancel(sub));
  return events;
}

void main() {
  setUpAll(() async {
    await RustLib.init(externalLibrary: openBuiltCoreLibrary());
  });

  tearDownAll(RustLib.dispose);

  late Directory dir;
  late String db;
  setUp(() {
    // The teardown deletes this test's own folder: a late teardown of a
    // timed-out test once deleted the media of the next one (BUG-007).
    final own = Directory.systemTemp.createTempSync('pa_events_test_');
    dir = own;
    db = '${own.path}/library.db';
    addTearDown(() => own.deleteSync(recursive: true));
  });

  test('a cancel ends the stream at once, on a quiet bus too', () async {
    // Nothing publishes while the stream is open: before BUG-007 the
    // cancel waited for the next event, forever.
    var done = false;
    final sub = coreEvents().listen((_) {}, onDone: () => done = true);
    await Future<void>.delayed(const Duration(milliseconds: 50));
    await _cancel(sub);
    expect(done, isFalse, reason: 'a cancelled stream reports no done');
  });

  test('coreEvents receives scan progress and the library change', () async {
    final events = _listen(db);
    final media = '${dir.path}/media';
    _media(media, 'Movies/The Matrix 1999.mkv', 1);
    _media(media, 'Movies/Heat 1995.mkv', 2);
    final library = await CoreLibrary.open(path: db);
    final source = await library.addSource(
      kind: 'local',
      name: 'Media',
      location: media,
    );
    // The scan's own stream and the bus carry the same updates.
    final own = await library.scan(sourceId: source.id).toList();
    expect(own.last.phase, CoreScanPhase.done);

    await _until(
      'library change event',
      () => events.any(
        (e) =>
            e.domain == CoreEventDomain.libraryChanged &&
            e.libraryChanged!.change == CoreLibraryChange.scan &&
            e.libraryChanged!.sourceId == source.id,
      ),
    );

    final scans = events
        .where((e) => e.domain == CoreEventDomain.scanProgress)
        .map((e) => e.scanProgress!)
        .toList();
    expect(scans, own);
    expect(scans.last.phase, CoreScanPhase.done);
    expect(scans.map((u) => u.filesFound).reduce((a, b) => a > b ? a : b), 2);
    // Only the payload named by the domain is set.
    for (final e in events.where(
      (e) => e.domain == CoreEventDomain.scanProgress,
    )) {
      expect(e.libraryChanged, isNull);
      expect(e.jobChanged, isNull);
      expect(e.syncChanged, isNull);
      expect(e.playbackSaved, isNull);
    }
    // The library change comes after the last scan update.
    expect(
      events.indexWhere((e) => e.domain == CoreEventDomain.libraryChanged),
      greaterThan(
        events.lastIndexWhere((e) => e.domain == CoreEventDomain.scanProgress),
      ),
    );
  });

  test('a scan of an unknown source reports an error code', () async {
    final events = _listen(db);
    final library = await CoreLibrary.open(path: db);
    final own = await library.scan(sourceId: 4242).toList();
    expect(own.last.errorCode, 'not_found');
    expect(own.last.error, isNotEmpty);
    await _until(
      'scan error event',
      () => events.any((e) => e.scanProgress?.errorCode == 'not_found'),
    );
  });

  test('a saved position is a playback event', () async {
    final events = _listen(db);
    final media = '${dir.path}/media';
    _media(media, 'Movies/Heat 1995.mkv', 3);
    final library = await CoreLibrary.open(path: db);
    await library.addSource(kind: 'local', name: 'Media', location: media);
    await library.scan().last;
    final id = (await library.movies()).single.id;

    expect(
      await library.saveProgress(
        itemId: id,
        positionMs: 60000,
        durationMs: 600000,
        updatedAtMs: DateTime.now().millisecondsSinceEpoch,
        watched: false,
      ),
      isTrue,
    );
    await _until(
      'playback event',
      () => events.any((e) => e.playbackSaved != null),
    );
    final p = events.firstWhere((e) => e.playbackSaved != null).playbackSaved!;
    expect(p.itemId, id);
    expect(p.positionMs, 60000);
    expect(p.durationMs, 600000);
    expect(p.watched, isFalse);
  });

  test('many listeners, and a cancelled one stops', () async {
    final a = <CoreEvent>[];
    final subA = coreEvents().where((e) => e.libraryPath == db).listen(a.add);
    final b = _listen(db);

    final library = await CoreLibrary.open(path: db);
    await library.scan(sourceId: 1).toList();
    await _until('both listeners', () => a.isNotEmpty && b.isNotEmpty);
    await _cancel(subA);
    final before = a.length;
    await library.scan(sourceId: 2).toList();
    await _until(
      'second scan',
      () => b.any((e) => e.scanProgress?.sourceId == 2),
    );
    expect(a.length, before);
  });

  group('error codes', () {
    test('a database that cannot be opened is a database error', () async {
      // A directory is no SQLite file.
      await expectLater(
        CoreLibrary.open(path: dir.path),
        throwsA(
          isA<LibraryError>()
              .having((e) => e.code, 'code', 'database')
              .having((e) => e.message, 'message', isNotEmpty),
        ),
      );
    });

    test('a bad library id is invalid', () async {
      final library = await CoreLibrary.open(path: db);
      await expectLater(
        library.details(itemId: 'not an id'),
        throwsA(isA<LibraryError>().having((e) => e.code, 'code', 'invalid')),
      );
    });
  });
}
