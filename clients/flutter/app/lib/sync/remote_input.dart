// SPDX-License-Identifier: MPL-2.0
//
// Typing on the TV from a phone (UI-027, ADR-NNNN). On a TV, any text field
// that takes the focus (search, a source's address and password, a PIN,
// a sign-in) asks the paired devices for input through pa-sync; nothing in
// the screens changes, [TvRemoteInput] follows the focus. On a phone,
// [PhoneRemoteInput] opens an input sheet by itself while the app is in
// front, or shows a local notification "Type from your phone" while it is
// in the background; what the user types reaches the TV as they type,
// sealed with the family key. The TV shows who types in an in-app notice
// card (UI-026) and its remote keeps working; a cancel on either side
// closes the other.
//
// No servers of ours: nothing arrives while the phone app is not running
// (killed by the system or by the user). See the ADR for the limits.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../notices/in_app_notices.dart';
import 'device_sync.dart';

/// Id of the TV's notice card.
const remoteInputNoticeId = 'remote-input';

/// The name of a field kind in the UI language.
String remoteInputFieldName(AppLocalizations l, RemoteInputField? f) =>
    switch (f) {
      RemoteInputField.search => l.remoteInputFieldSearch,
      RemoteInputField.address => l.remoteInputFieldAddress,
      RemoteInputField.username => l.remoteInputFieldUsername,
      RemoteInputField.password => l.remoteInputFieldPassword,
      RemoteInputField.pin => l.remoteInputFieldPin,
      RemoteInputField.text || null => l.remoteInputFieldText,
    };

/// A focused text field as remote input sees it.
class FocusedTextField {
  const FocusedTextField(this.state, this.field, this.secret, this.label);

  final EditableTextState state;
  final RemoteInputField field;

  /// The field hides what is typed (a password, a PIN).
  final bool secret;

  /// Its label or hint on screen.
  final String? label;

  /// The text field that holds the focus [node], if any.
  static FocusedTextField? of(FocusNode? node) {
    final ctx = node?.context;
    if (ctx == null) return null;
    final state = ctx.widget is EditableText
        ? (ctx as StatefulElement).state as EditableTextState
        : ctx.findAncestorStateOfType<EditableTextState>();
    if (state == null || !state.mounted) return null;
    final w = state.widget;
    if (w.readOnly) return null;
    final hints = w.autofillHints ?? const <String>[];
    final digits =
        w.keyboardType.index == TextInputType.number.index ||
        w.keyboardType.index == TextInputType.phone.index;
    final field = switch (w) {
      _ when w.obscureText && digits => RemoteInputField.pin,
      _ when w.obscureText || hints.contains(AutofillHints.password) =>
        RemoteInputField.password,
      _ when digits => RemoteInputField.pin,
      _
          when hints.contains(AutofillHints.username) ||
              w.keyboardType.index == TextInputType.emailAddress.index =>
        RemoteInputField.username,
      _ when w.keyboardType.index == TextInputType.url.index =>
        RemoteInputField.address,
      _ when w.textInputAction == TextInputAction.search =>
        RemoteInputField.search,
      _ => RemoteInputField.text,
    };
    final deco = state.context.findAncestorWidgetOfExactType<TextField>();
    final label = deco?.decoration?.labelText ?? deco?.decoration?.hintText;
    return FocusedTextField(
      state,
      field,
      w.obscureText,
      label?.trim().isEmpty ?? true ? null : label!.trim(),
    );
  }
}

class _TvSession {
  _TvSession(this.target);

  final FocusedTextField target;

  /// Null until the core answered the request.
  String? id;

  /// The device that types, once it said so.
  String? typist;
  String? typistName;
}

/// The TV side: follows the focus, asks the family for input into the
/// focused text field, writes what comes back into it.
class TvRemoteInput {
  TvRemoteInput({
    required this.sync,
    required this.notices,
    this.delay = const Duration(milliseconds: 400),
  });

  final DeviceSync sync;
  final InAppNotices notices;

