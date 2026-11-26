// SPDX-License-Identifier: MPL-2.0
//! Making a clip of a live stream (TWI-040): `POST /clips`, then
//! `GET /clips?id=` until the clip exists.
//!
//! Twitch documents the flow: `POST /clips` answers `202 Accepted` with the
//! new clip id and its edit page; the clip is ready once `GET /clips`
//! returns it, and when it does not within 15 seconds, the clip failed.
//! The scope `clips:edit` is not part of [`crate::DEFAULT_SCOPES`]: the app
//! asks for it with a repeated Device Code Flow on the first press
//! (minimal scopes, TWI-002).
//!
//! VODs have no button: Helix makes clips of live streams only.

use std::time::Duration;

use serde::Deserialize;

use crate::auth::{AuthError, AuthorizedHelix, SCOPE_CLIPS_EDIT};
use crate::client::{Envelope, HelixClient, Query};
use crate::error::{Error, Result};
use crate::model::Clip;
use crate::transport::Method;

/// How long Twitch may take to make a clip; after that it failed.
pub const CLIP_READY_TIMEOUT: Duration = Duration::from_secs(15);
/// Pause between two `GET /clips?id=` checks.
pub const CLIP_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Answer of `POST /clips`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CreatedClip {
    /// Clip id (slug).
    pub id: String,
    /// Twitch page where the user can trim and title the clip.
    #[serde(default)]
    pub edit_url: String,
}

/// Why a clip was not made, in the terms the player shows.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClipError {
    /// No sign-in: send the user to sign in.
    #[error("not signed in to Twitch")]
    SignedOut,
    /// The sign-in lacks `clips:edit`: sign in again asking for it.
    #[error("the sign-in does not allow making clips")]
    ScopeRequired,
    /// The sign-in is over (refresh token dead): sign in again.
    #[error("the Twitch sign-in expired")]
    ReauthRequired,
    /// The channel turned clips off (or allows them to followers or
    /// subscribers only): the button stays off with this explanation.
    #[error("clips are disabled on this channel: {message}")]
    Disabled {
        /// Twitch's explanation.
        message: String,
    },
    /// The channel is not live (`404`).
    #[error("the channel is offline")]
    Offline,
    /// Twitch did not finish the clip within [`CLIP_READY_TIMEOUT`].
    #[error("Twitch did not finish the clip")]
    NotReady,
    /// Rate limit; retry later.
    #[error("Twitch rate limit hit, retry in {retry_after:?}")]
    RateLimited {
        /// How long until the bucket refills.
        retry_after: Duration,
    },
    /// Anything else (network, unexpected answer).
    #[error(transparent)]
    Helix(Error),
}

impl From<Error> for ClipError {
    fn from(e: Error) -> Self {
        match e {
            Error::MissingToken => Self::SignedOut,
            Error::Unauthorized { message } if is_missing_scope(&message) => Self::ScopeRequired,
            Error::Unauthorized { .. } => Self::ReauthRequired,
            Error::RateLimited { retry_after } => Self::RateLimited { retry_after },
            Error::Api {
                status: 403,
                message,
            } => Self::Disabled { message },
            Error::Api { status: 404, .. } => Self::Offline,
            other => Self::Helix(other),
        }
    }
}

impl From<AuthError> for ClipError {
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::NotSignedIn => Self::SignedOut,
            AuthError::ReauthRequired => Self::ReauthRequired,
            AuthError::Helix(inner) => inner.into(),
            AuthError::Transport(m) => Self::Helix(Error::Transport(m)),
            other => Self::Helix(Error::Decode(other.to_string())),
        }
    }
}

/// Twitch's `401` for a token without the scope reads
/// `Missing scope: clips:edit`.
fn is_missing_scope(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("missing scope") || m.contains(SCOPE_CLIPS_EDIT)
}

impl HelixClient {
    /// `POST /clips?broadcaster_id=`: starts a clip of the live stream
    /// (scope `clips:edit`). `has_delay` adds the broadcaster's stream
    /// delay so the clip matches what the viewer saw.
    pub async fn create_clip(&self, broadcaster_id: &str, has_delay: bool) -> Result<CreatedClip> {
        if broadcaster_id.is_empty() {
            return Err(Error::InvalidArgument("clips: broadcaster_id is empty"));
        }
        let mut query: Query = vec![("broadcaster_id", broadcaster_id.to_owned())];
        if has_delay {
            query.push(("has_delay", "true".to_owned()));
        }
        let request = self.build_request(Method::Post, "clips", &query)?;
        let response = self
            .send(request, false)
            .await?
            .ok_or_else(|| Error::Decode("empty response".to_owned()))?;
        let env: Envelope<Vec<CreatedClip>> = serde_json::from_slice(&response.body)
            .map_err(|e| Error::Decode(format!("clips: {e}")))?;
        env.data
            .into_iter()
            .next()
            .ok_or_else(|| Error::Decode("clips: empty data".to_owned()))
    }

    /// `GET /clips?id=`: the clip once Twitch made it, `None` before.
    pub async fn clip_by_id(&self, id: &str) -> Result<Option<Clip>> {
        if id.is_empty() {
            return Err(Error::InvalidArgument("clips: id is empty"));
        }
        let query: Query = vec![("id", id.to_owned())];
        let env = self.get_opt::<Vec<Clip>>("clips", query, true).await?;
        Ok(env.and_then(|e| e.data.into_iter().find(|c| c.id == id)))
    }
}

impl AuthorizedHelix {
    /// Whether the signed-in account granted `clips:edit`. No network.
    pub fn can_make_clips(&self) -> bool {
        self.auth()
            .account()
            .is_some_and(|a| a.scopes.iter().any(|s| s == SCOPE_CLIPS_EDIT))
    }

    /// Makes a clip of a live channel and waits until Twitch finished it
    /// (at most [`CLIP_READY_TIMEOUT`], checking every
    /// [`CLIP_POLL_INTERVAL`]).
    ///
    /// Without `clips:edit` in the sign-in nothing is sent and the answer
    /// is [`ClipError::ScopeRequired`]: the app starts a sign-in with
    /// [`crate::TwitchAuth::start_device_login_with`] and
    /// [`SCOPE_CLIPS_EDIT`], then presses again.
    pub async fn make_clip(
        &self,
        broadcaster_id: &str,
        has_delay: bool,
    ) -> std::result::Result<(CreatedClip, Clip), ClipError> {
        if self.auth().account().is_none() {
            return Err(ClipError::SignedOut);
        }
        if !self.can_make_clips() {
            return Err(ClipError::ScopeRequired);
        }
        let created = self
            .call(|h| async move { h.create_clip(broadcaster_id, has_delay).await })
            .await?;
        let clock = self.helix().clock();
        let deadline = clock.now() + CLIP_READY_TIMEOUT;
        loop {
            clock.sleep(CLIP_POLL_INTERVAL).await;
            let id = created.id.as_str();
            if let Some(clip) = self
                .call(|h| async move { h.clip_by_id(id).await })
                .await?
            {
                return Ok((created, clip));
            }
            if clock.now() >= deadline {
                return Err(ClipError::NotReady);
            }
        }
    }
}
