// SPDX-License-Identifier: MPL-2.0
//
// The app's view of media sources (UI-005): what the Sources screen and the
// add/edit forms work with. Two implementations:
//
// * CoreSourceManager — the Rust source manager (`pa-sources`, NET-011)
//   through pa_core: local folders, SMB, WebDAV, Subsonic music servers
//   (NET-016), DLNA servers and M3U playlists (NET-018) with status, checks
//   and speed tests; passwords in the OS secret store.
// * LibrarySourceManager — the v0 fallback when the Rust core is not
//   available: local folders of the Dart library only, no status.

/// Kind of a source. Only [available] kinds can be added today; the rest
/// are shown as "coming soon" in the add flow.
enum SourceType {
  local,
  smb,
  webdav,
  nfs,
  sftp,
  ftp,
  plex,
  jellyfin,
  emby,
  subsonic,
  dlna,
  playlist;

  /// Kinds the Rust source manager supports (NET-011; Jellyfin: NET-014;
  /// Emby: NET-015; SFTP and FTP: NET-006; Subsonic and OpenSubsonic music
  /// servers such as Navidrome: NET-016; UPnP/DLNA servers and M3U
  /// playlists: NET-018).
  static const available = {
    local,
    smb,
    webdav,
    sftp,
    ftp,
    jellyfin,
    emby,
    subsonic,
    dlna,
    playlist,
  };

  /// `pa-sources` kind string (`local`, `smb`, `webdav`, `sftp`, `ftp`,
  /// `jellyfin`, `emby`, `subsonic`, `dlna`, `playlist`).
  String get coreKind => name;

  static SourceType? fromCoreKind(String kind) => switch (kind) {
    'local' => local,
    'smb' => smb,
    'webdav' => webdav,
    'sftp' => sftp,
    'ftp' => ftp,
    'jellyfin' => jellyfin,
    'emby' => emby,
    'subsonic' => subsonic,
    'dlna' => dlna,
    'playlist' => playlist,
    _ => null,
  };

  /// Kinds the library scan can index: the core reads local folders, SMB
  /// and WebDAV through `pa-vfs` (LIB-018), a Jellyfin or Emby server's
  /// libraries as a file tree (NET-014, NET-015) and a Subsonic server's
  /// songs as music files (NET-016).
  bool get indexable => available.contains(this);

  /// A media server: added by signing in (`JellyfinLink`), not with a
  /// [SourceDraft].
  bool get mediaServer => this == jellyfin || this == emby;

  /// A Subsonic music server (NET-016): added with the source form like
  /// WebDAV (address, user name, password — or an OpenSubsonic API key
  /// without a user name); its songs go to the music section.
  bool get musicServer => this == subsonic;

  /// A source without a login (NET-018): a UPnP/DLNA server (open to the
  /// home network) or an M3U playlist. The form has no user name,
  /// password or sync of the password.
  bool get openSource => this == dlna || this == playlist;

  /// A folder of files the app may write into: it can carry the journals of
  /// sync through a shared folder (SYNC-007).
  bool get folderSyncable =>
      const {local, smb, webdav, sftp, ftp}.contains(this);
}

/// Whether a source syncs to the paired devices (SYNC-010,
/// `pa_sources::SourceSync`). Network sources only.
enum SourceSyncMode {
  /// Stays on this device.
  off,

  /// Settings sync; each device keeps its own password.
  settings,

  /// Settings and the password, sealed end-to-end with the family key.
  password;

  /// Core value (`off`, `settings`, `password`).
  String get coreName => name;

  static SourceSyncMode fromCore(String v) => switch (v) {
    'settings' => settings,
    'password' => password,
    _ => off,
  };
}

/// SMB protocol and security options (`pa_sources::SourceOptions`).
class SmbSettings {
  const SmbSettings({
    this.domain,
    this.version = 'any',
    this.encryption = 'auto',
    this.signingRequired = false,
  });

