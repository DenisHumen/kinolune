// SPDX-License-Identifier: MPL-2.0

/// Product name shown in the UI. Working name per ADR-0011 (DEC-004); change it
/// here only. Native launch labels (AndroidManifest `android:label`, GTK window
/// title in linux/runner/my_application.cc) must be kept in sync.
const kAppName = 'Kinolune';

/// Shown in About. Keep in sync with `version:` in pubspec.yaml.
const kAppVersion = '0.1.0';

const kRepoUrl = 'https://github.com/DenisHumen/kinolune';
const kTmdbUrl = 'https://www.themoviedb.org';

/// Attribution required by the TMDB API terms of use.
const kTmdbNotice =
    'This product uses the TMDB API but is not endorsed or certified by TMDB.';

const kMusicBrainzUrl = 'https://musicbrainz.org';

/// Open Library: book descriptions and covers of audiobooks (MUS-019).
const kOpenLibraryUrl = 'https://openlibrary.org';

/// META-012: TVmaze (CC BY-SA, link required), TheTVDB, Fanart.tv,
/// OpenSubtitles.
const kTvmazeUrl = 'https://www.tvmaze.com';
const kTvdbUrl = 'https://thetvdb.com';
const kFanartUrl = 'https://fanart.tv';
const kOpenSubtitlesUrl = 'https://www.opensubtitles.com';

/// Privacy policy and third-party notices in the repository (About, Privacy).
const kPrivacyPolicyUrl = '$kRepoUrl/blob/main/docs/legal/privacy-policy.md';
const kThirdPartyNoticesUrl =
    '$kRepoUrl/blob/main/packaging/app/THIRD-PARTY-NOTICES.md';

/// A way to support the project with a voluntary donation (STRATEGY §4).
class DonationLink {
  const DonationLink(this.platform, this.url);

  /// Platform name, shown as is (a brand name, not translated).
  final String platform;

  /// Link to the project's page there; empty = not set up, not shown.
  final String url;
}

/// Donation links, shown in About only in the `full` flavor
/// (`AppFlavor.donationLinks`, ADR-0007, ADR-0041): app stores restrict
/// them. Read only behind that constant, so store builds do not contain
/// them (tools/flavor/check-artifacts.sh greps for the platforms' domains).
///
/// PLACEHOLDERS: the project's accounts are the owner's decision. Fill the
/// URLs in; a link with an empty URL stays hidden.
const kDonationLinks = <DonationLink>[
  DonationLink('GitHub Sponsors', ''),
  DonationLink('Open Collective', ''),
  DonationLink('Liberapay', ''),
  DonationLink('Ko-fi', ''),
  DonationLink('Boosty', ''),
];
