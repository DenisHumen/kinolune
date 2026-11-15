// SPDX-License-Identifier: MPL-2.0
//
// QA-006: the perf results format and the regression check, shared by the
// driver (test_driver/perf_driver.dart) and tool/perf/compare.dart. Plain
// Dart, no Flutter imports.

import 'dart:math' as math;

/// Version of the results and baseline JSON.
const perfSchema = 1;

/// One frame budget at 60 Hz, in milliseconds.
const frameBudgetMs = 1000 / 60;

/// A metric is a regression when it is more than this much worse than the
/// baseline...
const regressionRatio = 1.20;

/// ...and worse by more than this absolute amount (noise floor), per unit.
const noiseFloor = {'ms': 0.5, 'pct': 1.0, 'startup_ms': 100.0};

double _pct(List<double> sorted, double p) {
  if (sorted.isEmpty) return 0;
  final i = ((p / 100) * sorted.length).ceil() - 1;
  return sorted[i.clamp(0, sorted.length - 1)];
}

double _r2(double v) => (v * 100).roundToDouble() / 100;

Map<String, double> _stats(List<double> v) {
  final s = [...v]..sort();
  final avg = s.isEmpty ? 0.0 : s.reduce((a, b) => a + b) / s.length;
  return {
    'avg': _r2(avg),
    'p50': _r2(_pct(s, 50)),
    'p90': _r2(_pct(s, 90)),
    'p99': _r2(_pct(s, 99)),
    'worst': _r2(s.isEmpty ? 0 : s.last),
  };
}

/// Condenses a `watchPerformance` summary (FrameTimingSummarizer, times in
/// microseconds) to the numbers we track. A frame is janky when its build
/// or its raster phase alone takes longer than one 60 Hz frame: the two
/// run in a pipeline, so either one over budget drops a frame.
Map<String, Object?> condenseFrames(Map<String, dynamic> summary) {
  List<double> ms(String key) => [
    for (final v in (summary[key] as List? ?? const [])) (v as num) / 1000,
  ];
  final build = ms('frame_build_times');
  final raster = ms('frame_rasterizer_times');
  final n = math.min(build.length, raster.length);
  var jank = 0;
  for (var i = 0; i < n; i++) {
    if (build[i] > frameBudgetMs || raster[i] > frameBudgetMs) jank++;
  }
  return {
    'frames': n,
    'build_ms': _stats(build),
    'raster_ms': _stats(raster),
    'jank_frames': jank,
    'jank_pct': n == 0 ? 0.0 : _r2(100 * jank / n),
    'old_gen_gc': summary['old_gen_gc_count'],
    'new_gen_gc': summary['new_gen_gc_count'],
  };
}

/// One tracked number of a result: its path, value and unit.
typedef Metric = ({String name, double value, String unit});

/// The numbers compared against the baseline.
List<Metric> metricsOf(Map<String, dynamic> result) {
  final out = <Metric>[];
  final scenarios = (result['scenarios'] as Map?) ?? const {};
  for (final MapEntry(:key, :value) in scenarios.entries) {
    final s = value as Map;
    for (final phase in ['build_ms', 'raster_ms']) {
      final st = s[phase] as Map;
      for (final p in ['p90', 'p99']) {
        out.add((
          name: '$key.$phase.$p',
          value: (st[p] as num).toDouble(),
          unit: 'ms',
        ));
      }
    }
    out.add((
      name: '$key.jank_pct',
      value: (s['jank_pct'] as num).toDouble(),
      unit: 'pct',
    ));
  }
  final startup = result['startup'] as Map?;
  for (final k in ['first_frame_ms', 'library_visible_ms']) {
    final m = startup?[k] as Map?;
    if (m == null) continue;
    out.add((
      name: 'startup.$k.median',
      value: (m['median'] as num).toDouble(),
      unit: 'startup_ms',
    ));
  }
  return out;
}

/// The outcome of one metric.
class Comparison {
  Comparison(this.name, this.unit, this.baseline, this.current);

  final String name;
  final String unit;
  final double? baseline;
  final double? current;

  double? get ratio {
    final b = baseline, c = current;
    if (b == null || c == null) return null;
    if (b == 0) return c == 0 ? 1 : double.infinity;
    return c / b;
  }

  bool get regression {
    final b = baseline, c = current;
    if (b == null || c == null) return false;
    return c > b * regressionRatio && c - b > (noiseFloor[unit] ?? 0);
  }

  bool get improvement {
    final b = baseline, c = current;
    if (b == null || c == null) return false;
    return c * regressionRatio < b && b - c > (noiseFloor[unit] ?? 0);
  }
}

List<Comparison> compareResults(
  Map<String, dynamic> baseline,
  Map<String, dynamic> current,
) {
  final b = {for (final m in metricsOf(baseline)) m.name: m};
  final c = {for (final m in metricsOf(current)) m.name: m};
  return [
    for (final name in {...b.keys, ...c.keys})
      Comparison(
        name,
        (b[name] ?? c[name])!.unit,
        b[name]?.value,
        c[name]?.value,
      ),
  ];
}

String _fmt(double? v) => v == null ? '—' : v.toStringAsFixed(2);

/// A Markdown table of [rows], regressions first.
String markdownReport(List<Comparison> rows, {String title = 'Perf'}) {
  final sorted = [...rows]
    ..sort((a, b) {
      int rank(Comparison c) => c.regression ? 0 : (c.improvement ? 1 : 2);
      final r = rank(a).compareTo(rank(b));
      return r != 0 ? r : a.name.compareTo(b.name);
    });
  final regressions = rows.where((r) => r.regression).length;
  final b = StringBuffer()
    ..writeln('### $title')
    ..writeln()
    ..writeln(
      regressions == 0
          ? 'No regressions over ${((regressionRatio - 1) * 100).round()} %.'
          : '**$regressions regression(s) over '
                '${((regressionRatio - 1) * 100).round()} %.**',
    )
    ..writeln()
    ..writeln('| metric | baseline | current | change | |')
    ..writeln('|---|---:|---:|---:|---|');
  for (final r in sorted) {
    final ratio = r.ratio;
    final change = ratio == null || ratio.isInfinite
        ? '—'
        : '${ratio >= 1 ? '+' : ''}${((ratio - 1) * 100).toStringAsFixed(0)} %';
    final mark = r.regression
        ? 'REGRESSION'
        : (r.improvement ? 'better' : '');
    b.writeln(
      '| ${r.name} | ${_fmt(r.baseline)} | ${_fmt(r.current)} | $change | $mark |',
    );
  }
  return b.toString();
}
