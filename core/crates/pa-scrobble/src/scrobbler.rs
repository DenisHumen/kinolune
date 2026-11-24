// SPDX-License-Identifier: MPL-2.0
//! The scrobbler of the app: the [`Tracker`] of the player, a [`Queue`]
//! per service and the requests over a [`Transport`].

use std::path::Path;

use crate::key::LastFmKey;
use crate::queue::Queue;
use crate::rules::{Action, Tracker};
use crate::{
    ApiError, Listen, Outcome, Response, Service, Track, Transport, lastfm, listenbrainz,
};

/// A connected Last.fm account. `Debug` hides the keys.
#[derive(Clone, PartialEq, Eq)]
pub struct LastFmConfig {
    pub key: LastFmKey,
    pub session_key: String,
}

impl std::fmt::Debug for LastFmConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LastFmConfig(..)")
    }
}

/// Where scrobbles go now. `Debug` hides the token.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub lastfm: Option<LastFmConfig>,
    pub listenbrainz: Option<String>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("lastfm", &self.lastfm.is_some())
            .field("listenbrainz", &self.listenbrainz.is_some())
            .finish()
    }
}

/// What the settings show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub lastfm_pending: u64,
    pub listenbrainz_pending: u64,
    /// The account must be connected again.
    pub lastfm_needs_auth: bool,
    pub listenbrainz_needs_auth: bool,
}

/// Scrobbles what the player plays.
pub struct Scrobbler<T: Transport> {
    transport: T,
    config: Config,
    tracker: Tracker,
    lastfm: Queue,
    listenbrainz: Queue,
}

impl<T: Transport> Scrobbler<T> {
    /// A scrobbler that keeps its queues in `dir` (`None`: in memory).
    pub fn new(transport: T, dir: Option<&Path>) -> Self {
        let file = |s: Service| dir.map(|d| d.join(format!("scrobble-{}.json", s.as_str())));
        Self {
            transport,
            config: Config::default(),
            tracker: Tracker::default(),
            lastfm: Queue::open(file(Service::LastFm)),
            listenbrainz: Queue::open(file(Service::ListenBrainz)),
        }
    }

    /// Sets the accounts. A service that is gone forgets its queue; a
    /// changed account tries its queue again at once.
    pub fn configure(&mut self, config: Config) {
        match (&self.config.lastfm, &config.lastfm) {
            (Some(_), None) => self.lastfm.clear(),
            (old, Some(new)) if old.as_ref() != Some(new) => self.lastfm.reset(),
            _ => {}
        }
        match (&self.config.listenbrainz, &config.listenbrainz) {
            (Some(_), None) => self.listenbrainz.clear(),
            (old, Some(new)) if old.as_ref() != Some(new) => self.listenbrainz.reset(),
            _ => {}
        }
        self.config = config;
    }

    /// Whether any service is connected.
    pub fn enabled(&self) -> bool {
        self.config.lastfm.is_some() || self.config.listenbrainz.is_some()
    }

    /// The player moved to track `id` at `position_ms` (Unix ms `now_ms`).
    pub fn track_started(&mut self, id: &str, track: Track, position_ms: u64, now_ms: u64) {
        if !self.enabled() {
            self.tracker.stop();
            return;
        }
        let actions = self.tracker.start(id, track, position_ms, now_ms);
        self.run(actions, now_ms);
    }

    /// The playhead and the play state (reported every few seconds and on
    /// every change).
    pub fn progress(&mut self, position_ms: u64, playing: bool, now_ms: u64) {
        let actions = self.tracker.progress(position_ms, playing, now_ms);
        self.run(actions, now_ms);
    }

    /// Nothing plays any more.
    pub fn stopped(&mut self) {
        self.tracker.stop();
    }

