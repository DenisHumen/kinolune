// SPDX-License-Identifier: MPL-2.0
//
// Single import point for the Subsonic bridge (`CoreSubsonic`, NET-016)
// from pa_core. Only lib/sources/subsonic_link.dart uses it.

export 'package:pa_core/pa_core.dart'
    show CoreLibrary, CoreSubsonic, CoreSubsonicServer, SourcesError;
