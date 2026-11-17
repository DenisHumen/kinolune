// SPDX-License-Identifier: MPL-2.0
//! Subtitles from OpenSubtitles (PLAY-008) for Dart: «Найти субтитры» in
//! the player's subtitle menu.
//!
//! [`CoreLibrary::subtitle_search`] asks by the file's OSHash and the ids
//! of its work and ranks the offers (`pa_catalog::subtitle_search`);
//! [`CoreLibrary::subtitle_download`] spends one download of the quota and
//! keeps the file next to the video (writable source) or in the cache
//! (read-only source), byte for byte. [`CoreLibrary::cached_subtitles`]
//! lists what the cache holds for a file, for the player's track list.
//!
//! The account (login, password, the user's own key) comes from the app's
//! secure storage with every call and is only kept in memory here, with
//! the session token, for the next call. Failures the user can act on come
//! back as an `error` code, not as an exception: `no_key`, `key_rejected`,
//! `login`, `quota`, `busy`, `network`, `other`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use flutter_rust_bridge::frb;
use pa_catalog as c;
use pa_catalog::pa_metadata::opensubtitles::{
    Account, OpenSubtitlesClient, OpenSubtitlesError, Quota,
};

use crate::api::library::{CoreLibrary, LibraryError};
use crate::api::subtitles::CoreSidecarSubtitle;

type Result<T> = std::result::Result<T, LibraryError>;

/// The user's OpenSubtitles settings, from the app's secure storage.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct CoreOpenSubtitlesAccount {
    /// Login; with [`password`](Self::password) the quota is the account's.
    pub username: Option<String>,
    /// Password, only sent to OpenSubtitles' `/login`.
    pub password: Option<String>,
    /// The user's own API key; `None`: the key built into the app.
    pub api_key: Option<String>,
}

impl std::fmt::Debug for CoreOpenSubtitlesAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreOpenSubtitlesAccount")
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// The download quota as last seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSubtitleQuota {
    /// Downloads left; `None` until known (no account, nothing downloaded).
    pub remaining: Option<i64>,
    /// Downloads a day, when known.
    pub allowed: Option<u32>,
    /// When the quota is renewed (ISO 8601, UTC), when known.
    pub reset_time_utc: Option<String>,
    /// The account's quota (logged in), not the address's.
    pub logged_in: bool,
}

impl From<Quota> for CoreSubtitleQuota {
    fn from(q: Quota) -> Self {
        Self {
            remaining: q.remaining,
            allowed: q.allowed,
            reset_time_utc: q.reset_time_utc,
            logged_in: q.logged_in,
        }
    }
}

/// A subtitle the user can pick, best first.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreSubtitleOffer {
    /// What [`CoreLibrary::subtitle_download`] downloads.
    pub file_id: i64,
    pub subtitle_id: String,
    /// ISO 639-2/B (`rus`), or the provider's code when unknown.
    pub language: Option<String>,
    /// The provider's code (`ru`, `pt-br`).
    pub provider_language: Option<String>,
    pub release: String,
    pub download_count: i64,
    /// Made for this very file (moviehash match): in sync with it.
    pub hash_match: bool,
    pub hearing_impaired: bool,
    /// Only the foreign-language lines.
    pub forced: bool,
    /// Machine or AI translation.
    pub machine_translated: bool,
    pub trusted: bool,
    pub fps: Option<f64>,
    pub uploader: Option<String>,
}

impl From<c::SubtitleOffer> for CoreSubtitleOffer {
    fn from(o: c::SubtitleOffer) -> Self {
        Self {
            file_id: i64::try_from(o.file_id).unwrap_or(i64::MAX),
            subtitle_id: o.subtitle_id,
            language: o.language,
            provider_language: o.provider_language,
            release: o.release,
            download_count: i64::try_from(o.download_count).unwrap_or(i64::MAX),
            hash_match: o.hash_match,
            hearing_impaired: o.hearing_impaired,
            forced: o.forced,
            machine_translated: o.machine_translated,
            trusted: o.trusted,
            fps: o.fps,
            uploader: o.uploader,
        }
    }
}

