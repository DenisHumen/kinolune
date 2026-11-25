// SPDX-License-Identifier: MPL-2.0
//
// The network icon in Home's top bar and its panel (DIST-032): signal,
// "no network" and "sign-in needed" (captive portal) on the icon; in the
// panel, Wi-Fi on/off, the networks around with signal and lock, joining
// with or without a password, a hidden network, "Forget", auto-connect,
// the cable, the address and "Проверить интернет". Keyboard and mouse
// first; every control is a focus stop, so a remote works as well. The
// password field is an ordinary obscured field: on a TV it can be typed
// from the phone (UI-027).

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../services/external_links.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import 'network_backend.dart';
import 'network_controller.dart';

/// The icon and its label for [s].
({IconData icon, String label}) networkStatus(
  AppLocalizations l,
  NetworkSnapshot s,
) {
  final active = s.active;
  if (s.connectivity == NetConnectivity.portal) {
    return (icon: Icons.wifi_password_rounded, label: l.networkStatusPortal);
  }
  if (active != null) {
    final icon = s.connectivity == NetConnectivity.limited
        ? Icons.wifi_find_rounded
        : wifiSignalIcon(active.strength);
    return (
      icon: icon,
      label: s.connectivity == NetConnectivity.limited
          ? '${l.networkStatusWifi(active.ssid)} · ${l.networkStatusLimited}'
          : l.networkStatusWifi(active.ssid),
    );
  }
  if (s.connecting case final ssid?) {
    return (icon: Icons.wifi_find_rounded, label: l.networkStatusConnecting(ssid));
  }
  if (s.wired.connected) {
    return (
      icon: Icons.settings_ethernet_rounded,
      label: l.networkStatusWired,
    );
  }
  return (icon: Icons.wifi_off_rounded, label: l.networkStatusNone);
}

IconData wifiSignalIcon(int strength) => switch (strength) {
  < 30 => Icons.wifi_1_bar_rounded,
  < 60 => Icons.wifi_2_bar_rounded,
  _ => Icons.wifi_rounded,
};

/// The network icon for Home's top bar; nothing without a controller or
/// when the system has no NetworkManager or no Wi-Fi adapter.
class NetworkStatusButton extends StatelessWidget {
  const NetworkStatusButton({super.key, this.size = 22});

  final double size;

  @override
  Widget build(BuildContext context) {
    final net = Services.of(context).network;
    if (net == null) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: net,
      builder: (context, _) {
        if (!net.visible) return const SizedBox.shrink();
        final st = networkStatus(context.l10n, net.snapshot);
        final warn =
            net.snapshot.connectivity == NetConnectivity.portal ||
            net.snapshot.connectivity == NetConnectivity.limited ||
            (net.snapshot.active == null && !net.snapshot.wired.connected);
        return Tooltip(
          message: st.label,
          child: KFocusable(
            key: const ValueKey('network-icon'),
            onActivate: () => showNetworkPanel(context, net),
            focusScale: 1.08,
            light: false,
            radius: 999,
            semanticLabel: '${context.l10n.networkTitle}: ${st.label}',
            builder: (context, focused) => AnimatedContainer(
              duration: KMotion.focus,
              padding: const EdgeInsets.all(7),
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                color: focused
                    ? KColors.focusFill
                    : Colors.black.withValues(alpha: .35),
              ),
              child: Icon(
                st.icon,
                key: ValueKey('network-icon-${st.icon.codePoint}'),
                size: size,
                color: focused
                    ? KColors.focusLabel
                    : (warn ? KColors.statusWarning : Colors.white),
              ),
            ),
          ),
        );
      },
    );
  }
}

/// Opens the panel: a sheet on a phone, a panel by the top-right corner on
/// a computer, a centred panel on a TV.
Future<void> showNetworkPanel(BuildContext context, NetworkController net) {
  final d = KDensity.of(context);
  final narrow = MediaQuery.sizeOf(context).width < 600;
  net.panelOpened();
  final Future<void> f;
  if (d.isPhone || narrow) {
    f = showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      useSafeArea: true,
      backgroundColor: KColors.bgElevated,
      constraints: BoxConstraints(
        maxHeight: MediaQuery.sizeOf(context).height * .88,
      ),
      builder: (_) => NetworkPanel(controller: net),
    );
  } else {
    f = showDialog<void>(
      context: context,
      builder: (_) => Dialog(
        alignment: d.isTv ? Alignment.center : Alignment.topRight,
        insetPadding: d.isTv
            ? const EdgeInsets.all(48)
            : const EdgeInsets.fromLTRB(24, 64, 24, 24),
        backgroundColor: KColors.bgElevated,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(d.panelRadius * .75),
          side: BorderSide(color: KColors.stroke),
        ),
        child: ConstrainedBox(
          constraints: BoxConstraints(
            maxWidth: d.isTv ? 620 : 440,
            maxHeight: MediaQuery.sizeOf(context).height - 96,
          ),
          child: NetworkPanel(controller: net),
        ),
      ),
    );
  }
  return f.whenComplete(net.panelClosed);
}

