// SPDX-License-Identifier: MPL-2.0
//! The Last.fm API key and shared secret of the build, like the TMDB key
//! (ADR-0018): read at **compile time** from `PA_LASTFM_API_KEY` /
//! `PA_LASTFM_SECRET` (`.env.local`, CI Secrets), or from
//! `CARGO_PA_LASTFM_API_KEY` / `CARGO_PA_LASTFM_SECRET` set by the Flutter
//! build hook. Never in git. A build without them works: the user enters
//! their own key and secret in Settings (they always win).

/// API key and shared secret of a Last.fm API account. `Debug` hides both.
#[derive(Clone, PartialEq, Eq)]
pub struct LastFmKey {
    pub api_key: String,
    pub secret: String,
}

impl std::fmt::Debug for LastFmKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LastFmKey(..)")
    }
}

impl LastFmKey {
    /// A key of non-empty, trimmed parts; `None` otherwise.
    pub fn new(api_key: &str, secret: &str) -> Option<Self> {
        let (k, s) = (api_key.trim(), secret.trim());
        (!k.is_empty() && !s.is_empty()).then(|| Self {
            api_key: k.to_owned(),
            secret: s.to_owned(),
        })
    }

    /// The key of this build, if it has one.
    pub fn project() -> Option<Self> {
        let key = option_env!("PA_LASTFM_API_KEY").or(option_env!("CARGO_PA_LASTFM_API_KEY"))?;
        let secret = option_env!("PA_LASTFM_SECRET").or(option_env!("CARGO_PA_LASTFM_SECRET"))?;
        Self::new(key, secret)
    }

    /// The user's own key if both parts are given, else the build's.
    pub fn choose(user_key: Option<&str>, user_secret: Option<&str>) -> Option<Self> {
        match (user_key, user_secret) {
            (Some(k), Some(s)) => Self::new(k, s).or_else(Self::project),
            _ => Self::project(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn users_key_wins_and_blanks_do_not_count() {
        let k = LastFmKey::choose(Some(" key "), Some("sec")).unwrap();
        assert_eq!(k.api_key, "key");
        assert_eq!(k.secret, "sec");
        assert_eq!(LastFmKey::new("", "sec"), None);
        assert_eq!(
            LastFmKey::choose(Some(""), Some("")),
            LastFmKey::project()
        );
        assert!(!format!("{k:?}").contains("sec"));
    }
}
