// SPDX-License-Identifier: MPL-2.0
//
// Settings → Trakt (SYNC-011): sign in with a code (with a QR code of the
// activation page, so it works on a TV), sync now, sign out; with advanced
// settings on, the user's own Trakt client id and secret.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../widgets/focusable.dart';
import '../widgets/qr_code.dart';
import '../widgets/settings_tiles.dart';
import 'trakt_link.dart';

/// The Trakt section of the settings; nothing without the Rust core.
class TraktSettingsSection extends StatefulWidget {
  const TraktSettingsSection({super.key});

  @override
  State<TraktSettingsSection> createState() => _TraktSettingsSectionState();
}

class _TraktSettingsSectionState extends State<TraktSettingsSection> {
  bool _syncing = false;

  void _say(String text) =>
      ScaffoldMessenger.maybeOf(context)
          ?.showSnackBar(SnackBar(content: Text(text)));

  Future<void> _sync(TraktLink trakt) async {
    final l = context.l10n;
    setState(() => _syncing = true);
    try {
      final r = await trakt.sync();
      _say(
        l.traktSyncDone(
          r.watchedImported,
          r.watchedExported,
          r.ratingsImported + r.ratingsExported,
        ),
      );
    } on TraktException catch (e) {
      _say(traktErrorText(l, e.code));
    } finally {
      if (mounted) setState(() => _syncing = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final services = Services.of(context);
    final trakt = services.trakt;
    final l = context.l10n;
    if (trakt == null) return const SizedBox.shrink();
    return SettingsSection(
      key: const ValueKey('settings-section-trakt'),
      icon: Icons.sync_alt,
      title: 'Trakt', // l10n-ignore
      subtitle: l.traktSettingsSubtitle,
      children: [
        ValueListenableBuilder<String?>(
          valueListenable: trakt.account,
          builder: (context, account, _) {
            if (!trakt.configured) {
              return SettingsTile(
                key: const ValueKey('settings-trakt-not-configured'),
                icon: Icons.info_outline,
                title: l.traktNotConfigured,
                subtitle: l.traktNotConfiguredHint,
              );
            }
            if (account == null) {
              return SettingsTile(
                key: const ValueKey('settings-trakt-sign-in'),
                icon: Icons.login,
                title: l.traktSignIn,
                subtitle: l.traktSignInHint,
                onTap: () async {
                  final name = await showTraktSignIn(context);
                  if (name != null && context.mounted) unawaited(_sync(trakt));
                },
              );
            }
            return Column(
              children: [
                SettingsTile(
                  key: const ValueKey('settings-trakt-account'),
                  icon: Icons.account_circle_outlined,
                  title: account.isEmpty
                      ? l.traktSignedIn
                      : l.traktSignedInAs(account),
                  subtitle: l.traktWhatSyncs,
                  subtitleLines: 3,
                ),
                SettingsTile(
                  key: const ValueKey('settings-trakt-sync'),
                  icon: Icons.sync,
                  title: l.traktSyncNow,
                  trailing: _syncing
                      ? const SizedBox(
                          width: 20,
                          height: 20,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : null,
                  onTap: _syncing ? null : () => _sync(trakt),
                ),
                SettingsTile(
                  key: const ValueKey('settings-trakt-sign-out'),
                  icon: Icons.logout,
                  title: l.traktSignOut,
                  onTap: () async {
                    try {
                      await trakt.signOut();
                    } on TraktException catch (e) {
                      _say(traktErrorText(l, e.code));
                    }
                  },
                ),
              ],
            );
          },
        ),
        ValueListenableBuilder<bool>(
          valueListenable: services.showAdvancedSettings,
          builder: (context, advanced, _) => advanced
              ? _OwnClient(trakt: trakt, onSaved: () => setState(() {}))
              : const SizedBox.shrink(),
        ),
        Padding(
          padding: const EdgeInsets.only(top: 8),
          child: Text(
            l.traktNotEndorsed,
            style: Theme.of(context).textTheme.labelSmall,
          ),
        ),
      ],
    );
  }
}

/// Text of a Trakt error code.
String traktErrorText(AppLocalizations l, String code) => switch (code) {
  'not_configured' => l.traktNotConfigured,
  'signed_out' => l.traktSignedOutError,
  'rate_limited' => l.traktRateLimited,
  'network' => l.traktNetworkError,
  'expired' => l.traktSignInExpired,
  'denied' => l.traktSignInDenied,
  _ => l.traktFailed,
};

/// The user's own Trakt app (advanced settings).
class _OwnClient extends StatefulWidget {
  const _OwnClient({required this.trakt, required this.onSaved});

  final TraktLink trakt;
  final VoidCallback onSaved;

  @override
  State<_OwnClient> createState() => _OwnClientState();
}

class _OwnClientState extends State<_OwnClient> {
  final _id = TextEditingController();
  final _secret = TextEditingController();

  @override
  void dispose() {
    _id.dispose();
    _secret.dispose();
    super.dispose();
  }

  Future<void> _save({bool clear = false}) async {
    final l = context.l10n;
    final messenger = ScaffoldMessenger.maybeOf(context);
    if (clear) {
      _id.clear();
      _secret.clear();
    }
    try {
      await widget.trakt.setOwnClient(_id.text, _secret.text);
      messenger?.showSnackBar(
        SnackBar(
          content: Text(
            clear || _id.text.trim().isEmpty
                ? l.traktOwnClientRemoved
                : l.traktOwnClientSaved,
          ),
        ),
      );
      widget.onSaved();
    } on TraktException catch (e) {
      messenger?.showSnackBar(
        SnackBar(content: Text(traktErrorText(l, e.code))),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.only(top: 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(l.traktOwnClientTitle, style: theme.textTheme.titleSmall),
          const SizedBox(height: 4),
          Text(
            widget.trakt.ownClient
                ? l.traktOwnClientInUse
                : l.traktOwnClientHint,
            style: theme.textTheme.bodySmall,
          ),
          const SizedBox(height: 8),
          TextField(
            key: const ValueKey('settings-trakt-client-id'),
            controller: _id,
            decoration: InputDecoration(labelText: l.traktOwnClientId),
          ),
          const SizedBox(height: 8),
          TextField(
            key: const ValueKey('settings-trakt-client-secret'),
            controller: _secret,
            obscureText: true,
            decoration: InputDecoration(labelText: l.traktOwnClientSecret),
          ),
          const SizedBox(height: 8),
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              if (widget.trakt.ownClient)
                KButton(
                  label: l.traktOwnClientReset,
                  onPressed: () => _save(clear: true),
                ),
              const SizedBox(width: 8),
              KButton(label: l.commonSave, primary: true, onPressed: _save),
            ],
          ),
        ],
      ),
    );
  }
}

/// Device code sign-in: the code, a QR code of the activation page with
/// the code filled in, polling until the user confirms. Returns the user
/// name ('' if unknown) or null.
Future<String?> showTraktSignIn(BuildContext context) =>
    showDialog<String>(context: context, builder: (_) => const _SignInDialog());

class _SignInDialog extends StatefulWidget {
  const _SignInDialog();

