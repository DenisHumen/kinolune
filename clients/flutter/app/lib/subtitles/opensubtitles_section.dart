// SPDX-License-Identifier: MPL-2.0

import 'package:material_ui/material_ui.dart';
import 'package:pa_library/pa_library.dart'
    show
        LibraryOnlineSubtitles,
        OnlineSubtitleError,
        OpenSubtitlesAccount,
        OpenSubtitlesStatus;

import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../widgets/focusable.dart';
import '../widgets/settings_tiles.dart';
import 'online_subtitles.dart';

/// «Субтитры из интернета» (PLAY-008): the OpenSubtitles login and
/// password (for the account's download limit) and an optional own API
/// key, kept in the device's secure storage. «Сохранить и проверить»
/// signs in and shows the downloads left today.
class OpenSubtitlesSection extends StatefulWidget {
  const OpenSubtitlesSection({super.key});

  @override
  State<OpenSubtitlesSection> createState() => _OpenSubtitlesSectionState();
}

class _OpenSubtitlesSectionState extends State<OpenSubtitlesSection> {
  final _login = TextEditingController();
  final _password = TextEditingController();
  final _key = TextEditingController();
  bool _loaded = false;
  bool _hidePassword = true;
  bool _hideKey = true;
  bool _checking = false;
  String? _status;
  bool _statusBad = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_loaded) return;
    _loaded = true;
    readOpenSubtitlesAccount(Services.read(context).secureStore).then((a) {
      if (!mounted) return;
      setState(() {
        _login.text = a.username ?? '';
        _password.text = a.password ?? '';
        _key.text = a.apiKey ?? '';
      });
    });
  }

  @override
  void dispose() {
    _login.dispose();
    _password.dispose();
    _key.dispose();
    super.dispose();
  }

  Future<void> _saveAndCheck() async {
    final services = Services.read(context);
    final l = context.l10n;
    final account = OpenSubtitlesAccount(
      username: _login.text,
      password: _password.text,
      apiKey: _key.text,
    );
    setState(() {
      _checking = true;
      _status = null;
    });
    await writeOpenSubtitlesAccount(services.secureStore, account);
    final library = services.library;
    String status = l.subtitlesOnlineSaved;
    var bad = false;
    if (library is LibraryOnlineSubtitles && account.hasLogin) {
      try {
        final s = await (library as LibraryOnlineSubtitles).openSubtitlesStatus(
          account,
        );
        (status, bad) = _describe(l, s);
      } catch (_) {
        (status, bad) = (l.subtitlesOnlineErrorOther, true);
      }
    } else if (!account.hasLogin) {
      status = l.subtitlesOnlineNoAccount;
    }
    if (!mounted) return;
    setState(() {
      _checking = false;
      _status = status;
      _statusBad = bad;
    });
  }

  static (String, bool) _describe(AppLocalizations l, OpenSubtitlesStatus s) {
    final error = s.error;
    if (error != null) {
      return (
        switch (error) {
          OnlineSubtitleError.login => l.subtitlesOnlineErrorLogin,
          OnlineSubtitleError.keyRejected => l.subtitlesOnlineErrorKey,
          OnlineSubtitleError.noKey => l.subtitlesOnlineErrorNoKey,
          OnlineSubtitleError.network => l.subtitlesOnlineErrorNetwork,
          _ => l.subtitlesOnlineErrorOther,
        },
        true,
      );
    }
    final q = s.quota;
    if (q?.remaining != null && q?.allowed != null) {
      final left = q!.remaining! < 0 ? 0 : q.remaining!;
      return (l.subtitlesOnlineQuota(left, q.allowed!), false);
    }
    return (l.subtitlesOnlineSignedIn, false);
  }

  Widget _secret(
    TextEditingController c,
    String label,
    bool hidden,
    VoidCallback toggle,
    Key key,
  ) {
    final l = context.l10n;
    return TextField(
      key: key,
      controller: c,
      obscureText: hidden,
      autocorrect: false,
      enableSuggestions: false,
      decoration: InputDecoration(
        labelText: label,
        suffixIcon: IconButton(
          tooltip: hidden ? l.commonShow : l.commonHide,
          onPressed: toggle,
          icon: Icon(
            hidden ? Icons.visibility_outlined : Icons.visibility_off_outlined,
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    final library = Services.of(context).library;
    final hasProjectKey =
        library is LibraryOnlineSubtitles &&
        (library as LibraryOnlineSubtitles).openSubtitlesHasProjectKey;
    return SettingsSection(
      key: const ValueKey('settings-section-opensubtitles'),
      icon: Icons.travel_explore_outlined,
      title: l.subtitlesOnlineTitle,
      subtitle: l.subtitlesOnlineSubtitle,
      children: [
        TextField(
          key: const ValueKey('opensubtitles-login'),
          controller: _login,
          autocorrect: false,
          enableSuggestions: false,
          decoration: InputDecoration(labelText: l.subtitlesOnlineLogin),
        ),
        const SizedBox(height: 10),
        _secret(
          _password,
          l.subtitlesOnlinePassword,
          _hidePassword,
          () => setState(() => _hidePassword = !_hidePassword),
          const ValueKey('opensubtitles-password'),
        ),
        const SizedBox(height: 8),
        Text(l.subtitlesOnlineAccountHint, style: theme.textTheme.labelSmall),
        const SizedBox(height: 14),
        _secret(
          _key,
          hasProjectKey
              ? l.subtitlesOnlineApiKey
              : l.subtitlesOnlineApiKeyRequired,
          _hideKey,
          () => setState(() => _hideKey = !_hideKey),
          const ValueKey('opensubtitles-key'),
        ),
        const SizedBox(height: 8),
        Text(
          hasProjectKey ? l.subtitlesOnlineKeyHint : l.subtitlesOnlineNoKeyHint,
          style: theme.textTheme.labelSmall,
        ),
        const SizedBox(height: 14),
        Row(
          children: [
            KButton(
              key: const ValueKey('opensubtitles-save'),
              label: l.subtitlesOnlineSaveCheck,
              icon: Icons.login_rounded,
              onPressed: _checking ? null : _saveAndCheck,
            ),
            const SizedBox(width: 14),
            if (_checking)
              const SizedBox(
                width: 20,
                height: 20,
                child: CircularProgressIndicator(strokeWidth: 2),
              )
            else if (_status != null)
              Expanded(
                child: Text(
                  _status!,
                  key: const ValueKey('opensubtitles-status'),
                  style: theme.textTheme.bodyMedium?.copyWith(
                    color: _statusBad ? theme.colorScheme.error : null,
                  ),
                ),
              ),
          ],
        ),
      ],
    );
  }
}
