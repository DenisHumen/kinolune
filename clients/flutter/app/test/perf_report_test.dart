// SPDX-License-Identifier: MPL-2.0
//
// QA-006: the frame summary and the regression check of tool/perf.

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';

import '../tool/perf/perf_report.dart';

Map<String, dynamic> _result({
  double buildP90 = 4,
  double rasterP99 = 10,
  double jank = 2,
  int? visible,
}) => {
  'scenarios': {
    'grid': {
      'build_ms': {'p50': 2.0, 'p90': buildP90, 'p99': 8.0},
      'raster_ms': {'p50': 3.0, 'p90': 6.0, 'p99': rasterP99},
      'jank_pct': jank,
    },
  },
  if (visible != null)
    'startup': {
      'first_frame_ms': {'median': 300, 'min': 280},
      'library_visible_ms': {'median': visible + 50, 'min': visible},
    },
};

void main() {
  test('condenses frame timings and counts janky frames', () {
    final s = condenseFrames({
      // Microseconds; frame 3 is over budget in build, frame 4 in raster.
      'frame_build_times': [2000, 4000, 20000, 3000],
      'frame_rasterizer_times': [5000, 6000, 7000, 18000],
      'old_gen_gc_count': 0,
      'new_gen_gc_count': 3,
    });
    expect(s['frames'], 4);
    expect(s['jank_frames'], 2);
    expect(s['jank_pct'], 50.0);
    expect((s['build_ms'] as Map)['worst'], 20.0);
    expect((s['raster_ms'] as Map)['p50'], 6.0);
  });

  test('more than 20 % worse is a regression', () {
    final rows = compareResults(
      _result(visible: 1000),
      _result(buildP90: 5.2, visible: 1300),
    );
    final bad = {for (final r in rows.where((r) => r.regression)) r.name};
    // 4 -> 5.2 ms is +30 %; 1000 -> 1300 ms is +30 %.
    expect(bad, {'grid.build_ms.p90', 'startup.library_visible_ms.min'});
    expect(markdownReport(rows), contains('REGRESSION'));
  });

  test('only tracked metrics flag', () {
    // p99 is reported, not tracked by default; a baseline may narrow it.
    final worse = _result(rasterP99: 30, buildP90: 6);
    expect(
      compareResults(
        _result(),
        worse,
      ).where((r) => r.regression).map((r) => r.name),
      ['grid.build_ms.p90'],
    );
    final narrowed = {
      ..._result(),
      'tracked': ['raster_ms.p99'],
    };
    expect(
      compareResults(
        narrowed,
        worse,
      ).where((r) => r.regression).map((r) => r.name),
      ['grid.raster_ms.p99'],
    );
  });

  test('the best of repeated runs is kept', () {
    Map<String, Object?> run(double b, double r) => {
      'build_ms': {'p90': b},
      'raster_ms': {'p90': r},
    };
    final best = bestRun([run(9, 1), run(4, 8), run(4, 5)]);
    expect((best['raster_ms'] as Map)['p90'], 5);
    expect(best['runs'], 3);
    expect(best['build_p90_of_runs'], [9, 4, 4]);
  });

  test('tiny absolute changes stay under the noise floor', () {
    final rows = compareResults(
      _result(buildP90: 1, jank: 0.2),
      _result(buildP90: 1.4, jank: 0.9),
    );
    expect(rows.where((r) => r.regression), isEmpty);
    expect(markdownReport(rows), contains('No regressions'));
  });

  test('a much better number is marked, a new metric is not a regression', () {
    final rows = compareResults(_result(), _result(buildP90: 2, visible: 900));
    expect(
      rows.singleWhere((r) => r.name == 'grid.build_ms.p90').improvement,
      isTrue,
    );
    expect(rows.where((r) => r.regression), isEmpty);
  });

  test('results survive the log (base64 chunks, logcat prefixes)', () {
    final frames = {'schema': 1, 'info': {'os': 'Android 14 (x86_64)'}};
    final b64 = base64.encode(utf8.encode(jsonEncode(frames)));
    final parts = [for (var i = 0; i < b64.length; i += 7) b64.substring(i, i + 7 > b64.length ? b64.length : i + 7)];
    final log = [
      '11-15 03:50:00.1  123  140 I flutter : unrelated',
      for (final (i, p) in parts.indexed)
        '11-15 03:50:00.2  123  140 I flutter : $perfLogPrefix$i/${parts.length} $p\r',
    ].reversed.join('\n');
    expect(framesFromLog(log), frames);
    expect(framesFromLog(log.split('\n').skip(1).join('\n')), isNull);
  });
}
