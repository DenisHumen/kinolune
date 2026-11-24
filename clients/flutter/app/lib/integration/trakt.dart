// SPDX-License-Identifier: MPL-2.0
//
// Single import point for the Trakt bridge (`CoreTrakt`, SYNC-011) from
// pa_core. Only lib/trakt/trakt_link.dart uses it.

export 'package:pa_core/pa_core.dart'
    show
        CoreLibrary,
        CoreTrakt,
        CoreTraktDeviceLogin,
        CoreTraktSyncReport,
        TraktError;
