// SPDX-License-Identifier: MPL-2.0
//
// QA-006: merges the frame timings of `flutter drive` with the cold-start
// runs into one results file (the format of perf/baseline/*.json).
//
//   dart run tool/perf/merge.dart build/perf/frames.json \
//     build/perf/results.json [first_frame_ms,library_visible_ms ...]

import 'dart:convert';
import 'dart:io';

import 'perf_report.dart';

Map<String, Object> _summary(List<int> runs) {
  final s = [...runs]..sort();
  return {
    'runs': runs,
    'median': s[s.length ~/ 2],
    'min': s.first,
    'max': s.last,
  };
}

void main(List<String> args) {
  if (args.length < 2) {
    stderr.writeln(
      'usage: merge.dart <frames.json> <results.json> [first,visible ...]',
    );
    exit(64);
  }
  final frames =
      jsonDecode(File(args[0]).readAsStringSync()) as Map<String, dynamic>;
  final first = <int>[];
  final visible = <int>[];
  for (final run in args.skip(2)) {
    final parts = run.split(',');
    first.add(int.parse(parts[0]));
    visible.add(int.parse(parts[1]));
  }
  final result = <String, Object?>{
    'schema': perfSchema,
    'recorded_at': DateTime.now().toUtc().toIso8601String(),
    'info': frames['info'],
    'scenarios': frames['scenarios'],
    if (first.isNotEmpty)
      'startup': {
        'first_frame_ms': _summary(first),
        'library_visible_ms': _summary(visible),
      },
  };
  final out = File(args[1])..parent.createSync(recursive: true);
  out.writeAsStringSync(const JsonEncoder.withIndent('  ').convert(result));
}