  final String? domain;

  /// `any`, `smb2` or `smb3`.
  final String version;

  /// `auto`, `required` or `off`.
  final String encryption;
  final bool signingRequired;

  static const versions = ['any', 'smb2', 'smb3']; // l10n-ignore
  static const encryptions = ['auto', 'required', 'off']; // l10n-ignore
}

/// Device-local status of a source.
class SourceStatusInfo {
  const SourceStatusInfo({
    this.online,
    this.checkedAt,
    this.lastScanStartedAt,
    this.lastScanFinishedAt,
    this.lastError,
    this.lastErrorAt,
    this.speedBytesPerSec,
    this.latencyMs,
    this.speedMeasuredAt,
  });

  /// Result of the last check; null if never checked.
  final bool? online;
  final DateTime? checkedAt;
  final DateTime? lastScanStartedAt;
  final DateTime? lastScanFinishedAt;

  /// Last connection or scan error as the core wrote it (secrets masked).
  final String? lastError;
  final DateTime? lastErrorAt;
  final int? speedBytesPerSec;
  final int? latencyMs;
  final DateTime? speedMeasuredAt;

  bool get scanning => lastScanStartedAt != null && lastScanFinishedAt == null;

  /// The last error, if nothing has succeeded since: a successful check or
  /// a scan started after it hides it.
  String? get currentError {
    final at = lastErrorAt;
    if (lastError == null || at == null) return null;
    final okCheck = online == true ? checkedAt : null;
    for (final later in [okCheck, lastScanStartedAt]) {
      if (later != null && later.isAfter(at)) return null;
    }
    return lastError;
  }
}

/// A stored source.
class ManagedSource {
  const ManagedSource({
    required this.id,
    required this.type,
    required this.name,
    required this.address,
    this.username,
    this.hasPassword = false,
    this.smb = const SmbSettings(),
    this.tlsPin,
    this.sshHostKey,
    this.altAddresses = const [],
    this.scanIntoLibrary = true,
    this.autoScan = true,
    this.enabled = true,
    this.sync = SourceSyncMode.off,
    this.status = const SourceStatusInfo(),
  });

  /// Same id as the library's `MediaSource.id`.
  final String id;
  final SourceType type;
  final String name;

  /// Folder path (local) or URI without credentials.
  final String address;
  final String? username;
  final bool hasPassword;
  final SmbSettings smb;

  /// WebDAV and FTPS certificate pin, `sha256:<64 hex>`.
  final String? tlsPin;

  /// SFTP host key (`ssh-ed25519 SHA256:…`), stored by the core on the
  /// first connect (NET-006).
  final String? sshHostKey;

  /// Jellyfin: other addresses of the server, tried when [address] cannot
  /// be reached.
  final List<String> altAddresses;
  final bool scanIntoLibrary;
  final bool autoScan;
  final bool enabled;

  /// Sync to the paired devices (SYNC-010).
  final SourceSyncMode sync;
  final SourceStatusInfo status;
}

/// Settings of a new or edited source, as the form produced them.
class SourceDraft {
  const SourceDraft({
    required this.type,
    required this.name,
    required this.address,
    this.username,
    this.password,
    this.smb = const SmbSettings(),
    this.tlsPin,
    this.sshHostKey,
    this.sshPrivateKey,
    this.altAddresses = const [],
    this.scanIntoLibrary = true,
    this.autoScan = true,
    this.sync = SourceSyncMode.off,
  });

  final SourceType type;
  final String name;
  final String address;
  final String? username;

  /// On create: the password (null or empty: none). On update: null keeps
  /// the stored one, an empty string removes it.
  final String? password;
  final SmbSettings smb;
  final String? tlsPin;

  /// SFTP: the pinned host key; empty lets the next connect learn it.
  final String? sshHostKey;

  /// SFTP: a private key to log in with; [password] is then its
  /// passphrase. Goes to the secret store only.
  final String? sshPrivateKey;

