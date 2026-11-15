// SPDX-License-Identifier: MPL-2.0
//
// QA-006: UI performance budgets. Profile-mode scenarios measured with the
// engine's frame timings (`watchPerformance`): the library grid over 10 000
// titles (demo art, decoded poster files, the real Rust core), Home rows,
// opening a movie page, the player OSD. Run through tool/perf_bench.sh:
//
//   flutter drive --profile -d linux \
//     --driver test_driver/perf_driver.dart \
//     --target integration_test/perf_test.dart
//
// dart-defines:
//   KINOLUNE_PERF_ONLY=grid_scroll_demo,osd_toggle  only these scenarios
//   KINOLUNE_PERF_CORE=true    also the real-core grid (scans 10 000 files)
//   KINOLUNE_PERF_CORE_DIR=/abs/dir  where the core library lives; reused
//                              when already scanned (default: a temp dir)
//   KINOLUNE_PERF_SLOWDOWN_MS=4  burn this much CPU in every frame (the
//                              deliberately slowed run of the regression check)

import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/scheduler.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:kinolune/app.dart';
import 'package:kinolune/fakes/fake_library.dart';
import 'package:kinolune/integration/library.dart';
import 'package:kinolune/library/library_queries.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/screens/home_screen.dart';
import 'package:kinolune/screens/movie_detail_screen.dart';
import 'package:kinolune/screens/playback.dart';
import 'package:kinolune/services/app_services.dart';
import 'package:kinolune/services/secure_store.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/widgets/cards.dart';
import 'package:material_ui/material_ui.dart';
import 'package:pa_player/pa_player.dart' show PlayerScreen, PlayerScreenState;
import 'package:pa_player/testing.dart';
import 'package:path_provider/path_provider.dart';

import '../test/support/test_app.dart' show demoServices;

const _only = String.fromEnvironment('KINOLUNE_PERF_ONLY');
const _core = bool.fromEnvironment('KINOLUNE_PERF_CORE');
const _coreDir = String.fromEnvironment('KINOLUNE_PERF_CORE_DIR');
const _slowdownMs = int.fromEnvironment('KINOLUNE_PERF_SLOWDOWN_MS');

/// Titles in the grid scenarios.
const gridItems = 10000;

bool _enabled(String name) =>
    _only.isEmpty || _only.split(',').map((s) => s.trim()).contains(name);

final _now = DateTime(2026, 11, 14, 12);

/// 10 000 movies; [poster] gives each one its poster URL.
List<LibraryItem> _movies(String? Function(int i) poster) => [
  for (var i = 0; i < gridItems; i++)
    LibraryItem(
      id: 'm-$i',
      kind: ItemKind.movie,
      title:
          '${String.fromCharCode(65 + i % 26)} movie ${i.toString().padLeft(5, '0')}',
      year: 1950 + i % 75,
      filePath: '/media/Movies/movie-$i.mkv',
      durationMs: 100 * 60000,
      addedAt: _now.subtract(Duration(days: i % 400)),
      posterUrl: poster(i),
    ),
];

Future<AppServices> _fakeServices(List<LibraryItem> items) async {
  final lib = FakeLibrary.items(items);
  final s = AppServices(
    library: lib,
    secureStore: MemorySecureStore(),
    playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
    queries: MemoryLibraryQueries(
      lib,
      supportsFilters: true,
      genresOf: (_) => const ['Drama'],
      badgeSource: lib.badges,
    ),
    clock: () => _now,
  );
  await s.init();
  return s;
}

/// 48 distinct poster PNGs (500×750) in [dir], drawn here: the grid then
/// decodes real files at display size, as with the core's artwork cache.
Future<List<String>> _posterFiles(Directory dir) async {
  dir.createSync(recursive: true);
  final paths = <String>[];
  for (var i = 0; i < 48; i++) {
    final f = File('${dir.path}/poster-$i.png');
    paths.add(f.path);
    if (f.existsSync()) continue;
    final rec = ui.PictureRecorder();
    final c = Canvas(rec);
    const size = Size(500, 750);
    final rect = Offset.zero & size;
    c.drawRect(
      rect,
      Paint()
        ..shader = LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: [
            HSLColor.fromAHSL(1, i * 7.5, .6, .45).toColor(),
            HSLColor.fromAHSL(1, (i * 7.5 + 90) % 360, .5, .12).toColor(),
          ],
        ).createShader(rect),
    );
    for (var k = 0; k < 24; k++) {
      c.drawCircle(
        Offset((k * 97 + i * 31) % 500, (k * 53 + i * 17) % 750),
        20.0 + (k * 13 + i) % 60,
        Paint()..color = Color(0x33FFFFFF ^ (k * 0x10203 + i * 0x305)),
      );
    }
    final img = await rec.endRecording().toImage(500, 750);
    final png = await img.toByteData(format: ui.ImageByteFormat.png);
    img.dispose();
    f.writeAsBytesSync(png!.buffer.asUint8List());
  }
  return paths;
}

