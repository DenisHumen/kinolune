// SPDX-License-Identifier: MPL-2.0
//
// Form input → source address (UI-005). Pure functions, so the form rules
// are tested without widgets. The core checks addresses again (`pa-sources`).

/// Why a form field is not acceptable; `sourceFormErrorText` gives the text.
enum SourceFormError {
  folderEmpty,
  folderNotAbsolute,
  hostEmpty,
  shareEmpty,
  urlEmpty,
  urlScheme,

  /// SFTP/FTP address with another scheme or without a host.
  serverScheme,
  credentialsInAddress,
  tlsPin,

  /// An M3U playlist that is neither an http(s) link nor a full path.
  playlistAddress,
}

/// A built address, or why it cannot be built.
class AddressResult {
  const AddressResult.ok(String this.address) : error = null;
  const AddressResult.fail(SourceFormError this.error) : address = null;

  final String? address;
  final SourceFormError? error;
}

final _windowsPath = RegExp(r'^([A-Za-z]:[\\/]|\\\\)');

/// A local folder: an absolute path (POSIX, `C:\…` or `\\server\share`).
AddressResult localAddress(String input) {
  final path = input.trim();
  if (path.isEmpty) {
    return const AddressResult.fail(SourceFormError.folderEmpty);
  }
  if (!path.startsWith('/') && !_windowsPath.hasMatch(path)) {
    return const AddressResult.fail(SourceFormError.folderNotAbsolute);
  }
  return AddressResult.ok(path);
}

/// Parts of an `smb://host/share/folder` address.
class SmbParts {
  const SmbParts(this.host, this.share, [this.folder = '']);

  final String host;
  final String share;

  /// Path inside the share, without leading or trailing slashes.
  final String folder;
}

String _trimSlashes(String s) =>
    s.replaceAll('\\', '/').replaceAll(RegExp(r'^/+|/+$'), '');

/// Splits `smb://host/share/a/b`, `\\host\share\a` or `host/share/a`.
/// Null if there is no host.
SmbParts? splitSmb(String input) {
  var s = input.trim().replaceAll('\\', '/');
  if (s.toLowerCase().startsWith('smb://')) s = s.substring(6);
  s = _trimSlashes(s);
  if (s.isEmpty) return null;
  final parts = s.split('/').where((p) => p.isNotEmpty).toList();
  return SmbParts(
    parts.first,
    parts.length > 1 ? parts[1] : '',
    parts.length > 2 ? parts.sublist(2).join('/') : '',
  );
}

/// `smb://host/share[/folder]` from the form fields. A full address pasted
/// into [host] is split.
AddressResult smbAddress({
  required String host,
  required String share,
  String folder = '',
}) {
  var h = host.trim();
  var sh = _trimSlashes(share.trim());
  var f = _trimSlashes(folder.trim());
  if (h.contains('/') || h.contains('\\')) {
    final p = splitSmb(h);
    if (p != null) {
      h = p.host;
      if (sh.isEmpty) sh = p.share;
      if (f.isEmpty) f = p.folder;
    }
  }
  if (h.isEmpty) return const AddressResult.fail(SourceFormError.hostEmpty);
  if (h.contains('@')) {
    return const AddressResult.fail(SourceFormError.credentialsInAddress);
  }
  if (sh.isEmpty) return const AddressResult.fail(SourceFormError.shareEmpty);
  return AddressResult.ok(['smb://$h', sh, if (f.isNotEmpty) f].join('/'));
}

/// A Subsonic server URL (NET-016); `http://` is added when the scheme is
/// missing (Navidrome and most home servers listen on plain HTTP, port
/// 4533), a pasted `/rest` is dropped.
AddressResult subsonicAddress(String input) {
  var s = input.trim();
  if (s.isEmpty) return const AddressResult.fail(SourceFormError.urlEmpty);
  if (!s.contains('://')) s = 'http://$s';
  while (s.endsWith('/')) {
    s = s.substring(0, s.length - 1);
  }
  if (s.endsWith('/rest')) s = s.substring(0, s.length - 5);
  return webdavAddress(s);
}

/// The description URL of a UPnP/DLNA server (NET-018), as the LAN search
/// finds it (`http://192.168.1.10:8200/rootDesc.xml`); `http://` is added
/// when the scheme is missing.
AddressResult dlnaAddress(String input) {
  var s = input.trim();
  if (s.isEmpty) return const AddressResult.fail(SourceFormError.urlEmpty);
  if (!s.contains('://')) s = 'http://$s';
  return webdavAddress(s);
}

/// How the core stores a playlist file on this device (`m3u+file:///…`).
const playlistFilePrefix = 'm3u+';

/// An M3U/M3U8 playlist (NET-018): an `http(s)` link, or the full path of
/// a file on this device.
AddressResult playlistAddress(String input) {
  final s = input.trim();
  if (s.isEmpty) return const AddressResult.fail(SourceFormError.urlEmpty);
  if (s.startsWith(playlistFilePrefix) ||
      s.startsWith('file://') ||
      s.startsWith('/') ||
      _windowsPath.hasMatch(s)) {
    return AddressResult.ok(s);
  }
  if (!s.contains('://')) {
    return const AddressResult.fail(SourceFormError.playlistAddress);
  }
  return webdavAddress(s);
}

/// The path or link to show for a stored playlist address.
String playlistDisplay(String address) {
  if (!address.startsWith(playlistFilePrefix)) return address;
  try {
    return Uri.parse(address.substring(playlistFilePrefix.length)).toFilePath();
  } on UnsupportedError {
    return address;
  } on FormatException {
    return address;
  }
}

