// SPDX-License-Identifier: MPL-2.0
//
// Adding a Plex Media Server (NET-012, ADR-0131) the way Plex apps on TVs
// do: the form gets a code at once and shows it with plex.tv/link; the
// user enters it on a phone or computer where they are signed in, and no
// password is typed here. Then, only when there is a choice: who is
// watching (a Plex Home user, with their PIN when protected) and which
// server. A server found on the network (NET-010) is preselected. The
// core picks the connection that answers (local, then remote, then
// relay). Editing changes the name; the sign-in stays.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../sources/discovery.dart';
import '../sources/jellyfin_link.dart' show jellyfinDeviceName;
import '../sources/plex_link.dart';
import '../sources/source_manager.dart';
import '../sources/source_text.dart';
import 'add_source_dialog.dart' show SourceErrorView;

enum _Step { code, users, servers, connecting }

/// The add or edit form of a Plex source; pops the saved source.
class PlexSourceForm extends StatefulWidget {
  const PlexSourceForm({
    super.key,
    this.initial,
    this.discovered,
    this.onBack,
    this.pollInterval = const Duration(seconds: 2),
  });

  final ManagedSource? initial;

  /// A server found on the network (NET-010): preselected.
  final DiscoveredSource? discovered;
  final VoidCallback? onBack;

  /// How often the code is checked.
  final Duration pollInterval;

  @override
  State<PlexSourceForm> createState() => _PlexSourceFormState();
}

class _PlexSourceFormState extends State<PlexSourceForm> {
  final _name = TextEditingController();
  final _pin = TextEditingController();
  PlexLink? _link;
  PlexPin? _ticket;
  PlexSignInState? _state;
  Timer? _poll;
  bool _checking = false;
  bool _busy = false;
  bool _expired = false;
  bool _wrongPin = false;
  _Step _step = _Step.code;
  String? _pinFor;
  String? _server;
  SourceException? _error;

  bool get _editing => widget.initial != null;

  @override
  void initState() {
    super.initState();
    if (widget.initial case final s?) _name.text = s.name;
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final first = _link == null;
    _link = Services.read(context).plex;
    if (first && !_editing) unawaited(_start());
  }

  @override
  void dispose() {
    _stop();
    _name.dispose();
    _pin.dispose();
    super.dispose();
  }

  /// Ends the sign-in at the core (cancel, back, closing the dialog).
  void _stop() {
    _poll?.cancel();
    _poll = null;
    final t = _ticket;
    _ticket = null;
    if (t != null) _link?.cancel(t.handle);
  }

  void _fail(Object e) {
    if (!mounted) return;
    setState(() {
      _busy = false;
      _error = e is SourceException
          ? e
          : SourceException(SourceErrorCode.other, '$e');
    });
  }

  Future<void> _start() async {
    final link = _link;
    if (link == null) {
      _fail(const SourceException(SourceErrorCode.unsupported));
      return;
    }
    _stop();
    setState(() {
      _busy = true;
      _expired = false;
      _error = null;
      _state = null;
      _step = _Step.code;
    });
    try {
      final ticket = await link.start(
        deviceName: jellyfinDeviceName(),
        hint: widget.discovered?.url,
      );
      if (!mounted) {
        link.cancel(ticket.handle);
        return;
      }
      setState(() {
        _ticket = ticket;
        _busy = false;
      });
      _poll = Timer.periodic(widget.pollInterval, (_) => _check(link));
    } catch (e) {
      _fail(e);
    }
  }

  Future<void> _check(PlexLink link) async {
    final t = _ticket;
    if (t == null || _checking) return;
    _checking = true;
    try {
      final s = await link.poll(t.handle);
      if (!mounted || !s.linked) return;
      _poll?.cancel();
      _poll = null;
      _show(s);
    } on SourceException catch (e) {
      _poll?.cancel();
      _poll = null;
      if (!mounted) return;
      if (e.code == SourceErrorCode.pathNotFound) {
        // The code expired before it was entered.
        _ticket = null;
        setState(() => _expired = true);
      } else {
        _fail(e);
      }
    } catch (e) {
      _fail(e);
    } finally {
      _checking = false;
    }
  }

  /// The next step after the sign-in or a user switch; a single server is
  /// added without asking.
  void _show(PlexSignInState s, {bool userChosen = false}) {
    final suggested =
        s.servers.where((r) => r.suggested).firstOrNull ?? s.servers.firstOrNull;
    setState(() {
      _state = s;
      _busy = false;
      _error = null;
      _server = suggested?.id;
      _step = s.users.isNotEmpty && !userChosen ? _Step.users : _Step.servers;
    });
    if (_step == _Step.servers && s.servers.length == 1) {
      unawaited(_finish());
    }
  }