class NetworkPanel extends StatefulWidget {
  const NetworkPanel({super.key, required this.controller});

  final NetworkController controller;

  @override
  State<NetworkPanel> createState() => _NetworkPanelState();
}

class _NetworkPanelState extends State<NetworkPanel> {
  NetworkController get net => widget.controller;

  /// The last outcome to show under the status: (text, is a problem).
  (String, bool)? _message;
  NetConnectivity? _checked;

  void _say(String text, {bool problem = false}) {
    if (mounted) setState(() => _message = (text, problem));
  }

  Future<void> _join(WifiNetwork n) async {
    final l = context.l10n;
    if (n.security == WifiSecurity.enterprise) {
      _say(l.networkUnsupported, problem: true);
      return;
    }
    String? password;
    if (n.security.needsPassword && !n.saved) {
      password = await _askPassword(n.ssid);
      if (password == null) return;
    }
    var r = await net.connect(n, password: password);
    // A saved password the network no longer takes: ask a new one.
    while (r == ConnectResult.wrongPassword && mounted) {
      password = await _askPassword(n.ssid, wrong: true);
      if (password == null) return;
      r = await net.connect(n, password: password);
    }
    _report(r, n.ssid);
  }

  void _report(ConnectResult r, String ssid) {
    if (!mounted) return;
    final l = context.l10n;
    switch (r) {
      case ConnectResult.ok:
        _say(l.networkConnectedTo(ssid));
      case ConnectResult.wrongPassword:
        _say(l.networkWrongPassword, problem: true);
      case ConnectResult.notFound:
        _say(l.networkNotFound(ssid), problem: true);
      case ConnectResult.unsupported:
        _say(l.networkUnsupported, problem: true);
      case ConnectResult.failed:
        _say(l.networkFailed(ssid), problem: true);
    }
  }

  Future<String?> _askPassword(String ssid, {bool wrong = false}) =>
      showDialog<String>(
        context: context,
        builder: (_) => _PasswordDialog(ssid: ssid, wrong: wrong),
      );

  Future<void> _hidden() async {
    final r = await showDialog<_HiddenRequest>(
      context: context,
      builder: (_) => const _HiddenDialog(),
    );
    if (r == null) return;
    final res = await net.connectHidden(
      r.ssid,
      security: r.security,
      password: r.password,
    );
    _report(res, r.ssid);
  }

  Future<void> _details(WifiNetwork n) async {
    final forget = await showDialog<bool>(
      context: context,
      builder: (_) => _DetailsDialog(net: net, ssid: n.ssid),
    );
    if (forget == true && mounted) {
      await net.forget(n.ssid);
      _say(context.l10n.networkForgotten(n.ssid));
    }
  }

