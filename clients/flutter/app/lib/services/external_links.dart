// SPDX-License-Identifier: MPL-2.0
//
// Opening a link outside the app (TWI-005, owner decision on ADR-0049):
// the Twitch app or the browser on phones and desktops. TVs have nothing
// to open links with and keep the QR code.

import 'package:url_launcher/url_launcher.dart';

/// Opens [url] in the app that handles it (the Twitch app for twitch.tv
/// links when installed) or the browser. False when nothing could.
typedef ExternalLinkOpener = Future<bool> Function(Uri url);

Future<bool> _launch(Uri url) async {
  try {
    return await launchUrl(url, mode: LaunchMode.externalApplication);
  } catch (_) {
    return false;
  }
}

/// The opener the app uses; tests replace it.
ExternalLinkOpener openExternalLink = _launch;
