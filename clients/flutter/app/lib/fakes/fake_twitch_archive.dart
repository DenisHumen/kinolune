// SPDX-License-Identifier: MPL-2.0
//
// TWI-019: an in-memory TwitchArchive for tests, screenshots and demo
// mode. Made-up channels of FakeTwitchBackend.demo; no files, no network.

import '../twitch/live_notifications.dart' show LiveWatch;
import '../twitch/twitch_archive.dart';
import '../twitch/twitch_downloads.dart';

class FakeTwitchArchive implements TwitchArchive {
  FakeTwitchArchive({
    List<TwitchRecordingItem>? recordings,
    List<TwitchSavedFile>? saved,
    TwitchArchiveSettings? settings,
  }) : _recordings = recordings ?? [],
       _saved = saved ?? [],
       _settings =
           settings ??
           const TwitchArchiveSettings(
             downloadsDir: '/media/twitch/downloads',
             recordingsDir: '/media/twitch/recordings',
             recordingsQuotaBytes: 500 * 1024 * 1024 * 1024,
           );

  /// Recordings in every state, saved files of three origins.
  factory FakeTwitchArchive.demo(DateTime now) {
    DateTime ago(int h) => now.subtract(Duration(hours: h));
    const gb = 1024 * 1024 * 1024;
    return FakeTwitchArchive(
      recordings: [
        TwitchRecordingItem(
          jobId: 41,
          userId: '7002',
          login: 'lunaforge',
          streamId: '90002',
          state: TwitchDownloadState.running,
          media: const Duration(hours: 2, minutes: 14),
          bytesDone: 5 * gb,
          createdAt: ago(2),
        ),
        TwitchRecordingItem(
          jobId: 40,
          userId: '7004',
          login: 'novamarsh',
          streamId: '90004',
          state: TwitchDownloadState.paused,
          gaps: 1,
          waitingFor: 'resources',
          media: const Duration(minutes: 48),
          bytesDone: 2 * gb,
          createdAt: ago(3),
        ),
        TwitchRecordingItem(
          jobId: 38,
          userId: '7001',
          login: 'pixelheron',
          streamId: '90001',
          state: TwitchDownloadState.done,
          gaps: 2,
          media: const Duration(hours: 4, minutes: 2),
          file: '2026-11-13 LIVE Starfall Tactics — Night shift [s90001].mkv',
          createdAt: ago(28),
        ),
      ],
      saved: [
        TwitchSavedFile(
          assetId: 3,
          kind: TwitchSavedKind.recording,
          userId: '7001',
          login: 'pixelheron',
          displayName: 'PixelHeron',
          streamId: '90001',
          title: 'Night shift',
          recordedAt: ago(28),
          duration: const Duration(hours: 4, minutes: 2),
          sizeBytes: 9 * gb,
          path: '/media/twitch/recordings/pixelheron [7001]/2026/x.mkv',
          origin: 'recording',
          hasChat: true,
          hasAdsMarked: true,
        ),
        TwitchSavedFile(
          assetId: 2,
          kind: TwitchSavedKind.vod,
          userId: '7002',
          login: 'lunaforge',
          displayName: 'LunaForge',
          videoId: '880002',
          title: 'Any% practice — Neon Drift',
          recordedAt: ago(50),
          duration: const Duration(hours: 3, minutes: 5),
          sizeBytes: 6 * gb,
          path: '/media/twitch/downloads/lunaforge [7002]/2026/y.mkv',
          hasChat: true,
          chapters: 2,
        ),
        TwitchSavedFile(
          assetId: 1,
          kind: TwitchSavedKind.vod,
          userId: '7003',
          login: 'quietkettle',
          displayName: 'QuietKettle',
          videoId: '880001',
          title: 'Tea and pixels',
          recordedAt: ago(200),
          duration: const Duration(hours: 1, minutes: 40),
          sizeBytes: 2 * gb,
          path: '/media/archive/quietkettle/880001.mp4',
          origin: 'import',
          hasChat: true,
          chapters: 3,
        ),
      ],
    );
  }

  final List<TwitchRecordingItem> _recordings;
  final List<TwitchSavedFile> _saved;
  TwitchArchiveSettings _settings;
  final Map<(int, String), TwitchRecordRule> _rules = {};
  final actions = <(int, TwitchDownloadAction)>[];
  final imported = <String>[];
  TwitchImportResult importResult = const TwitchImportResult(
    added: 3,
    duplicates: 1,
    chapters: 4,
  );

  @override
  Future<List<TwitchRecordingItem>> recordings() async => List.of(_recordings);

