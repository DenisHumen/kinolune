// SPDX-License-Identifier: MPL-2.0
//
// Adding a cloud storage (NET-017) in the simple «Добавить» flow, in the
// step-by-step style of the SMB setup (UI-028/UI-042):
//
// * Google Drive, OneDrive, Dropbox — one button opens the browser; the
//   user signs in there and allows read-only access; the form waits for
//   the browser to come back (a loopback address on this device, or the
//   `kinolune://oauth` link on Android with an own client registered for
//   it), then offers an optional folder and adds the source. No password
//   is typed into the app. «Дополнительно» takes the user's own client id
//   when this build has none.
// * MEGA — e-mail and password (and the 2FA code when the account has it),
//   or a public folder/file link with its key. The password is used for
//   the sign-in only; the core keeps the session.
//
// Editing changes the name; the sign-in stays.

import 'dart:async';
import 'dart:io' show Platform;

import 'package:material_ui/material_ui.dart';
import 'package:url_launcher/url_launcher.dart';

import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../sources/cloud_link.dart';
import '../sources/source_manager.dart';
import '../sources/source_text.dart';
import 'add_source_dialog.dart' show SourceErrorView;

enum _Step { start, waiting, signedIn, adding }

enum _MegaMode { account, link }

/// Opens [url] in the browser; false when that is not possible.
typedef UrlOpener = Future<bool> Function(Uri url);

Future<bool> _launch(Uri url) =>
    launchUrl(url, mode: LaunchMode.externalApplication);

/// The add or edit form of a cloud source; pops the saved source.
class CloudSourceForm extends StatefulWidget {
  const CloudSourceForm({
    super.key,
    required this.type,
    this.initial,
    this.onBack,
    this.openUrl = _launch,
  });

  final SourceType type;
  final ManagedSource? initial;
  final VoidCallback? onBack;

  /// Opens the sign-in page (tests replace it).
  final UrlOpener openUrl;

  @override
  State<CloudSourceForm> createState() => _CloudSourceFormState();
}

class _CloudSourceFormState extends State<CloudSourceForm> {
  final _name = TextEditingController();
  final _folder = TextEditingController();
  final _clientId = TextEditingController();
  final _clientSecret = TextEditingController();
  final _email = TextEditingController();
  final _password = TextEditingController();
  final _mfa = TextEditingController();
  final _megaLink = TextEditingController();

  CloudLink? _link;
  CloudAuth? _auth;
  StreamSubscription<String>? _redirects;
  _Step _step = _Step.start;
  _MegaMode _megaMode = _MegaMode.account;
  bool _busy = false;
  bool _needMfa = false;
  bool _cancelled = false;

  /// Android with an own client registered for `kinolune://oauth`.
  bool _appRedirect = false;
  String _account = '';
  SourceException? _error;

  bool get _editing => widget.initial != null;
  bool get _mega => widget.type == SourceType.mega;

  CloudProvider? get _provider =>
      _link?.providers().where((p) => p.type == widget.type).firstOrNull;

  @override
  void initState() {
    super.initState();
    if (widget.initial case final s?) _name.text = s.name;
    // The action button follows what is filled in.
    for (final c in [_email, _password, _megaLink, _clientId]) {
      c.addListener(_changed);
    }
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _link = Services.read(context).cloud;
  }

  @override
  void dispose() {
    _stop();
    for (final c in [
      _name,
      _folder,
      _clientId,
      _clientSecret,
      _email,
      _password,
      _mfa,
      _megaLink,
    ]) {
      c.dispose();
    }
    super.dispose();
  }

  /// Ends a browser sign-in at the core (cancel, back, closing the dialog).
  void _stop() {
    unawaited(_redirects?.cancel());
    _redirects = null;
    final a = _auth;
    _auth = null;
    if (a != null) _link?.cancel(a.handle);
  }

