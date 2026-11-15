// SPDX-License-Identifier: MPL-2.0
//
// Adding a network folder (SMB) without knowing what a share is (UI-028):
// 1. the server — found on the network or any address the user pastes;
// 2. sign in — user and password, or guest, checked right away;
// 3. the folder — the server's shared folders, then folders inside them,
//    and a big "Use this folder" button.
// A path typed in step 1 (`/home/alex/films`, `\\nas\home\alex\films`) is
// looked up on the server by the core (`smb_find`), so the user never has
// to know the share name. The same folder browser fixes an existing source
// ("Choose folder" in the edit form and on a broken source).

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../sources/discovery.dart';
import '../sources/source_address.dart';
import '../sources/source_manager.dart';
import '../sources/source_text.dart';

/// A plain sentence with the next step for an error of the SMB setup.
String smbErrorText(
  AppLocalizations l,
  SourceErrorCode code, {
  bool guest = false,
}) => switch (code) {
  SourceErrorCode.unreachable ||
  SourceErrorCode.timedOut => l.smbErrorUnreachable,
  SourceErrorCode.authFailed when guest => l.smbErrorGuestRefused,
  SourceErrorCode.permissionDenied when guest => l.smbErrorGuestRefused,
  SourceErrorCode.authFailed => l.smbErrorAuth,
  SourceErrorCode.shareNotFound => l.sourcesErrorShareNotFound,
  SourceErrorCode.pathNotFound ||
  SourceErrorCode.notADirectory => l.smbErrorFolderNotFound,
  SourceErrorCode.permissionDenied => l.smbErrorPermission,
  _ => sourceErrorText(l, code),
};

/// "nas › Media › Movies".
String smbPlaceLabel(String server, SmbPlace? place) => [
  server,
  if (place != null) place.share,
  if (place != null) ...place.folder.split('/').where((s) => s.isNotEmpty),
].join(' › ');

/// "share" or "share / folder" for the user.
String smbFoundText(AppLocalizations l, SmbPlace p) => p.folder.isEmpty
    ? l.smbSetupFoundShare(p.share)
    : l.smbSetupFoundOne(p.share, p.folder);

class _ErrorBox extends StatelessWidget {
  const _ErrorBox(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Container(
      key: const ValueKey('smb-error'),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: theme.colorScheme.errorContainer.withValues(alpha: .35),
        borderRadius: BorderRadius.circular(10),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Icon(Icons.error_outline, color: theme.colorScheme.error),
          const SizedBox(width: 10),
          Expanded(child: Text(text, style: theme.textTheme.bodyMedium)),
        ],
      ),
    );
  }
}

class _InfoBox extends StatelessWidget {
  const _InfoBox(this.text, {super.key, this.icon = Icons.check_circle});

  final String text;
  final IconData icon;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Container(
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: theme.colorScheme.primaryContainer.withValues(alpha: .35),
        borderRadius: BorderRadius.circular(10),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Icon(icon, color: theme.colorScheme.primary),
          const SizedBox(width: 10),
          Expanded(child: Text(text, style: theme.textTheme.bodyMedium)),
        ],
      ),
    );
  }
}

/// Domain, SMB version, encryption and signing: collapsed under
/// "Advanced"; the defaults work for almost every server.
class SmbAdvancedFields extends StatelessWidget {
  const SmbAdvancedFields({
    super.key,
    required this.domain,
    required this.settings,
    required this.onChanged,
  });

  final TextEditingController domain;
  final SmbSettings settings;
  final ValueChanged<SmbSettings> onChanged;

  SmbSettings _with({String? version, String? encryption, bool? signing}) =>
      SmbSettings(
        domain: settings.domain,
        version: version ?? settings.version,
        encryption: encryption ?? settings.encryption,
        signingRequired: signing ?? settings.signingRequired,
      );