  /// The focus must stay on a field this long before it asks: walking
  /// through a form with the remote does not wake every phone.
  final Duration delay;

  StreamSubscription<SyncEvent>? _sub;
  Timer? _timer;
  FocusedTextField? _waiting;
  _TvSession? _session;
  bool _started = false;

  /// The session of the focused field, if one is open (tests).
  String? get session => _session?.id;

  void start() {
    if (_started) return;
    _started = true;
    FocusManager.instance.addListener(_onFocus);
    _sub = sync.events.listen(_onEvent, onError: (_) {});
    _onFocus();
  }

  void dispose() {
    if (!_started) return;
    _started = false;
    FocusManager.instance.removeListener(_onFocus);
    unawaited(_sub?.cancel());
    _timer?.cancel();
    _end(send: true);
  }

  void _onFocus() {
    final target = FocusedTextField.of(FocusManager.instance.primaryFocus);
    final current = _session?.target.state ?? _waiting?.state;
    if (target != null && identical(target.state, current)) return;
    _timer?.cancel();
    _waiting = null;
    if (_session != null) _end(send: true);
    if (target == null) return;
    _waiting = target;
    _timer = Timer(delay, () => unawaited(_request(target)));
  }

  Future<void> _request(FocusedTextField target) async {
    if (!identical(_waiting, target) || !target.state.mounted) return;
    _waiting = null;
    final s = _TvSession(target);
    _session = s;
    String? id;
    try {
      id = await sync.requestInput(
        field: target.field,
        label: target.label,
        secret: target.secret,
        text: target.secret ? '' : target.state.textEditingValue.text,
      );
    } catch (e) {
      debugPrint('Remote input request failed: ${e.runtimeType}');
    }
    if (!identical(_session, s)) {
      // The focus moved on meanwhile.
      if (id != null) _cancel(id);
      return;
    }
    if (id == null) {
      _session = null;
      return;
    }
    s.id = id;
  }

  void _onEvent(SyncEvent e) {
    final f = e.input;
    final s = _session;
    if (f == null || s == null || s.id == null || f.session != s.id) return;
    switch (f.action) {
      case RemoteInputAction.request:
        return;
      case RemoteInputAction.open:
        _typing(s, f);
      case RemoteInputAction.text:
        _typing(s, f);
        _apply(s, f.text ?? '');
      case RemoteInputAction.submit:
        _apply(s, f.text ?? '');
        final state = s.target.state;
        _end(send: false);
        if (state.mounted) {
          state.performAction(
            state.widget.textInputAction ?? TextInputAction.done,
          );
        }
      case RemoteInputAction.cancel:
        _end(send: false);
    }
  }

  /// Shows (or updates) the card "Typing from phone: `device`".
  void _typing(_TvSession s, RemoteInputFrame f) {
    if (s.typist == f.deviceId && notices.contains(remoteInputNoticeId)) {
      return;
    }
    s.typist = f.deviceId;
    s.typistName = f.deviceName;
    final name = f.deviceName ?? f.deviceId;
    final target = s.target;
    notices.post(
      InAppNotice(
        id: remoteInputNoticeId,
        kind: InAppNoticeKind.remoteInput,
        title: (l) => l.remoteInputNoticeTitle(name),
        body: (l) => target.label ?? remoteInputFieldName(l, target.field),
        dismissLabel: (l) => l.commonCancel,
        onDismiss: () {
          if (identical(_session, s)) _end(send: true);
        },
        duration: const Duration(minutes: 5),
      ),
    );
  }

  void _apply(_TvSession s, String text) {
    final state = s.target.state;
    if (!state.mounted) return;
    state.userUpdateTextEditingValue(
      TextEditingValue(
        text: text,
        selection: TextSelection.collapsed(offset: text.length),
      ),
      SelectionChangedCause.keyboard,
    );
  }