  void _fail(Object e) {
    if (!mounted) return;
    setState(() {
      _busy = false;
      if (e is CloudSignInCancelled) {
        _cancelled = true;
        _step = _Step.start;
        return;
      }
      _error = e is SourceException
          ? e
          : SourceException(SourceErrorCode.other, '$e');
    });
  }

  String? _text(TextEditingController c) {
    final t = c.text.trim();
    return t.isEmpty ? null : t;
  }

  // Browser sign-in -------------------------------------------------------

  Future<void> _signIn() async {
    final link = _link;
    if (link == null) {
      _fail(const SourceException(SourceErrorCode.unsupported));
      return;
    }
    _stop();
    setState(() {
      _busy = true;
      _cancelled = false;
      _error = null;
    });
    try {
      final appRedirect = _appRedirect && _text(_clientId) != null;
      final auth = await link.start(
        widget.type,
        clientId: _text(_clientId),
        clientSecret: _text(_clientSecret),
        appRedirect: appRedirect,
      );
      if (!mounted) {
        link.cancel(auth.handle);
        return;
      }
      _auth = auth;
      setState(() {
        _busy = false;
        _step = _Step.waiting;
      });
      if (appRedirect) {
        // The browser comes back through `kinolune://oauth` (app_links).
        _redirects = CloudRedirects.stream.listen((r) => _complete(link, r));
        await _open(auth);
        return;
      }
      await _open(auth);
      final account = await link.wait(auth.handle);
      _signedIn(account);
    } catch (e) {
      if (_step == _Step.signedIn) return;
      _fail(e);
      if (mounted && _error != null) setState(() => _step = _Step.start);
    }
  }

  Future<void> _open(CloudAuth auth) async {
    final opened = await widget.openUrl(Uri.parse(auth.url));
    if (!opened && mounted) {
      _fail(const SourceException(SourceErrorCode.unsupported));
    }
  }

  Future<void> _complete(CloudLink link, String redirect) async {
    final a = _auth;
    if (a == null || _step != _Step.waiting) return;
    try {
      _signedIn(await link.complete(a.handle, redirect));
    } catch (e) {
      _fail(e);
    }
  }

  void _signedIn(String account) {
    if (!mounted) return;
    setState(() {
      _account = account;
      _busy = false;
      _error = null;
      _step = _Step.signedIn;
    });
  }

  Future<void> _finish() async {
    final link = _link;
    final a = _auth;
    if (link == null || a == null) return;
    setState(() {
      _busy = true;
      _error = null;
      _step = _Step.adding;
    });
    try {
      final added = await link.finish(
        a.handle,
        name: _text(_name),
        folder: _text(_folder),
      );
      _auth = null;
      await _done(added);
    } catch (e) {
      _fail(e);
      if (mounted) setState(() => _step = _Step.signedIn);
    }
  }

  // MEGA ------------------------------------------------------------------

