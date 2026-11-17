// SPDX-License-Identifier: MPL-2.0

/// Automatic recognition (LIB-024, ADR-NNNN): nobody has to press
/// "Rescan". What recognition still owes and when the library last looked
/// at its sources, for the Settings status line.
library;

import 'models.dart' show ScanProgress;

/// State of automatic recognition.
class RecognitionStatus {
  const RecognitionStatus({
    this.lastCheck,
    this.lastCheckError,
    this.running = false,
    this.unrecognizedFiles = 0,
    this.worksWaiting = 0,
    this.animePending = 0,
    this.worksFailed = 0,
    this.deferredProbes = 0,
    this.unfinishedScans = 0,
    this.progress,
  });

  /// When the last automatic check of a source ended (watcher: start-up,
  /// timer, changes, foreground, network back); null before the first one
  /// of this session.
  final DateTime? lastCheck;

  /// Why the last automatic check failed (share offline), or null.
  final String? lastCheckError;

  /// A catch-up run (metadata, headers) is going on.
  final bool running;

  /// Videos whose names are not read yet (a scan was stopped).
  final int unrecognizedFiles;

  /// Films, series and anime waiting for titles and posters.
  final int worksWaiting;

  /// Of [worksWaiting]: anime waiting for AniList and Shikimori.
  final int animePending;

  /// Works whose last fetch failed; asked again later or when the network
  /// comes back.
  final int worksFailed;

  /// Files whose headers (duration, tracks) are not read yet.
  final int deferredProbes;

  /// Sources whose last scan never finished.
  final int unfinishedScans;

  /// Step and counts of the running catch-up run (titles, anime, headers),
  /// null when none runs.
  final ScanProgress? progress;

  /// Items still waiting for recognition.
  int get waiting => unrecognizedFiles + worksWaiting;

  RecognitionStatus copyWith({
    DateTime? lastCheck,
    String? Function()? lastCheckError,
    bool? running,
    int? unrecognizedFiles,
    int? worksWaiting,
    int? animePending,
    int? worksFailed,
    int? deferredProbes,
    int? unfinishedScans,
    ScanProgress? Function()? progress,
  }) => RecognitionStatus(
    lastCheck: lastCheck ?? this.lastCheck,
    lastCheckError: lastCheckError == null
        ? this.lastCheckError
        : lastCheckError(),
    running: running ?? this.running,
    unrecognizedFiles: unrecognizedFiles ?? this.unrecognizedFiles,
    worksWaiting: worksWaiting ?? this.worksWaiting,
    animePending: animePending ?? this.animePending,
    worksFailed: worksFailed ?? this.worksFailed,
    deferredProbes: deferredProbes ?? this.deferredProbes,
    unfinishedScans: unfinishedScans ?? this.unfinishedScans,
    progress: progress == null ? this.progress : progress(),
  );

  @override
  String toString() =>
      'RecognitionStatus(lastCheck: $lastCheck, running: $running, '
      'unrecognized: $unrecognizedFiles, works: $worksWaiting '
      '(anime $animePending), failed: $worksFailed, '
      'probes: $deferredProbes, unfinished scans: $unfinishedScans)';
}

/// A library that recognises on its own (LIB-024): while it watches its
/// sources ([Library.startWatching]) it finishes whatever a stopped or
/// killed run left — on start, after every automatic check and every few
/// minutes — and checks its sources when asked ([checkNow]).
abstract interface class LibraryAutoRecognition {
  /// The current state.
  RecognitionStatus get recognitionStatus;

  /// Every change of [recognitionStatus].
  Stream<RecognitionStatus> get recognitionStatusChanges;

  /// The app came back to the foreground ([networkBack]: the network
  /// returned): sources not checked in the last [minAge] are listed again
  /// at once (incrementally: only new and changed files are read), and
  /// unfinished recognition goes on. With [networkBack] works whose fetch
  /// failed are asked again right away. Does nothing unless watching.
  Future<void> checkNow({
    Duration minAge = const Duration(minutes: 2),
    bool networkBack = false,
  });
}
