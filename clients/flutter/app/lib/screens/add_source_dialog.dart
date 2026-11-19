// SPDX-License-Identifier: MPL-2.0
//
// Adding and editing sources (UI-005): sources found on the network first,
// then the kinds that can be added by hand; forms for a local folder, SMB
// and WebDAV, sign-in to a Jellyfin or Emby server (NET-014, NET-015), a
// Subsonic music server with a password or an API key (NET-016), a
// UPnP/DLNA server (found on the network or by address) and an M3U
// playlist link or file, both without a login (NET-018), and sign-in to a
// Plex server with a code at plex.tv/link (NET-012). Windows shares, Plex,
// Jellyfin and Emby servers found on the network (NET-010) prefill their
// forms. Other kinds are listed as "soon".

import 'dart:async';
import 'dart:io';

import 'package:file_picker/file_picker.dart';
import 'package:material_ui/material_ui.dart';
import 'package:permission_handler/permission_handler.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../sources/discovery.dart';
import '../sources/source_address.dart';
import '../sources/source_manager.dart';
import '../sources/source_text.dart';
import 'jellyfin_source_form.dart';
import 'plex_source_form.dart';
import 'smb_setup.dart';

/// Opens the add flow. With [type] the kind chooser is skipped; with
/// [discovered] (a share from [SourceKindList]) the form is prefilled.
/// After a source is added it is checked and — if it goes to the library —
/// scanned.
Future<MediaSource?> showAddSourceDialog(
  BuildContext context, {
  bool scanAfter = true,
  SourceType? type,
  DiscoveredSource? discovered,
}) async {
  final services = Services.read(context);
  final added = await showDialog<ManagedSource>(
    context: context,
    builder: (_) => AddSourceDialog(
      initialType: type ?? discovered?.type,
      initialDiscovered: discovered,
    ),
  );
  if (added == null) return null;
  return _afterAdd(services, added, scan: scanAfter);
}

/// Checks a new source and — if it goes to the library — scans it.
MediaSource? _afterAdd(
  AppServices services,
  ManagedSource added, {
  bool scan = true,
}) {
  unawaited(_checkQuietly(services, added.id));
  if (scan && added.type.indexable && added.scanIntoLibrary) {
    unawaited(services.scan(sourceId: added.id));
  }
  return services.sources.where((s) => s.id == added.id).firstOrNull;
}

/// «Добавить» on a source found on the network (UI-042). What needs no
/// answer is added at once and scanned: a DLNA server, or an SMB server
/// whose guest sees exactly one shared folder and may open it. Anything
/// else opens its own form prefilled ([showAddSourceDialog]) — for SMB the
/// sign-in step, for Plex the plex.tv code, for Jellyfin and Emby the
/// login — so the user is asked only what the server needs. The content
/// type is never asked: the core tells it (ADR-0099).
Future<MediaSource?> addDiscoveredSource(
  BuildContext context,
  DiscoveredSource found,
) async {
  final services = Services.read(context);
  ManagedSource? added;
  try {
    final draft = await quickSourceDraft(services.sourceManager, found);
    if (draft != null) added = await services.createSource(draft);
  } on SourceException {
    // The form below tells what is wrong, in its own words.
    added = null;
  }
  if (added != null) return _afterAdd(services, added);
  if (!context.mounted) return null;
  return showAddSourceDialog(context, discovered: found);
}

/// The source [found] becomes without a question, or null when the user
/// has to answer something (a login, a folder among several, a code).
/// Throws [SourceException] when the server cannot be asked.
@visibleForTesting
Future<SourceDraft?> quickSourceDraft(
  SourceManager manager,
  DiscoveredSource found,
) async {
  if (!manager.creatableTypes.contains(found.type)) return null;
  final stable = found.stableName;
  switch (found.type) {
    case SourceType.dlna:
      final address = dlnaAddress(found.url ?? '').address;
      if (address == null) return null;
      return SourceDraft(
        type: SourceType.dlna,
        name: found.name,
        address: address,
        lanName: stable == null || stable.isEmpty ? null : stable,
      );
    case SourceType.smb:
      if (!manager.canBrowseSmb) return null;
      final host = splitSmb(found.url ?? '')?.host ?? found.host;
      if (host.isEmpty) return null;
      final guest = SmbLogin(server: host, username: '', password: '');
      final shares = [
        for (final s in await manager.smbShares(guest))
          // `IPC$`, `ADMIN$`, `C$`: not folders for people.
          if (!s.name.endsWith(r'$')) s.name,
      ];
      if (shares.length != 1) return null;
      final share = shares.single;
      // Listed is not opened: the guest must really get in.
      await manager.smbListFolder(guest, share, '');
      final address = smbAddress(host: host, share: share).address;
      if (address == null) return null;
      return SourceDraft(
        type: SourceType.smb,
        name: '${found.name} · $share',
        address: address,
        lanName: stable == null || stable.isEmpty ? null : stable,
      );
    default:
      return null;
  }
}

Future<void> _checkQuietly(AppServices services, String id) async {
  try {
    await services.checkSource(id);
  } on SourceException {
    // The result is in the source status.
  }
}