  Widget _chips(
    Map<String, String> options,
    String value,
    ValueChanged<String> on,
  ) => Padding(
    padding: const EdgeInsets.only(top: 6, bottom: 8),
    child: Wrap(
      spacing: 8,
      runSpacing: 6,
      children: [
        for (final MapEntry(:key, value: label) in options.entries)
          ChoiceChip(
            label: Text(label),
            selected: key == value,
            onSelected: (_) => on(key),
          ),
      ],
    ),
  );

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        TextField(
          key: const ValueKey('source-domain'),
          controller: domain,
          decoration: InputDecoration(labelText: l.sourcesFormDomain),
        ),
        const SizedBox(height: 12),
        Text(l.sourcesFormSmbVersion),
        _chips(
          {
            'any': l.sourcesFormSmbVersionAny, // l10n-ignore
            'smb2': l.sourcesFormSmbVersionSmb2, // l10n-ignore
            'smb3': l.sourcesFormSmbVersionSmb3, // l10n-ignore
          },
          settings.version,
          (v) => onChanged(_with(version: v)),
        ),
        Text(l.sourcesFormEncryption),
        _chips(
          {
            'auto': l.sourcesFormEncryptionAuto, // l10n-ignore
            'required': l.sourcesFormEncryptionRequired, // l10n-ignore
            'off': l.sourcesFormEncryptionOff, // l10n-ignore
          },
          settings.encryption,
          (v) => onChanged(_with(encryption: v)),
        ),
        SwitchListTile(
          contentPadding: EdgeInsets.zero,
          value: settings.signingRequired,
          title: Text(l.sourcesFormSigning),
          onChanged: (v) => onChanged(_with(signing: v)),
        ),
      ],
    );
  }
}

enum _Step { server, login, folder }

/// The step-by-step SMB add flow. Pops the created [ManagedSource].
class SmbSetupFlow extends StatefulWidget {
  const SmbSetupFlow({super.key, this.discovered, this.onBack});

  /// A server picked on the network before the flow opened.
  final DiscoveredSource? discovered;
  final VoidCallback? onBack;

  @override
  State<SmbSetupFlow> createState() => _SmbSetupFlowState();
}

class _SmbSetupFlowState extends State<SmbSetupFlow> {
  final _server = TextEditingController();
  final _path = TextEditingController();
  final _user = TextEditingController();
  final _password = TextEditingController();
  final _domain = TextEditingController();
  SmbSettings _smb = const SmbSettings();
  Stream<List<DiscoveredSource>>? _found;

  _Step _step = _Step.server;
  bool _guest = false;
  bool _busy = false;
  bool _serverMissing = false;
  SourceException? _error;

  /// Server as parsed in step 1.
  String _host = '';

  /// Share and folder typed in step 1, for `smbFind`.
  String _typedShare = '';
  String _typedFolder = '';
  List<SmbShareInfo>? _shares;

  @override
  void initState() {
    super.initState();
    final d = widget.discovered;
    if (d != null) {
      _server.text = splitSmb(d.url ?? '')?.host ?? d.host;
      _host = _server.text;
      _step = _Step.login;
    }
  }

  @override
  void dispose() {
    for (final c in [_server, _path, _user, _password, _domain]) {
      c.dispose();
    }
    super.dispose();
  }

  SmbLogin get _login => SmbLogin(
    server: _host,
    username: _guest ? '' : _user.text.trim(),
    password: _guest ? '' : _password.text,
    smb: SmbSettings(
      domain: _domain.text.trim().isEmpty ? null : _domain.text.trim(),
      version: _smb.version,
      encryption: _smb.encryption,
      signingRequired: _smb.signingRequired,
    ),
  );

  void _goToLogin([String? host]) {
    final typed = parseSmbInput(host ?? _server.text);
    final path = parseSmbInput(
      _path.text.trim().isEmpty ? '' : '/${_path.text.trim()}',
    );
    if (typed.host.isEmpty) {
      setState(() => _serverMissing = true);
      return;
    }
    final full = typed.path.isNotEmpty ? typed : path;
    setState(() {
      _serverMissing = false;
      _error = null;
      _host = typed.host;
      _typedShare = full.share;
      _typedFolder = full.folder;
      if (typed.user != null && _user.text.isEmpty) _user.text = typed.user!;
      _step = _Step.login;
    });
  }

