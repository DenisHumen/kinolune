// SPDX-License-Identifier: MPL-2.0
//
// Twitch VOD downloads (TWI-017), full builds only: the queue of the Rust
// core (`CoreTwitchDownloads`: pa-jobs + pa-storage + pa-twitch-archive).
// The app calls this only under `AppFlavor.twitchArchive`, so store builds
// drop it; the store core has no downloader either.

import 'package:pa_core/pa_core.dart' as core;

/// State of a download, as the core's job queue reports it.
enum TwitchDownloadState {
  queued,
  waiting,
  running,
  paused,
  retrying,
  done,
  failed,
  cancelled;

  static TwitchDownloadState parse(String s) => switch (s) {
    'queued' => queued,
    'waiting_resources' => waiting,
    'running' => running,
    'paused' => paused,
    'retrying' => retrying,
    'done' => done,
    'failed' => failed,
    _ => cancelled,
  };

  bool get isActive =>
      this == queued || this == waiting || this == running || this == retrying;
}

/// What the user can do with a download.
enum TwitchDownloadAction { pause, resume, cancel, retry, remove }

/// One download in the list.
class TwitchDownloadItem {
  const TwitchDownloadItem({
    required this.jobId,
    required this.videoId,
    required this.state,
    this.partLabel,
    this.progress,
    this.bytesDone = 0,
    this.bytesTotal,
    this.waitingFor,
    this.error,
    this.folder,
    this.file,
  });

  final int jobId;
  final String videoId;
  final String? partLabel;
  final TwitchDownloadState state;

  /// 0–1 when known.
  final double? progress;
  final int bytesDone;
  final int? bytesTotal;

  /// `disk_reserve`, `quota`, `network`, `user`…
  final String? waitingFor;

  /// `usher_403: …` and the like.
  final String? error;

  /// Folder and file name of the finished download in the downloads folder.
  final String? folder;
  final String? file;

  List<TwitchDownloadAction> get actions => switch (state) {
    TwitchDownloadState.queued ||
    TwitchDownloadState.waiting ||
    TwitchDownloadState.running ||
    TwitchDownloadState.retrying => const [
      TwitchDownloadAction.pause,
      TwitchDownloadAction.cancel,
    ],
    TwitchDownloadState.paused => const [
      TwitchDownloadAction.resume,
      TwitchDownloadAction.cancel,
    ],
    TwitchDownloadState.failed => const [
      TwitchDownloadAction.retry,
      TwitchDownloadAction.remove,
    ],
    TwitchDownloadState.done ||
    TwitchDownloadState.cancelled => const [TwitchDownloadAction.remove],
  };
}

/// The download queue the UI talks to.
abstract interface class TwitchDownloads {
  /// Queues a VOD (or the part `[start, end)` of it: a chapter); returns
  /// the job id.
  Future<int> download({
    required String videoId,
    Duration? duration,
    Duration? start,
    Duration? end,
    String? partLabel,
  });

  /// Every download, newest first.
  Future<List<TwitchDownloadItem>> list();

  Future<void> control(int jobId, TwitchDownloadAction action);
}

/// The VOD (or this part of it) is saved or being downloaded already
/// (TWI-019, research 06 §4.3 deduplication).
class TwitchDuplicateDownload implements Exception {
  const TwitchDuplicateDownload(this.message);
  final String message;
  @override
  String toString() => message;
}

/// [TwitchDownloads] over the Rust core.
class CoreTwitchDownloadsSource implements TwitchDownloads {
  CoreTwitchDownloadsSource(this._core);

  final core.CoreTwitchDownloads _core;

  /// The core queue (recordings, rules, saved files: TWI-019).
  core.CoreTwitchDownloads get coreQueue => _core;

  /// Opens the queue; null when this build or the signed remote config
  /// does not allow downloads, or nobody is signed in to Twitch.
  static Future<CoreTwitchDownloadsSource?> open({
    required core.CoreTwitch twitch,
    required core.CoreRemoteConfig config,
    required String downloadsDir,
    required String recordingsDir,
  }) async {
    if (!core.CoreTwitchDownloads.available()) return null;
    try {
      return CoreTwitchDownloadsSource(
        await core.CoreTwitchDownloads.open(
          twitch: twitch,
          config: config,
          downloadsDir: downloadsDir,
          recordingsDir: recordingsDir,
        ),
      );
    } on core.CoreDownloadError {
      return null;
    }
  }

  @override
  Future<int> download({
    required String videoId,
    Duration? duration,
    Duration? start,
    Duration? end,
    String? partLabel,
  }) async {
    try {
      final id = await _core.download(
        request: core.CoreDownloadRequest(
          videoId: videoId,
          quality: 'source',
          container: 'mkv',
          startMs: start?.inMilliseconds,
          endMs: end?.inMilliseconds,
          partLabel: partLabel,
          durationMs: duration?.inMilliseconds,
          allowDuplicate: false,
        ),
      );
      return id.toInt();
    } on core.CoreDownloadError catch (e) {
      if (e.code == 'duplicate') throw TwitchDuplicateDownload(e.message);
      rethrow;
    }
  }

  @override
  Future<List<TwitchDownloadItem>> list() async => [
    for (final d in await _core.list())
      TwitchDownloadItem(
        jobId: d.jobId.toInt(),
        videoId: d.videoId,
        partLabel: d.partLabel,
        state: TwitchDownloadState.parse(d.state),
        progress: d.progress,
        bytesDone: d.bytesDone.toInt(),
        bytesTotal: d.bytesTotal?.toInt(),
        waitingFor: d.waitingFor,
        error: d.error,
        folder: d.folder,
        file: d.file,
      ),
  ];

  @override
  Future<void> control(int jobId, TwitchDownloadAction action) =>
      _core.control(jobId: jobId, action: action.name);
}