/// Opens the form of an existing source.
Future<ManagedSource?> showEditSourceDialog(
  BuildContext context,
  ManagedSource source,
) => showDialog<ManagedSource>(
  context: context,
  builder: (_) => _FormDialog(
    child: source.type.mediaServer
        ? JellyfinSourceForm(server: source.type, initial: source)
        : source.type.plexServer
        ? PlexSourceForm(initial: source)
        : SourceForm(type: source.type, initial: source),
  ),
);

class _FormDialog extends StatelessWidget {
  const _FormDialog({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) => Dialog(
    child: ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 520),
      child: child,
    ),
  );
}

/// Kind chooser, then the form of the chosen kind.
class AddSourceDialog extends StatefulWidget {
  const AddSourceDialog({super.key, this.initialType, this.initialDiscovered});

  final SourceType? initialType;

  /// A share found on the network that prefills the form.
  final DiscoveredSource? initialDiscovered;

  @override
  State<AddSourceDialog> createState() => _AddSourceDialogState();
}

class _AddSourceDialogState extends State<AddSourceDialog> {
  SourceType? _type;
  DiscoveredSource? _found;

  @override
  void initState() {
    super.initState();
    _type = widget.initialType;
    _found = widget.initialDiscovered;
  }

  @override
  Widget build(BuildContext context) {
    final type = _type;
    final back = widget.initialType == null
        ? () => setState(() {
            _type = null;
            _found = null;
          })
        : null;
    return _FormDialog(
      child: type == null
          ? _KindChooser(
              onPick: (t, [found]) => setState(() {
                _type = t;
                _found = found;
              }),
            )
          : type.mediaServer
          ? JellyfinSourceForm(server: type, discovered: _found, onBack: back)
          : type.plexServer
          ? PlexSourceForm(discovered: _found, onBack: back)
          // UI-028: SMB step by step (server, sign in, pick the folder).
          : type == SourceType.smb &&
                Services.read(context).sourceManager.canBrowseSmb
          ? SmbSetupFlow(discovered: _found, onBack: back)
          : SourceForm(type: type, discovered: _found, onBack: back),
    );
  }
}

class _KindChooser extends StatelessWidget {
  const _KindChooser({required this.onPick});

  final void Function(SourceType type, [DiscoveredSource? found]) onPick;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _DialogTitle(l.addSourceTitle),
        Flexible(
          child: SingleChildScrollView(
            padding: const EdgeInsets.only(bottom: 8),
            child: SourceKindList(onPick: onPick),
          ),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 0, 16, 12),
          child: Align(
            alignment: Alignment.centerRight,
            child: TextButton(
              onPressed: () => Navigator.pop(context),
              child: Text(l.commonCancel),
            ),
          ),
        ),
      ],
    );
  }
}

/// Why this build cannot add a source of kind [t] ("soon", "needs the
/// full core"), or null when it can.
String? sourceKindBadge(
  AppLocalizations l,
  AppServices services,
  SourceType t,
) {
  if (!SourceType.available.contains(t)) return l.sourcesSoon;
  // SMB everywhere through the core (libsmb2 in the APK, LIB-019); the v0
  // fallback manager cannot create it and says so.
  if (!services.sourceManager.creatableTypes.contains(t)) {
    return l.sourcesUnavailableReduced;
  }
  // NET-014: a media server needs the core's Jellyfin client.
  if (t.mediaServer && services.jellyfin == null) {
    return l.sourcesUnavailableReduced;
  }
  // NET-012: Plex needs the core's Plex client.
  if (t.plexServer && services.plex == null) {
    return l.sourcesUnavailableReduced;
  }
  return null;
}

/// Sources found on the network ("On this network", NET-020), then every
/// kind that can be added by hand; the add dialog and the first-run setup
/// (UI-004) show it. Not scrollable itself.
class SourceKindList extends StatefulWidget {
  const SourceKindList({super.key, required this.onPick});

  final void Function(SourceType type, [DiscoveredSource? found]) onPick;

  @override
  State<SourceKindList> createState() => _SourceKindListState();
}

class _SourceKindListState extends State<SourceKindList> {
  /// One subscription while the list is shown: LAN browsing runs as long
  /// as someone listens (NET-020), so rebuilds must not restart it.
  Stream<List<DiscoveredSource>>? _found;

  void onPick(SourceType type, [DiscoveredSource? found]) =>
      widget.onPick(type, found);

