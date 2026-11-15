// SPDX-License-Identifier: MPL-2.0
//
// Single import point for the Jellyfin bridge (`CoreJellyfin`, NET-014)
// from pa_core. Only lib/sources/jellyfin_link.dart uses it.

export 'package:pa_core/pa_core.dart'
    show CoreJellyfin, CoreLibrary, SourcesError;
