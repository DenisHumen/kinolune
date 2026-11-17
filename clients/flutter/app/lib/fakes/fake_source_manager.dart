// SPDX-License-Identifier: MPL-2.0
//
// In-memory full source manager (like the Rust one) over a [Library], for
// the demo mode, screenshots and widget tests of the Sources screen.

import '../integration/library.dart';
import '../sources/source_manager.dart';
import 'fake_smb_server.dart';

class FakeSourceManager extends SourceManager {
  FakeSourceManager(this.library, {this.numericIds = false});

  final Library library;

  /// New sources get numeric ids, as the Rust core's: what the personal
  /// media calls (UI-034) take.
  final bool numericIds;
  var _nextNumericId = 100;
  final Map<String, ManagedSource> _settings = {};
  final Map<String, SourceStatusInfo> _status = {};

  /// Every draft passed to [create] and [update], for tests.
  final List<SourceDraft> drafts = [];

  /// Thrown by the next [create], [update], [check] or [speedTest] call.
  SourceException? nextError;

  /// Last value of [setSshRsaAllowed], for tests.
  bool? sshRsaAllowed;

  @override
  void setSshRsaAllowed(bool allowed) => sshRsaAllowed = allowed;

  /// Result of [speedTest].
  SpeedResult speed = const SpeedResult(
    latencyMs: 4,
    bytesRead: 64 * 1000 * 1000,
    elapsedMs: 2000,
    bytesPerSec: 32 * 1000 * 1000,
    file: '/Movies/Demo.mkv', // l10n-ignore
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

  void _fail() {
    final e = nextError;
    if (e != null) {
      nextError = null;
      throw e;
    }
  }

  ManagedSource _view(MediaSource s) {
    final m = _settings[s.id];
    final status = _status[s.id] ?? const SourceStatusInfo();
    return ManagedSource(
      id: s.id,
      type:
          m?.type ??
          (s.kind == SourceKind.smb ? SourceType.smb : SourceType.local),
      name: s.name,
      address: m?.address ?? s.path,
      username: s.username,
      hasPassword: m?.hasPassword ?? false,
      smb: m?.smb ?? const SmbSettings(),
      tlsPin: m?.tlsPin,
      scanIntoLibrary: m?.scanIntoLibrary ?? true,
      autoScan: m?.autoScan ?? true,
      sync: m?.sync ?? SourceSyncMode.off,
      status: status,
    );
  }

  ManagedSource _store(String id, SourceDraft d, {bool? hadPassword}) {
    final pwd = d.password;
    if (pwd != null) _passwords[id] = pwd;
    final m = ManagedSource(
      id: id,
      type: d.type,
      name: d.name,
      address: d.address,
      username: d.username,
      hasPassword: pwd == null ? (hadPassword ?? false) : pwd.isNotEmpty,
      smb: d.smb,
      tlsPin: d.tlsPin,
      scanIntoLibrary: d.scanIntoLibrary,
      autoScan: d.autoScan,
      sync: d.type == SourceType.local ? SourceSyncMode.off : d.sync,
    );
    _settings[id] = m;
    return m;
  }

  MediaSource _media(String id, SourceDraft d) => MediaSource(
    id: id,
    kind: d.type == SourceType.smb ? SourceKind.smb : SourceKind.localFolder,
    name: d.name,
    path: d.address,
    username: d.username,
  );

  @override
  Future<List<ManagedSource>> list() async =>
      (await library.sources()).map(_view).toList();

  @override
  Future<ManagedSource> create(SourceDraft draft) async {
    drafts.add(draft);
    _fail();
    final added = await library.addSource(
      _media(numericIds ? '${_nextNumericId++}' : '', draft),
    );
    _store(added.id, draft);
    return _view(added);
  }

  @override
  Future<ManagedSource> update(String id, SourceDraft draft) async {
    drafts.add(draft);
    _fail();
    final had = _settings[id]?.hasPassword;
    final saved = await library.addSource(_media(id, draft));
    _store(id, draft, hadPassword: had);
    return _view(saved);
  }

  @override
  Future<void> delete(String id) async {
    _settings.remove(id);
    _status.remove(id);
  }

  @override
  Future<SourceStatusInfo> check(String id) async {
    final now = DateTime.now();
    final prev = _status[id] ?? const SourceStatusInfo();
    final e = nextError;
    nextError = null;
    final status = SourceStatusInfo(
      online: e == null,
      checkedAt: now,
      lastScanStartedAt: prev.lastScanStartedAt,
      lastScanFinishedAt: prev.lastScanFinishedAt,
      lastError: e?.detail ?? prev.lastError,
      lastErrorAt: e == null ? prev.lastErrorAt : now,
      speedBytesPerSec: prev.speedBytesPerSec,
      latencyMs: prev.latencyMs,
      speedMeasuredAt: prev.speedMeasuredAt,
    );
    _status[id] = status;
    return status;
  }

  /// SMB servers the step-by-step setup can reach (UI-028).
  List<FakeSmbServer> smbServers = [FakeSmbServer(address: '192.168.1.10')];

  /// Every SMB login used, for tests.
  final List<SmbLogin> smbLogins = [];

  /// Delay of each SMB call, so screenshots and tests can see progress.
  Duration smbDelay = Duration.zero;

  Future<FakeSmbServer> _smb(SmbLogin login) async {
    smbLogins.add(login);
    if (smbDelay > Duration.zero) await Future<void>.delayed(smbDelay);
    _fail();
    var resolved = login;
    final id = login.sourceId;
    if (id != null) {
      final m = _settings[id];
      final host = login.server.isNotEmpty
          ? login.server
          : (Uri.tryParse(m?.address ?? '')?.host ?? '');
      resolved = SmbLogin(
        server: host,
        username: login.username ?? m?.username,
        password: login.password ?? _passwords[id],
        smb: login.smb,
      );
    }
    final server = smbServers
        .where((s) => s.address == resolved.server.trim())
        .firstOrNull;
    if (server == null) {
      throw SourceException(
        SourceErrorCode.unreachable,
        'source unavailable: smb://${login.server}', // l10n-ignore
      );
    }
    _lastLogin = resolved;
    return server;
  }

  SmbLogin? _lastLogin;
  final Map<String, String> _passwords = {};

  /// Whether the step-by-step SMB setup is offered (tests of the plain SMB
  /// form turn it off).
  bool browseSmb = true;

  @override
  bool get canBrowseSmb => browseSmb;

  @override
  Future<List<SmbShareInfo>> smbShares(SmbLogin login) async =>
      (await _smb(login)).listShares(_lastLogin!);

  @override
  Future<List<SourceFolderEntry>> smbListFolder(
    SmbLogin login,
    String share,
    String folder,
  ) async => (await _smb(login)).list(_lastLogin!, share, folder);

  @override
  Future<List<SmbPlace>> smbFind(
    SmbLogin login,
    String share,
    String folder,
  ) async => (await _smb(login)).find(_lastLogin!, share, folder);

  @override
  Future<SpeedResult> speedTest(String id, {int maxSeconds = 10}) async {
    _fail();
    final now = DateTime.now();
    final prev = _status[id] ?? const SourceStatusInfo();
    _status[id] = SourceStatusInfo(
      online: true,
      checkedAt: now,
      lastScanStartedAt: prev.lastScanStartedAt,
      lastScanFinishedAt: prev.lastScanFinishedAt,
      lastError: prev.lastError,
      lastErrorAt: prev.lastErrorAt,
      speedBytesPerSec: speed.bytesPerSec,
      latencyMs: speed.latencyMs,
      speedMeasuredAt: now,
    );
    return speed;
  }
}