  @override
  Widget build(BuildContext context) {
    final services = Services.read(context);
    final discovered = _found ??= services.discovery.watch();
    final l = context.l10n;
    final theme = Theme.of(context);
    String? badge(SourceType t) => sourceKindBadge(l, services, t);

    return Material(
      type: MaterialType.transparency,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (services.discovery.supported) ...[
            _Header(l.sourcesAddDiscovered),
            StreamBuilder<List<DiscoveredSource>>(
              stream: discovered,
              builder: (context, snap) {
                // Kinds this build cannot add are hidden.
                final found = [
                  for (final d in snap.data ?? const <DiscoveredSource>[])
                    if (badge(d.type) == null) d,
                ];
                if (found.isEmpty) {
                  return Padding(
                    padding: const EdgeInsets.fromLTRB(24, 4, 24, 8),
                    child: Text(
                      l.sourcesAddDiscoveredEmpty,
                      style: theme.textTheme.bodySmall,
                    ),
                  );
                }
                return Column(
                  children: [
                    for (final d in found)
                      Builder(
                        builder: (context) {
                          final b = badge(d.type);
                          return ListTile(
                            enabled: b == null,
                            leading: Icon(sourceTypeIcon(d.type)),
                            title: Text(d.name),
                            subtitle: Text(
                              '${sourceTypeLabel(l, d.type)} · ${d.host}',
                            ),
                            trailing: b == null ? null : SourceBadge(b),
                            onTap: b == null ? () => onPick(d.type, d) : null,
                          );
                        },
                      ),
                  ],
                );
              },
            ),
            _Header(l.sourcesAddManual),
          ],
          for (final t in SourceType.values)
            Builder(
              builder: (context) {
                final b = badge(t);
                return ListTile(
                  enabled: b == null,
                  leading: Icon(sourceTypeIcon(t)),
                  title: Text(sourceTypeLabel(l, t)),
                  trailing: b == null
                      ? const Icon(Icons.chevron_right)
                      : SourceBadge(b),
                  onTap: b == null ? () => onPick(t) : null,
                );
              },
            ),
        ],
      ),
    );
  }
}

class _DialogTitle extends StatelessWidget {
  const _DialogTitle(this.text, {this.onBack});

  final String text;
  final VoidCallback? onBack;

  @override
  Widget build(BuildContext context) => Padding(
    padding: EdgeInsets.fromLTRB(onBack == null ? 24 : 8, 20, 24, 8),
    child: Row(
      children: [
        if (onBack != null)
          IconButton(
            tooltip: context.l10n.commonBack,
            onPressed: onBack,
            icon: const Icon(Icons.arrow_back),
          ),
        Expanded(
          child: Text(text, style: Theme.of(context).textTheme.headlineSmall),
        ),
      ],
    ),
  );
}

class _Header extends StatelessWidget {
  const _Header(this.text);

  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(24, 12, 24, 4),
    child: Text(text, style: Theme.of(context).textTheme.labelLarge),
  );
}

class SourceBadge extends StatelessWidget {
  const SourceBadge(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 140),
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(8),
          border: Border.all(color: theme.colorScheme.outline),
        ),
        child: Text(
          text,
          maxLines: 2,
          overflow: TextOverflow.ellipsis,
          textAlign: TextAlign.center,
          style: theme.textTheme.labelSmall,
        ),
      ),
    );
  }
}

/// Readable error with the technical message under "Details".
class SourceErrorView extends StatelessWidget {
  const SourceErrorView({super.key, required this.code, this.detail});

  final SourceErrorCode code;
  final String? detail;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final style = theme.textTheme.bodySmall?.copyWith(
      color: theme.colorScheme.error,
    );
    final d = detail;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(sourceErrorText(context.l10n, code), style: style),
        if (d != null && d.isNotEmpty)
          // Own Material: the view also sits on decorated cards.
          Theme(
            data: theme.copyWith(dividerColor: Colors.transparent),
            child: Material(
              type: MaterialType.transparency,
              child: ExpansionTile(
                tilePadding: EdgeInsets.zero,
                dense: true,
                title: Text(
                  context.l10n.sourcesErrorDetails,
                  style: theme.textTheme.labelSmall,
                ),
                children: [
                  Align(
                    alignment: Alignment.centerLeft,
                    child: SelectableText(d, style: theme.textTheme.labelSmall),
                  ),
                ],
              ),
            ),
          ),
      ],
    );
  }
}

/// Form of one source kind: add (no [initial]) or edit.
class SourceForm extends StatefulWidget {
  const SourceForm({
    super.key,
    required this.type,
    this.initial,
    this.discovered,
    this.onBack,
  });

  final SourceType type;
  final ManagedSource? initial;
  final DiscoveredSource? discovered;
  final VoidCallback? onBack;

  @override
  State<SourceForm> createState() => _SourceFormState();
}

class _SourceFormState extends State<SourceForm> {
  final _name = TextEditingController();
  final _folder = TextEditingController();
  final _host = TextEditingController();
  final _share = TextEditingController();
  final _subfolder = TextEditingController();
  final _url = TextEditingController();
  final _user = TextEditingController();
  final _password = TextEditingController();
  final _domain = TextEditingController();
  final _pin = TextEditingController();
  final _sshKey = TextEditingController();
  final _hostKey = TextEditingController();
  String _smbVersion = 'any';
  String _smbEncryption = 'auto';
  bool _signing = false;
  bool _scanIntoLibrary = true;
  bool _autoScan = true;
  bool _forgetPassword = false;

  /// Subsonic (NET-016): sign in with an OpenSubsonic API key (stored in
  /// place of the password, no user name).
  bool _apiKey = false;
  SourceSyncMode _sync = SourceSyncMode.off;

  /// A source moved to «Личное» never syncs (ADR-0098/0099): no switch.
  bool _personal = false;

  /// Field errors are shown after the first submit.
  bool _submitted = false;
  bool _busy = false;
  SourceException? _error;
  String? _pickerError;

  bool get _editing => widget.initial != null;