    fn run(&mut self, actions: Vec<Action>, now_ms: u64) {
        let mut queued = false;
        for a in actions {
            match a {
                Action::NowPlaying(t) => self.now_playing(&t),
                Action::Scrobble(l) => {
                    self.enqueue(l);
                    queued = true;
                }
            }
        }
        if queued {
            self.flush(now_ms);
        }
    }

    /// «Now playing» is sent once and not retried.
    fn now_playing(&self, track: &Track) {
        if let Some(c) = &self.config.lastfm
            && !self.lastfm.needs_auth()
        {
            let _ = self
                .transport
                .send(&lastfm::now_playing(&c.key, &c.session_key, track));
        }
        if let Some(token) = &self.config.listenbrainz
            && !self.listenbrainz.needs_auth()
        {
            let _ = self
                .transport
                .send(&listenbrainz::now_playing(token, track));
        }
    }

    fn enqueue(&mut self, listen: Listen) {
        if self.config.lastfm.is_some() {
            self.lastfm.push(listen.clone());
        }
        if self.config.listenbrainz.is_some() {
            self.listenbrainz.push(listen);
        }
    }

    /// Sends what is due in both queues, batch after batch, until a queue
    /// is empty or an attempt fails. Call it now and then (the app does
    /// every minute and when the network comes back).
    pub fn flush(&mut self, now_ms: u64) -> Status {
        if let Some(c) = self.config.lastfm.clone() {
            while let Some(batch) = self.lastfm.due(lastfm::MAX_BATCH, now_ms) {
                let outcome = match self
                    .transport
                    .send(&lastfm::scrobble(&c.key, &c.session_key, &batch))
                {
                    Ok(r) => lastfm::outcome(&r),
                    Err(_) => Outcome::Retry { after_ms: None },
                };
                self.lastfm.answer(batch.len(), &outcome, now_ms);
                if outcome != Outcome::Done && outcome != Outcome::Drop {
                    break;
                }
            }
        }
        if let Some(token) = self.config.listenbrainz.clone() {
            while let Some(batch) = self.listenbrainz.due(listenbrainz::MAX_BATCH, now_ms) {
                let outcome = match self.transport.send(&listenbrainz::listens(&token, &batch)) {
                    Ok(r) => listenbrainz::outcome(&r),
                    Err(_) => Outcome::Retry { after_ms: None },
                };
                self.listenbrainz.answer(batch.len(), &outcome, now_ms);
                if outcome != Outcome::Done && outcome != Outcome::Drop {
                    break;
                }
            }
        }
        self.status()
    }

    pub fn status(&self) -> Status {
        Status {
            lastfm_pending: self.lastfm.len() as u64,
            listenbrainz_pending: self.listenbrainz.len() as u64,
            lastfm_needs_auth: self.lastfm.needs_auth(),
            listenbrainz_needs_auth: self.listenbrainz.needs_auth(),
        }
    }

    // -- sign-in -------------------------------------------------------------

    fn send(&self, r: &crate::Request) -> Result<Response, ApiError> {
        self.transport
            .send(r)
            .map_err(|e| ApiError::Unavailable(e.0))
    }

    /// Step 1 of the Last.fm sign-in: a token and the page where the user
    /// approves it.
    pub fn lastfm_begin(&self, key: &LastFmKey) -> Result<(String, String), ApiError> {
        let token = lastfm::parse_token(&self.send(&lastfm::get_token(key))?)?;
        let url = lastfm::auth_url(key, &token);
        Ok((token, url))
    }

    /// Step 2: the session once the user approved `token`.
    pub fn lastfm_finish(&self, key: &LastFmKey, token: &str) -> Result<lastfm::Session, ApiError> {
        lastfm::parse_session(&self.send(&lastfm::get_session(key, token))?)
    }

    /// The ListenBrainz user of `token`.
    pub fn listenbrainz_user(&self, token: &str) -> Result<String, ApiError> {
        listenbrainz::parse_validate(&self.send(&listenbrainz::validate_token(token))?)
    }
}
