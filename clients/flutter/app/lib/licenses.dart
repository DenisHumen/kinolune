// SPDX-License-Identifier: MPL-2.0
//
// Our own licence notices for the licence page (About → Licenses). The
// Flutter tool adds the pub packages' licences itself (NOTICES asset).
// Registered from KinoluneApp, not only from main(), so every way of
// building the app (tests, screenshot tooling) shows them (UI-023).
// Licence texts stay in English (legal notices, shown verbatim).

import 'package:flutter/foundation.dart';

import 'branding.dart';

bool _registered = false;

/// Adds the app's licence entries to [LicenseRegistry]; safe to call twice.
void registerAppLicenses() {
  if (_registered) return;
  _registered = true;
  LicenseRegistry.addLicense(_appLicenses);
}

Stream<LicenseEntry> _appLicenses() async* {
  yield const LicenseEntryWithLineBreaks(
    [kAppName],
    '$kAppName is licensed under the Mozilla Public License, v. 2.0. '
    'You can obtain a copy at https://mozilla.org/MPL/2.0/.\n\n'
    'Source code: $kRepoUrl',
  );
  yield const LicenseEntryWithLineBreaks(['TMDB'], kTmdbNotice);
  yield const LicenseEntryWithLineBreaks(
    ['Inter'],
    'Inter Variable font. Copyright (c) 2016 The Inter Project Authors '
    '(https://github.com/rsms/inter). Licensed under the SIL Open Font '
    'License, Version 1.1 (https://openfontlicense.org).',
  );
  yield const LicenseEntryWithLineBreaks(
    ['libmpv', 'FFmpeg'],
    '$kAppName plays media with libmpv (https://mpv.io) and FFmpeg '
    '(https://ffmpeg.org), used as separate shared libraries under the GNU '
    'Lesser General Public License v2.1 or later. Only LGPL builds are used. '
    'You may replace these libraries with your own builds. Source code: '
    'https://github.com/mpv-player/mpv and https://git.ffmpeg.org/ffmpeg.git; '
    'build scripts: $kRepoUrl.',
  );
  yield const LicenseEntryWithLineBreaks(
    ['libsmb2'],
    '$kAppName reads SMB network shares with libsmb2 '
    '(https://github.com/sahlberg/libsmb2), a separate shared library under '
    'the GNU Lesser General Public License v2.1 or later, loaded at run time. '
    'You may replace it with your own build. Build recipe: '
    '$kRepoUrl/tree/main/tools/libsmb2.',
  );
  // META-030: the bundled anime database (ODbL §4.2/§4.3 notice).
  yield const LicenseEntryWithLineBreaks(
    ['anime-offline-database'],
    'Contains information from anime-offline-database (cedya77, '
    'continuation of manami-project), made available under the Open '
    'Database License v1.0 (ODbL, https://opendatacommons.org/licenses/odbl/1-0/); '
    'its contents under the Database Contents License v1.0 '
    '(https://opendatacommons.org/licenses/dbcl/1-0/). '
    '$kAppName ships a derived database built from it '
    '(data/anime-mappings/build.sh in $kRepoUrl), which is itself under the '
    'ODbL v1.0; the app updates it from the upstream releases. Upstream: '
    '$kAnimeOfflineDatabaseUrl and '
    'https://github.com/manami-project/anime-offline-database.\n\n'
    'Episode mappings from Anime-Lists (https://github.com/Anime-Lists/anime-lists) '
    'are downloaded by this device from upstream; they are not part of the app.',
  );
  yield const LicenseEntryWithLineBreaks(
    ['Third-party notices'],
    'The libraries shipped next to the app, their licences and where to get '
    'their corresponding source: $kThirdPartyNoticesUrl. The source archives '
    'are attached to every GitHub Release of $kAppName.',
  );
}

/// A native library shipped with the app under its own licence, shown in
/// About (UI-016). Matches packaging/app/THIRD-PARTY-NOTICES.md.
class NativeComponent {
  const NativeComponent({
    required this.name,
    required this.licence,
    required this.source,
    required this.recipe,
  });

  final String name;

  /// SPDX expression.
  final String licence;

  /// Upstream source code.
  final String source;

  /// Our build recipe in the repository.
  final String recipe;
}

/// LGPL libraries the app ships as separate shared libraries (ADR-0012,
/// LIB-019). The corresponding source of each release is attached to it.
const kNativeComponents = <NativeComponent>[
  NativeComponent(
    name: 'libmpv',
    licence: 'LGPL-2.1-or-later',
    source: 'https://github.com/mpv-player/mpv',
    recipe: '$kRepoUrl/tree/main/tools/mpv',
  ),
  NativeComponent(
    name: 'FFmpeg',
    licence: 'LGPL-2.1-or-later',
    source: 'https://git.ffmpeg.org/ffmpeg.git',
    recipe: '$kRepoUrl/tree/main/tools/ffmpeg',
  ),
  NativeComponent(
    name: 'libsmb2',
    licence: 'LGPL-2.1-or-later',
    source: 'https://github.com/sahlberg/libsmb2',
    recipe: '$kRepoUrl/tree/main/tools/libsmb2',
  ),
];

/// The anime database the app ships and updates (META-030, ODbL).
const kAnimeOfflineDatabaseUrl =
    'https://github.com/cedya77/anime-offline-database';

/// Where the corresponding source of the release is published.
const kReleasesUrl = '$kRepoUrl/releases';
