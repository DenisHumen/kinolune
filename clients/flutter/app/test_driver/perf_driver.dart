// SPDX-License-Identifier: MPL-2.0
//
// QA-006: host side of `flutter drive` for integration_test/perf_test.dart.
// Writes the condensed frame timings of every scenario to
// $KINOLUNE_PERF_OUT (default build/perf/frames.json).

import 'dart:convert';
import 'dart:io';

import 'package:integration_test/integration_test_driver.dart';

import '../tool/perf/perf_report.dart';

Future<void> main() => integrationDriver(
  responseDataCallback: (data) async {
    final raw = data ?? const <String, dynamic>{};
    // `<scenario>#<n>`: repeated runs of one scenario; keep the best.
    final runs = <String, List<Map<String, Object?>>>{};
    for (final MapEntry(:key, :value) in raw.entries) {
      if (key.startsWith('_') || value is! Map) continue;
      runs
          .putIfAbsent(key.split('#').first, () => [])
          .add(condenseFrames(Map<String, dynamic>.from(value)));
    }
    final scenarios = {
      for (final MapEntry(:key, :value) in runs.entries) key: bestRun(value),
    };
    final out = File(
      Platform.environment['KINOLUNE_PERF_OUT'] ?? 'build/perf/frames.json',
    );
    out.parent.createSync(recursive: true);
    out.writeAsStringSync(
      const JsonEncoder.withIndent('  ').convert({
        'schema': perfSchema,
        'info': raw['_info'],
        'scenarios': scenarios,
      }),
    );
    stdout.writeln('perf: wrote ${out.path}');
  },
);