  Future<void> _pickUser(PlexUser u) async {
    final t = _ticket;
    final link = _link;
    if (t == null || link == null) return;
    // A protected user (other than the owner, who is signed in already)
    // first asks for the PIN.
    if (u.protected && !u.admin && _pinFor != u.uuid) {
      setState(() {
        _pinFor = u.uuid;
        _wrongPin = false;
        _pin.clear();
      });
      return;
    }
    setState(() {
      _busy = true;
      _wrongPin = false;
      _error = null;
    });
    try {
      final pin = _pinFor == u.uuid ? _pin.text.trim() : null;
      final s = await link.switchUser(t.handle, u.uuid, pin: pin);
      if (!mounted) return;
      _pinFor = null;
      _show(s, userChosen: true);
    } on SourceException catch (e) {
      if (!mounted) return;
      if (e.code == SourceErrorCode.authFailed ||
          e.code == SourceErrorCode.permissionDenied) {
        setState(() {
          _busy = false;
          _wrongPin = true;
        });
      } else {
        _fail(e);
      }
    } catch (e) {
      _fail(e);
    }
  }

  Future<void> _finish() async {
    final t = _ticket;
    final link = _link;
    final server = _server;
    if (t == null || link == null || server == null) return;
    setState(() {
      _busy = true;
      _error = null;
      _step = _Step.connecting;
    });
    try {
      final added = await link.finish(t.handle, server);
      _ticket = null;
      if (!mounted) return;
      final services = Services.read(context);
      final saved = await services.addedMediaServer(added.source);
      if (mounted) Navigator.of(context).pop(saved);
    } catch (e) {
      if (!mounted) return;
      _fail(e);
      setState(() => _step = _Step.servers);
    }
  }

