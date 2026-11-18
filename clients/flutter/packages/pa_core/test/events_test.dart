// SPDX-License-Identifier: MPL-2.0
// CORE-009 through the Dart bindings: the core event stream carries scan
// progress and library changes, and core errors carry stable codes.
import 'dart:async';
import 'dart:io';

import 'package:pa_core/pa_core.dart';
import 'package:test/test.dart';

import 'native_library.dart';

/// A fake media file (bytes only: the scan lists it, probing may fail).
void _media(String root, String rel) {
  final f = File('$root/$rel')..createSync(recursive: true);
  f.writeAsBytesSync([for (var i = 0; i < 150000; i++) (i * 7) & 0xff]);
}

Future<void> _until(String what, bool Function() ok) async {
  final deadline = DateTime.now().add(const Duration(seconds: 15));
  while (!ok()) {
    if (DateTime.now().isAfter(deadline)) fail('timed out: $what');
    await Future<void>.delayed(const Duration(milliseconds: 50));
  }
}

void main() {
  setUpAll(() async {
    await RustLib.init(externalLibrary: openBuiltCoreLibrary());
  });

  tearDownAll(RustLib.dispose);

  late Directory dir;
  setUp(() => dir = Directory.systemTemp.createTempSync('pa_events_test_'));
  tearDown(() => dir.deleteSync(recursive: true));

  test('coreEvents receives scan progress and the library change', () async {
    final events = <CoreEvent>[];
    final sub = coreEvents().listen(events.add);
    addTearDown(sub.cancel);

    final media = '${dir.path}/media';
    _media(media, 'Movies/The Matrix 1999.mkv');
    _media(media, 'Movies/Heat 1995.mkv');
    final db = '${dir.path}/library.db';
    final library = await CoreLibrary.open(path: db);
    final source = await library.addSource(
      kind: 'local',
      name: 'Media',
      location: media,
    );
    // The scan's own stream and the bus see the same updates.
    final own = await library.scan(sourceId: source.id).toList();
    expect(own.last.phase, CoreScanPhase.done);

    bool changed() => events.any(
      (e) =>
          e.domain == CoreEventDomain.libraryChanged &&
          e.libraryChanged!.change == CoreLibraryChange.scan &&
          e.libraryChanged!.libraryPath == db,
    );
    await _until('library change event', changed);

    final scans = events
        .where((e) => e.domain == CoreEventDomain.scanProgress)
        .map((e) => e.scanProgress!)
        .where((u) => u.sourceId == source.id)
        .toList();
    expect(scans, isNotEmpty);
    expect(scans.last.phase, CoreScanPhase.done);
    expect(scans.map((u) => u.filesFound).reduce((a, b) => a > b ? a : b), 2);
    // Only the payload named by the domain is set.
    for (final e in events.where((e) => e.domain == CoreEventDomain.scanProgress)) {
      expect(e.libraryChanged, isNull);
      expect(e.jobChanged, isNull);
      expect(e.syncChanged, isNull);
      expect(e.playbackSaved, isNull);
    }
    // The library change comes after the last scan update.
    final lastScan = events.lastIndexWhere(
      (e) => e.domain == CoreEventDomain.scanProgress,
    );
    final change = events.indexWhere(
      (e) => e.domain == CoreEventDomain.libraryChanged,
    );
    expect(change, greaterThan(lastScan));
  });

  test('a scan of an unknown source reports an error code', () async {
    final events = <CoreEvent>[];
    final sub = coreEvents().listen(events.add);
    addTearDown(sub.cancel);

    final library = await CoreLibrary.open(path: '${dir.path}/library.db');
    final own = await library.scan(sourceId: 4242).toList();
    expect(own.last.errorCode, 'not_found');
    await _until(
      'scan error event',
      () => events.any((e) => e.scanProgress?.errorCode == 'not_found'),
    );
  });

  test('a saved position is a playback event', () async {
    final events = <CoreEvent>[];
    final sub = coreEvents().listen(events.add);
    addTearDown(sub.cancel);

    final media = '${dir.path}/media';
    _media(media, 'Movies/Heat 1995.mkv');
    final library = await CoreLibrary.open(path: '${dir.path}/library.db');
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
      () => events.any((e) => e.playbackSaved?.itemId == id),
    );
    final p = events.firstWhere((e) => e.playbackSaved?.itemId == id).playbackSaved!;
    expect(p.positionMs, 60000);
    expect(p.watched, isFalse);
  });

  test('many listeners, and a cancelled one stops', () async {
    final a = <CoreEvent>[];
    final b = <CoreEvent>[];
    final subA = coreEvents().listen(a.add);
    final subB = coreEvents().listen(b.add);
    addTearDown(subB.cancel);

    final library = await CoreLibrary.open(path: '${dir.path}/library.db');
    await library.scan(sourceId: 1).toList();
    await _until('both listeners', () => a.isNotEmpty && b.isNotEmpty);
    await subA.cancel();
    final before = a.length;
    await library.scan(sourceId: 2).toList();
    await _until('second scan', () => b.any((e) => e.scanProgress?.sourceId == 2));
    expect(a.length, before);
  });

  group('error codes', () {
    test('a database that cannot be opened is a database error', () async {
      await expectLater(
        CoreLibrary.open(path: '${dir.path}/no/such/dir/library.db'),
        throwsA(
          isA<LibraryError>()
              .having((e) => e.code, 'code', 'database')
              .having((e) => e.message, 'message', isNotEmpty),
        ),
      );
    });

    test('a bad library id is invalid', () async {
      final library = await CoreLibrary.open(path: '${dir.path}/library.db');
      await expectLater(
        library.details(itemId: 'not an id'),
        throwsA(isA<LibraryError>().having((e) => e.code, 'code', 'invalid')),
      );
    });
  });
}