  @override
  Future<void> controlRecording(int jobId, TwitchDownloadAction action) async =>
      actions.add((jobId, action));

  @override
  Future<int> recordNow(int profileId, String userId, {String? login}) async {
    _recordings.insert(
      0,
      TwitchRecordingItem(
        jobId: 100 + _recordings.length,
        userId: userId,
        login: login,
        state: TwitchDownloadState.queued,
      ),
    );
    return _recordings.first.jobId;
  }

  @override
  Future<List<TwitchRecordRule>> rules(int profileId) async => [
    for (final e in _rules.entries)
      if (e.key.$1 == profileId) e.value,
  ];

  @override
  Future<void> setRule(int profileId, TwitchRecordRule rule) async =>
      _rules[(profileId, rule.userId)] = rule;

  @override
  Future<void> deleteRule(int profileId, String userId) async =>
      _rules.remove((profileId, userId));

  /// Every profile's enabled rules, once per channel; the login is the
  /// user id here (made-up channels).
  @override
  Future<List<LiveWatch>> recordedChannels() async => [
    for (final id in {
      for (final r in _rules.values)
        if (r.enabled) r.userId,
    })
      LiveWatch(userId: id, login: id, displayName: id, notify: false),
  ];

  @override
  Future<List<TwitchSavedFile>> saved({TwitchSavedKind? kind}) async => [
    for (final f in _saved)
      if (kind == null || f.kind == kind) f,
  ];

  @override
  Future<TwitchImportResult> importFolder(String path) async {
    imported.add(path);
    return importResult;
  }

  @override
  Future<void> forget(int assetId) async =>
      _saved.removeWhere((f) => f.assetId == assetId);

  @override
  Future<TwitchArchiveSettings> settings() async => _settings;

  @override
  Future<void> setSettings(TwitchArchiveSettings settings) async {
    preview(
      settings.folderTemplate,
      settings.vodTemplate,
      settings.liveTemplate,
    );
    _settings = settings;
  }

  @override
  Future<List<TwitchArchiveUsage>> usage() async => [
    TwitchArchiveUsage(
      recordings: false,
      path: _settings.downloadsDir,
      used: 8 * 1024 * 1024 * 1024,
      quota: _settings.downloadsQuotaBytes,
      available: 300 * 1024 * 1024 * 1024,
    ),
    TwitchArchiveUsage(
      recordings: true,
      path: _settings.recordingsDir,
      used: 16 * 1024 * 1024 * 1024,
      quota: _settings.recordingsQuotaBytes,
      available: 300 * 1024 * 1024 * 1024,
    ),
  ];

  /// A rough stand-in for the core's renderer: enough for the preview.
  @override
  (String, String) preview(String folder, String vod, String live) {
    for (final t in [folder, vod, live]) {
      if (t.trim().isEmpty) {
        throw const TwitchTemplateError('empty: the template is empty');
      }
      for (final m in RegExp(r'\{([^}]*)\}').allMatches(t)) {
        if (!_vars.contains(m.group(1))) {
          throw TwitchTemplateError(
            'unknown_variable: unknown variable {${m.group(1)}}',
          );
        }
      }
    }
    String render(String t, String id, String cats) {
      final s = t
          .replaceAllMapped(
            RegExp(r'<([^>]*)>'),
            (m) => m.group(1)!.contains('{part}') ? '' : m.group(1)!,
          )
          .replaceAll('{date}', '2026-11-12')
          .replaceAll('{year}', '2026')
          .replaceAll('{month}', '11')
          .replaceAll('{time}', '18-00')
          .replaceAll('{login}', 'streamer')
          .replaceAll('{display_name}', 'Streamer')
          .replaceAll('{user_id}', '12345')
          .replaceAll('{title}', 'Title here')
          .replaceAll('{categories}', cats)
          .replaceAll('{part}', '')
          .replaceAll('{id}', id.substring(3, id.length - 1))
          .replaceAll('{type}', 'vod');
      return s;
    }

    final dir = render(folder, ' [v2290012345]', '');
    final v =
        '${render(vod, ' [v2290012345]', 'Elden Ring, Hades II')}'
        ' [v2290012345]';
    final l = '${render(live, ' [s51234567890]', 'Elden Ring')} [s51234567890]';
    return ('$dir/$v/$v.mkv', '$dir/$l/$l.mkv');
  }

  static const _vars = {
    'date',
    'year',
    'month',
    'time',
    'login',
    'display_name',
    'user_id',
    'title',
    'categories',
    'part',
    'id',
    'type',
  };
}
