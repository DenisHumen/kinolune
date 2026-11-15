// SPDX-License-Identifier: MPL-2.0
//
// Adding a Jellyfin server (NET-014): the address (plus other addresses
// tried when it does not answer), then sign-in with a password or Quick
// Connect. Editing changes the name and addresses; the sign-in stays.

import 'dart:async';
import 'dart:io' show Platform;

import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../sources/jellyfin_link.dart';
import '../sources/source_address.dart';
import '../sources/source_manager.dart';
import '../sources/source_text.dart';
import 'add_source_dialog.dart' show SourceErrorView;

/// The server address as typed, normalized: `http://` added when no scheme
/// is given, no trailing slash.
AddressResult jellyfinAddress(String text) {
  var a = text.trim();
  if (a.isEmpty) return const AddressResult.fail(SourceFormError.urlEmpty);
  if (!a.contains('://')) a = 'http://$a'; // l10n-ignore
  final uri = Uri.tryParse(a);
  if (uri == null || !(uri.scheme == 'http' || uri.scheme == 'https')) {
    return const AddressResult.fail(SourceFormError.urlScheme);
  }
  if (uri.userInfo.isNotEmpty) {
    return const AddressResult.fail(SourceFormError.credentialsInAddress);
  }
  if (uri.host.isEmpty) {
    return const AddressResult.fail(SourceFormError.urlEmpty);
  }
  while (a.endsWith('/')) {
    a = a.substring(0, a.length - 1);
  }
  return AddressResult.ok(a);
}

/// Other addresses, one per line; malformed lines are dropped.
List<String> jellyfinAltAddresses(String text) => [
  for (final line in text.split(RegExp(r'[\n,]')))
    if (jellyfinAddress(line).address case final a?) a,
];

/// How the device is named in the server's dashboard.
String jellyfinDeviceName() {
  final os = Platform.operatingSystem;
  return os.isEmpty ? 'Kinolune' : '${os[0].toUpperCase()}${os.substring(1)}';
}

enum _Mode { password, quickConnect }

/// The add or edit form of a Jellyfin source; pops the saved source.
class JellyfinSourceForm extends StatefulWidget {
  const JellyfinSourceForm({
    super.key,
    this.initial,
    this.onBack,
    this.pollInterval = const Duration(seconds: 2),
  });

  final ManagedSource? initial;
  final VoidCallback? onBack;

  /// How often a Quick Connect code is checked.
  final Duration pollInterval;

  @override
  State<JellyfinSourceForm> createState() => _JellyfinSourceFormState();
}

class _JellyfinSourceFormState extends State<JellyfinSourceForm> {
  final _address = TextEditingController();
  final _alt = TextEditingController();
  final _user = TextEditingController();
  final _password = TextEditingController();
  final _name = TextEditingController();
  _Mode _mode = _Mode.password;
  bool _submitted = false;
  bool _busy = false;
  SourceException? _error;
  QuickConnectTicket? _ticket;
  Timer? _poll;
  JellyfinLink? _link;

  bool get _editing => widget.initial != null;

  @override
  void initState() {
    super.initState();
    final s = widget.initial;
    if (s != null) {
      _address.text = s.address;
      _alt.text = s.altAddresses.join('\n');
      _name.text = s.name;
    }
    _address.addListener(() => setState(() {}));
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _link = Services.read(context).jellyfin;
  }

  @override
  void dispose() {
    _cancelQuickConnect();
    for (final c in [_address, _alt, _user, _password, _name]) {
      c.dispose();
    }
    super.dispose();
  }

  void _cancelQuickConnect() {
    _poll?.cancel();
    _poll = null;
    final t = _ticket;
    _ticket = null;
    if (t != null) _link?.quickConnectCancel(t.handle);
  }

  List<String> _addresses() => [
    jellyfinAddress(_address.text).address!,
    ...jellyfinAltAddresses(_alt.text),
  ];

  String? get _nameOrNull =>
      _name.text.trim().isEmpty ? null : _name.text.trim();