  @override
  void initState() {
    super.initState();
    final s = widget.initial;
    final d = widget.discovered;
    if (s != null) {
      _name.text = s.name;
      _user.text = s.username ?? '';
      _scanIntoLibrary = s.scanIntoLibrary;
      _autoScan = s.autoScan;
      _sync = s.sync;
      WidgetsBinding.instance.addPostFrameCallback((_) => _readPersonal(s.id));
      _smbVersion = s.smb.version;
      _smbEncryption = s.smb.encryption;
      _signing = s.smb.signingRequired;
      _domain.text = s.smb.domain ?? '';
      _pin.text = s.tlsPin ?? '';
      _hostKey.text = s.sshHostKey ?? '';
      switch (s.type) {
        case SourceType.smb:
          final p = splitSmb(s.address);
          _host.text = p?.host ?? '';
          _share.text = p?.share ?? '';
          _subfolder.text = p?.folder ?? '';
        case SourceType.webdav || SourceType.sftp || SourceType.ftp:
          _url.text = s.address;
        case SourceType.subsonic:
          _url.text = s.address;
          _apiKey = s.username == null && s.hasPassword;
        case SourceType.dlna:
          _url.text = s.address;
        case SourceType.playlist:
          _url.text = playlistDisplay(s.address);
        default:
          _folder.text = _localPath(s.address);
      }
    } else if (d != null) {
      _name.text = d.name;
      // The core's URL is built from the IP address (NET-020); the host
      // name is the fallback.
      if (d.type == SourceType.smb) {
        _host.text = splitSmb(d.url ?? '')?.host ?? d.host;
      } else if (d.type == SourceType.webdav) {
        final port = d.port == null ? '' : ':${d.port}';
        final scheme = d.secure ? 'https' : 'http';
        _url.text = d.url ?? '$scheme://${d.host}$port${d.path ?? '/'}';
      } else if (d.type == SourceType.dlna) {
        // NET-018: the URL of the server's description, from the search.
        _url.text = d.url ?? '';
      }
    }
    for (final c in [_host, _share, _subfolder, _url, _folder]) {
      c.addListener(_changed);
    }
  }

  void _changed() => setState(() {});

  /// The stable name of the server picked in «В этой сети» (BUG-004),
  /// while the address still points at the host it was found at.
  String? _lanNameFor(String address) {
    final d = widget.discovered;
    final name = d?.stableName;
    if (_editing || d == null || name == null || name.isEmpty) return null;
    final found = Uri.tryParse(d.url ?? '')?.host ?? d.host;
    final host = Uri.tryParse(address)?.host;
    return host != null && host.isNotEmpty && host == found ? name : null;
  }

  /// `file:///a/b` (how the core stores folders) as a path.
  static String _localPath(String address) {
    if (!address.startsWith('file://')) return address;
    try {
      return Uri.parse(address).toFilePath();
    } on UnsupportedError {
      return address;
    }
  }

  @override
  void dispose() {
    for (final c in [
      _name,
      _folder,
      _host,
      _share,
      _subfolder,
      _url,
      _user,
      _password,
      _domain,
      _pin,
      _sshKey,
      _hostKey,
    ]) {
      c.dispose();
    }
    super.dispose();
  }

  AddressResult _address() => switch (widget.type) {
    SourceType.smb => smbAddress(
      host: _host.text,
      share: _share.text,
      folder: _subfolder.text,
    ),
    SourceType.webdav => webdavAddress(_url.text),
    SourceType.sftp => sftpAddress(_url.text),
    SourceType.ftp => ftpAddress(_url.text),
    SourceType.subsonic => subsonicAddress(_url.text),
    SourceType.dlna => dlnaAddress(_url.text),
    SourceType.playlist => playlistAddress(_url.text),
    _ => localAddress(_folder.text),
  };

  AddressResult? _pinResult() =>
      widget.type == SourceType.webdav || widget.type == SourceType.ftp
      ? tlsPin(_pin.text)
      : null;

  String? _fieldError(SourceFormError? e, Set<SourceFormError> mine) {
    if (!_submitted || e == null || !mine.contains(e)) return null;
    return sourceFormErrorText(context.l10n, e);
  }

  Future<void> _readPersonal(String sourceId) async {
    final personal = Services.read(context).personal;
    final id = int.tryParse(sourceId);
    if (personal == null || id == null) return;
    try {
      final c = await personal.sourceContent(id);
      if (mounted && c == SourceContent.personal) {
        setState(() => _personal = true);
      }
    } catch (_) {}
  }

  Future<void> _browse() async {
    final l = context.l10n;
    try {
      final dir = await FilePicker.getDirectoryPath(
        dialogTitle: l.addSourcePickerTitle,
      );
      if (dir != null) setState(() => _folder.text = dir);
    } catch (e) {
      setState(() => _pickerError = l.addSourcePickerUnavailable('$e'));
    }
  }

  /// NET-018: a playlist file on this device.
  Future<void> _pickPlaylist() async {
    final l = context.l10n;
    try {
      final picked = await FilePicker.pickFiles(
        type: FileType.custom,
        allowedExtensions: const ['m3u', 'm3u8'],
      );
      final path = picked.firstOrNull?.path;
      if (path != null && mounted) setState(() => _url.text = path);
    } catch (e) {
      if (mounted) {
        setState(() => _pickerError = l.addSourcePickerUnavailable('$e'));
      }
    }
  }