  /// Jellyfin: other addresses of the server.
  final List<String> altAddresses;
  final bool scanIntoLibrary;
  final bool autoScan;
  final SourceSyncMode sync;

  @override
  String toString() =>
      'SourceDraft(${type.name}, $name, $address, user: $username, '
      'password: ${password == null ? 'null' : '***'}, '
      'key: ${sshPrivateKey == null ? 'null' : '***'})';
}

/// The draft that saves [m] as it is, with [password] (null: keep).
SourceDraft draftOf(ManagedSource m, {String? password}) => SourceDraft(
  type: m.type,
  name: m.name,
  address: m.address,
  username: m.username,
  password: password,
  smb: m.smb,
  tlsPin: m.tlsPin,
  sshHostKey: m.sshHostKey,
  altAddresses: m.altAddresses,
  scanIntoLibrary: m.scanIntoLibrary,
  autoScan: m.autoScan,
  sync: m.sync,
);

/// Result of a speed test.
class SpeedResult {
  const SpeedResult({
    required this.latencyMs,
    required this.bytesRead,
    required this.elapsedMs,
    this.bytesPerSec,
    this.file,
  });

  final int latencyMs;
  final int bytesRead;
  final int elapsedMs;

  /// Null when no file was found to read (only the latency was measured).
  final int? bytesPerSec;
  final String? file;
}

/// What went wrong, independent of the language. The Sources screen turns it
/// into a readable sentence (`sourceErrorText`).
///
/// The Rust core has no stable error codes yet (CORE-009): the bridge gives
/// `SourcesError.code` (`not_found`, `invalid`, `secrets`, `unreachable`,
/// `other`) and the message holds the `pa-vfs` error kind as text
/// ("source 3: authentication failed: …"). [classifySourceError] maps both;
/// it moves to the CORE-009 codes once they exist.
enum SourceErrorCode {
  /// The source was removed meanwhile.
  sourceMissing,

  /// The core refused the settings (bad address, option).
  invalidSettings,

  /// The OS keychain is locked or unavailable.
  secretStore,

  /// The OS keychain is locked (CORE-007).
  secretStoreLocked,

  /// The OS keychain did not answer in time (CORE-007).
  secretStoreTimeout,
  authFailed,
  permissionDenied,

  /// Host down, connection refused, share offline.
  unreachable,
  timedOut,

  /// The SFTP server shows another host key than the stored one (NET-006).
  hostKeyChanged,

  /// An RSA host or user key while RSA keys are off in the settings.
  rsaDisabled,

  /// The folder inside the share does not exist.
  pathNotFound,

  /// SMB: the server has no shared folder with this name (UI-028). Not a
  /// moved folder or a disconnected drive: another share must be picked.
  shareNotFound,

  /// SMB: the login worked, but the server does not show its shared
  /// folders (UI-031): the share name has to be typed.
  shareListRefused,
  notADirectory,
  invalidPath,

  /// The backend is missing on this device (e.g. libsmb2 not installed).
  unsupported,

  /// The scan cannot read this kind of source yet.
  notIndexable,
  other,
}

/// Error of a source operation.
class SourceException implements Exception {
  const SourceException(this.code, [this.detail = '']);

  /// Classifies an error of the Rust bridge (`SourcesError`).
  factory SourceException.fromCore(String code, String message) =>
      SourceException(classifySourceError(code, message), message);

  final SourceErrorCode code;

  /// Technical message (secrets masked), shown under "Details".
  final String detail;

  @override
  String toString() => 'SourceException(${code.name}: $detail)';
}

/// Maps a bridge error code and message to a [SourceErrorCode].
SourceErrorCode classifySourceError(String code, String message) =>
    switch (code) {
      'not_found' => SourceErrorCode.sourceMissing,
      'invalid' => SourceErrorCode.invalidSettings,
      'secrets' => classifySecretStoreMessage(message),
      _ => classifySourceMessage(message),
    };

