// SPDX-License-Identifier: MPL-2.0
//
// QA-006: the frames file from the log of perf_test.dart run as a plain app
// (no `flutter drive`, KINOLUNE_PERF_LOG=true), e.g. `adb logcat -d`:
//
//   dart run tool/perf/from_log.dart logcat.txt build/perf/frames.json

import 'dart:convert';
import 'dart:io';

import 'perf_report.dart';

void main(List<String> args) {
  if (args.length != 2) {
    stderr.writeln('usage: from_log.dart <log> <frames.json>');
    exit(64);
  }
  final frames = framesFromLog(File(args[0]).readAsStringSync());
  if (frames == null) {
    stderr.writeln('no complete $perfLogPrefix block in ${args[0]}');
    exit(1);
  }
  final out = File(args[1])..parent.createSync(recursive: true);
  out.writeAsStringSync(const JsonEncoder.withIndent('  ').convert(frames));
}