impl From<CoreSubtitleOffer> for c::SubtitleOffer {
    fn from(o: CoreSubtitleOffer) -> Self {
        Self {
            file_id: u64::try_from(o.file_id).unwrap_or(0),
            subtitle_id: o.subtitle_id,
            language: o.language,
            provider_language: o.provider_language,
            release: o.release,
            download_count: u64::try_from(o.download_count).unwrap_or(0),
            hash_match: o.hash_match,
            hearing_impaired: o.hearing_impaired,
            forced: o.forced,
            machine_translated: o.machine_translated,
            trusted: o.trusted,
            fps: o.fps,
            uploader: o.uploader,
        }
    }
}

/// The result of a search.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreSubtitleSearch {
    /// Ranked: in sync with the file, the profile's languages, human
    /// translations, downloads.
    pub offers: Vec<CoreSubtitleOffer>,
    /// The quota (the account's after login), when known.
    pub quota: Option<CoreSubtitleQuota>,
    /// `no_key`, `key_rejected`, `login`, `quota`, `busy`, `network`,
    /// `other`; `None` when the search worked.
    pub error: Option<String>,
}

/// The result of a download.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreSubtitleDownload {
    /// The subtitle for the player, like the sidecars; `None` on error.
    pub subtitle: Option<CoreSidecarSubtitle>,
    /// Written next to the video (else into the app's cache).
    pub next_to_video: bool,
    /// The quota after the download (or after the refusal).
    pub quota: Option<CoreSubtitleQuota>,
    /// As in [`CoreSubtitleSearch::error`].
    pub error: Option<String>,
}

/// The account check of the settings screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSubtitleAccountStatus {
    /// The quota; `None` without an account.
    pub quota: Option<CoreSubtitleQuota>,
    /// Rank name (`Sub leecher`), when logged in.
    pub level: Option<String>,
    /// As in [`CoreSubtitleSearch::error`].
    pub error: Option<String>,
}

/// The build has the project's OpenSubtitles key (else the user needs
/// their own key for the search to work).
#[frb(sync)]
pub fn opensubtitles_has_project_key() -> bool {
    c::pa_metadata::key::Keys::opensubtitles_from_build().has_project_key()
}

/// Logs in with `account` and reads its quota. Blocks on the network.
pub fn opensubtitles_account_status(account: CoreOpenSubtitlesAccount) -> CoreSubtitleAccountStatus {
    let client = client_for(&account);
    match client.user_info() {
        Ok(info) => CoreSubtitleAccountStatus {
            quota: client.quota().map(Into::into),
            level: info.and_then(|i| i.level),
            error: None,
        },
        Err(e) => CoreSubtitleAccountStatus {
            quota: client.quota().map(Into::into),
            level: None,
            error: Some(error_code(&e).to_owned()),
        },
    }
}

impl CoreLibrary {
    /// Searches OpenSubtitles for the file `rel_path` of source
    /// `source_id` (as in `CoreItem` and `CoreVersion`). `languages`: the
    /// profile's subtitle languages (ISO 639), best first; English is
    /// asked for too. Spends no download quota. Blocks on the network.
    pub fn subtitle_search(
        &self,
        source_id: i64,
        rel_path: String,
        languages: Vec<String>,
        account: CoreOpenSubtitlesAccount,
    ) -> Result<CoreSubtitleSearch> {
        let client = client_for(&account);
        if account_of(&account).is_some() && client.quota().is_none() {
            // Logging in shows the account's quota from the start; a
            // wrong password still lets the search run.
            let _ = client.user_info();
        }
        let found = self
            .catalog()
            .search_subtitles(&client, source_id, &rel_path, &languages);
        let quota = client.quota().map(Into::into);
        match found {
            Ok(offers) => Ok(CoreSubtitleSearch {
                offers: offers.into_iter().map(Into::into).collect(),
                quota,
                error: None,
            }),
            Err(c::SubtitleSearchError::Provider(e)) => Ok(CoreSubtitleSearch {
                offers: Vec::new(),
                quota,
                error: Some(error_code(&e).to_owned()),
            }),
            Err(c::SubtitleSearchError::Library(e)) => Err(e.into()),
        }
    }