  Future<void> _save() async {
    final s = widget.initial!;
    final services = Services.read(context);
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final name = _name.text.trim();
      final saved = await services.updateSource(
        s.id,
        SourceDraft(
          type: SourceType.plex,
          name: name.isEmpty ? s.name : name,
          address: s.address,
          username: s.username,
          scanIntoLibrary: s.scanIntoLibrary,
          autoScan: s.autoScan,
        ),
      );
      if (mounted) Navigator.of(context).pop(saved);
    } catch (e) {
      _fail(e);
    }
  }

  Widget _spinnerLine(String text) => Row(
    mainAxisAlignment: MainAxisAlignment.center,
    children: [
      const SizedBox.square(
        dimension: 16,
        child: CircularProgressIndicator(strokeWidth: 2),
      ),
      const SizedBox(width: 8),
      Flexible(child: Text(text)),
    ],
  );

  List<Widget> _codeStep(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final ticket = _ticket;
    return [
      Text(l.sourcesPlexIntro),
      const SizedBox(height: 16),
      if (_expired) ...[
        Text(
          l.sourcesPlexCodeExpired,
          key: const ValueKey('plex-expired'),
          textAlign: TextAlign.center,
        ),
      ] else if (ticket == null) ...[
        if (_error == null) _spinnerLine(l.sourcesPlexStarting),
      ] else ...[
        Center(
          child: SelectableText(
            ticket.code,
            key: const ValueKey('plex-code'),
            style: theme.textTheme.displayMedium?.copyWith(letterSpacing: 8),
          ),
        ),
        const SizedBox(height: 8),
        Text(
          l.sourcesPlexLinkHow(_linkLabel(ticket.linkUrl)),
          textAlign: TextAlign.center,
        ),
        const SizedBox(height: 12),
        _spinnerLine(l.sourcesPlexWaiting),
      ],
    ];
  }

  /// `plex.tv/link` without the scheme: shorter to read off a TV.
  static String _linkLabel(String url) =>
      url.replaceFirst(RegExp('^https?://'), '');

  List<Widget> _usersStep(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final users = _state?.users ?? const <PlexUser>[];
    return [
      Text(l.sourcesPlexWhoIsWatching, style: theme.textTheme.titleMedium),
      const SizedBox(height: 8),
      for (final u in users) ...[
        ListTile(
          key: ValueKey('plex-user-${u.uuid}'),
          enabled: !_busy,
          leading: const Icon(Icons.person_outline),
          title: Text(u.name),
          subtitle: u.admin
              ? Text(l.sourcesPlexOwner)
              : u.restricted
              ? Text(l.sourcesPlexManaged)
              : null,
          trailing: u.protected && !u.admin
              ? const Icon(Icons.lock_outline)
              : const Icon(Icons.chevron_right),
          onTap: () => _pickUser(u),
        ),
        if (_pinFor == u.uuid)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
            child: TextField(
              key: const ValueKey('plex-pin'),
              controller: _pin,
              autofocus: true,
              obscureText: true,
              keyboardType: TextInputType.number,
              maxLength: 4,
              enabled: !_busy,
              decoration: InputDecoration(
                labelText: l.sourcesPlexUserPin(u.name),
                errorText: _wrongPin ? l.sourcesPlexWrongPin : null,
                counterText: '',
              ),
              onSubmitted: (_) => _pickUser(u),
            ),
          ),
      ],
    ];
  }

  List<Widget> _serversStep(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final servers = _state?.servers ?? const <PlexServer>[];
    if (servers.isEmpty) {
      return [
        Text(l.sourcesPlexNoServers, key: const ValueKey('plex-no-servers')),
      ];
    }
    return [
      Text(l.sourcesPlexChooseServer, style: theme.textTheme.titleMedium),
      const SizedBox(height: 8),
      for (final s in servers)
        ListTile(
          key: ValueKey('plex-server-${s.id}'),
          enabled: !_busy,
          selected: s.id == _server,
          leading: Icon(
            s.id == _server
                ? Icons.radio_button_checked
                : Icons.radio_button_unchecked,
          ),
          title: Text(s.name),
          subtitle: Text(
            [
              if (!s.owned) l.sourcesPlexServerShared,
              if (!s.online) l.sourcesPlexServerOffline,
              if (s.version.isNotEmpty) s.version,
            ].join(' · '),
          ),
          trailing: const Icon(Icons.dns_outlined),
          onTap: () => setState(() => _server = s.id),
        ),
    ];
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    const gap = SizedBox(height: 12);
    final title = _editing
        ? l.sourcesEditTitle
        : '${l.addSourceTitle}: ${sourceTypeLabel(l, SourceType.plex)}';

    final fields = <Widget>[
      if (_editing)
        TextField(
          key: const ValueKey('plex-name'),
          controller: _name,
          decoration: InputDecoration(labelText: l.addSourceDisplayName),
          onSubmitted: (_) => _busy ? null : _save(),
        )
      else ...switch (_step) {
        _Step.code => _codeStep(context),
        _Step.users => _usersStep(context),
        _Step.servers => _serversStep(context),
        _Step.connecting => [_spinnerLine(l.sourcesPlexConnecting)],
      },
      if (!_editing) ...[
        gap,
        Text(l.sourcesPlexNote, style: theme.textTheme.bodySmall),
      ],
      if (_error case final e?) ...[
        gap,
        SourceErrorView(code: e.code, detail: e.detail),
      ],
    ];

    final Widget? action = _editing
        ? FilledButton(
            key: const ValueKey('plex-save'),
            onPressed: _busy ? null : _save,
            child: Text(l.commonSave),
          )
        : switch (_step) {
            _Step.code when _expired || (_error != null && _ticket == null) =>
              FilledButton(
                key: const ValueKey('plex-new-code'),
                onPressed: _busy ? null : _start,
                child: Text(l.sourcesPlexNewCode),
              ),
            _Step.users when _pinFor != null => FilledButton(
              key: const ValueKey('plex-pin-continue'),
              onPressed: _busy
                  ? null
                  : () {
                      final u = _state?.users
                          .where((u) => u.uuid == _pinFor)
                          .firstOrNull;
                      if (u != null) _pickUser(u);
                    },
              child: Text(l.sourcesPlexContinue),
            ),
            _Step.servers when (_state?.servers.isNotEmpty ?? false) =>
              FilledButton(
                key: const ValueKey('plex-add'),
                onPressed: _busy || _server == null ? null : _finish,
                child: Text(l.sourcesPlexAdd),
              ),
            _ => null,
          };

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: EdgeInsets.fromLTRB(
            widget.onBack == null ? 24 : 8,
            20,
            24,
            8,
          ),
          child: Row(
            children: [
              if (widget.onBack != null)
                IconButton(
                  tooltip: l.commonBack,
                  onPressed: _step == _Step.connecting
                      ? null
                      : () {
                          _stop();
                          widget.onBack!();
                        },
                  icon: const Icon(Icons.arrow_back),
                ),
              Expanded(
                child: Text(title, style: theme.textTheme.headlineSmall),
              ),
            ],
          ),
        ),
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
                key: const ValueKey('plex-cancel'),
                onPressed: () {
                  _stop();
                  Navigator.pop(context);
                },
                child: Text(l.commonCancel),
              ),
              ?action,
            ],
          ),
        ),
      ],
    );
  }
}