  /// UI-028: whether the SMB folder is picked from the server instead of
  /// typed as share and folder names.
  bool get _browseSmb =>
      widget.type == SourceType.smb &&
      Services.read(context).sourceManager.canBrowseSmb;

  /// Opens the server's folders with the form's login (or the stored
  /// password of the edited source) and fills share and folder.
  Future<void> _chooseSmbFolder() async {
    final host = parseSmbInput(_host.text).host;
    if (host.isEmpty) {
      setState(() => _submitted = true);
      return;
    }
    final editing = widget.initial;
    final typedPassword = _password.text;
    final place = await showSmbFolderPicker(
      context,
      login: SmbLogin(
        server: host,
        username: _user.text.trim(),
        password: typedPassword.isNotEmpty || editing == null || _forgetPassword
            ? typedPassword
            : null,
        sourceId: editing != null && typedPassword.isEmpty && !_forgetPassword
            ? editing.id
            : null,
        smb: SmbSettings(
          domain: _domain.text.trim().isEmpty ? null : _domain.text.trim(),
          version: _smbVersion,
          encryption: _smbEncryption,
          signingRequired: _signing,
        ),
      ),
      hintShare: _share.text.trim(),
      hintFolder: _subfolder.text.trim(),
    );
    if (place == null || !mounted) return;
    setState(() {
      _share.text = place.share;
      _subfolder.text = place.folder;
      _error = null;
    });
  }

  Future<bool> _ensureStoragePermission() async {
    if (!Platform.isAndroid) return true;
    final statuses = await [
      Permission.videos,
      Permission.audio,
      Permission.storage,
    ].request();
    return statuses.values.any((s) => s.isGranted || s.isLimited);
  }