/// A `secrets` error by the `pa-secrets` error it carries
/// (`SecretError` as text): locked, timed out, or anything else.
SourceErrorCode classifySecretStoreMessage(String message) {
  final text = message.toLowerCase();
  const timeout = 'did not answer'; // l10n-ignore
  const locked = 'is locked'; // l10n-ignore
  if (text.contains(timeout)) return SourceErrorCode.secretStoreTimeout;
  if (text.contains(locked)) return SourceErrorCode.secretStoreLocked;
  return SourceErrorCode.secretStore;
}

/// `pa-vfs` error kinds as `VfsError` prints them (`<kind>: <message>`), and
/// the scan's own fatal messages (pa-catalog).
const _kindTexts = <(String, SourceErrorCode)>[
  ('authentication failed', SourceErrorCode.authFailed), // l10n-ignore
  ('permission denied', SourceErrorCode.permissionDenied), // l10n-ignore
  ('source unavailable', SourceErrorCode.unreachable), // l10n-ignore
  ('timed out', SourceErrorCode.timedOut), // l10n-ignore
  ('not a directory', SourceErrorCode.notADirectory), // l10n-ignore
  ('invalid path', SourceErrorCode.invalidPath), // l10n-ignore
  ('unsupported', SourceErrorCode.unsupported), // l10n-ignore
  ('no local path', SourceErrorCode.notIndexable), // l10n-ignore
  ('folder not found', SourceErrorCode.pathNotFound), // l10n-ignore
  ('not found', SourceErrorCode.pathNotFound), // l10n-ignore
];

const _specificTexts = <(String, SourceErrorCode)>[
  ('host key changed', SourceErrorCode.hostKeyChanged), // l10n-ignore
  ('rsa keys are turned off', SourceErrorCode.rsaDisabled), // l10n-ignore
  // `pa_vfs_smb::SHARE_NOT_FOUND` (UI-028): a "not found" of the share.
  ('share not found', SourceErrorCode.shareNotFound), // l10n-ignore
  // `pa_vfs_smb::SHARE_LIST_REFUSED` (UI-031): logged in, no share list.
  ('share list refused', SourceErrorCode.shareListRefused), // l10n-ignore
];

/// Classifies a stored `last_error` or an error message by the `pa-vfs`
/// kind it names. The earliest kind in the text wins, so a path that
/// happens to contain "not found" does not override the kind before it.
SourceErrorCode classifySourceMessage(String message) {
  final text = message.toLowerCase();
  // SFTP (NET-006): more specific than the kind they come with
  // (`authentication failed`, `unsupported`).
  for (final (marker, code) in _specificTexts) {
    if (text.contains(marker)) return code;
  }
  SourceErrorCode? best;
  var bestAt = text.length + 1;
  for (final (kind, code) in _kindTexts) {
    final at = text.indexOf(kind);
    if (at >= 0 && at < bestAt) {
      best = code;
      bestAt = at;
    }
  }
  return best ?? SourceErrorCode.other;
}

/// An entry of a live folder listing ([SourceManager.listFolder]).
class SourceFolderEntry {
  const SourceFolderEntry({
    required this.name,
    required this.relPath,
    required this.isFolder,
    this.sizeBytes = 0,
    this.modified,
  });

  final String name;

  /// Relative to the source root, `/`-separated, no leading slash.
  final String relPath;
  final bool isFolder;

  /// Files: size in bytes; folders: 0.
  final int sizeBytes;
  final DateTime? modified;

  @override
  String toString() => isFolder ? '$relPath/' : relPath;
}

/// Server and logon for picking a folder on an SMB server (UI-028).
class SmbLogin {
  const SmbLogin({
    required this.server,
    this.username,
    this.password,
    this.sourceId,
    this.smb = const SmbSettings(),
  });

  /// `host` or `host:port`.
  final String server;

  /// Null with [sourceId]: the source's user. Empty: guest.
  final String? username;