  Future<void> _check() async {
    final c = await net.checkInternet();
    if (mounted) setState(() => _checked = c);
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final theme = Theme.of(context);
    return ListenableBuilder(
      listenable: net,
      builder: (context, _) {
        final s = net.snapshot;
        final st = networkStatus(l, s);
        return FocusTraversalGroup(
          child: Column(
            key: const ValueKey('network-panel'),
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(20, 16, 8, 4),
                child: Row(
                  children: [
                    Expanded(
                      child: Semantics(
                        header: true,
                        child: Text(
                          l.networkTitle,
                          style: theme.textTheme.titleLarge,
                        ),
                      ),
                    ),
                    IconButton(
                      key: const ValueKey('network-close'),
                      tooltip: l.networkClose,
                      onPressed: () => Navigator.of(context).maybePop(),
                      icon: const Icon(Icons.close_rounded),
                    ),
                  ],
                ),
              ),
              Flexible(
                child: ListView(
                  shrinkWrap: true,
                  padding: const EdgeInsets.fromLTRB(8, 0, 8, 16),
                  children: [
                    _Row(
                      key: const ValueKey('network-status'),
                      icon: st.icon,
                      title: st.label,
                      subtitle: switch (s.address) {
                        final a? => l.networkAddress(a),
                        null => null,
                      },
                    ),
                    if (s.wired.present)
                      _Row(
                        key: const ValueKey('network-wired'),
                        icon: Icons.settings_ethernet_rounded,
                        title: l.networkWired,
                        subtitle: s.wired.connected
                            ? [
                                l.networkWiredConnected,
                                if (s.wired.address case final a?)
                                  l.networkAddress(a),
                              ].join(' · ')
                            : (s.wired.cable
                                  ? l.networkWiredNoAddress
                                  : l.networkWiredNoCable),
                      ),
                    _Row(
                      key: const ValueKey('network-check'),
                      icon: Icons.public_rounded,
                      title: net.checking
                          ? l.networkChecking
                          : l.networkCheckInternet,
                      subtitle: switch (_checked) {
                        null => null,
                        final c => _internetText(l, c),
                      },
                      onTap: net.checking ? null : _check,
                    ),
                    if (s.connectivity == NetConnectivity.portal &&
                        s.portalUrl != null)
                      _Row(
                        key: const ValueKey('network-portal'),
                        icon: Icons.open_in_browser_rounded,
                        title: l.networkOpenPortal,
                        subtitle: l.networkInternetPortal,
                        onTap: () => openExternalLink(Uri.parse(s.portalUrl!)),
                      ),
                    if (_message case (final text, final problem))
                      Padding(
                        padding: const EdgeInsets.fromLTRB(12, 6, 12, 2),
                        child: Text(
                          text,
                          key: const ValueKey('network-message'),
                          style: theme.textTheme.bodyMedium?.copyWith(
                            color: problem
                                ? KColors.statusWarning
                                : KColors.statusOk,
                          ),
                        ),
                      ),
                    const SizedBox(height: 10),
                    _Row(
                      key: const ValueKey('network-wifi-switch'),
                      icon: s.wifiEnabled
                          ? Icons.wifi_rounded
                          : Icons.wifi_off_rounded,
                      title: s.wifiEnabled ? l.networkWifi : l.networkWifiOff,
                      onTap: () => net.setWifiEnabled(!s.wifiEnabled),
                      toggled: s.wifiEnabled,
                      trailing: ExcludeFocus(
                        child: ExcludeSemantics(
                          child: Switch(
                            value: s.wifiEnabled,
                            onChanged: net.setWifiEnabled,
                          ),
                        ),
                      ),
                    ),
                    if (s.wifiEnabled) ...[
                      Padding(
                        padding: const EdgeInsets.fromLTRB(12, 10, 0, 2),
                        child: Row(
                          children: [
                            Expanded(
                              child: Text(
                                net.scanning
                                    ? l.networkScanning
                                    : l.networkNetworks,
                                style: theme.textTheme.labelLarge?.copyWith(
                                  color: KColors.textSecondary,
                                ),
                              ),
                            ),
                            IconButton(
                              key: const ValueKey('network-scan'),
                              tooltip: l.networkScan,
                              onPressed: net.scanning ? null : net.scan,
                              icon: const Icon(Icons.refresh_rounded),
                            ),
                          ],
                        ),
                      ),
                      if (s.networks.isEmpty)
                        Padding(
                          padding: const EdgeInsets.all(12),
                          child: Text(
                            l.networkNoNetworks,
                            style: theme.textTheme.bodySmall,
                          ),
                        ),
                      for (final n in s.networks)
                        _NetworkTile(
                          network: n,
                          joining: net.joining == n.ssid,
                          onJoin: () => _join(n),
                          onDetails: n.saved ? () => _details(n) : null,
                        ),
                      _Row(
                        key: const ValueKey('network-hidden'),
                        icon: Icons.add_rounded,
                        title: l.networkHidden,
                        onTap: _hidden,
                      ),
                    ],
                  ],
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}

String _internetText(AppLocalizations l, NetConnectivity c) => switch (c) {
  NetConnectivity.full => l.networkInternetFull,
  NetConnectivity.none => l.networkInternetNone,
  NetConnectivity.limited => l.networkInternetLimited,
  NetConnectivity.portal => l.networkInternetPortal,
  NetConnectivity.unknown => l.networkInternetUnknown,
};

/// A focusable row of the panel.
class _Row extends StatelessWidget {
  const _Row({
    super.key,
    required this.icon,
    required this.title,
    this.subtitle,
    this.onTap,
    this.trailing,
    this.toggled,
  });

  final IconData icon;
  final String title;
  final String? subtitle;
  final VoidCallback? onTap;
  final Widget? trailing;
  final bool? toggled;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final body = Padding(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
      child: Row(
        children: [
          Icon(icon, color: KColors.textSecondary),
          const SizedBox(width: 14),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(title, style: theme.textTheme.titleMedium),
                if (subtitle != null)
                  Text(
                    subtitle!,
                    style: theme.textTheme.labelSmall,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                  ),
              ],
            ),
          ),
          ?trailing,
        ],
      ),
    );
    if (onTap == null) return Semantics(container: true, child: body);
    return Semantics(
      toggled: toggled,
      child: KFocusable(
        onActivate: onTap,
        focusScale: 1.0,
        light: false,
        radius: 12,
        semanticLabel: title,
        builder: (context, focused) => AnimatedContainer(
          duration: KMotion.focus,
          decoration: BoxDecoration(
            color: focused
                ? Colors.white.withValues(alpha: .09)
                : Colors.transparent,
            borderRadius: BorderRadius.circular(12),
          ),
          child: body,
        ),
      ),
    );
  }
}

class _NetworkTile extends StatelessWidget {
  const _NetworkTile({
    required this.network,
    required this.joining,
    required this.onJoin,
    this.onDetails,
  });

  final WifiNetwork network;
  final bool joining;
  final VoidCallback onJoin;
  final VoidCallback? onDetails;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final n = network;
    final kind = switch (n.security) {
      WifiSecurity.open => l.networkOpen,
      WifiSecurity.enterprise => l.networkEnterprise,
      _ => l.networkSecured,
    };
    final state = joining
        ? l.networkConnecting
        : n.active
        ? l.networkConnected
        : n.saved
        ? l.networkSaved
        : null;
    return Row(
      children: [
        Expanded(
          child: _Row(
            key: ValueKey('network-ap-${n.ssid}'),
            icon: wifiSignalIcon(n.strength),
            title: n.ssid,
            subtitle: [?state, kind, l.networkSignal(n.strength)].join(' · '),
            onTap: n.active || joining ? (onDetails ?? () {}) : onJoin,
            trailing: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (joining)
                  const SizedBox.square(
                    dimension: 18,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                else if (n.active)
                  Icon(Icons.check_rounded, color: KColors.statusOk),
                if (n.security != WifiSecurity.open) ...[
                  const SizedBox(width: 6),
                  Icon(
                    Icons.lock_rounded,
                    size: 16,
                    color: KColors.textTertiary,
                  ),
                ],
              ],
            ),
          ),
        ),
        if (onDetails != null)
          IconButton(
            key: ValueKey('network-details-${n.ssid}'),
            tooltip: l.networkDetails,
            onPressed: onDetails,
            icon: const Icon(Icons.tune_rounded),
          ),
      ],
    );
  }
}

