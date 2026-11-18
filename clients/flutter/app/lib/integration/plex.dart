// SPDX-License-Identifier: MPL-2.0
//
// Single import point for the Plex bridge (`CorePlex`, NET-012) from
// pa_core. Only lib/sources/plex_link.dart uses it.

export 'package:pa_core/pa_core.dart'
    show CoreLibrary, CorePlex, CorePlexState, SourcesError;