  Future<void> _submit() async {
    setState(() {
      _submitted = true;
      _error = null;
    });
    final address = _address();
    final pin = _pinResult();
    if (address.error != null || pin?.error != null) return;
    final services = Services.read(context);
    final permissionError = context.l10n.addSourceErrorPermission;
    setState(() => _busy = true);
    if (widget.type == SourceType.local && !await _ensureStoragePermission()) {
      setState(() {
        _busy = false;
        _pickerError = permissionError;
      });
      return;
    }
    final user = widget.type == SourceType.subsonic && _apiKey
        ? ''
        : _user.text.trim();
    final String? password;
    if (widget.type == SourceType.local || widget.type.openSource) {
      password = null;
    } else if (_editing) {
      password = _forgetPassword
          ? ''
          : (_password.text.isEmpty ? null : _password.text);
    } else {
      password = _password.text.isEmpty ? null : _password.text;
    }
    final draft = SourceDraft(
      type: widget.type,
      name: _name.text.trim().isNotEmpty
          ? _name.text.trim()
          : widget.type == SourceType.dlna
          ? (Uri.tryParse(address.address!)?.host ?? address.address!)
          : suggestSourceName(playlistDisplay(address.address!)),
      address: address.address!,
      username:
          widget.type == SourceType.local ||
              widget.type.openSource ||
              user.isEmpty
          ? null
          : user,
      password: password,
      smb: SmbSettings(
        domain: _domain.text.trim().isEmpty ? null : _domain.text.trim(),
        version: _smbVersion,
        encryption: _smbEncryption,
        signingRequired: _signing,
      ),
      tlsPin: pin?.address,
      // SFTP (NET-006): an empty key lets the next connect store a new one.
      sshHostKey: widget.type == SourceType.sftp ? _hostKey.text.trim() : null,
      sshPrivateKey:
          widget.type == SourceType.sftp && _sshKey.text.trim().isNotEmpty
          ? _sshKey.text
          : null,
      scanIntoLibrary: widget.type.indexable && _scanIntoLibrary,
      autoScan: _autoScan,
      sync:
          widget.type == SourceType.local || widget.type.openSource || _personal
          ? SourceSyncMode.off
          : _sync,
      lanName: _lanNameFor(address.address!),
    );
    try {
      // NET-016: a Subsonic server is checked before it is saved, so a
      // wrong password or address is told here, not later in the status.
      final subsonic = services.subsonic;
      if (widget.type == SourceType.subsonic &&
          subsonic != null &&
          password != null &&
          password.isNotEmpty) {
        await subsonic.probe(
          draft.address,
          username: draft.username,
          secret: password,
        );
      }
      final saved = _editing
          ? await services.updateSource(widget.initial!.id, draft)
          : await services.createSource(draft);
      if (mounted) Navigator.of(context).pop(saved);
    } on SourceException catch (e) {
      if (mounted) {
        setState(() {
          _busy = false;
          _error = e;
        });
      }
    } catch (e) {
      // Never leave the form spinning (CORE-007): any other failure is
      // shown too.
      if (mounted) {
        setState(() {
          _busy = false;
          _error = SourceException(SourceErrorCode.other, '$e');
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final address = _address();
    final addrError = address.error;
    final type = widget.type;
    final title = _editing
        ? l.sourcesEditTitle
        : '${l.addSourceTitle}: ${sourceTypeLabel(l, type)}';
    final submitLabel = _editing
        ? l.commonSave
        : (type.indexable && _scanIntoLibrary
              ? l.addSourceSubmit
              : l.sourcesFormAdd);
    const gap = SizedBox(height: 12);

    final fields = <Widget>[
      if (type == SourceType.local) ...[
        Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Expanded(
              child: TextField(
                key: const ValueKey('source-folder'),
                controller: _folder,
                decoration: InputDecoration(
                  labelText: l.addSourceFolder,
                  hintText: l.addSourceFolderHint,
                  errorText: _fieldError(addrError, {
                    SourceFormError.folderEmpty,
                    SourceFormError.folderNotAbsolute,
                  }),
                  errorMaxLines: 3,
                ),
              ),
            ),
            const SizedBox(width: 8),
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: IconButton.filledTonal(
                tooltip: l.addSourceBrowse,
                onPressed: _busy ? null : _browse,
                icon: const Icon(Icons.folder_open),
              ),
            ),
          ],
        ),
        const SizedBox(height: 8),
        Text(l.addSourceFolderTip, style: theme.textTheme.bodySmall),
        if (_pickerError != null) ...[
          const SizedBox(height: 8),
          Text(
            _pickerError!,
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.error,
            ),
          ),
        ],
      ],
      if (type == SourceType.smb) ...[
        TextField(
          key: const ValueKey('source-host'),
          controller: _host,
          keyboardType: TextInputType.url,
          decoration: InputDecoration(
            labelText: l.sourcesFormHost,
            hintText: l.sourcesFormHostHint,
            errorText: _fieldError(addrError, {
              SourceFormError.hostEmpty,
              SourceFormError.credentialsInAddress,
            }),
            errorMaxLines: 3,
          ),
        ),
        if (_browseSmb) ...[
          gap,
          _SmbFolderRow(
            share: _share.text.trim(),
            folder: _subfolder.text.trim(),
            error: _fieldError(addrError, {SourceFormError.shareEmpty}),
            onChoose: _busy ? null : _chooseSmbFolder,
          ),
        ] else
          ..._smbNameFields(l, addrError),
        if (address.address != null) ...[
          const SizedBox(height: 6),
          Text(
            l.sourcesFormAddress(address.address!),
            style: theme.textTheme.labelSmall,
          ),
        ],
      ],
      if (type == SourceType.webdav ||
          type == SourceType.sftp ||
          type == SourceType.ftp ||
          type == SourceType.subsonic ||
          type.openSource)
        TextField(
          key: const ValueKey('source-url'),
          controller: _url,
          keyboardType: TextInputType.url,
          autocorrect: false,
          decoration: InputDecoration(
            labelText: type == SourceType.playlist
                ? l.sourcesPlaylistAddress
                : l.sourcesFormUrl,
            hintText: switch (type) {
              SourceType.sftp => l.sourcesFormSftpUrlHint,
              SourceType.ftp => l.sourcesFormFtpUrlHint,
              SourceType.subsonic => l.sourcesSubsonicUrlHint,
              SourceType.dlna => l.sourcesDlnaUrlHint,
              SourceType.playlist => l.sourcesPlaylistHint,
              _ => l.sourcesFormUrlHint,
            },
            helperText: switch (type) {
              SourceType.sftp => l.sourcesFormSftpUrlHelper,
              SourceType.ftp => l.sourcesFormFtpUrlHelper,
              _ => null,
            },
            helperMaxLines: 3,
            errorText: _fieldError(addrError, {
              SourceFormError.urlEmpty,
              SourceFormError.urlScheme,
              SourceFormError.serverScheme,
              SourceFormError.credentialsInAddress,
              SourceFormError.playlistAddress,
            }),
            errorMaxLines: 3,
          ),
        ),
      if (type == SourceType.playlist) ...[
        const SizedBox(height: 4),
        Align(
          alignment: Alignment.centerLeft,
          child: TextButton.icon(
            key: const ValueKey('source-playlist-pick'),
            onPressed: _busy ? null : _pickPlaylist,
            icon: const Icon(Icons.folder_open),
            label: Text(l.sourcesPlaylistPick),
          ),
        ),
        if (_pickerError != null)
          Text(
            _pickerError!,
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.error,
            ),
          ),
      ],
      if (type.openSource) ...[
        gap,
        Text(
          type == SourceType.dlna ? l.sourcesDlnaNote : l.sourcesPlaylistNote,
          key: const ValueKey('open-source-note'),
          style: theme.textTheme.bodySmall,
        ),
      ],
      if (type == SourceType.subsonic)
        SwitchListTile(
          key: const ValueKey('source-subsonic-api-key'),
          contentPadding: EdgeInsets.zero,
          value: _apiKey,
          title: Text(l.sourcesSubsonicApiKey),
          subtitle: Text(l.sourcesSubsonicApiKeyHint),
          onChanged: (v) => setState(() => _apiKey = v),
        ),
      if (type != SourceType.local && !type.openSource) ...[
        if (!(type == SourceType.subsonic && _apiKey)) ...[
          gap,
          TextField(
            key: const ValueKey('source-user'),
            controller: _user,
            autocorrect: false,
            decoration: InputDecoration(labelText: l.addSourceUsername),
          ),
        ],
        gap,
        TextField(
          key: const ValueKey('source-password'),
          controller: _password,
          obscureText: true,
          enabled: !_forgetPassword,
          decoration: InputDecoration(
            labelText: type == SourceType.subsonic && _apiKey
                ? l.sourcesSubsonicKeyField
                : l.addSourcePassword,
            helperText: _editing && widget.initial!.hasPassword
                ? l.sourcesFormPasswordKeep
                : l.addSourcePasswordHelper,
            helperMaxLines: 3,
          ),
        ),
        if (_editing && widget.initial!.hasPassword)
          SwitchListTile(
            contentPadding: EdgeInsets.zero,
            value: _forgetPassword,
            title: Text(l.sourcesFormForgetPassword),
            onChanged: (v) => setState(() => _forgetPassword = v),
          ),
        if (type != SourceType.subsonic)
          Theme(
            data: theme.copyWith(dividerColor: Colors.transparent),
            child: ExpansionTile(
              key: const ValueKey('source-advanced'),
              tilePadding: EdgeInsets.zero,
              childrenPadding: const EdgeInsets.only(bottom: 8),
              title: Text(l.sourcesFormAdvanced),
              expandedCrossAxisAlignment: CrossAxisAlignment.start,
              children: switch (type) {
                SourceType.smb => _smbAdvanced(l),
                SourceType.sftp => _sftpAdvanced(l),
                _ => _webdavAdvanced(l),
              },
            ),
          ),
      ],
      if (type == SourceType.subsonic) ...[
        gap,
        Text(
          l.sourcesSubsonicNote,
          key: const ValueKey('subsonic-note'),
          style: theme.textTheme.bodySmall,
        ),
      ],
      gap,
      TextField(
        key: const ValueKey('source-name'),
        controller: _name,
        decoration: InputDecoration(labelText: l.addSourceDisplayName),
      ),
      if (type.indexable)
        SwitchListTile(
          key: const ValueKey('source-scan-into-library'),
          contentPadding: EdgeInsets.zero,
          value: _scanIntoLibrary,
          title: Text(l.sourcesFormScanIntoLibrary),
          subtitle: Text(l.sourcesFormScanIntoLibraryHint),
          onChanged: (v) => setState(() => _scanIntoLibrary = v),
        ),
      // LIB-005 watcher: events for local folders, a timer for the rest.
      if (type.indexable && _scanIntoLibrary)
        SwitchListTile(
          key: const ValueKey('source-auto-scan'),
          contentPadding: EdgeInsets.zero,
          value: _autoScan,
          title: Text(l.sourcesFormAutoScan),
          subtitle: Text(l.sourcesFormAutoScanHint),
          onChanged: (v) => setState(() => _autoScan = v),
        ),
      // SYNC-010: network sources can follow the user to paired devices.
      // UI-034, ADR-0099: no content type here — personal media is a place
      // (the folder action in the source browser), and the core never
      // syncs a personal source.
      if (type != SourceType.local &&
          !type.openSource &&
          !_personal &&
          Services.read(context).sourceManager.canSync) ...[
        SwitchListTile(
          key: const ValueKey('source-sync'),
          contentPadding: EdgeInsets.zero,
          value: _sync != SourceSyncMode.off,
          title: Text(l.sourcesFormSync),
          subtitle: Text(l.sourcesFormSyncHint),
          onChanged: (v) => setState(
            () => _sync = v ? SourceSyncMode.settings : SourceSyncMode.off,
          ),
        ),
        if (_sync != SourceSyncMode.off)
          SwitchListTile(
            key: const ValueKey('source-sync-password'),
            contentPadding: EdgeInsets.zero,
            value: _sync == SourceSyncMode.password,
            title: Text(l.sourcesFormSyncPassword),
            subtitle: Text(l.sourcesFormSyncPasswordHint),
            onChanged: (v) => setState(
              () =>
                  _sync = v ? SourceSyncMode.password : SourceSyncMode.settings,
            ),
          ),
      ],
      if (!type.indexable) ...[
        gap,
        Text(l.sourcesNotIndexable, style: theme.textTheme.bodySmall),
      ],
      if (_error != null) ...[
        gap,
        SourceErrorView(code: _error!.code, detail: _error!.detail),
      ],
    ];

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _DialogTitle(title, onBack: _busy ? null : widget.onBack),
        Flexible(
          child: SingleChildScrollView(
            padding: const EdgeInsets.fromLTRB(24, 8, 24, 8),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: fields,
            ),
          ),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 4, 16, 12),
          child: Wrap(
            alignment: WrapAlignment.end,
            spacing: 8,
            runSpacing: 4,
            children: [
              TextButton(
                onPressed: _busy ? null : () => Navigator.pop(context),
                child: Text(l.commonCancel),
              ),
              FilledButton(
                key: const ValueKey('source-submit'),
                onPressed: _busy ? null : _submit,
                child: Text(submitLabel),
              ),
            ],
          ),
        ),
      ],
    );
  }

  /// Share and folder typed by name: the plain form, or under "Advanced"
  /// when the folder can be picked from the server (UI-028).
  List<Widget> _smbNameFields(AppLocalizations l, SourceFormError? addrError) =>
      [
        const SizedBox(height: 12),
        TextField(
          key: const ValueKey('source-share'),
          controller: _share,
          decoration: InputDecoration(
            labelText: l.sourcesFormShare,
            hintText: l.sourcesFormShareHint,
            errorText: _fieldError(addrError, {SourceFormError.shareEmpty}),
            errorMaxLines: 3,
          ),
        ),
        const SizedBox(height: 12),
        TextField(
          key: const ValueKey('source-subfolder'),
          controller: _subfolder,
          decoration: InputDecoration(
            labelText: l.sourcesFormSubfolder,
            hintText: l.sourcesFormSubfolderHint,
          ),
        ),
      ];

  List<Widget> _smbAdvanced(AppLocalizations l) => [
    if (_browseSmb) ...[..._smbNameFields(l, null), const SizedBox(height: 12)],
    TextField(
      key: const ValueKey('source-domain'),
      controller: _domain,
      decoration: InputDecoration(labelText: l.sourcesFormDomain),
    ),
    const SizedBox(height: 12),
    Text(l.sourcesFormSmbVersion),
    _Choices(
      value: _smbVersion,
      options: {
        'any': l.sourcesFormSmbVersionAny, // l10n-ignore
        'smb2': l.sourcesFormSmbVersionSmb2, // l10n-ignore
        'smb3': l.sourcesFormSmbVersionSmb3, // l10n-ignore
      },
      onChanged: (v) => setState(() => _smbVersion = v),
    ),
    const SizedBox(height: 8),
    Text(l.sourcesFormEncryption),
    _Choices(
      value: _smbEncryption,
      options: {
        'auto': l.sourcesFormEncryptionAuto, // l10n-ignore
        'required': l.sourcesFormEncryptionRequired, // l10n-ignore
        'off': l.sourcesFormEncryptionOff, // l10n-ignore
      },
      onChanged: (v) => setState(() => _smbEncryption = v),
    ),
    SwitchListTile(
      contentPadding: EdgeInsets.zero,
      value: _signing,
      title: Text(l.sourcesFormSigning),
      onChanged: (v) => setState(() => _signing = v),
    ),
  ];

  /// SFTP (NET-006): key login and the stored host key.
  List<Widget> _sftpAdvanced(AppLocalizations l) => [
    TextField(
      key: const ValueKey('source-ssh-key'),
      controller: _sshKey,
      autocorrect: false,
      enableSuggestions: false,
      minLines: 2,
      maxLines: 4,
      style: const TextStyle(fontFamily: 'monospace', fontSize: 12),
      decoration: InputDecoration(
        labelText: l.sourcesFormSshKey,
        helperText: l.sourcesFormSshKeyHelper,
        helperMaxLines: 3,
      ),
    ),
    const SizedBox(height: 12),
    TextField(
      key: const ValueKey('source-ssh-host-key'),
      controller: _hostKey,
      autocorrect: false,
      enableSuggestions: false,
      decoration: InputDecoration(
        labelText: l.sourcesFormSshHostKey,
        helperText: l.sourcesFormSshHostKeyHelper,
        helperMaxLines: 3,
      ),
    ),
  ];

  List<Widget> _webdavAdvanced(AppLocalizations l) {
    final pin = _pinResult();
    return [
      TextField(
        key: const ValueKey('source-tls-pin'),
        controller: _pin,
        autocorrect: false,
        onChanged: (_) => setState(() {}),
        decoration: InputDecoration(
          labelText: l.sourcesFormTlsPin,
          helperText: l.sourcesFormTlsPinHelper,
          helperMaxLines: 3,
          errorText: _fieldError(pin?.error, {SourceFormError.tlsPin}),
          errorMaxLines: 3,
        ),
      ),
    ];
  }
}