/// Asks the password of [ssid]; it goes from here to the system only.
class _PasswordDialog extends StatefulWidget {
  const _PasswordDialog({required this.ssid, this.wrong = false});

  final String ssid;
  final bool wrong;

  @override
  State<_PasswordDialog> createState() => _PasswordDialogState();
}

class _PasswordDialogState extends State<_PasswordDialog> {
  final _text = TextEditingController();
  bool _show = false;

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  void _done() {
    if (_text.text.isEmpty) return;
    Navigator.of(context).pop(_text.text);
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return AlertDialog(
      title: Text(l.networkPasswordTitle(widget.ssid)),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          TextField(
            key: const ValueKey('network-password'),
            controller: _text,
            autofocus: true,
            obscureText: !_show,
            enableSuggestions: false,
            autocorrect: false,
            autofillHints: const [AutofillHints.password],
            textInputAction: TextInputAction.done,
            onSubmitted: (_) => _done(),
            decoration: InputDecoration(
              labelText: l.networkPassword,
              errorText: widget.wrong ? l.networkWrongPassword : null,
              suffixIcon: IconButton(
                tooltip: _show ? l.commonHide : l.commonShow,
                onPressed: () => setState(() => _show = !_show),
                icon: Icon(
                  _show
                      ? Icons.visibility_off_rounded
                      : Icons.visibility_rounded,
                ),
              ),
            ),
          ),
          const SizedBox(height: 10),
          Text(
            l.networkPasswordNote,
            style: Theme.of(context).textTheme.bodySmall,
          ),
        ],
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.commonCancel),
        ),
        FilledButton(
          key: const ValueKey('network-password-connect'),
          onPressed: _done,
          child: Text(l.networkConnect),
        ),
      ],
    );
  }
}