  void _fail(Object e) {
    if (!mounted) return;
    setState(() {
      _busy = false;
      _error = e is SourceException
          ? e
          : SourceException(SourceErrorCode.other, '$e');
    });
  }

  Future<void> _done(ManagedSource added) async {
    final services = Services.read(context);
    final saved = await services.addedMediaServer(added);
    if (mounted) Navigator.of(context).pop(saved);
  }

  Future<void> _submit() async {
    setState(() {
      _submitted = true;
      _error = null;
    });
    if (jellyfinAddress(_address.text).error != null) return;
    final services = Services.read(context);
    if (_editing) {
      final s = widget.initial!;
      setState(() => _busy = true);
      try {
        final saved = await services.updateSource(
          s.id,
          SourceDraft(
            type: SourceType.jellyfin,
            name: _nameOrNull ?? s.name,
            address: jellyfinAddress(_address.text).address!,
            username: s.username,
            altAddresses: jellyfinAltAddresses(_alt.text),
            scanIntoLibrary: s.scanIntoLibrary,
            autoScan: s.autoScan,
          ),
        );
        if (mounted) Navigator.of(context).pop(saved);
      } catch (e) {
        _fail(e);
      }
      return;
    }
    final link = _link;
    if (link == null) {
      _fail(const SourceException(SourceErrorCode.unsupported));
      return;
    }
    setState(() => _busy = true);
    try {
      switch (_mode) {
        case _Mode.password:
          await _done(
            await link.signIn(
              addresses: _addresses(),
              username: _user.text.trim(),
              password: _password.text,
              deviceName: jellyfinDeviceName(),
              name: _nameOrNull,
            ),
          );
        case _Mode.quickConnect:
          final ticket = await link.quickConnectStart(
            addresses: _addresses(),
            deviceName: jellyfinDeviceName(),
          );
          if (!mounted) {
            link.quickConnectCancel(ticket.handle);
            return;
          }
          setState(() => _ticket = ticket);
          _poll = Timer.periodic(widget.pollInterval, (_) => _check(link));
      }
    } catch (e) {
      _fail(e);
    }
  }

  bool _checking = false;

  Future<void> _check(JellyfinLink link) async {
    final t = _ticket;
    if (t == null || _checking) return;
    _checking = true;
    try {
      if (!await link.quickConnectPoll(t.handle)) return;
      _poll?.cancel();
      _poll = null;
      _ticket = null;
      await _done(await link.quickConnectFinish(t.handle, name: _nameOrNull));
    } catch (e) {
      _cancelQuickConnect();
      _fail(e);
    } finally {
      _checking = false;
    }
  }

  /// The server refused to start Quick Connect: it is switched off there.
  bool _quickConnectOff(SourceException e) =>
      _mode == _Mode.quickConnect &&
      _ticket == null &&
      (e.code == SourceErrorCode.authFailed ||
          e.code == SourceErrorCode.permissionDenied);

