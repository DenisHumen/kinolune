// SPDX-License-Identifier: MPL-2.0
//
// Twitch recordings, record rules, saved files, import and storage
// settings (TWI-019), full builds only: the core's `CoreTwitchDownloads`
// (pa-jobs + pa-storage + pa-twitch-archive). The app calls this only
// under `AppFlavor.twitchArchive`, so store builds drop it; the store core
// has none of it either.

import 'package:pa_core/pa_core.dart' as core;

import 'twitch_downloads.dart';

/// What a recording is doing now, for the list's indicator.
enum RecordingActivity {
  /// Writing segments.
  active,

  /// Paused (by the user or the space monitor) or waiting for space.
  paused,

  /// Finished.
  done,

  /// Failed or cancelled.
  failed;

  static RecordingActivity of(TwitchDownloadState s) => switch (s) {
    TwitchDownloadState.running ||
    TwitchDownloadState.queued ||
    TwitchDownloadState.retrying => active,
    TwitchDownloadState.paused || TwitchDownloadState.waiting => paused,
    TwitchDownloadState.done => done,
    _ => failed,
  };
}

/// A live recording job.
class TwitchRecordingItem {
  const TwitchRecordingItem({
    required this.jobId,
    required this.userId,
    required this.state,
    this.login,
    this.streamId,
    this.gaps = 0,
    this.media = Duration.zero,
    this.bytesDone = 0,
    this.waitingFor,
    this.error,
    this.file,
    this.createdAt,
  });

  final int jobId;
  final String userId;
  final String? login;
  final String? streamId;
  final TwitchDownloadState state;

  /// Parts of the broadcast that were not recorded (outage, pause,
  /// restart): shown as a "gap" indicator.
  final int gaps;
  final Duration media;
  final int bytesDone;
  final String? waitingFor;
  final String? error;
  final String? file;
  final DateTime? createdAt;

  RecordingActivity get activity => RecordingActivity.of(state);

  /// Pause/resume and stop of a running recording; remove when finished.
  List<TwitchDownloadAction> get actions => switch (activity) {
    RecordingActivity.active => const [
      TwitchDownloadAction.pause,
      TwitchDownloadAction.cancel,
    ],
    RecordingActivity.paused => const [
      TwitchDownloadAction.resume,
      TwitchDownloadAction.cancel,
    ],
    RecordingActivity.failed when state == TwitchDownloadState.failed =>
      const [TwitchDownloadAction.retry, TwitchDownloadAction.remove],
    _ => const [TwitchDownloadAction.remove],
  };
}

/// A saved file of the library: a download, a recording or an import.
enum TwitchSavedKind { vod, recording, clip }

class TwitchSavedFile {
  const TwitchSavedFile({
    required this.assetId,
    required this.kind,
    required this.userId,
    required this.title,
    required this.path,
    this.login,
    this.displayName,
    this.videoId,
    this.streamId,
    this.recordedAt,
    this.duration,
    this.sizeBytes = 0,
    this.origin = 'download',
    this.hasChat = false,
    this.chapters = 0,
    this.hasAdsMarked = false,
  });

  final int assetId;
  final TwitchSavedKind kind;
  final String userId;
  final String? login;
  final String? displayName;
  final String? videoId;
  final String? streamId;
  final String title;
  final DateTime? recordedAt;
  final Duration? duration;
  final int sizeBytes;
  final String path;

  /// `download`, `recording`, `import`.
  final String origin;
  final bool hasChat;
  final int chapters;
  final bool hasAdsMarked;

  String get channel => displayName ?? login ?? userId;
}

/// Automatic recording of one favourite channel for the profile.
class TwitchRecordRule {
  const TwitchRecordRule({
    required this.userId,
    this.enabled = true,
    this.quality = 'best',
    this.withChat = true,
    this.keepLast = defaultKeepLast,
    this.keepDays = defaultKeepDays,
    this.categories,
  });

  /// Research 06 §4.3 / ROADMAP TWI-019: 14 days or 10 broadcasts.
  static const defaultKeepLast = 10;
  static const defaultKeepDays = 14;

  /// Qualities the rule can pick.
  static const qualities = ['best', '1080p60', '720p', '480p', 'audio_only'];

  final String userId;
  final bool enabled;
  final String quality;
  final bool withChat;

  /// Keep the last N recordings; null keeps all.
  final int? keepLast;

  /// Delete recordings older than this many days; null keeps all.
  final int? keepDays;

  /// Only these categories (game ids); null records every category.
  final Set<String>? categories;

  TwitchRecordRule copyWith({
    bool? enabled,
    String? quality,
    bool? withChat,
    int? Function()? keepLast,
    int? Function()? keepDays,
    Set<String>? Function()? categories,
  }) => TwitchRecordRule(
    userId: userId,
    enabled: enabled ?? this.enabled,
    quality: quality ?? this.quality,
    withChat: withChat ?? this.withChat,
    keepLast: keepLast != null ? keepLast() : this.keepLast,
    keepDays: keepDays != null ? keepDays() : this.keepDays,
    categories: categories != null ? categories() : this.categories,
  );
}