class _HiddenRequest {
  const _HiddenRequest(this.ssid, this.security, this.password);

  final String ssid;
  final WifiSecurity security;
  final String? password;
}

class _HiddenDialog extends StatefulWidget {
  const _HiddenDialog();

  @override
  State<_HiddenDialog> createState() => _HiddenDialogState();
}

class _HiddenDialogState extends State<_HiddenDialog> {
  final _ssid = TextEditingController();
  final _password = TextEditingController();
  WifiSecurity _security = WifiSecurity.wpaPersonal;

  @override
  void dispose() {
    _ssid.dispose();
    _password.dispose();
    super.dispose();
  }

  void _done() {
    final ssid = _ssid.text.trim();
    if (ssid.isEmpty) return;
    final needs = _security.needsPassword;
    if (needs && _password.text.isEmpty) return;
    Navigator.of(
      context,
    ).pop(_HiddenRequest(ssid, _security, needs ? _password.text : null));
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    String label(WifiSecurity s) => switch (s) {
      WifiSecurity.open => l.networkSecurityOpen,
      WifiSecurity.wpa3 => l.networkSecurityWpa3,
      _ => l.networkSecurityWpa,
    };
    return AlertDialog(
      title: Text(l.networkHiddenTitle),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            TextField(
              key: const ValueKey('network-hidden-ssid'),
              controller: _ssid,
              autofocus: true,
              autocorrect: false,
              textInputAction: TextInputAction.next,
              decoration: InputDecoration(labelText: l.networkHiddenName),
            ),
            const SizedBox(height: 14),
            Text(l.networkSecurity, style: Theme.of(context).textTheme.labelLarge),
            const SizedBox(height: 6),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                for (final s in const [
                  WifiSecurity.wpaPersonal,
                  WifiSecurity.wpa3,
                  WifiSecurity.open,
                ])
                  ChoiceChip(
                    key: ValueKey('network-hidden-security-${s.name}'),
                    label: Text(label(s)),
                    selected: _security == s,
                    onSelected: (_) => setState(() => _security = s),
                  ),
              ],
            ),
            if (_security.needsPassword) ...[
              const SizedBox(height: 14),
              TextField(
                key: const ValueKey('network-hidden-password'),
                controller: _password,
                obscureText: true,
                enableSuggestions: false,
                autocorrect: false,
                autofillHints: const [AutofillHints.password],
                textInputAction: TextInputAction.done,
                onSubmitted: (_) => _done(),
                decoration: InputDecoration(labelText: l.networkPassword),
              ),
              const SizedBox(height: 10),
              Text(
                l.networkPasswordNote,
                style: Theme.of(context).textTheme.bodySmall,
              ),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l.commonCancel),
        ),
        FilledButton(
          key: const ValueKey('network-hidden-connect'),
          onPressed: _done,
          child: Text(l.networkConnect),
        ),
      ],
    );
  }
}

/// A saved network: auto-connect and "Forget"; pops true to forget it.
class _DetailsDialog extends StatelessWidget {
  const _DetailsDialog({required this.net, required this.ssid});

  final NetworkController net;
  final String ssid;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return ListenableBuilder(
      listenable: net,
      builder: (context, _) {
        final n = net.snapshot.networks
            .where((n) => n.ssid == ssid)
            .firstOrNull;
        final auto = n?.autoconnect ?? true;
        return AlertDialog(
          title: Text(ssid),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _Row(
                key: const ValueKey('network-autoconnect'),
                icon: Icons.autorenew_rounded,
                title: l.networkAutoconnect,
                toggled: auto,
                onTap: () => unawaited(net.setAutoconnect(ssid, !auto)),
                trailing: ExcludeFocus(
                  child: ExcludeSemantics(
                    child: Switch(
                      value: auto,
                      onChanged: (v) => net.setAutoconnect(ssid, v),
                    ),
                  ),
                ),
              ),
              _Row(
                key: const ValueKey('network-forget'),
                icon: Icons.delete_outline_rounded,
                title: l.networkForget,
                subtitle: l.networkForgetConfirm(ssid),
                onTap: () => Navigator.of(context).pop(true),
              ),
            ],
          ),
          actions: [
            TextButton(
              autofocus: true,
              onPressed: () => Navigator.of(context).pop(false),
              child: Text(l.networkClose),
            ),
          ],
        );
      },
    );
  }
}
