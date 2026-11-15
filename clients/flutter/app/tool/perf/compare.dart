// SPDX-License-Identifier: MPL-2.0
//
// QA-006: compares a perf run with a baseline and flags regressions over
// 20 % (above a small noise floor, see perf_report.dart).
//
//   dart run tool/perf/compare.dart perf/baseline/linux-xvfb.json \
//     build/perf/results.json [--github] [--fail-on-regression]
//
// --github  writes `::warning` annotations (they show on the PR) and, when
//           $GITHUB_STEP_SUMMARY is set, the table to the job summary.
// --fail-on-regression  exit code 1 when anything regressed (default 0:
//           informational).

import 'dart:convert';
import 'dart:io';

import 'perf_report.dart';

void main(List<String> args) {
  final files = args.where((a) => !a.startsWith('--')).toList();
  if (files.length != 2) {
    stderr.writeln(
      'usage: compare.dart <baseline.json> <results.json> '
      '[--github] [--fail-on-regression]',
    );
    exit(64);
  }
  Map<String, dynamic> read(String p) =>
      jsonDecode(File(p).readAsStringSync()) as Map<String, dynamic>;
  final baseline = read(files[0]);
  final current = read(files[1]);
  final rows = compareResults(baseline, current);
  final report = markdownReport(
    rows,
    title: 'UI performance vs ${files[0].split('/').last}',
  );
  stdout.write(report);

  final regressions = rows.where((r) => r.regression).toList();
  if (args.contains('--github')) {
    for (final r in regressions) {
      stdout.writeln(
        '::warning title=Perf regression::${r.name}: '
        '${r.baseline!.toStringAsFixed(2)} -> ${r.current!.toStringAsFixed(2)} '
        '(+${((r.ratio! - 1) * 100).toStringAsFixed(0)} %)',
      );
    }
    final summary = Platform.environment['GITHUB_STEP_SUMMARY'];
    if (summary != null && summary.isNotEmpty) {
      File(summary).writeAsStringSync('$report\n', mode: FileMode.append);
    }
  }
  if (regressions.isNotEmpty && args.contains('--fail-on-regression')) {
    exit(1);
  }
}