/// A WebDAV URL; `https://` is added when the scheme is missing.
AddressResult webdavAddress(String input) {
  var s = input.trim();
  if (s.isEmpty) return const AddressResult.fail(SourceFormError.urlEmpty);
  if (!s.contains('://')) s = 'https://$s';
  final uri = Uri.tryParse(s);
  if (uri == null || (uri.scheme != 'https' && uri.scheme != 'http')) {
    return const AddressResult.fail(SourceFormError.urlScheme);
  }
  if (uri.host.isEmpty) {
    return const AddressResult.fail(SourceFormError.urlScheme);
  }
  if (uri.userInfo.isNotEmpty) {
    return const AddressResult.fail(SourceFormError.credentialsInAddress);
  }
  return AddressResult.ok(s);
}

/// A server URL with one of [schemes]; [schemes.first] is added when the
/// scheme is missing.
AddressResult _serverAddress(String input, List<String> schemes) {
  var s = input.trim();
  if (s.isEmpty) return const AddressResult.fail(SourceFormError.urlEmpty);
  if (!s.contains('://')) s = '${schemes.first}://$s';
  final uri = Uri.tryParse(s);
  if (uri == null || !schemes.contains(uri.scheme) || uri.host.isEmpty) {
    return const AddressResult.fail(SourceFormError.serverScheme);
  }
  if (uri.userInfo.isNotEmpty) {
    return const AddressResult.fail(SourceFormError.credentialsInAddress);
  }
  return AddressResult.ok(s);
}

/// An SFTP address, `sftp://host[:port][/folder]` (NET-006).
AddressResult sftpAddress(String input) => _serverAddress(input, ['sftp']);

/// An FTP address: `ftp://`, `ftpes://` (explicit TLS) or `ftps://`
/// (implicit TLS) (NET-006).
AddressResult ftpAddress(String input) =>
    _serverAddress(input, ['ftp', 'ftpes', 'ftps']);

/// A certificate pin as `sha256:<64 hex>`; colons and spaces of a pasted
/// fingerprint are dropped. Empty input is no pin (null address).
AddressResult? tlsPin(String input) {
  var s = input.trim().toLowerCase();
  if (s.isEmpty) return null;
  if (s.startsWith('sha256:')) s = s.substring(7);
  s = s.replaceAll(RegExp(r'[\s:]'), '');
  if (!RegExp(r'^[0-9a-f]{64}$').hasMatch(s)) {
    return const AddressResult.fail(SourceFormError.tlsPin);
  }
  return AddressResult.ok('sha256:$s');
}

/// A display name when the user gave none: the last folder, or the share.
String suggestSourceName(String address) {
  final s = address.replaceAll('\\', '/');
  final uri = Uri.tryParse(s);
  if (uri != null &&
      uri.hasScheme &&
      uri.scheme != 'file' &&
      s.contains('://')) {
    final segs = uri.pathSegments.where((p) => p.isNotEmpty).toList();
    return segs.isEmpty ? uri.host : segs.last;
  }
  final segs = s.split('/').where((p) => p.isNotEmpty).toList();
  return segs.isEmpty ? address : segs.last;
}

/// What the user typed or pasted as "server" in the SMB setup (UI-028),
/// taken apart: `192.168.1.10`, `nas.local:445`, `\\nas\Media\Movies`,
/// `smb://nas/Media/Movies`, `nas/Media/Movies`, `user@nas`, or a path as
/// on the server's disk (`/home/alex/films`, no host).
class SmbTyped {
  const SmbTyped({this.host = '', this.path = '', this.user});

  /// Server, empty if only a path was typed.
  final String host;

  /// Everything after the host (`share/folder…` or a disk path),
  /// `/`-separated, without leading or trailing slashes.
  final String path;

  /// A user name given as `user@host`.
  final String? user;

  /// First folder of [path]: the share, if the user knew it.
  String get share => path.split('/').first;

  /// The rest of [path] after [share].
  String get folder {
    final i = path.indexOf('/');
    return i < 0 ? '' : path.substring(i + 1);
  }
}

/// Takes apart any server text the user typed or pasted ([SmbTyped]).
SmbTyped parseSmbInput(String input) {
  var s = input.trim().replaceAll('"', '').replaceAll("'", '');
  s = s.replaceAll('\\', '/');
  final lower = s.toLowerCase();
  if (lower.startsWith('smb://')) {
    s = s.substring(6);
  } else if (lower.startsWith('cifs://')) {
    s = s.substring(7);
  } else if (s.startsWith('//')) {
    s = s.substring(2);
  } else if (s.startsWith('/')) {
    // A path as on the server's disk: no host in it.
    return SmbTyped(path: _trimSlashes(s));
  }
  s = _trimSlashes(s);
  if (s.isEmpty) return const SmbTyped();
  final slash = s.indexOf('/');
  var host = slash < 0 ? s : s.substring(0, slash);
  final path = slash < 0 ? '' : _trimSlashes(s.substring(slash + 1));
  String? user;
  final at = host.lastIndexOf('@');
  if (at >= 0) {
    // `user:password@host`: only the user name is kept.
    user = host.substring(0, at).split(':').first;
    host = host.substring(at + 1);
    if (user.isEmpty) user = null;
  }
  return SmbTyped(host: host, path: path, user: user);
}