/// The real core over 10 000 small files named like movies, scanned once
/// (reused from [_coreDir] when it is already there).
Future<RustLibrary> _coreLibrary() async {
  await RustLibrary.ensureInitialized();
  // `app`: the app's own support directory (on the emulator: so a cold
  // start of the same package finds this library).
  final root = switch (_coreDir) {
    '' => Directory(
      '${(await getTemporaryDirectory()).path}/kinolune-perf-core',
    ),
    'app' => await getApplicationSupportDirectory(),
    _ => Directory(_coreDir),
  };
  final media = Directory('${root.path}/media/Movies');
  final db = '${root.path}/$coreDbFileName';
  final done = File('${root.path}/.scanned');
  if (!done.existsSync()) {
    media.createSync(recursive: true);
    final sw = Stopwatch()..start();
    for (var i = 0; i < gridItems; i++) {
      final title =
          '${String.fromCharCode(65 + i % 26)} Movie ${i.toString().padLeft(5, '0')}';
      final f = File('${media.path}/$title (${1950 + i % 75}).mkv');
      if (!f.existsSync()) f.writeAsBytesSync(List.filled(64, i & 0xff));
    }
    final lib = await RustLibrary.open(db);
    await lib.addSource(
      MediaSource(
        id: '',
        kind: SourceKind.localFolder,
        name: 'Movies',
        path: media.parent.path,
      ),
    );
    await lib.scan().toList();
    await lib.close();
    done.writeAsStringSync('${sw.elapsedMilliseconds}\n');
  }
  return RustLibrary.open(db);
}

/// Burns [_slowdownMs] in every frame's build phase.
void _installSlowdown() {
  if (_slowdownMs <= 0) return;
  SchedulerBinding.instance.addPersistentFrameCallback((_) {
    final sw = Stopwatch()..start();
    while (sw.elapsedMicroseconds < _slowdownMs * 1000) {}
  });
}

Future<void> _waitFor(
  WidgetTester t,
  Finder f, {
  Duration timeout = const Duration(seconds: 30),
}) async {
  final sw = Stopwatch()..start();
  while (f.evaluate().isEmpty) {
    if (sw.elapsed > timeout) throw TimeoutException('waiting for $f');
    await t.pump(const Duration(milliseconds: 16));
  }
}

Future<void> _idle(WidgetTester t, [int ms = 500]) async {
  final end = DateTime.now().add(Duration(milliseconds: ms));
  while (DateTime.now().isBefore(end)) {
    await t.pump(const Duration(milliseconds: 16));
  }
}

Future<void> _pumpApp(
  WidgetTester t,
  AppServices s, {
  Section section = Section.home,
}) async {
  await t.pumpWidget(
    KinoluneApp(
      services: s,
      formFactor: FormFactor.desktop,
      initialSection: section,
    ),
  );
  await _idle(t, 800);
}