  /// Null with [sourceId]: the stored password.
  final String? password;

  /// A stored source whose password is used (fixing a broken source).
  final String? sourceId;
  final SmbSettings smb;

  bool get guest => (username ?? '').trim().isEmpty && sourceId == null;

  @override
  String toString() =>
      'SmbLogin($server, user: $username, '
      'password: ${password == null ? 'null' : '***'}, source: $sourceId)';
}

/// A shared folder of an SMB server.
class SmbShareInfo {
  const SmbShareInfo(this.name, [this.comment = '']);

  final String name;
  final String comment;
}

/// A folder inside a share: what an SMB source points to.
class SmbPlace {
  const SmbPlace(this.share, [this.folder = '']);

  final String share;

  /// `/`-separated, empty for the share itself.
  final String folder;

  @override
  bool operator ==(Object other) =>
      other is SmbPlace && other.share == share && other.folder == folder;

  @override
  int get hashCode => Object.hash(share, folder);

  @override
  String toString() => folder.isEmpty ? share : '$share/$folder';
}

/// The source list and its operations. Implementations: CoreSourceManager
/// (Rust core) and LibrarySourceManager (v0 fallback).
abstract class SourceManager {
  /// True on the v0 fallback: local folders only, no status or speed test.
  bool get reduced;

  /// Kinds that can be added.
  Set<SourceType> get creatableTypes;

  bool get canEdit;
  bool get canSpeedTest;

  /// Whether network sources can sync to the paired devices (SYNC-010).
  bool get canSync => false;

  /// Why passwords are kept in an encrypted file instead of the OS
  /// keychain (shown as a warning), or null.
  String? get secretWarning => null;

  /// Whether SFTP sources accept RSA keys (NET-006, Settings → "RSA keys").
  /// Only the core's manager has SFTP; the others ignore it.
  void setSshRsaAllowed(bool allowed) {}

  Future<List<ManagedSource>> list();
  Future<ManagedSource> create(SourceDraft draft);
  Future<ManagedSource> update(String id, SourceDraft draft);

  /// Forgets the source's settings and password. The app removes its
  /// library items separately (`Library.removeSource`).
  Future<void> delete(String id);

  /// Checks that the source can be reached; the result is in the status.
  Future<SourceStatusInfo> check(String id);
  Future<SpeedResult> speedTest(String id, {int maxSeconds = 10});

  /// Folders and files of folder [dir] (relative, `''` = root) of source
  /// [id], read from the source now (UI-006 file browser), in no particular
  /// order. Throws [SourceException] with [SourceErrorCode.unsupported]
  /// where the manager cannot list (the file browser then shows what the
  /// library knows).
  Future<List<SourceFolderEntry>> listFolder(String id, String dir) async =>
      throw const SourceException(SourceErrorCode.unsupported);

  /// Whether SMB servers can be browsed (UI-028: the step-by-step add flow
  /// and "Choose folder"); without it the plain SMB form is shown.
  bool get canBrowseSmb => false;

  /// Shared folders of the server, without printers and administrative
  /// shares. Also checks the logon: wrong user or password throws
  /// [SourceErrorCode.authFailed].
  Future<List<SmbShareInfo>> smbShares(SmbLogin login) async =>
      throw const SourceException(SourceErrorCode.unsupported);

  /// Folders and files of [folder] (`''` = the share itself) in [share].
  Future<List<SourceFolderEntry>> smbListFolder(
    SmbLogin login,
    String share,
    String folder,
  ) async => throw const SourceException(SourceErrorCode.unsupported);

  /// Where a typed path really is on the server ([share] and [folder] as
  /// the user typed them, possibly a path as on the server's disk):
  /// verified places, most specific first; empty if nothing fits.
  Future<List<SmbPlace>> smbFind(
    SmbLogin login,
    String share,
    String folder,
  ) async => throw const SourceException(SourceErrorCode.unsupported);
}