  void _end({required bool send}) {
    final s = _session;
    _session = null;
    notices.remove(remoteInputNoticeId);
    final id = s?.id;
    if (send && id != null) _cancel(id);
  }

  void _cancel(String id) => unawaited(
    sync
        .sendInput(id, action: RemoteInputAction.cancel)
        .catchError((Object _) {}),
  );
}

/// Shows "Type from your phone" while the app is in the background.
abstract class RemoteInputNotifier {
  /// Asks for the permission to notify (when the user is looking).
  Future<void> ensurePermission();

  Future<void> show({required String title, required String body});

  Future<void> cancel();
}

/// The phone side: the request the sheet answers, and the frames it sends.
class PhoneRemoteInput extends ChangeNotifier {
  PhoneRemoteInput({required this.sync, this.notifier, required this.l10n});

  final DeviceSync sync;
  final RemoteInputNotifier? notifier;

  /// Strings for the notification in the current UI language.
  final AppLocalizations Function() l10n;

  StreamSubscription<SyncEvent>? _sub;
  RemoteInputFrame? _request;
  bool _foreground = true;
  bool _notified = false;

  /// The open request of a TV; null when there is none.
  RemoteInputFrame? get request => _request;

  /// Sends of the text, one after another (in order).
  Future<void> _queue = Future.value();
  String? _latest;
  bool _sending = false;

  void start() => _sub ??= sync.events.listen(_onEvent, onError: (_) {});

  @override
  void dispose() {
    unawaited(_sub?.cancel());
    _sub = null;
    if (_notified) unawaited(notifier?.cancel());
    super.dispose();
  }

  /// The app came to the front or went to the background.
  set foreground(bool value) {
    if (_foreground == value) return;
    _foreground = value;
    if (value && _notified) {
      _notified = false;
      unawaited(notifier?.cancel());
    }
    if (value && _request != null) notifyListeners();
  }

  bool get foreground => _foreground;

  void _onEvent(SyncEvent e) {
    final f = e.input;
    if (f == null) return;
    final open = _request;
    switch (f.action) {
      case RemoteInputAction.request:
        if (open?.session == f.session) return;
        _request = f;
        _latest = null;
        if (!_foreground) unawaited(_notify(f));
        notifyListeners();
      case RemoteInputAction.cancel || RemoteInputAction.submit:
        if (open?.session != f.session) return;
        _close();
      case RemoteInputAction.open || RemoteInputAction.text:
        // Frames of a phone go to its TV only.
        return;
    }
  }

  Future<void> _notify(RemoteInputFrame f) async {
    final n = notifier;
    if (n == null) return;
    final l = l10n();
    _notified = true;
    await n.show(
      title: l.remoteInputNotificationTitle,
      body: l.remoteInputNotificationBody(
        f.deviceName ?? l.remoteInputUnknownDevice,
        f.label ?? remoteInputFieldName(l, f.field),
      ),
    );
  }

  void _close() {
    _request = null;
    _latest = null;
    if (_notified) {
      _notified = false;
      unawaited(notifier?.cancel());
    }
    notifyListeners();
  }

  /// The sheet for [request] is up.
  Future<void> opened() async {
    final r = _request;
    if (r == null) return;
    await _send(r, RemoteInputAction.open);
    unawaited(notifier?.ensurePermission());
  }

  /// The text so far; sent in order, only the latest while one is on its
  /// way.
  void type(String text) {
    final r = _request;
    if (r == null) return;
    _latest = text;
    if (_sending) return;
    _sending = true;
    _queue = _queue.then((_) async {
      while (_latest != null && identical(_request, r)) {
        final t = _latest!;
        _latest = null;
        await _send(r, RemoteInputAction.text, t);
      }
      _sending = false;
    });
  }

  /// The user finished: the final text, and the sheet closes.
  Future<void> submit(String text) async {
    final r = _request;
    if (r == null) return;
    _close();
    await _queue;
    await _send(r, RemoteInputAction.submit, text);
  }