  @override
  State<_SignInDialog> createState() => _SignInDialogState();
}

class _SignInDialogState extends State<_SignInDialog> {
  TraktDeviceLogin? _login;
  String? _error;
  Timer? _timer;
  TraktLink? _trakt;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _start());
  }

  Future<void> _start() async {
    final trakt = _trakt = Services.read(context).trakt;
    if (trakt == null) return;
    try {
      final login = await trakt.startLogin();
      if (!mounted) return;
      setState(() => _login = login);
      _schedule(login.interval);
    } on TraktException catch (e) {
      if (mounted) setState(() => _error = e.code);
    }
  }

  void _schedule(Duration interval) {
    _timer?.cancel();
    _timer = Timer(interval, () => _poll(interval));
  }

  Future<void> _poll(Duration interval) async {
    final trakt = _trakt;
    if (trakt == null || !mounted) return;
    try {
      final name = await trakt.pollLogin();
      if (!mounted) return;
      if (name != null) {
        Navigator.of(context).pop(name);
        return;
      }
      _schedule(interval);
    } on TraktException catch (e) {
      if (!mounted) return;
      // A network hiccup: keep polling; an expired or refused code ends.
      if (e.code == 'network' || e.code == 'rate_limited') {
        _schedule(interval * 2);
      } else {
        setState(() => _error = e.code);
      }
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    _trakt?.cancelLogin();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final login = _login;
    final error = _error;
    return AlertDialog(
      key: const ValueKey('trakt-sign-in'),
      title: Text(l.traktSignIn),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (error != null)
              Text(traktErrorText(l, error))
            else if (login == null)
              const Padding(
                padding: EdgeInsets.all(24),
                child: CircularProgressIndicator(),
              )
            else ...[
              Text(
                l.traktSignInSteps(
                  Uri.parse(login.verificationUrl).host +
                      Uri.parse(login.verificationUrl).path,
                ),
                style: theme.textTheme.bodyMedium,
              ),
              const SizedBox(height: 12),
              SelectableText(
                login.userCode,
                key: const ValueKey('trakt-user-code'),
                style: theme.textTheme.headlineMedium?.copyWith(
                  letterSpacing: 4,
                  fontWeight: FontWeight.w700,
                ),
              ),
              const SizedBox(height: 12),
              QrCodeView(data: login.qrUrl, size: 180),
              const SizedBox(height: 12),
              Text(l.traktSignInWaiting, style: theme.textTheme.bodySmall),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          autofocus: true,
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.commonCancel),
        ),
      ],
    );
  }
}
