// SPDX-License-Identifier: MPL-2.0
//
// An in-memory SMB server for the demo mode, screenshots and widget tests
// of the step-by-step SMB setup (UI-028). It behaves like the core's
// `pa_sources::smb_*` calls: shares, folders, finding a typed path and the
// same errors.

import '../sources/source_manager.dart';

/// One fake server: users, shares and their folders.
class FakeSmbServer {
  FakeSmbServer({
    required this.address,
    Map<String, String>? users,
    Map<String, List<String>>? shares,
    this.guestShares = const {'public'}, // l10n-ignore
    this.homeShares = true,
    this.hidesShareList = false,
  }) : users = users ?? {'alex': 'secret'}, // l10n-ignore
       shares = shares ?? _demoShares;

  /// Host the server answers on.
  final String address;

  /// User name → password.
  final Map<String, String> users;

  /// Share → its files and folders (paths inside the share; a folder is
  /// every prefix of a path).
  final Map<String, List<String>> shares;

  /// Shares a guest may open.
  final Set<String> guestShares;

  /// Each user's home folder is a share named after the user (Samba
  /// `[homes]`), not listed.
  final bool homeShares;

  /// Lets users in but does not tell its shares, like the owner's Samba
  /// (UI-031): the share name has to be typed or guessed.
  final bool hidesShareList;

  /// Share names the core tries when the list is hidden
  /// (`pa_sources::smb_browse::COMMON_SHARES`).
  static const commonShares = [
    'shared',
    'share',
    'public',
    'media',
    'video',
    'Multimedia',
  ];

  static const _demoShares = <String, List<String>>{
    'Media': [
      'Movies/Dune (2021)/Dune (2021).mkv',
      'Movies/Arrival (2016).mkv',
      'TV Shows/Severance/Season 1/S01E01.mkv',
      'Music/Album/01.flac',
    ],
    'smb_shared': [
      'torrents/downloads/Dune (2021).mkv',
      'torrents/downloads/Arrival (2016).mkv',
      'torrents/incomplete/file.part',
    ],
    'public': ['Music/song.flac'],
  };

  static const _home = <String>[
    'smb_shared/torrents/downloads/Dune (2021).mkv',
    'smb_shared/torrents/downloads/Arrival (2016).mkv',
    'Documents/notes.txt',
  ];

  String? _userOf(SmbLogin login) {
    final user = (login.username ?? '').trim();
    if (user.isEmpty) return null;
    if (users[user] != (login.password ?? '')) {
      throw const SourceException(
        SourceErrorCode.authFailed,
        'authentication failed: Session setup failed', // l10n-ignore
      );
    }
    return user;
  }

  /// Files of [share] for [user], or a "share not found" / "permission
  /// denied" error.
  List<String> _tree(SmbLogin login, String share) {
    final user = _userOf(login);
    if (user != null && homeShares && share == user) return _home;
    final key = shares.keys.where(
      (k) => k.toLowerCase() == share.toLowerCase(),
    );
    if (key.isEmpty) {
      throw SourceException(
        SourceErrorCode.shareNotFound,
        'not found: smb://$address/$share: share not found', // l10n-ignore
      );
    }
    if (user == null && !guestShares.contains(key.first)) {
      throw SourceException(
        SourceErrorCode.permissionDenied,
        'permission denied: smb://$address/$share', // l10n-ignore
      );
    }
    return shares[key.first]!;
  }

  List<SmbShareInfo> listShares(SmbLogin login) {
    final user = _userOf(login);
    if (hidesShareList) {
      throw SourceException(
        SourceErrorCode.shareListRefused,
        'permission denied: share list refused: smb://$address/: '
        'ShareEnum failed', // l10n-ignore
      );
    }
    return [
      for (final s in shares.keys)
        if (user != null || guestShares.contains(s)) SmbShareInfo(s),
    ];
  }

  List<SourceFolderEntry> list(SmbLogin login, String share, String folder) {
    final files = _tree(login, share);
    final base = folder.split('/').where((s) => s.isNotEmpty).join('/');
    final prefix = base.isEmpty ? '' : '$base/';
    if (base.isNotEmpty && !files.any((f) => f.startsWith(prefix))) {
      throw SourceException(
        SourceErrorCode.pathNotFound,
        'not found: smb://$address/$share/$base', // l10n-ignore
      );
    }
    final seen = <String, SourceFolderEntry>{};
    for (final f in files) {
      if (!f.startsWith(prefix)) continue;
      final rest = f.substring(prefix.length);
      final name = rest.split('/').first;
      final isFolder = rest.contains('/');
      seen[name] ??= SourceFolderEntry(
        name: name,
        relPath: '$prefix$name',
        isFolder: isFolder,
        sizeBytes: isFolder ? 0 : 1000000,
      );
    }
    return seen.values.toList();
  }

  bool _isFolder(SmbLogin login, String share, String folder) {
    try {
      list(login, share, folder);
      return true;
    } on SourceException catch (e) {
      if (e.code == SourceErrorCode.authFailed) rethrow;
      return false;
    }
  }

  List<SmbPlace> find(SmbLogin login, String share, String folder) {
    final segs = '$share/$folder'
        .replaceAll('\\', '/')
        .split('/')
        .where((s) => s.isNotEmpty && s != '.')
        .toList();
    final user = _userOf(login);
    if (segs.isEmpty) return hidesShareList ? _common(login, segs) : const [];
    final out = <SmbPlace>[];
    for (var i = 0; i < segs.length; i++) {
      final rest = segs.sublist(i + 1).join('/');
      if (_isFolder(login, segs[i], rest)) out.add(SmbPlace(segs[i], rest));
    }
    if (user != null &&
        const ['home', 'homes', 'users'].contains(segs.first.toLowerCase()) &&
        !segs.contains(user) &&
        _isFolder(login, user, segs.sublist(1).join('/'))) {
      out.add(SmbPlace(user, segs.sublist(1).join('/')));
    }
    if (out.isEmpty && hidesShareList) return _common(login, segs);
    return _mostSpecific(out);
  }

  /// The home share and [commonShares] that open, each with the deepest
  /// tail of the typed path found in it, like the core (UI-031).
  List<SmbPlace> _common(SmbLogin login, List<String> segs) {
    final user = _userOf(login);
    final out = <SmbPlace>[];
    for (final name in [?user, ...commonShares]) {
      for (var i = 0; i <= segs.length; i++) {
        final tail = segs.sublist(i).join('/');
        if (_isFolder(login, name, tail)) {
          out.add(SmbPlace(name, tail));
          break;
        }
      }
    }
    if (out.any((p) => p.folder.isNotEmpty)) {
      out.removeWhere((p) => p.folder.isEmpty);
    }
    return _mostSpecific(out);
  }

  /// The home share and a share inside it lead to the same folder: keep
  /// the most specific one, like the core does by file id.
  List<SmbPlace> _mostSpecific(List<SmbPlace> out) {
    out.sort((a, b) => a.folder.length.compareTo(b.folder.length));
    final kept = <SmbPlace>[];
    for (final p in out) {
      final same = kept.any(
        (k) => '${p.share}/${p.folder}'.endsWith('${k.share}/${k.folder}'),
      );
      if (!same) kept.add(p);
    }
    return kept;
  }
}