  /// The user closed the sheet.
  Future<void> cancel() async {
    final r = _request;
    if (r == null) return;
    _close();
    await _queue;
    await _send(r, RemoteInputAction.cancel);
  }

  Future<void> _send(
    RemoteInputFrame r,
    RemoteInputAction action, [
    String? text,
  ]) async {
    try {
      await sync.sendInput(
        r.session,
        to: r.deviceId,
        action: action,
        text: text,
        secret: r.secret,
      );
    } catch (e) {
      // Never with the text.
      debugPrint('Remote input not sent: ${e.runtimeType}');
    }
  }
}

/// The phone's input sheet for [request].
class RemoteInputSheet extends StatefulWidget {
  const RemoteInputSheet({
    super.key,
    required this.controller,
    required this.request,
  });

  final PhoneRemoteInput controller;
  final RemoteInputFrame request;

  @override
  State<RemoteInputSheet> createState() => _RemoteInputSheetState();
}

class _RemoteInputSheetState extends State<RemoteInputSheet> {
  late final _text = TextEditingController(
    text: widget.request.secret ? '' : widget.request.text ?? '',
  );
  bool _obscured = true;
  bool _closing = false;

  @override
  void initState() {
    super.initState();
    widget.controller.addListener(_onChange);
    unawaited(widget.controller.opened());
  }

  @override
  void dispose() {
    widget.controller.removeListener(_onChange);
    // Nothing of a password stays behind.
    _text.clear();
    _text.dispose();
    super.dispose();
  }

  /// The session ended elsewhere (the TV cancelled): close.
  void _onChange() {
    if (widget.controller.request?.session == widget.request.session) return;
    _close();
  }