  String? _addressError() {
    final e = jellyfinAddress(_address.text).error;
    if (!_submitted || e == null) return null;
    return sourceFormErrorText(context.l10n, e);
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final ticket = _ticket;
    const gap = SizedBox(height: 12);
    final title = _editing
        ? l.sourcesEditTitle
        : '${l.addSourceTitle}: ${sourceTypeLabel(l, SourceType.jellyfin)}';

    final fields = <Widget>[
      TextField(
        key: const ValueKey('jellyfin-address'),
        controller: _address,
        enabled: ticket == null,
        keyboardType: TextInputType.url,
        autocorrect: false,
        decoration: InputDecoration(
          labelText: l.sourcesFormUrl,
          hintText: l.jellyfinFormAddressHint,
          errorText: _addressError(),
          errorMaxLines: 3,
        ),
      ),
      gap,
      TextField(
        key: const ValueKey('jellyfin-alt-addresses'),
        controller: _alt,
        enabled: ticket == null,
        keyboardType: TextInputType.url,
        autocorrect: false,
        minLines: 1,
        maxLines: 3,
        decoration: InputDecoration(
          labelText: l.jellyfinFormAltAddresses,
          helperText: l.jellyfinFormAltAddressesHelper,
          helperMaxLines: 3,
        ),
      ),
      if (!_editing) ...[
        gap,
        SegmentedButton<_Mode>(
          key: const ValueKey('jellyfin-mode'),
          segments: [
            ButtonSegment(
              value: _Mode.password,
              label: Text(l.jellyfinFormModePassword),
              icon: const Icon(Icons.password),
            ),
            ButtonSegment(
              value: _Mode.quickConnect,
              label: Text(l.jellyfinFormModeQuickConnect),
              icon: const Icon(Icons.pin_outlined),
            ),
          ],
          selected: {_mode},
          onSelectionChanged: _busy
              ? null
              : (m) => setState(() {
                  _mode = m.first;
                  _error = null;
                }),
        ),
        if (_mode == _Mode.password) ...[
          gap,
          TextField(
            key: const ValueKey('jellyfin-user'),
            controller: _user,
            autocorrect: false,
            decoration: InputDecoration(labelText: l.addSourceUsername),
          ),
          gap,
          TextField(
            key: const ValueKey('jellyfin-password'),
            controller: _password,
            obscureText: true,
            decoration: InputDecoration(labelText: l.addSourcePassword),
            onSubmitted: (_) => _busy ? null : _submit(),
          ),
        ],
        if (ticket != null) ...[
          gap,
          Center(
            child: SelectableText(
              ticket.code,
              key: const ValueKey('jellyfin-code'),
              style: theme.textTheme.displaySmall?.copyWith(letterSpacing: 6),
            ),
          ),
          const SizedBox(height: 8),
          Text(l.jellyfinQuickConnectHow, textAlign: TextAlign.center),
          const SizedBox(height: 8),
          Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              const SizedBox.square(
                dimension: 16,
                child: CircularProgressIndicator(strokeWidth: 2),
              ),
              const SizedBox(width: 8),
              Text(l.jellyfinQuickConnectWaiting),
            ],
          ),
        ],
        const SizedBox(height: 8),
        Text(l.jellyfinFormTokenNote, style: theme.textTheme.bodySmall),
      ],
      gap,
      TextField(
        key: const ValueKey('jellyfin-name'),
        controller: _name,
        decoration: InputDecoration(labelText: l.addSourceDisplayName),
      ),
      if (_error case final e?) ...[
        gap,
        if (_quickConnectOff(e))
          Text(
            l.jellyfinQuickConnectUnavailable,
            key: const ValueKey('jellyfin-quick-connect-off'),
            style: theme.textTheme.bodyMedium?.copyWith(
              color: theme.colorScheme.error,
            ),
          )
        else
          SourceErrorView(code: e.code, detail: e.detail),
      ],
    ];

    final submitLabel = _editing
        ? l.commonSave
        : (_mode == _Mode.password
              ? l.jellyfinFormSignIn
              : l.jellyfinFormGetCode);
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: EdgeInsets.fromLTRB(widget.onBack == null ? 24 : 8, 20, 24, 8),
          child: Row(
            children: [
              if (widget.onBack != null)
                IconButton(
                  tooltip: l.commonBack,
                  onPressed: _busy && ticket == null
                      ? null
                      : () {
                          _cancelQuickConnect();
                          widget.onBack!();
                        },
                  icon: const Icon(Icons.arrow_back),
                ),
              Expanded(child: Text(title, style: theme.textTheme.headlineSmall)),
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
                onPressed: () {
                  _cancelQuickConnect();
                  Navigator.pop(context);
                },
                child: Text(l.commonCancel),
              ),
              if (ticket == null)
                FilledButton(
                  key: const ValueKey('jellyfin-submit'),
                  onPressed: _busy ? null : _submit,
                  child: Text(submitLabel),
                ),
            ],
          ),
        ),
      ],
    );
  }
}