class _Choices extends StatelessWidget {
  const _Choices({
    required this.value,
    required this.options,
    required this.onChanged,
  });

  final String value;
  final Map<String, String> options;
  final ValueChanged<String> onChanged;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: 6),
    child: Wrap(
      spacing: 8,
      runSpacing: 6,
      children: [
        for (final MapEntry(:key, value: label) in options.entries)
          ChoiceChip(
            label: Text(label),
            selected: key == value,
            onSelected: (_) => onChanged(key),
          ),
      ],
    ),
  );
}

/// The chosen SMB folder with "Choose folder" (UI-028).
class _SmbFolderRow extends StatelessWidget {
  const _SmbFolderRow({
    required this.share,
    required this.folder,
    required this.onChoose,
    this.error,
  });

  final String share;
  final String folder;
  final VoidCallback? onChoose;
  final String? error;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final where = share.isEmpty
        ? l.sourcesSmbSetupFolderNone
        : [share, ...folder.split('/').where((s) => s.isNotEmpty)].join(' › ');
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(l.sourcesSmbSetupFolderLabel, style: theme.textTheme.labelMedium),
        const SizedBox(height: 4),
        Row(
          children: [
            const Icon(Icons.folder_outlined),
            const SizedBox(width: 8),
            Expanded(
              child: Text(
                where,
                key: const ValueKey('source-smb-folder'),
                style: theme.textTheme.bodyLarge,
              ),
            ),
            const SizedBox(width: 8),
            OutlinedButton.icon(
              key: const ValueKey('source-choose-folder'),
              onPressed: onChoose,
              icon: const Icon(Icons.folder_open),
              label: Text(l.sourcesSmbSetupChooseFolder),
            ),
          ],
        ),
        if (error != null)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(
              error!,
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.error,
              ),
            ),
          ),
      ],
    );
  }
}