  Future<void> _addMega() async {
    final link = _link;
    if (link == null) {
      _fail(const SourceException(SourceErrorCode.unsupported));
      return;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final ManagedSource added;
      if (_megaMode == _MegaMode.link) {
        added = await link.addMegaLink(
          _megaLink.text.trim(),
          name: _text(_name),
        );
      } else {
        added = await link.addMega(
          email: _email.text.trim(),
          password: _password.text,
          mfa: _needMfa ? _text(_mfa) : null,
          name: _text(_name),
          folder: _text(_folder),
        );
      }
      await _done(added);
    } on CloudMfaRequired {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _needMfa = true;
      });
    } catch (e) {
      _fail(e);
    }
  }

  Future<void> _done(ManagedSource added) async {
    if (!mounted) return;
    final saved = await Services.read(context).addedMediaServer(added);
    if (mounted) Navigator.of(context).pop(saved);
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
          type: s.type,
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

  // Layout ----------------------------------------------------------------

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

  Widget _folderField(AppLocalizations l) => TextField(
    key: const ValueKey('cloud-folder'),
    controller: _folder,
    decoration: InputDecoration(
      labelText: l.sourcesCloudFolder,
      helperText: l.sourcesCloudFolderHelper,
      helperMaxLines: 2,
    ),
  );

  Widget _nameField(AppLocalizations l) => TextField(
    key: const ValueKey('cloud-name'),
    controller: _name,
    decoration: InputDecoration(labelText: l.addSourceDisplayName),
  );

  List<Widget> _oauthFields(AppLocalizations l, String cloud) {
    final theme = Theme.of(context);
    final configured = _provider?.configured ?? false;
    return switch (_step) {
      _Step.start => [
        Text(l.sourcesCloudIntro(cloud)),
        if (_cancelled) ...[
          const SizedBox(height: 8),
          Text(l.sourcesCloudCancelled, key: const ValueKey('cloud-cancelled')),
        ],
        if (!configured) ...[
          const SizedBox(height: 8),
          Text(
            l.sourcesCloudNoClient(cloud),
            key: const ValueKey('cloud-no-client'),
            style: theme.textTheme.bodySmall,
          ),
        ],
        const SizedBox(height: 8),
        Theme(
          data: theme.copyWith(dividerColor: Colors.transparent),
          child: ExpansionTile(
            key: const ValueKey('cloud-advanced'),
            initiallyExpanded: !configured,
            tilePadding: EdgeInsets.zero,
            title: Text(l.sourcesFormAdvanced),
            children: [
              TextField(
                key: const ValueKey('cloud-client-id'),
                controller: _clientId,
                autocorrect: false,
                decoration: InputDecoration(
                  labelText: l.sourcesCloudOwnClient,
                  helperText: l.sourcesCloudOwnClientHelper,
                  helperMaxLines: 3,
                ),
              ),
              if (widget.type == SourceType.gdrive)
                TextField(
                  key: const ValueKey('cloud-client-secret'),
                  controller: _clientSecret,
                  autocorrect: false,
                  obscureText: true,
                  decoration: InputDecoration(
                    labelText: l.sourcesCloudOwnSecret,
                  ),
                ),
              if (Platform.isAndroid && _clientId.text.trim().isNotEmpty)
                SwitchListTile(
                  key: const ValueKey('cloud-app-redirect'),
                  contentPadding: EdgeInsets.zero,
                  value: _appRedirect,
                  onChanged: (v) => setState(() => _appRedirect = v),
                  title: Text(l.sourcesCloudAppRedirect),
                ),
            ],
          ),
        ),
      ],
      _Step.waiting => [
        _spinnerLine(l.sourcesCloudWaiting),
        const SizedBox(height: 8),
        Center(
          child: TextButton(
            key: const ValueKey('cloud-open-again'),
            onPressed: () {
              final a = _auth;
              if (a != null) unawaited(_open(a));
            },
            child: Text(l.sourcesCloudOpenAgain),
          ),
        ),
      ],
      _Step.signedIn => [
        Text(
          l.sourcesCloudSignedIn(_account),
          key: const ValueKey('cloud-account'),
        ),
        const SizedBox(height: 12),
        _folderField(l),
        const SizedBox(height: 8),
        _nameField(l),
      ],
      _Step.adding => [_spinnerLine(l.sourcesCloudAdding)],
    };
  }

  List<Widget> _megaFields(AppLocalizations l) => [
    SegmentedButton<_MegaMode>(
      key: const ValueKey('cloud-mega-mode'),
      segments: [
        ButtonSegment(
          value: _MegaMode.account,
          label: Text(l.sourcesCloudMegaAccount),
        ),
        ButtonSegment(
          value: _MegaMode.link,
          label: Text(l.sourcesCloudMegaLink),
        ),
      ],
      selected: {_megaMode},
      onSelectionChanged: _busy
          ? null
          : (s) => setState(() {
              _megaMode = s.first;
              _error = null;
            }),
    ),
    const SizedBox(height: 12),
    if (_megaMode == _MegaMode.account) ...[
      TextField(
        key: const ValueKey('cloud-mega-email'),
        controller: _email,
        keyboardType: TextInputType.emailAddress,
        autocorrect: false,
        enabled: !_needMfa,
        decoration: InputDecoration(labelText: l.sourcesCloudMegaEmail),
      ),
      TextField(
        key: const ValueKey('cloud-mega-password'),
        controller: _password,
        obscureText: true,
        enabled: !_needMfa,
        decoration: InputDecoration(labelText: l.addSourcePassword),
      ),
      if (_needMfa)
        TextField(
          key: const ValueKey('cloud-mega-mfa'),
          controller: _mfa,
          autofocus: true,
          keyboardType: TextInputType.number,
          decoration: InputDecoration(
            labelText: l.sourcesCloudMegaMfa,
            helperText: l.sourcesCloudMegaMfaHelper,
            helperMaxLines: 2,
          ),
        ),
      const SizedBox(height: 8),
      _folderField(l),
    ] else
      TextField(
        key: const ValueKey('cloud-mega-link'),
        controller: _megaLink,
        keyboardType: TextInputType.url,
        autocorrect: false,
        decoration: InputDecoration(
          labelText: l.sourcesCloudMegaLinkLabel,
          hintText: l.sourcesCloudMegaLinkHint,
          helperText: l.sourcesCloudMegaLinkHelper,
          helperMaxLines: 2,
        ),
      ),
    const SizedBox(height: 8),
    _nameField(l),
    const SizedBox(height: 12),
    Text(
      _megaMode == _MegaMode.account
          ? l.sourcesCloudMegaNote
          : l.sourcesCloudMegaLinkNote,
      style: Theme.of(context).textTheme.bodySmall,
    ),
  ];

  Widget? _action(AppLocalizations l) {
    if (_editing) {
      return FilledButton(
        key: const ValueKey('cloud-save'),
        onPressed: _busy ? null : _save,
        child: Text(l.commonSave),
      );
    }
    if (_mega) {
      final ready = _megaMode == _MegaMode.link
          ? _megaLink.text.trim().isNotEmpty
          : _email.text.trim().isNotEmpty && _password.text.isNotEmpty;
      return FilledButton(
        key: const ValueKey('cloud-mega-add'),
        onPressed: _busy || !ready ? null : _addMega,
        child: Text(
          _megaMode == _MegaMode.link
              ? l.sourcesCloudAdd
              : l.sourcesCloudMegaSignIn,
        ),
      );
    }
    final configured =
        (_provider?.configured ?? false) || _clientId.text.trim().isNotEmpty;
    return switch (_step) {
      _Step.start => FilledButton(
        key: const ValueKey('cloud-sign-in'),
        onPressed: _busy || !configured ? null : _signIn,
        child: Text(l.sourcesCloudSignIn),
      ),
      _Step.signedIn => FilledButton(
        key: const ValueKey('cloud-add'),
        onPressed: _busy ? null : _finish,
        child: Text(l.sourcesCloudAdd),
      ),
      _ => null,
    };
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final cloud = sourceTypeLabel(l, widget.type);
    final title = _editing ? l.sourcesEditTitle : cloud;
    const gap = SizedBox(height: 12);

    final fields = <Widget>[
      if (_editing)
        _nameField(l)
      else if (_mega)
        ..._megaFields(l)
      else
        ..._oauthFields(l, cloud),
      if (!_editing && !_mega) ...[
        gap,
        Text(l.sourcesCloudReadOnly, style: theme.textTheme.bodySmall),
      ],
      if (_error case final e?) ...[
        gap,
        SourceErrorView(code: e.code, detail: e.detail),
      ],
    ];

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
                  onPressed: _step == _Step.adding
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
                key: const ValueKey('cloud-cancel'),
                onPressed: () {
                  _stop();
                  Navigator.pop(context);
                },
                child: Text(l.commonCancel),
              ),
              ?_action(l),
            ],
          ),
        ),
      ],
    );
  }
}