/// Scrolls the grid down [distance] logical pixels at [speed] px/s, back up
/// half way faster, and jumps near the end and scrolls there too (pages
/// load all along: 120 items each).
Future<void> _scrollGrid(WidgetTester t) async {
  final scrollable = find.descendant(
    of: find.byType(CustomScrollView),
    matching: find.byType(Scrollable),
  );
  final pos = t.state<ScrollableState>(scrollable.first).position;
  Future<void> to(double offset, double speed) async {
    final ms = ((offset - pos.pixels).abs() / speed * 1000).round();
    await pos.animateTo(
      offset.clamp(0, pos.maxScrollExtent),
      duration: Duration(milliseconds: ms.clamp(16, 60000)),
      curve: Curves.linear,
    );
  }

  await to(20000, 2500);
  await to(10000, 5000);
  pos.jumpTo(pos.maxScrollExtent * .9);
  await _idle(t, 300);
  await to(pos.maxScrollExtent, 2500);
  // Flings, as a finger or a wheel would.
  for (var i = 0; i < 4; i++) {
    await t.fling(scrollable.first, const Offset(0, 600), 3000);
    await _idle(t, 700);
  }
}

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  binding.framePolicy = LiveTestWidgetsFlutterBindingFramePolicy.benchmarkLive;
  _installSlowdown();
  final info = <String, Object?>{
    'platform': Platform.operatingSystem,
    'os_version': Platform.operatingSystemVersion,
    'processors': Platform.numberOfProcessors,
    'items': gridItems,
    'slowdown_ms': _slowdownMs,
  };

  tearDownAll(() {
    binding.reportData ??= {};
    binding.reportData!['_info'] = info;
  });

  Future<void> gridScenario(
    WidgetTester t,
    String key,
    Future<AppServices> Function() services,
  ) async {
    final s = await services();
    await _pumpApp(t, s, section: Section.movies);
    await _waitFor(t, find.byType(PosterCard));
    await _idle(t, 500);
    await binding.watchPerformance(() => _scrollGrid(t), reportKey: key);
    expect(find.byType(PosterCard).evaluate().length, lessThan(200));
    await t.pumpWidget(const SizedBox());
    await _idle(t, 200);
  }

  testWidgets(
    'grid_scroll_demo: 10 000 titles, art drawn in code',
    (t) => gridScenario(
      t,
      'grid_scroll_demo',
      () => _fakeServices(_movies((_) => 'demo:poster')),
    ),
    skip: !_enabled('grid_scroll_demo'),
  );

  testWidgets(
    'grid_scroll_posters: 10 000 titles, poster files decoded at size',
    (t) async {
      final dir = Directory(
        '${(await getTemporaryDirectory()).path}/kinolune-perf-posters',
      );
      final files = await t.runAsync(() => _posterFiles(dir));
      await gridScenario(
        t,
        'grid_scroll_posters',
        () => _fakeServices(_movies((i) => files![i % files.length])),
      );
    },
    skip: !_enabled('grid_scroll_posters'),
  );

  testWidgets(
    'grid_scroll_core: 10 000 scanned files on the real Rust core',
    (t) async {
      final sw = Stopwatch()..start();
      final lib = (await t.runAsync(_coreLibrary))!;
      info['core_open_ms'] = sw.elapsedMilliseconds;
      final total = (await t.runAsync(
        () => LibraryQueries.forLibrary(
          lib,
        ).query(const LibraryQuery(ItemKind.movie), limit: 1),
      ))!.total;
      info['core_items'] = total;
      expect(total, gridItems);
      await gridScenario(t, 'grid_scroll_core', () async {
        final s = AppServices(
          library: lib,
          secureStore: MemorySecureStore(),
          playerEngineFactory: ({bool video = true}) => FakePlayerEngine(),
        );
        await s.init();
        return s;
      });
      await t.runAsync(lib.close);
    },
    skip: !_core || !_enabled('grid_scroll_core'),
  );

  // tool/perf_bench.sh: only scans the core library for the cold start.
  testWidgets('seed_only: scan the core library', (t) async {
    final lib = (await t.runAsync(_coreLibrary))!;
    await t.runAsync(lib.close);
  }, skip: !_core || _only != 'seed_only');

  testWidgets('home_scroll: Home down and along its rows', (t) async {
    final s = await demoServices();
    await _pumpApp(t, s);
    await _waitFor(t, find.byType(PosterCard));
    await _idle(t, 500);
    final vertical = find
        .descendant(
          of: find.byType(HomeScreen),
          matching: find.byType(Scrollable),
        )
        .first;
    await binding.watchPerformance(() async {
      final pos = t.state<ScrollableState>(vertical).position;
      await pos.animateTo(
        pos.maxScrollExtent,
        duration: const Duration(seconds: 3),
        curve: Curves.easeInOut,
      );
      await pos.animateTo(
        0,
        duration: const Duration(seconds: 2),
        curve: Curves.easeInOut,
      );
      // Along each visible row and back.
      for (var row = 0; row < 3; row++) {
        final rows = find.byWidgetPredicate(
          (w) => w is Scrollable && w.axisDirection == AxisDirection.right,
        );
        if (rows.evaluate().length <= row) break;
        final p = t.state<ScrollableState>(rows.at(row)).position;
        await p.animateTo(
          p.maxScrollExtent,
          duration: const Duration(milliseconds: 1500),
          curve: Curves.easeInOut,
        );
        await p.animateTo(
          0,
          duration: const Duration(milliseconds: 800),
          curve: Curves.easeInOut,
        );
      }
    }, reportKey: 'home_scroll');
    await t.pumpWidget(const SizedBox());
    await _idle(t, 200);
  }, skip: !_enabled('home_scroll'));

  testWidgets('detail_open: open a movie page from the grid, back', (
    t,
  ) async {
    final s = await demoServices();
    await _pumpApp(t, s, section: Section.movies);
    await _waitFor(t, find.byType(PosterCard));
    await _idle(t, 500);
    final opens = <int>[];
    await binding.watchPerformance(() async {
      for (var i = 0; i < 6; i++) {
        final poster = find.byType(PosterCard).at(i % 4);
        final sw = Stopwatch()..start();
        await t.tap(poster);
        await _waitFor(t, find.byType(MovieDetailScreen));
        await _waitFor(t, find.byKey(detailPlayKey));
        opens.add(sw.elapsedMilliseconds);
        await _idle(t, 900);
        await t.pageBack();
        await _idle(t, 700);
      }
    }, reportKey: 'detail_open');
    opens.sort();
    info['detail_open_ms_median'] = opens[opens.length ~/ 2];
    await t.pumpWidget(const SizedBox());
    await _idle(t, 200);
  }, skip: !_enabled('detail_open'));

  testWidgets('osd_toggle: show and hide the player OSD', (t) async {
    final s = await demoServices();
    await _pumpApp(t, s);
    final item = (await s.library.movies().first).first;
    final ctx = t.element(find.byType(Scaffold).first);
    unawaited(playVideo(ctx, item));
    await _waitFor(t, find.byType(PlayerScreen));
    await _idle(t, 800);
    final player = t.state<PlayerScreenState>(find.byType(PlayerScreen));
    await binding.watchPerformance(() async {
      for (var i = 0; i < 8; i++) {
        player.osd.hide();
        await _idle(t, 500);
        player.osd.show();
        await _idle(t, 500);
      }
    }, reportKey: 'osd_toggle');
    await t.pumpWidget(const SizedBox());
    await _idle(t, 200);
  }, skip: !_enabled('osd_toggle'));
}
