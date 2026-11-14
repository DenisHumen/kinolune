// SPDX-License-Identifier: MPL-2.0
//! The two Helix calls the monitor needs, behind a trait so the made-up
//! Twitch of [`crate::demo`] and recorded fixtures can stand in.

use std::time::{Duration, SystemTime};

use pa_twitch::{
    AuthError, AuthorizedHelix, BoxFuture, Clock, EventSubSubscription,
    EventSubSubscriptionRequest, HelixClient, MAX_PAGE_SIZE, Stream, StreamsQuery, Video,
};

/// Why a call failed, sorted by what the monitor does about it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LiveError {
    /// Not signed in, or the sign-in expired: stop until the user signs in.
    #[error("Twitch sign-in required: {0}")]
    Unauthorized(String),
    /// `429` on `POST /eventsub/subscriptions`: the WebSocket cost budget
    /// (`max_total_cost`) is spent. The remaining channels are polled.
    #[error("EventSub subscription budget exhausted")]
    BudgetExceeded,
    /// Helix rate limit; retry after the given time.
    #[error("Twitch rate limit, retry in {0:?}")]
    RateLimited(Duration),
    /// The subscription exists already (`409`); fine.
    #[error("subscription exists")]
    Conflict,
    /// Network failure.
    #[error("network error: {0}")]
    Network(String),
    /// Any other answer.
    #[error("Twitch error: {0}")]
    Other(String),
}

impl LiveError {
    /// Short code for logs and the UI.
    pub fn code(&self) -> &'static str {
        match self {
            LiveError::Unauthorized(_) => "unauthorized",
            LiveError::BudgetExceeded => "budget",
            LiveError::RateLimited(_) => "rate_limited",
            LiveError::Conflict => "conflict",
            LiveError::Network(_) => "network",
            LiveError::Other(_) => "other",
        }
    }
}

impl From<pa_twitch::Error> for LiveError {
    fn from(e: pa_twitch::Error) -> Self {
        use pa_twitch::Error as E;
        match e {
            E::MissingToken => LiveError::Unauthorized(e.to_string()),
            E::Unauthorized { message } => LiveError::Unauthorized(message),
            E::RateLimited { retry_after } => LiveError::RateLimited(retry_after),
            E::Api { status: 409, .. } => LiveError::Conflict,
            E::Api { status: 429, .. } => LiveError::BudgetExceeded,
            E::Transport(t) => LiveError::Network(t),
            other => LiveError::Other(other.to_string()),
        }
    }
}

impl From<AuthError> for LiveError {
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::Helix(h) => h.into(),
            AuthError::Transport(t) => LiveError::Network(t),
            AuthError::NotSignedIn | AuthError::ReauthRequired | AuthError::AccessDenied => {
                LiveError::Unauthorized(e.to_string())
            }
            other => LiveError::Other(other.to_string()),
        }
    }
}

/// Helix for the monitor.
pub trait LiveApi: Send + Sync {
    /// `POST /eventsub/subscriptions` (WebSocket transport).
    fn subscribe(
        &self,
        request: EventSubSubscriptionRequest,
    ) -> BoxFuture<'_, Result<EventSubSubscription, LiveError>>;

    /// `GET /streams?user_id=…` for any number of channels (batches of
    /// 100): the channels that are live now.
    fn live_streams(&self, user_ids: Vec<String>) -> BoxFuture<'_, Result<Vec<Stream>, LiveError>>;

    /// The archive VOD of a broadcast that just ended, to map our timeline
    /// onto it (TWI-008). `None` when unknown; the default asks nobody.
    fn archive_for_stream(
        &self,
        user_id: String,
        stream_id: String,
    ) -> BoxFuture<'_, Result<Option<Video>, LiveError>> {
        let _ = (user_id, stream_id);
        Box::pin(async { Ok(None) })
    }
}

async fn streams_of(helix: &HelixClient, user_ids: &[String]) -> pa_twitch::Result<Vec<Stream>> {
    let mut out = Vec::new();
    for chunk in user_ids.chunks(MAX_PAGE_SIZE as usize) {
        let q = StreamsQuery {
            user_ids: chunk.to_vec(),
            first: Some(MAX_PAGE_SIZE),
            ..StreamsQuery::default()
        };
        out.extend(helix.get_streams(&q).await?.items);
    }
    Ok(out)
}

impl LiveApi for HelixClient {
    fn subscribe(
        &self,
        request: EventSubSubscriptionRequest,
    ) -> BoxFuture<'_, Result<EventSubSubscription, LiveError>> {
        Box::pin(async move { Ok(self.create_eventsub_subscription(&request).await?) })
    }

    fn live_streams(&self, user_ids: Vec<String>) -> BoxFuture<'_, Result<Vec<Stream>, LiveError>> {
        Box::pin(async move { Ok(streams_of(self, &user_ids).await?) })
    }

    fn archive_for_stream(
        &self,
        user_id: String,
        stream_id: String,
    ) -> BoxFuture<'_, Result<Option<Video>, LiveError>> {
        Box::pin(async move { Ok(HelixClient::archive_for_stream(self, &user_id, &stream_id).await?) })
    }
}

impl LiveApi for AuthorizedHelix {
    fn subscribe(
        &self,
        request: EventSubSubscriptionRequest,
    ) -> BoxFuture<'_, Result<EventSubSubscription, LiveError>> {
        Box::pin(async move {
            let request = &request;
            Ok(self
                .call(|helix| async move { helix.create_eventsub_subscription(request).await })
                .await?)
        })
    }

    fn live_streams(&self, user_ids: Vec<String>) -> BoxFuture<'_, Result<Vec<Stream>, LiveError>> {
        Box::pin(async move {
            let ids = &user_ids;
            Ok(self
                .call(|helix| async move { streams_of(&helix, ids).await })
                .await?)
        })
    }

    fn archive_for_stream(
        &self,
        user_id: String,
        stream_id: String,
    ) -> BoxFuture<'_, Result<Option<Video>, LiveError>> {
        Box::pin(async move {
            let (user_id, stream_id) = (&user_id, &stream_id);
            Ok(self
                .call(|helix| async move { helix.archive_for_stream(user_id, stream_id).await })
                .await?)
        })
    }
}

/// Unix milliseconds of a clock.
pub(crate) fn unix_ms(clock: &dyn Clock) -> i64 {
    clock
        .now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Tokio time anchored at a wall-clock start. With paused Tokio time
/// (`#[tokio::test(start_paused = true)]`) both `now` and `sleep` are
/// virtual, so tests of minutes of polling run instantly and in order.
#[derive(Debug, Clone, Copy)]
pub struct TokioClock {
    start_wall: SystemTime,
    start: tokio::time::Instant,
}

impl TokioClock {
    /// Clock reading `start_wall` now.
    pub fn starting_at(start_wall: SystemTime) -> Self {
        Self {
            start_wall,
            start: tokio::time::Instant::now(),
        }
    }

    /// Clock at `secs` after the Unix epoch.
    pub fn at_unix(secs: u64) -> Self {
        Self::starting_at(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
    }
}

impl Clock for TokioClock {
    fn now(&self) -> SystemTime {
        self.start_wall + self.start.elapsed()
    }

    fn sleep(&self, duration: Duration) -> BoxFuture<'_, ()> {
        Box::pin(tokio::time::sleep(duration))
    }
}
