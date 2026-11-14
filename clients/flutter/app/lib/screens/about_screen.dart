// SPDX-License-Identifier: MPL-2.0
//
// About (UI-016): version, build flavor and capabilities (ADR-0041), the
// source code, licences (the licence page, the LGPL libraries with their
// source), attributions (TMDB, MusicBrainz, Twitch) and — in the full
// flavor only — donation links (STRATEGY §4: app stores restrict them).

import 'package:material_ui/material_ui.dart';

import '../branding.dart';
import '../build_info.dart';
import '../flavor.dart';
import '../l10n/l10n.dart';
import '../licenses.dart';
import '../services/app_services.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/focusable.dart';
import '../widgets/logo.dart';
import '../widgets/settings_tiles.dart';

class AboutScreen extends StatelessWidget {
  const AboutScreen({super.key, this.donationLinks});

  /// Overrides [kDonationLinks] (tests). Shown only when the build has the
  /// `donation-links` capability, whatever is passed here.
  final List<DonationLink>? donationLinks;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    final l = context.l10n;
    final info = Services.of(context).buildInfo;
    return Scaffold(
      body: SafeArea(
        child: SingleChildScrollView(
          padding: EdgeInsets.fromLTRB(
            d.pageInsets.left,
            d.pageInsets.top,
            d.pageInsets.right,
            d.pageInsets.bottom + 90,
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (!d.isTv)
                Align(
                  alignment: Alignment.centerLeft,
                  child: IconButton(
                    tooltip: l.commonBack,
                    onPressed: () => Navigator.of(context).maybePop(),
                    icon: const Icon(Icons.arrow_back),
                  ),
                ),
              const SizedBox(height: 8),
              Align(
                alignment: Alignment.centerLeft,
                child: KinoluneLogo(size: d.isTv ? 44 : 56),
              ),
              const SizedBox(height: 16),
              Text(
                l.aboutVersion(info.appVersion),
                style: theme.textTheme.labelMedium,
              ),
              const SizedBox(height: 12),
              _Narrow(
                child: Text(
                  l.aboutDescription(kAppName),
                  style: theme.textTheme.bodyLarge?.copyWith(
                    color: KColors.textSecondary,
                  ),
                ),
              ),
              const SizedBox(height: 24),
              _Narrow(child: _BuildSection(info: info)),
              _Narrow(
                child: SettingsSection(
                  key: const ValueKey('about-source'),
                  icon: Icons.code,
                  title: l.aboutSourceCode,
                  children: [
                    SettingsLinkTile(
                      key: const ValueKey('about-repo'),
                      icon: Icons.public,
                      title: kRepoUrl.replaceFirst('https://', ''),
                      url: kRepoUrl,
                      subtitle: 'Mozilla Public License 2.0', // l10n-ignore
                    ),
                  ],
                ),
              ),
              if (AppFlavor.donationLinks)
                _Narrow(
                  child: _DonationSection(
                    links: donationLinks ?? kDonationLinks,
                  ),
                ),
              const _Narrow(child: _LicencesSection()),
              const _Narrow(child: _AttributionSection()),
            ],
          ),
        ),
      ),
    );
  }
}

class _Narrow extends StatelessWidget {
  const _Narrow({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) => Align(
    alignment: Alignment.topLeft,
    child: ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 680),
      child: child,
    ),
  );
}

/// Version, flavor and capabilities of the Dart half and of the core.
class _BuildSection extends StatelessWidget {
  const _BuildSection({required this.info});

  final BuildInfo info;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    String flavor(String name) => switch (name) {
      'full' => l.aboutFlavorFull(name),
      _ => l.aboutFlavorStore(name),
    };
    String capabilities(List<String> ids) => ids.isEmpty
        ? l.aboutCapabilitiesNone
        : [
            for (final id in ids)
              switch (id) {
                'twitch-native' => l.aboutCapTwitchNative,
                'twitch-archive' => l.aboutCapTwitchArchive,
                'donation-links' => l.aboutCapDonationLinks,
                _ => id,
              },
          ].join(', ');
    final core = info.coreVersion;
    return SettingsSection(
      key: const ValueKey('about-build'),
      icon: Icons.build_circle_outlined,
      title: l.aboutInfoVersion,
      children: [
        _InfoRow(label: l.aboutInfoVersion, value: info.appVersion),
        _InfoRow(
          key: const ValueKey('about-flavor'),
          label: l.aboutInfoFlavor,
          value: flavor(info.flavor),
        ),
        _InfoRow(
          key: const ValueKey('about-capabilities'),
          label: l.aboutInfoCapabilities,
          value: capabilities(info.capabilities),
        ),
        _InfoRow(
          key: const ValueKey('about-core'),
          label: l.aboutInfoCore,
          value: core == null
              ? l.aboutCoreMissing
              : l.aboutCoreLine(core, flavor(info.coreFlavor ?? info.flavor)),
        ),
        if (info.mismatch)
          Padding(
            padding: const EdgeInsets.only(top: 6),
            child: Text(
              l.aboutFlavorMismatch(info.coreFlavor!),
              style: Theme.of(context).textTheme.bodySmall
                  ?.copyWith(color: KColors.live),
            ),
          ),
      ],
    );
  }
}

class _InfoRow extends StatelessWidget {
  const _InfoRow({super.key, required this.label, required this.value});