/// Folders, quotas and file name templates.
class TwitchArchiveSettings {
  const TwitchArchiveSettings({
    required this.downloadsDir,
    required this.recordingsDir,
    this.downloadsQuotaBytes,
    this.recordingsQuotaBytes,
    this.folderTemplate = defaultFolder,
    this.vodTemplate = defaultVod,
    this.liveTemplate = defaultLive,
  });

  /// The core's defaults (research 06 §4.3).
  static const defaultFolder = '{login} [{user_id}]/{year}';
  static const defaultVod = '{date} <{categories} — >{title}';
  static const defaultLive = '{date} LIVE <{categories} — >{title}';

  final String downloadsDir;
  final String recordingsDir;
  final int? downloadsQuotaBytes;
  final int? recordingsQuotaBytes;
  final String folderTemplate;
  final String vodTemplate;
  final String liveTemplate;

  TwitchArchiveSettings copyWith({
    String? downloadsDir,
    String? recordingsDir,
    int? Function()? downloadsQuotaBytes,
    int? Function()? recordingsQuotaBytes,
    String? folderTemplate,
    String? vodTemplate,
    String? liveTemplate,
  }) => TwitchArchiveSettings(
    downloadsDir: downloadsDir ?? this.downloadsDir,
    recordingsDir: recordingsDir ?? this.recordingsDir,
    downloadsQuotaBytes: downloadsQuotaBytes != null
        ? downloadsQuotaBytes()
        : this.downloadsQuotaBytes,
    recordingsQuotaBytes: recordingsQuotaBytes != null
        ? recordingsQuotaBytes()
        : this.recordingsQuotaBytes,
    folderTemplate: folderTemplate ?? this.folderTemplate,
    vodTemplate: vodTemplate ?? this.vodTemplate,
    liveTemplate: liveTemplate ?? this.liveTemplate,
  );
}

/// Space of one target.
class TwitchArchiveUsage {
  const TwitchArchiveUsage({
    required this.recordings,
    required this.path,
    required this.used,
    this.quota,
    this.available,
    this.low = false,
  });

  final bool recordings;
  final String path;
  final int used;
  final int? quota;
  final int? available;
  final bool low;
}

/// Result of an import.
class TwitchImportResult {
  const TwitchImportResult({
    this.added = 0,
    this.duplicates = 0,
    this.known = 0,
    this.noUser = 0,
    this.skipped = 0,
    this.chapters = 0,
  });

  final int added;
  final int duplicates;
  final int known;
  final int noUser;
  final int skipped;
  final int chapters;
}

/// Error of a template check, with the core's message.
class TwitchTemplateError implements Exception {
  const TwitchTemplateError(this.message);
  final String message;
  @override
  String toString() => message;
}

/// Recordings, rules, saved files and settings the UI talks to.
abstract interface class TwitchArchive {
  Future<List<TwitchRecordingItem>> recordings();
  Future<void> controlRecording(int jobId, TwitchDownloadAction action);
  Future<int> recordNow(int profileId, String userId, {String? login});

  Future<List<TwitchRecordRule>> rules(int profileId);
  Future<void> setRule(int profileId, TwitchRecordRule rule);
  Future<void> deleteRule(int profileId, String userId);

  Future<List<TwitchSavedFile>> saved({TwitchSavedKind? kind});
  Future<TwitchImportResult> importFolder(String path);
  Future<void> forget(int assetId);

  Future<TwitchArchiveSettings> settings();
  Future<void> setSettings(TwitchArchiveSettings settings);
  Future<List<TwitchArchiveUsage>> usage();

  /// Example paths `(vod, live)` of templates; throws
  /// [TwitchTemplateError].
  (String, String) preview(String folder, String vod, String live);
}

TwitchSavedKind _kind(String s) => switch (s) {
  'recording' => TwitchSavedKind.recording,
  'clip' => TwitchSavedKind.clip,
  _ => TwitchSavedKind.vod,
};

DateTime? _at(int? ms) =>
    ms == null ? null : DateTime.fromMillisecondsSinceEpoch(ms);

/// [TwitchArchive] over the Rust core.
class CoreTwitchArchive implements TwitchArchive {
  CoreTwitchArchive(this._core);

  final core.CoreTwitchDownloads _core;

  /// Starts recordings by the rules from the live monitor's events.
  void followLive(core.CoreTwitchLive live) => _core.followLive(live: live);

  @override
  Future<List<TwitchRecordingItem>> recordings() async => [
    for (final r in await _core.recordings())
      TwitchRecordingItem(
        jobId: r.jobId.toInt(),
        userId: r.userId ?? '',
        login: r.login,
        streamId: r.streamId,
        state: TwitchDownloadState.parse(r.state),
        gaps: r.gaps,
        media: Duration(milliseconds: r.mediaMs.toInt()),
        bytesDone: r.bytesDone.toInt(),
        waitingFor: r.waitingFor,
        error: r.error,
        file: r.file,
        createdAt: _at(r.createdAtMs.toInt()),
      ),
  ];

