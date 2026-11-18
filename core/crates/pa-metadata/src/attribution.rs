// SPDX-License-Identifier: MPL-2.0
//! Attribution each provider requires, for the "About / Credits" screen
//! (UI-016) and wherever provider data is shown.
//!
//! Logos are not bundled here: UI-016 uses the provider's approved assets
//! under its brand guidelines (docs/legal/providers.md).

/// What the UI must show for one provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attribution {
    /// Provider id, as in `ProviderRequest::provider`.
    pub provider: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Required notice, verbatim (not translated).
    pub notice: &'static str,
    /// Link to the provider's site.
    pub url: &'static str,
    /// Where the approved logos and their usage rules are published.
    pub logo_guidelines_url: Option<&'static str>,
}

/// TMDB: approved logo, less prominent than our own brand, plus this exact
/// sentence (TMDB API Terms of Use; docs/legal/providers.md, TMDB §4).
pub const TMDB: Attribution = Attribution {
    provider: "tmdb",
    name: "TMDB",
    notice: "This product uses the TMDB API but is not endorsed or certified by TMDB.",
    url: "https://www.themoviedb.org/",
    logo_guidelines_url: Some("https://www.themoviedb.org/about/logos-attribution"),
};

/// MusicBrainz: core data is CC0, genres are supplementary data
/// (CC BY-NC-SA 3.0) that must credit MusicBrainz (docs/legal/providers.md).
pub const MUSICBRAINZ: Attribution = Attribution {
    provider: crate::musicbrainz::PROVIDER,
    name: "MusicBrainz",
    notice: "Music metadata from MusicBrainz (core data CC0, genres CC BY-NC-SA 3.0).",
    url: "https://musicbrainz.org/",
    logo_guidelines_url: Some("https://metabrainz.org/projects"),
};

/// Cover Art Archive: images are not licensed by the archive; shown and
/// cached on the device only.
pub const COVER_ART_ARCHIVE: Attribution = Attribution {
    provider: crate::coverart::PROVIDER,
    name: "Cover Art Archive",
    notice: "Album covers from the Cover Art Archive; rights stay with their owners.",
    url: "https://coverartarchive.org/",
    logo_guidelines_url: None,
};

/// Open Library: no rights asserted over the catalogue data; a link back
/// is asked for, covers are shown and cached on the device only (MUS-019).
pub const OPEN_LIBRARY: Attribution = Attribution {
    provider: crate::openlibrary::PROVIDER,
    name: "Open Library",
    notice: "Book descriptions and covers from Open Library (Internet Archive).",
    url: "https://openlibrary.org/",
    logo_guidelines_url: None,
};

/// TVmaze: the data is CC BY-SA 4.0, which asks for credit with a link
/// (META-012, ADR-0132). Images are not used.
pub const TVMAZE: Attribution = Attribution {
    provider: crate::tvmaze::PROVIDER,
    name: "TVmaze",
    notice: "TV schedules and episode data from TVmaze (CC BY-SA 4.0).",
    url: "https://www.tvmaze.com/",
    logo_guidelines_url: None,
};

/// TheTVDB: only with the user's own key; TheTVDB asks apps to credit it
/// (META-012).
pub const THETVDB: Attribution = Attribution {
    provider: crate::tvdb::PROVIDER,
    name: "TheTVDB",
    notice: "Metadata provided by TheTVDB. Please consider adding missing information or subscribing.",
    url: "https://thetvdb.com/",
    logo_guidelines_url: Some("https://thetvdb.com/api-information"),
};

/// Fanart.tv: clear logos and clear art made by its community; shown and
/// cached on the device only (META-012).
pub const FANART_TV: Attribution = Attribution {
    provider: crate::fanart::PROVIDER,
    name: "Fanart.tv",
    notice: "Clear logos and clear art from Fanart.tv; rights stay with their artists.",
    url: "https://fanart.tv/",
    logo_guidelines_url: None,
};

/// Attributions of all providers implemented in this crate.
pub const ALL: &[Attribution] = &[
    TMDB,
    MUSICBRAINZ,
    COVER_ART_ARCHIVE,
    OPEN_LIBRARY,
    TVMAZE,
    THETVDB,
    FANART_TV,
];