  Future<void> _connect() async {
    final manager = Services.read(context).sourceManager;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final shares = await manager.smbShares(_login);
      if (!mounted) return;
      setState(() {
        _shares = shares;
        _busy = false;
        _step = _Step.folder;
      });
    } on SourceException catch (e) {
      if (!mounted) return;
      // A server that does not list its shares to this user may still open
      // the folder that was typed: let the folder step try it.
      if (e.code == SourceErrorCode.permissionDenied &&
          _typedShare.isNotEmpty &&
          !_guest) {
        setState(() {
          _shares = const [];
          _busy = false;
          _step = _Step.folder;
        });
        return;
      }
      setState(() {
        _busy = false;
        _error = e;
      });
    }
  }

  Future<void> _create(SmbPlace place) async {
    final services = Services.read(context);
    final login = _login;
    final folder = place.folder;
    final name = folder.isEmpty
        ? place.share
        : folder.split('/').where((s) => s.isNotEmpty).last;
    final draft = SourceDraft(
      type: SourceType.smb,
      name: name,
      address: 'smb://$_host/${place.share}${folder.isEmpty ? '' : '/$folder'}', // l10n-ignore
      username: login.guest ? null : login.username,
      password: login.guest ? null : login.password,
      smb: login.smb,
    );
    final saved = await services.createSource(draft);
    if (mounted) Navigator.of(context).pop(saved);
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final (index, heading) = switch (_step) {
      _Step.server => (1, l.smbSetupServerHeading),
      _Step.login => (2, l.smbSetupLoginHeading(_host)),
      _Step.folder => (3, l.smbSetupFolderHeading),
    };
    final back = switch (_step) {
      _Step.server => widget.onBack,
      _Step.login =>
        widget.discovered != null && widget.onBack != null
            ? widget.onBack
            : () => setState(() {
                _error = null;
                _step = _Step.server;
              }),
      _Step.folder => () => setState(() => _step = _Step.login),
    };
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: EdgeInsets.fromLTRB(back == null ? 24 : 8, 20, 24, 4),
          child: Row(
            children: [
              if (back != null)
                IconButton(
                  tooltip: l.commonBack,
                  onPressed: _busy ? null : back,
                  icon: const Icon(Icons.arrow_back),
                ),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      l.smbSetupStep(index, 3),
                      key: const ValueKey('smb-step'),
                      style: theme.textTheme.labelMedium,
                    ),
                    Text(heading, style: theme.textTheme.headlineSmall),
                  ],
                ),
              ),
            ],
          ),
        ),
        Flexible(
          child: switch (_step) {
            _Step.server => _serverStep(l, theme),
            _Step.login => _loginStep(l, theme),
            _Step.folder => SmbFolderBrowser(
              key: const ValueKey('smb-browser'),
              login: _login,
              shares: _shares,
              hintShare: _typedShare,
              hintFolder: _typedFolder,
              onPick: _create,
            ),
          },
        ),
        if (_step != _Step.folder)
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
                if (_step == _Step.server)
                  FilledButton(
                    key: const ValueKey('smb-next'),
                    onPressed: () => _goToLogin(),
                    child: Text(l.smbSetupNext),
                  )
                else
                  FilledButton.icon(
                    key: const ValueKey('smb-connect'),
                    onPressed: _busy ? null : _connect,
                    icon: _busy
                        ? const SizedBox.square(
                            dimension: 16,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          )
                        : const Icon(Icons.login),
                    label: Text(
                      _busy ? l.smbSetupConnecting : l.smbSetupConnect,
                    ),
                  ),
              ],
            ),
          ),
      ],
    );
  }

  Widget _serverStep(AppLocalizations l, ThemeData theme) {
    final services = Services.read(context);
    final discovery = services.discovery;
    final found = discovery.supported ? (_found ??= discovery.watch()) : null;
    return SingleChildScrollView(
      padding: const EdgeInsets.fromLTRB(24, 8, 24, 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (found != null)
            StreamBuilder<List<DiscoveredSource>>(
              stream: found,
              builder: (context, snap) {
                final servers = <String, DiscoveredSource>{};
                for (final d in snap.data ?? const <DiscoveredSource>[]) {
                  if (d.type != SourceType.smb) continue;
                  servers.putIfAbsent(
                    splitSmb(d.url ?? '')?.host ?? d.host,
                    () => d,
                  );
                }
                return Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text(l.smbSetupFound, style: theme.textTheme.labelLarge),
                    if (servers.isEmpty)
                      Padding(
                        padding: const EdgeInsets.symmetric(vertical: 8),
                        child: Text(
                          l.smbSetupSearching,
                          style: theme.textTheme.bodySmall,
                        ),
                      ),
                    for (final (i, MapEntry(key: host, value: d))
                        in servers.entries.indexed)
                      ListTile(
                        key: ValueKey('smb-server-$host'),
                        autofocus: i == 0,
                        contentPadding: EdgeInsets.zero,
                        leading: const Icon(Icons.dns_outlined),
                        title: Text(d.name),
                        subtitle: Text(host),
                        trailing: const Icon(Icons.chevron_right),
                        onTap: () {
                          _server.text = host;
                          _goToLogin(host);
                        },
                      ),
                    const SizedBox(height: 12),
                  ],
                );
              },
            ),
          TextField(
            key: const ValueKey('smb-server'),
            controller: _server,
            keyboardType: TextInputType.url,
            autocorrect: false,
            textInputAction: TextInputAction.next,
            decoration: InputDecoration(
              labelText: l.smbSetupServerField,
              hintText: l.smbSetupServerHint,
              helperText: l.smbSetupServerHelper,
              helperMaxLines: 3,
              errorText: _serverMissing ? l.smbSetupServerEmpty : null,
              errorMaxLines: 3,
            ),
            onSubmitted: (_) => _goToLogin(),
          ),
          const SizedBox(height: 12),
          TextField(
            key: const ValueKey('smb-path'),
            controller: _path,
            autocorrect: false,
            decoration: InputDecoration(
              labelText: l.smbSetupPathField,
              hintText: l.smbSetupPathHint,
              helperText: l.smbSetupPathHelper,
              helperMaxLines: 3,
            ),
            onSubmitted: (_) => _goToLogin(),
          ),
        ],
      ),
    );
  }

  Widget _loginStep(AppLocalizations l, ThemeData theme) {
    final error = _error;
    return SingleChildScrollView(
      padding: const EdgeInsets.fromLTRB(24, 8, 24, 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SwitchListTile(
            key: const ValueKey('smb-guest'),
            contentPadding: EdgeInsets.zero,
            value: _guest,
            title: Text(l.smbSetupGuest),
            subtitle: Text(l.smbSetupGuestHint),
            onChanged: _busy ? null : (v) => setState(() => _guest = v),
          ),
          if (!_guest) ...[
            TextField(
              key: const ValueKey('smb-user'),
              controller: _user,
              autofocus: true,
              autocorrect: false,
              enabled: !_busy,
              textInputAction: TextInputAction.next,
              decoration: InputDecoration(
                labelText: l.smbSetupUser,
                helperText: l.smbSetupUserHint,
                helperMaxLines: 2,
              ),
            ),
            const SizedBox(height: 12),
            TextField(
              key: const ValueKey('smb-password'),
              controller: _password,
              obscureText: true,
              enabled: !_busy,
              decoration: InputDecoration(labelText: l.smbSetupPassword),
              onSubmitted: (_) => _connect(),
            ),
          ],
          if (error != null) ...[
            const SizedBox(height: 12),
            _ErrorBox(smbErrorText(l, error.code, guest: _guest)),
          ],
          Theme(
            data: theme.copyWith(dividerColor: Colors.transparent),
            child: ExpansionTile(
              key: const ValueKey('smb-advanced'),
              tilePadding: EdgeInsets.zero,
              title: Text(l.sourcesFormAdvanced),
              children: [
                SmbAdvancedFields(
                  domain: _domain,
                  settings: _smb,
                  onChanged: (s) => setState(() => _smb = s),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// The server's shared folders, then the folders inside them, with "Use
/// this folder". With a hint (a path the user typed) it first asks the core
/// where that path is and opens there.
class SmbFolderBrowser extends StatefulWidget {
  const SmbFolderBrowser({
    super.key,
    required this.login,
    required this.onPick,
    this.shares,
    this.hintShare = '',
    this.hintFolder = '',
    this.actionLabel,
  });

  final SmbLogin login;

  /// The share list if already loaded (the add flow loads it to check the
  /// logon).
  final List<SmbShareInfo>? shares;
  final String hintShare;
  final String hintFolder;

  /// Called with the chosen folder; errors it throws are shown.
  final Future<void> Function(SmbPlace place) onPick;

  /// Label of the main button; "Use this folder" by default.
  final String? actionLabel;

  @override
  State<SmbFolderBrowser> createState() => _SmbFolderBrowserState();
}

class _SmbFolderBrowserState extends State<SmbFolderBrowser> {
  /// Null: the list of shares.
  SmbPlace? _at;
  List<SmbShareInfo> _shares = const [];
  List<SourceFolderEntry> _entries = const [];
  List<SmbPlace> _candidates = const [];
  String? _note;
  bool _noteIsWarning = false;
  bool _loading = true;
  bool _saving = false;
  SourceException? _error;

  SourceManager get _manager => Services.read(context).sourceManager;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _start());
  }

  Future<void> _start() async {
    final l = context.l10n;
    try {
      _shares = widget.shares ?? await _manager.smbShares(widget.login);
    } on SourceException catch (e) {
      // Listing may be refused while the typed folder still opens.
      if (widget.hintShare.isEmpty ||
          e.code != SourceErrorCode.permissionDenied) {
        if (mounted) setState(() => _fail(e));
        return;
      }
    }
    final hint = [
      widget.hintShare,
      widget.hintFolder,
    ].where((s) => s.isNotEmpty).join('/');
    if (hint.isNotEmpty) {
      try {
        final found = await _manager.smbFind(
          widget.login,
          widget.hintShare,
          widget.hintFolder,
        );
        if (!mounted) return;
        if (found.length == 1) {
          _note = smbFoundText(l, found.first);
          await _open(found.first);
          return;
        }
        if (found.length > 1) {
          setState(() {
            _candidates = found;
            _note = l.smbSetupFoundMany;
          });
        } else {
          setState(() {
            _note = l.smbSetupNotFound(hint);
            _noteIsWarning = true;
          });
        }
      } on SourceException catch (e) {
        if (mounted) setState(() => _fail(e));
        return;
      }
    }
    await _open(null);
  }

  void _fail(SourceException e) {
    _loading = false;
    _error = e;
  }

  Future<void> _open(SmbPlace? place) async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final entries = place == null
          ? const <SourceFolderEntry>[]
          : await _manager.smbListFolder(
              widget.login,
              place.share,
              place.folder,
            );
      if (!mounted) return;
      final folders = [...entries]
        ..sort((a, b) {
          if (a.isFolder != b.isFolder) return a.isFolder ? -1 : 1;
          return a.name.toLowerCase().compareTo(b.name.toLowerCase());
        });
      setState(() {
        _at = place;
        _entries = folders;
        _loading = false;
      });
    } on SourceException catch (e) {
      if (mounted) setState(() => _fail(e));
    }
  }

  void _up() {
    final at = _at;
    if (at == null) return;
    final parts = at.folder.split('/').where((s) => s.isNotEmpty).toList();
    _note = null;
    if (parts.isEmpty) {
      unawaited(_open(null));
    } else {
      unawaited(
        _open(SmbPlace(at.share, parts.sublist(0, parts.length - 1).join('/'))),
      );
    }
  }

  Future<void> _pick() async {
    final at = _at;
    if (at == null) return;
    setState(() {
      _saving = true;
      _error = null;
    });
    try {
      await widget.onPick(at);
    } on SourceException catch (e) {
      if (mounted) setState(() => _error = e);
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final at = _at;
    final error = _error;
    final note = _note;
    final folders = _entries.where((e) => e.isFolder).toList();
    final files = _entries.length - folders.length;
    final rows = <Widget>[
      if (at == null)
        for (final (i, s) in _shares.indexed)
          ListTile(
            key: ValueKey('smb-share-${s.name}'),
            autofocus: i == 0 && _candidates.isEmpty,
            leading: const Icon(Icons.folder_shared_outlined),
            title: Text(s.name),
            subtitle: s.comment.isEmpty ? null : Text(s.comment),
            trailing: const Icon(Icons.chevron_right),
            onTap: () {
              _note = null;
              unawaited(_open(SmbPlace(s.name)));
            },
          )
      else
        for (final (i, f) in folders.indexed)
          ListTile(
            key: ValueKey('smb-folder-${f.name}'),
            autofocus: i == 0,
            leading: const Icon(Icons.folder_outlined),
            title: Text(f.name),
            trailing: const Icon(Icons.chevron_right),
            onTap: () {
              _note = null;
              unawaited(_open(SmbPlace(at.share, f.relPath)));
            },
          ),
    ];
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 0, 24, 4),
          child: Row(
            children: [
              IconButton(
                key: const ValueKey('smb-up'),
                tooltip: l.smbSetupUp,
                onPressed: at == null || _loading ? null : _up,
                icon: const Icon(Icons.arrow_upward),
              ),
              const SizedBox(width: 4),
              Expanded(
                child: Text(
                  smbPlaceLabel(widget.login.server, at),
                  key: const ValueKey('smb-location'),
                  style: theme.textTheme.titleSmall,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ],
          ),
        ),
        Flexible(
          child: ListView(
            shrinkWrap: true,
            padding: const EdgeInsets.fromLTRB(24, 0, 24, 8),
            children: [
              if (note != null) ...[
                _InfoBox(
                  note,
                  key: const ValueKey('smb-note'),
                  icon: _noteIsWarning
                      ? Icons.info_outline
                      : Icons.check_circle,
                ),
                const SizedBox(height: 8),
              ],
              for (final c in _candidates)
                ListTile(
                  key: ValueKey('smb-candidate-$c'),
                  autofocus: c == _candidates.first,
                  leading: const Icon(Icons.place_outlined),
                  title: Text(smbFoundText(l, c)),
                  onTap: () {
                    setState(() {
                      _candidates = const [];
                      _note = smbFoundText(l, c);
                      _noteIsWarning = false;
                    });
                    unawaited(_open(c));
                  },
                ),
              if (error != null) ...[
                _ErrorBox(
                  smbErrorText(l, error.code, guest: widget.login.guest),
                ),
                Align(
                  alignment: Alignment.centerLeft,
                  child: TextButton.icon(
                    onPressed: () {
                      if (at != null) return unawaited(_open(at));
                      setState(() {
                        _error = null;
                        _loading = true;
                      });
                      unawaited(_start());
                    },
                    icon: const Icon(Icons.refresh),
                    label: Text(l.smbSetupRetry),
                  ),
                ),
              ] else if (_loading)
                const Padding(
                  padding: EdgeInsets.all(24),
                  child: Center(child: CircularProgressIndicator()),
                )
              else ...[
                if (at == null && _shares.isEmpty)
                  Text(l.smbSetupNoShares, style: theme.textTheme.bodyMedium),
                if (at == null && _shares.isNotEmpty)
                  Padding(
                    padding: const EdgeInsets.only(bottom: 4),
                    child: Text(
                      l.smbSetupSharesHint,
                      style: theme.textTheme.bodySmall,
                    ),
                  ),
                ...rows,
                if (at != null && folders.isEmpty && files == 0)
                  Padding(
                    padding: const EdgeInsets.symmetric(vertical: 8),
                    child: Text(
                      l.smbSetupEmptyFolder,
                      style: theme.textTheme.bodySmall,
                    ),
                  ),
                if (at != null && files > 0)
                  Padding(
                    padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
                    child: Text(
                      l.smbSetupFiles(files),
                      key: const ValueKey('smb-files'),
                      style: theme.textTheme.bodySmall,
                    ),
                  ),
              ],
            ],
          ),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(24, 4, 24, 16),
          child: FilledButton.icon(
            key: const ValueKey('smb-use-folder'),
            style: FilledButton.styleFrom(
              minimumSize: const Size.fromHeight(52),
            ),
            onPressed: at == null || _loading || _saving ? null : _pick,
            icon: _saving
                ? const SizedBox.square(
                    dimension: 18,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                : const Icon(Icons.check),
            label: Text(
              at == null
                  ? l.smbSetupPickShareFirst
                  : (widget.actionLabel ?? l.smbSetupUseFolder),
            ),
          ),
        ),
      ],
    );
  }
}

/// Opens the folder browser in a dialog ("Choose folder" in the edit form
/// and on a broken source). Returns the chosen place, or null.
Future<SmbPlace?> showSmbFolderPicker(
  BuildContext context, {
  required SmbLogin login,
  String hintShare = '',
  String hintFolder = '',
}) => showDialog<SmbPlace>(
  context: context,
  builder: (c) => Dialog(
    child: ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 520, maxHeight: 640),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(8, 16, 24, 4),
            child: Row(
              children: [
                IconButton(
                  tooltip: c.l10n.commonCancel,
                  onPressed: () => Navigator.pop(c),
                  icon: const Icon(Icons.close),
                ),
                Expanded(
                  child: Text(
                    c.l10n.smbSetupFolderHeading,
                    style: Theme.of(c).textTheme.headlineSmall,
                  ),
                ),
              ],
            ),
          ),
          Flexible(
            child: SmbFolderBrowser(
              login: login,
              hintShare: hintShare,
              hintFolder: hintFolder,
              onPick: (p) async => Navigator.pop(c, p),
            ),
          ),
        ],
      ),
    ),
  ),
);