  @override
  Future<void> controlRecording(int jobId, TwitchDownloadAction action) =>
      _core.control(jobId: jobId, action: action.name);

  @override
  Future<int> recordNow(int profileId, String userId, {String? login}) async =>
      (await _core.recordNow(
        profileId: profileId,
        userId: userId,
        login: login,
      )).toInt();

  @override
  Future<List<TwitchRecordRule>> rules(int profileId) async => [
    for (final r in await _core.recordRules(profileId: profileId))
      TwitchRecordRule(
        userId: r.userId,
        enabled: r.enabled,
        quality: r.quality,
        withChat: r.withChat,
        keepLast: r.keepLastN,
        keepDays: r.retentionDays,
        categories: r.categories?.toSet(),
      ),
  ];

  @override
  Future<void> setRule(int profileId, TwitchRecordRule rule) =>
      _core.setRecordRule(
        profileId: profileId,
        rule: core.CoreRecordRule(
          userId: rule.userId,
          enabled: rule.enabled,
          quality: rule.quality,
          withChat: rule.withChat,
          keepLastN: rule.keepLast,
          retentionDays: rule.keepDays,
          categories: rule.categories?.toList(),
        ),
      );

  @override
  Future<void> deleteRule(int profileId, String userId) =>
      _core.deleteRecordRule(profileId: profileId, userId: userId);

  @override
  Future<List<TwitchSavedFile>> saved({TwitchSavedKind? kind}) async => [
    for (final f in await _core.saved(kind: kind?.name))
      TwitchSavedFile(
        assetId: f.assetId.toInt(),
        kind: _kind(f.kind),
        userId: f.userId,
        login: f.login,
        displayName: f.displayName,
        videoId: f.videoId,
        streamId: f.streamId,
        title: f.title,
        recordedAt: _at(f.recordedAtMs?.toInt()),
        duration: f.durationMs == null
            ? null
            : Duration(milliseconds: f.durationMs!.toInt()),
        sizeBytes: f.sizeBytes.toInt(),
        path: f.path,
        origin: f.origin,
        hasChat: f.hasChat,
        chapters: f.chapters,
        hasAdsMarked: f.hasAdsMarked,
      ),
  ];

  @override
  Future<TwitchImportResult> importFolder(String path) async {
    final r = await _core.importFolder(path: path);
    return TwitchImportResult(
      added: r.added,
      duplicates: r.duplicates,
      known: r.known,
      noUser: r.noUser,
      skipped: r.skipped,
      chapters: r.chapters,
    );
  }

  @override
  Future<void> forget(int assetId) => _core.forget(assetId: assetId);

  @override
  Future<TwitchArchiveSettings> settings() async {
    final s = await _core.settings();
    return TwitchArchiveSettings(
      downloadsDir: s.downloadsDir,
      recordingsDir: s.recordingsDir,
      downloadsQuotaBytes: s.downloadsQuotaBytes?.toInt(),
      recordingsQuotaBytes: s.recordingsQuotaBytes?.toInt(),
      folderTemplate: s.folderTemplate,
      vodTemplate: s.vodTemplate,
      liveTemplate: s.liveTemplate,
    );
  }

  @override
  Future<void> setSettings(TwitchArchiveSettings s) async {
    try {
      await _core.setSettings(
        settings: core.CoreArchiveSettings(
          downloadsDir: s.downloadsDir,
          recordingsDir: s.recordingsDir,
          downloadsQuotaBytes: s.downloadsQuotaBytes == null
              ? null
              : BigInt.from(s.downloadsQuotaBytes!),
          recordingsQuotaBytes: s.recordingsQuotaBytes == null
              ? null
              : BigInt.from(s.recordingsQuotaBytes!),
          folderTemplate: s.folderTemplate,
          vodTemplate: s.vodTemplate,
          liveTemplate: s.liveTemplate,
        ),
      );
    } on core.CoreDownloadError catch (e) {
      if (e.code == 'invalid_template') throw TwitchTemplateError(e.message);
      rethrow;
    }
  }

  @override
  Future<List<TwitchArchiveUsage>> usage() async => [
    for (final u in await _core.usage())
      TwitchArchiveUsage(
        recordings: u.kind == 'recordings',
        path: u.path,
        used: u.used.toInt(),
        quota: u.quota?.toInt(),
        available: u.available?.toInt(),
        low: u.low,
      ),
  ];

  @override
  (String, String) preview(String folder, String vod, String live) {
    try {
      final p = core.archiveNamePreview(
        folderTemplate: folder,
        vodTemplate: vod,
        liveTemplate: live,
      );
      return (p[0], p[1]);
    } on core.CoreDownloadError catch (e) {
      throw TwitchTemplateError(e.message);
    }
  }
}