  void _close() {
    if (_closing || !mounted) return;
    _closing = true;
    final route = ModalRoute.of(context);
    if (route == null || !route.isActive) return;
    if (route.isCurrent) {
      Navigator.of(context).pop();
    } else {
      Navigator.of(context).removeRoute(route);
    }
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final r = widget.request;
    final secret = r.secret;
    final label = r.label ?? remoteInputFieldName(l, r.field);
    final keyboard = switch (r.field) {
      RemoteInputField.pin => TextInputType.number,
      RemoteInputField.address => TextInputType.url,
      RemoteInputField.username => TextInputType.emailAddress,
      _ => TextInputType.text,
    };
    return Padding(
      padding: EdgeInsets.only(bottom: MediaQuery.viewInsetsOf(context).bottom),
      child: SafeArea(
        top: false,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(20, 8, 20, 16),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(
                children: [
                  const Icon(Icons.tv),
                  const SizedBox(width: 12),
                  Expanded(
                    child: Text(
                      l.remoteInputSheetTitle(
                        r.deviceName ?? l.remoteInputUnknownDevice,
                      ),
                      style: Theme.of(context).textTheme.titleMedium,
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 16),
              TextField(
                key: const ValueKey('remote-input-field'),
                controller: _text,
                autofocus: true,
                obscureText: secret && _obscured,
                enableSuggestions: !secret,
                autocorrect: !secret,
                keyboardType: keyboard,
                textInputAction: r.field == RemoteInputField.search
                    ? TextInputAction.search
                    : TextInputAction.done,
                inputFormatters: [
                  LengthLimitingTextInputFormatter(
                    256,
                    maxLengthEnforcement: MaxLengthEnforcement.enforced,
                  ),
                ],
                onChanged: widget.controller.type,
                onSubmitted: (t) {
                  unawaited(widget.controller.submit(t));
                },
                decoration: InputDecoration(
                  labelText: label,
                  helperText: secret ? l.remoteInputSecretHint : null,
                  suffixIcon: secret
                      ? IconButton(
                          icon: Icon(
                            _obscured ? Icons.visibility : Icons.visibility_off,
                          ),
                          onPressed: () =>
                              setState(() => _obscured = !_obscured),
                        )
                      : null,
                ),
              ),
              const SizedBox(height: 16),
              Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  TextButton(
                    key: const ValueKey('remote-input-cancel'),
                    onPressed: () => unawaited(widget.controller.cancel()),
                    child: Text(l.commonCancel),
                  ),
                  const SizedBox(width: 8),
                  FilledButton(
                    key: const ValueKey('remote-input-done'),
                    onPressed: () =>
                        unawaited(widget.controller.submit(_text.text)),
                    child: Text(l.remoteInputDone),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Wraps the app: the TV side on a TV, the phone side on a phone; nothing
/// on a desktop.
class RemoteInputHost extends StatefulWidget {
  const RemoteInputHost({
    super.key,
    required this.sync,
    required this.notices,
    required this.navigatorKey,
    required this.tv,
    required this.phone,
    this.notifier,
    required this.child,
  });

  final DeviceSync sync;
  final InAppNotices notices;

  /// The app's navigator: the phone's sheet opens on it.
  final GlobalKey<NavigatorState> navigatorKey;
  final bool tv;
  final bool phone;
  final RemoteInputNotifier? notifier;
  final Widget child;

  @override
  State<RemoteInputHost> createState() => _RemoteInputHostState();
}

class _RemoteInputHostState extends State<RemoteInputHost>
    with WidgetsBindingObserver {
  TvRemoteInput? _tv;
  PhoneRemoteInput? _phone;

  /// The session whose sheet is up.
  String? _shown;

  @override
  void initState() {
    super.initState();
    _attach();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void didUpdateWidget(RemoteInputHost old) {
    super.didUpdateWidget(old);
    if (old.sync != widget.sync ||
        old.notices != widget.notices ||
        old.tv != widget.tv ||
        old.phone != widget.phone) {
      _detach();
      _attach();
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _detach();
    super.dispose();
  }

  void _attach() {
    if (!widget.sync.available) return;
    if (widget.tv) {
      _tv = TvRemoteInput(sync: widget.sync, notices: widget.notices)..start();
    } else if (widget.phone) {
      final phone = PhoneRemoteInput(
        sync: widget.sync,
        notifier: widget.notifier,
        l10n: () => lookupAppLocalizations(
          Localizations.maybeLocaleOf(
                widget.navigatorKey.currentContext ?? context,
              ) ??
              const Locale('en'),
        ),
      );
      final state = WidgetsBinding.instance.lifecycleState;
      phone.foreground = state == null || state == AppLifecycleState.resumed;
      _phone = phone
        ..addListener(_onPhone)
        ..start();
    }
  }

  void _detach() {
    _tv?.dispose();
    _tv = null;
    _phone
      ?..removeListener(_onPhone)
      ..dispose();
    _phone = null;
    _shown = null;
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    _phone?.foreground = state == AppLifecycleState.resumed;
  }

  void _onPhone() {
    final phone = _phone;
    final r = phone?.request;
    if (phone == null || r == null) {
      _shown = null;
      return;
    }
    if (!phone.foreground || _shown == r.session) return;
    // After the frame: an old sheet closes itself first.
    WidgetsBinding.instance
      ..addPostFrameCallback((_) => _show(phone, r))
      // An idle app draws no frame by itself.
      ..ensureVisualUpdate();
  }

  void _show(PhoneRemoteInput phone, RemoteInputFrame r) {
    if (!mounted || !identical(phone, _phone) || phone.request != r) return;
    if (_shown == r.session) return;
    final ctx = widget.navigatorKey.currentContext;
    if (ctx == null) return;
    _shown = r.session;
    unawaited(
      showModalBottomSheet<void>(
        context: ctx,
        isScrollControlled: true,
        useSafeArea: true,
        builder: (_) => RemoteInputSheet(controller: phone, request: r),
      ).whenComplete(() {
        if (_shown == r.session) _shown = null;
        // Swiped away or Back: the TV stops waiting.
        if (phone.request?.session == r.session) unawaited(phone.cancel());
      }),
    );
  }

  @override
  Widget build(BuildContext context) => widget.child;
}
