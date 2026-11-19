// SPDX-License-Identifier: MPL-2.0

import '../integration/sources.dart';
import 'source_manager.dart';

/// [SourceManager] on the Rust source manager (`pa-sources`, NET-011).
///
/// It works on the same `sources` table as the Rust library (`CoreLibrary`),
/// so a source created here is scanned by `Library.scan` with the same id.
class CoreSourceManager extends SourceManager {
  CoreSourceManager(this._core);

  final CoreSources _core;

  /// Opens the manager over the core database at [dbPath]; passwords go to
  /// the secret store opened at app start (`openPlatformSecrets`, Android
  /// Keystore; CORE-015), else the OS keychain or an encrypted file in
  /// [dataDir].
  static Future<CoreSourceManager> open({
    required String dbPath,
    required String dataDir,
  }) async => CoreSourceManager(
    await CoreSources.open(dbPath: dbPath, dataDir: dataDir),
  );

  @override
  bool get reduced => false;

  @override
  Set<SourceType> get creatableTypes => SourceType.available;

  @override
  bool get canEdit => true;

  @override
  bool get canSpeedTest => true;

  @override
  bool get canSync => true;

  @override
  String? get secretWarning => _core.secretFallbackReason();

  static int _id(String id) {
    final n = int.tryParse(id);
    if (n == null) {
      throw SourceException(SourceErrorCode.sourceMissing, 'source $id');
    }
    return n;
  }

  /// Runs [call] and turns bridge errors into [SourceException]s.
  static Future<T> _guard<T>(Future<T> Function() call) async {
    try {
      return await call();
    } on SourcesError catch (e) {
      throw SourceException.fromCore(e.code, e.message);
    }
  }

  static DateTime? _time(int? ms) =>
      ms == null ? null : DateTime.fromMillisecondsSinceEpoch(ms);

  static SourceStatusInfo _status(CoreSourceStatus s) => SourceStatusInfo(
    online: s.online,
    checkedAt: _time(s.checkedAtMs),
    lastScanStartedAt: _time(s.lastScanStartedAtMs),
    lastScanFinishedAt: _time(s.lastScanFinishedAtMs),
    lastError: s.lastError,
    lastErrorAt: _time(s.lastErrorAtMs),
    speedBytesPerSec: s.speedBytesPerSec?.toInt(),
    latencyMs: s.speedLatencyMs?.toInt(),
    speedMeasuredAt: _time(s.speedMeasuredAtMs),
  );

  /// The app's view of a source of the core.
  static ManagedSource sourceOf(CoreManagedSource s) => _source(s);

  static ManagedSource _source(CoreManagedSource s) => ManagedSource(
    id: '${s.id}',
    type: SourceType.fromCoreKind(s.kind) ?? SourceType.local,
    name: s.name,
    address: s.uri,
    username: s.username,
    hasPassword: s.hasPassword,
    smb: SmbSettings(
      domain: s.options.smbDomain,
      version: s.options.smbVersion,
      encryption: s.options.smbEncryption,
      signingRequired: s.options.smbSigningRequired,
    ),
    tlsPin: s.options.tlsPin,
    sshHostKey: s.options.sshHostKey,
    altAddresses: s.options.jellyfinAltAddresses,
    scanIntoLibrary: s.scanIntoLibrary,
    autoScan: s.autoScan,
    enabled: s.enabled,
    sync: SourceSyncMode.fromCore(s.sync_),
    status: _status(s.status),
    lanName: s.options.lanName,
  );

  static CoreSourceInput _input(SourceDraft d) => CoreSourceInput(
    kind: d.type.coreKind,
    name: d.name,
    address: d.address,
    username: d.username,
    password: d.password,
    sshPrivateKey: d.type == SourceType.sftp ? d.sshPrivateKey : null,
    options: CoreSourceOptions(
      smbDomain: d.type == SourceType.smb ? d.smb.domain : null,
      smbVersion: d.smb.version,
      smbSigningRequired: d.smb.signingRequired,
      smbEncryption: d.smb.encryption,
      tlsPin: d.type == SourceType.webdav || d.type == SourceType.ftp
          ? d.tlsPin
          : null,
      sshHostKey: d.type == SourceType.sftp ? d.sshHostKey : null,
      jellyfinAltAddresses: d.type.mediaServer ? d.altAddresses : const [],
      lanName: d.lanName,
    ),
    scanIntoLibrary: d.scanIntoLibrary,
    autoScan: d.autoScan,
    enabled: true,
    // Local folders never sync (a path means nothing on another device).
    sync_: d.type == SourceType.local ? 'off' : d.sync.coreName,
  );