  final String label;
  final String value;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return MergeSemantics(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 5),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              width: 150,
              child: Text(label, style: theme.textTheme.bodySmall),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: SelectableText(value, style: theme.textTheme.bodyMedium),
            ),
          ],
        ),
      ),
    );
  }
}

/// Donation links (full flavor only; the caller checks the capability).
class _DonationSection extends StatelessWidget {
  const _DonationSection({required this.links});

  final List<DonationLink> links;

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final shown = [
      for (final link in links)
        if (link.url.isNotEmpty) link,
    ];
    if (shown.isEmpty) return const SizedBox.shrink();
    return SettingsSection(
      key: const ValueKey('about-donations'),
      icon: Icons.favorite_outline,
      title: l.aboutSupportTitle,
      subtitle: l.aboutSupportBody(kAppName),
      children: [
        for (final link in shown)
          SettingsLinkTile(
            icon: Icons.volunteer_activism_outlined,
            title: link.platform,
            url: link.url,
          ),
      ],
    );
  }
}

/// The licence page and the LGPL libraries with their source.
class _LicencesSection extends StatelessWidget {
  const _LicencesSection();

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final theme = Theme.of(context);
    return SettingsSection(
      key: const ValueKey('about-licences'),
      icon: Icons.gavel_outlined,
      title: l.aboutLicensesTitle,
      subtitle: l.aboutLicensesSubtitle(kAppName),
      children: [
        Align(
          alignment: Alignment.centerLeft,
          child: KButton(
            autofocus: !d.isPhone,
            label: l.aboutLicenses,
            icon: Icons.description_outlined,
            onPressed: () => showLicensePage(
              context: context,
              applicationName: kAppName,
              applicationVersion: kAppVersion,
              applicationIcon: const Padding(
                padding: EdgeInsets.all(12),
                child: KinoluneLogo(size: 48, showName: false),
              ),
              // Legal notice, kept verbatim in English.
              applicationLegalese:
                  'Mozilla Public License 2.0. $kTmdbNotice', // l10n-ignore
            ),
          ),
        ),
        const SizedBox(height: 16),
        Text(l.aboutNativeTitle, style: theme.textTheme.titleMedium),
        const SizedBox(height: 4),
        Text(l.aboutNativeBody, style: theme.textTheme.bodySmall),
        const SizedBox(height: 6),
        for (final c in kNativeComponents)
          SettingsLinkTile(
            key: ValueKey('about-native-${c.name}'),
            icon: Icons.memory,
            title: '${c.name} · ${c.licence}',
            url: c.source,
            subtitle:
                '${l.aboutNativeSource}: ${c.source}\n'
                '${l.aboutNativeRecipe}: ${c.recipe}',
          ),
        SettingsLinkTile(
          key: const ValueKey('about-third-party'),
          icon: Icons.article_outlined,
          title: l.aboutThirdPartyNotices,
          url: kThirdPartyNoticesUrl,
        ),
      ],
    );
  }
}

/// TMDB (required by its API terms), MusicBrainz and Twitch.
class _AttributionSection extends StatelessWidget {
  const _AttributionSection();

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return SettingsSection(
      key: const ValueKey('about-attribution'),
      icon: Icons.copyright,
      title: l.aboutAttributionTitle,
      children: [
        const TmdbAttribution(),
        const SizedBox(height: 12),
        _Attribution(
          mark: 'MusicBrainz', // l10n-ignore: brand name
          heading: l.aboutMusicBrainzHeading,
          notice: l.aboutMusicBrainzNotice,
          url: kMusicBrainzUrl,
        ),
        const SizedBox(height: 12),
        _Attribution(
          mark: 'Twitch', // l10n-ignore: brand name
          heading: l.aboutTwitchHeading,
          notice: l.aboutTwitchNotice(kAppName),
        ),
      ],
    );
  }
}

/// TMDB attribution required by the API terms. The official TMDB logo is a
/// third-party brand asset: it is added only via design/LICENSES.md (DSN-007),
/// until then a neutral text mark is shown.
class TmdbAttribution extends StatelessWidget {
  const TmdbAttribution({super.key});

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return _Attribution(
      mark: 'TMDB', // l10n-ignore: brand name
      heading: l.aboutTmdbHeading,
      notice: l.commonTmdbNotice,
      url: kTmdbUrl,
    );
  }
}

/// A neutral text mark (no third-party logos, AGENTS.md), a heading and the
/// notice.
class _Attribution extends StatelessWidget {
  const _Attribution({
    required this.mark,
    required this.heading,
    required this.notice,
    this.url,
  });

  final String mark;
  final String heading;
  final String notice;
  final String? url;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Container(
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: KColors.bgElevated2,
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: KColors.stroke),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            constraints: const BoxConstraints(minWidth: 64),
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
            decoration: BoxDecoration(
              border: Border.all(color: Colors.white54),
              borderRadius: BorderRadius.circular(6),
            ),
            child: Text(
              mark,
              textAlign: TextAlign.center,
              style: theme.textTheme.labelMedium?.copyWith(letterSpacing: 1),
            ),
          ),
          const SizedBox(width: 14),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(heading, style: theme.textTheme.titleMedium),
                const SizedBox(height: 4),
                Text(notice, style: theme.textTheme.bodySmall),
                if (url != null) ...[
                  const SizedBox(height: 4),
                  SelectableText(
                    url!,
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: KColors.accentSoft,
                    ),
                  ),
                ],
              ],
            ),
          ),
        ],
      ),
    );
  }
}