    /// Downloads `offer` (one download of the quota) for the file
    /// `rel_path` of source `source_id` and keeps it: next to the video if
    /// the source can be written, else under `cache_dir`. Blocks on the
    /// network and the source.
    pub fn subtitle_download(
        &self,
        source_id: i64,
        rel_path: String,
        offer: CoreSubtitleOffer,
        account: CoreOpenSubtitlesAccount,
        cache_dir: String,
    ) -> Result<CoreSubtitleDownload> {
        let client = client_for(&account);
        let saved = self.catalog().download_subtitle(
            &client,
            source_id,
            &rel_path,
            &offer.into(),
            &PathBuf::from(cache_dir),
        );
        let quota = client.quota().map(Into::into);
        match saved {
            Ok(s) => Ok(CoreSubtitleDownload {
                subtitle: Some(s.sidecar.into()),
                next_to_video: s.next_to_video,
                quota,
                error: None,
            }),
            Err(c::SubtitleSearchError::Provider(e)) => Ok(CoreSubtitleDownload {
                subtitle: None,
                next_to_video: false,
                quota,
                error: Some(error_code(&e).to_owned()),
            }),
            Err(c::SubtitleSearchError::Library(e)) => Err(e.into()),
        }
    }

    /// Subtitles downloaded into `cache_dir` for the file `rel_path` of
    /// source `source_id` (its source could not be written). Empty for an
    /// unknown file or none.
    pub fn cached_subtitles(
        &self,
        source_id: i64,
        rel_path: String,
        cache_dir: String,
    ) -> Result<Vec<CoreSidecarSubtitle>> {
        match self
            .catalog()
            .cached_subtitles(source_id, &rel_path, &PathBuf::from(cache_dir))
        {
            Ok(subs) => Ok(subs.into_iter().map(Into::into).collect()),
            Err(c::Error::NotFound(_)) => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }
}

fn account_of(a: &CoreOpenSubtitlesAccount) -> Option<Account> {
    Account::new(a.username.as_deref()?, a.password.as_deref()?)
}

/// One client per account and key, kept for its session token and quota.
fn client_for(account: &CoreOpenSubtitlesAccount) -> Arc<OpenSubtitlesClient> {
    static CLIENT: Mutex<Option<(CoreOpenSubtitlesAccount, Arc<OpenSubtitlesClient>)>> =
        Mutex::new(None);
    let mut slot = CLIENT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((a, client)) = slot.as_ref()
        && a == account
    {
        return Arc::clone(client);
    }
    let key = account
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty());
    let client = Arc::new(OpenSubtitlesClient::live(key, account_of(account)));
    *slot = Some((account.clone(), Arc::clone(&client)));
    client
}

fn error_code(e: &OpenSubtitlesError) -> &'static str {
    match e {
        OpenSubtitlesError::NoKey => "no_key",
        OpenSubtitlesError::KeyRejected(_) => "key_rejected",
        OpenSubtitlesError::Login(_) => "login",
        OpenSubtitlesError::QuotaExhausted { .. } => "quota",
        OpenSubtitlesError::Busy { .. } => "busy",
        OpenSubtitlesError::Transport(_) => "network",
        OpenSubtitlesError::Http { status: 401, .. } => "key_rejected",
        OpenSubtitlesError::Http { .. } | OpenSubtitlesError::Decode(_) => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_debug_hides_secrets() {
        let a = CoreOpenSubtitlesAccount {
            username: Some("me".into()),
            password: Some("hunter2".into()),
            api_key: Some("k-123".into()),
        };
        let shown = format!("{a:?}");
        assert!(!shown.contains("hunter2") && !shown.contains("k-123"), "{shown}");
    }

    /// Without a key nothing reaches the network and the search says why.
    #[test]
    fn search_without_a_key_says_no_key() {
        if opensubtitles_has_project_key() {
            return; // A release build: the project key is there.
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("media");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Film.2019.mkv"), vec![3u8; 150_000]).unwrap();
        let lib = CoreLibrary::open(dir.path().join("l.db").to_string_lossy().into()).unwrap();
        let catalog = lib.catalog();
        catalog
            .add_source(None, "local", "Media", root.to_str().unwrap(), None)
            .unwrap();
        catalog
            .scan(None, &c::CancelToken::new(), &mut |_| {})
            .unwrap();
        let source = catalog.sources().unwrap()[0].id;
        let found = lib
            .subtitle_search(
                source,
                "Film.2019.mkv".into(),
                vec!["rus".into()],
                CoreOpenSubtitlesAccount::default(),
            )
            .unwrap();
        assert_eq!(found.error.as_deref(), Some("no_key"));
        assert!(found.offers.is_empty());
        assert!(
            lib.cached_subtitles(source, "Film.2019.mkv".into(), dir.path().to_string_lossy().into())
                .unwrap()
                .is_empty()
        );
    }
}