  @override
  void setSshRsaAllowed(bool allowed) =>
      sourcesSetSshRsaAllowed(allowed: allowed);

  @override
  Future<List<ManagedSource>> list() =>
      _guard(() async => (await _core.list()).map(_source).toList());

  @override
  Future<ManagedSource> create(SourceDraft draft) {
    // A Jellyfin, Emby or Plex server is added by signing in (JellyfinLink,
    // PlexLink).
    if (!SourceType.available.contains(draft.type) ||
        draft.type.mediaServer ||
        draft.type.plexServer) {
      throw SourceException(SourceErrorCode.unsupported, draft.type.name);
    }
    final pwd = draft.password;
    final input = _input(
      SourceDraft(
        type: draft.type,
        name: draft.name,
        address: draft.address,
        username: draft.username,
        password: pwd == null || pwd.isEmpty ? null : pwd,
        smb: draft.smb,
        tlsPin: draft.tlsPin,
        sshHostKey: draft.sshHostKey,
        sshPrivateKey: draft.sshPrivateKey,
        scanIntoLibrary: draft.scanIntoLibrary,
        autoScan: draft.autoScan,
        sync: draft.sync,
      ),
    );
    return _guard(() async => _source(await _core.create(input: input)));
  }

  @override
  Future<ManagedSource> update(String id, SourceDraft draft) => _guard(
    () async => _source(await _core.update(id: _id(id), input: _input(draft))),
  );

  @override
  Future<void> delete(String id) => _guard(() => _core.delete(id: _id(id)));

  @override
  Future<SourceStatusInfo> check(String id) =>
      _guard(() async => _status(await _core.check(id: _id(id))));

  @override
  Future<List<SourceFolderEntry>> listFolder(String id, String dir) =>
      _guard(() async {
        final entries = await _core.listFolder(id: _id(id), dir: dir);
        return [
          for (final e in entries)
            SourceFolderEntry(
              name: e.name,
              relPath: e.relPath,
              isFolder: e.isFolder,
              sizeBytes: e.sizeBytes.toInt(),
              modified: _time(e.modifiedMs),
            ),
        ];
      });

  @override
  bool get canBrowseSmb => true;

  static CoreSmbLogin _login(SmbLogin l) => CoreSmbLogin(
    sourceId: l.sourceId == null ? null : _id(l.sourceId!),
    server: l.server,
    username: l.username,
    password: l.password,
    options: CoreSourceOptions(
      smbDomain: l.smb.domain,
      smbVersion: l.smb.version,
      smbSigningRequired: l.smb.signingRequired,
      smbEncryption: l.smb.encryption,
      jellyfinAltAddresses: const [],
      lanName: null,
    ),
  );

  @override
  Future<List<SmbShareInfo>> smbShares(SmbLogin login) => _guard(
    () async => [
      for (final s in await _core.smbShares(login: _login(login)))
        SmbShareInfo(s.name, s.comment),
    ],
  );

  @override
  Future<List<SourceFolderEntry>> smbListFolder(
    SmbLogin login,
    String share,
    String folder,
  ) => _guard(
    () async => [
      for (final e in await _core.smbListFolder(
        login: _login(login),
        share: share,
        folder: folder,
      ))
        SourceFolderEntry(
          name: e.name,
          relPath: e.relPath,
          isFolder: e.isFolder,
          sizeBytes: e.sizeBytes.toInt(),
          modified: _time(e.modifiedMs),
        ),
    ],
  );

  @override
  Future<List<SmbPlace>> smbFind(SmbLogin login, String share, String folder) =>
      _guard(
        () async => [
          for (final p in await _core.smbFind(
            login: _login(login),
            share: share,
            folder: folder,
          ))
            SmbPlace(p.share, p.folder),
        ],
      );

  @override
  Future<SpeedResult> speedTest(String id, {int maxSeconds = 10}) =>
      _guard(() async {
        final r = await _core.speedTest(id: _id(id), maxSeconds: maxSeconds);
        return SpeedResult(
          latencyMs: r.latencyMs.toInt(),
          bytesRead: r.bytesRead.toInt(),
          elapsedMs: r.elapsedMs.toInt(),
          bytesPerSec: r.bytesPerSec?.toInt(),
          file: r.file,
        );
      });
}
