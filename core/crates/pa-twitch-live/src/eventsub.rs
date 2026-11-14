// SPDX-License-Identifier: MPL-2.0
//! One EventSub WebSocket session for `stream.online`, `channel.update` and
//! `stream.offline` of the watched channels: connect, wait for the welcome,
//! subscribe within the budget ([`SubscriptionPlan`]), forward timeline
//! events, follow `session_reconnect`, reconnect with backoff when the
//! connection dies and stop for good when the sign-in is gone.

use std::collections::{BTreeSet, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use pa_twitch::Clock;
use pa_twitch::chapters::{TimelineEvent, parse_eventsub_message};
use pa_twitch_chat::eventsub::{EventSubFrame, parse_frame};
use pa_twitch_chat::{Backoff, WsConnection, WsConnector};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::api::{LiveApi, LiveError};
use crate::plan::{SUB_OFFLINE, SUB_ONLINE, SUB_UPDATE, SubscriptionPlan};

/// EventSub WebSocket endpoint.
pub const DEFAULT_EVENTSUB_URL: &str = "wss://eventsub.wss.twitch.tv/ws";

/// Frame ids remembered against Twitch's duplicate deliveries.
const DEDUP_WINDOW: usize = 512;

/// Connection settings.
#[derive(Debug, Clone)]
pub struct EventSubSettings {
    /// WebSocket URL.
    pub url: String,
    /// Reconnect backoff.
    pub backoff: Backoff,
    /// Added to the session's keepalive timeout before the connection is
    /// considered dead.
    pub keepalive_grace: Duration,
    /// Keepalive when the welcome does not state one.
    pub default_keepalive: Duration,
    /// How long to wait for `session_welcome`.
    pub welcome_timeout: Duration,
}

impl Default for EventSubSettings {
    fn default() -> Self {
        Self {
            url: DEFAULT_EVENTSUB_URL.to_owned(),
            backoff: Backoff {
                initial: Duration::from_secs(2),
                max: Duration::from_secs(120),
                jitter: true,
            },
            keepalive_grace: Duration::from_secs(5),
            default_keepalive: Duration::from_secs(10),
            welcome_timeout: Duration::from_secs(10),
        }
    }
}

/// What the session covers right now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    /// Channels with `stream.online`.
    pub online: BTreeSet<String>,
    /// Channels with `channel.update`.
    pub update: BTreeSet<String>,
    /// Channels with `stream.offline`.
    pub offline: BTreeSet<String>,
    /// Cost spent.
    pub cost: u32,
    /// Twitch refused a subscription for cost before the plan was done.
    pub budget_hit: bool,
}

/// State of the EventSub session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventSubState {
    /// Not used (signed out, no channels, or disabled).
    Off,
    /// Connecting or subscribing.
    Connecting,
    /// Subscribed.
    Connected(Coverage),
    /// Lost; the next attempt is in `delay`.
    Reconnecting {
        /// Attempt number.
        attempt: u32,
        /// Wait before it.
        delay: Duration,
        /// Why the last connection ended.
        reason: String,
    },
    /// Gave up (sign-in gone, authorisation revoked); polling only.
    Stopped {
        /// Why.
        reason: String,
    },
}

pub(crate) enum EsMsg {
    Event(TimelineEvent),
    State(EventSubState),
}

enum End {
    Gone,
    Fatal(String),
    Lost { reason: String, healthy: bool },
}

enum Wait {
    Text(String),
    Closed(String),
    Timeout,
}

struct Session {
    settings: EventSubSettings,
    plan: SubscriptionPlan,
    api: Arc<dyn LiveApi>,
    connector: Arc<dyn WsConnector>,
    clock: Arc<dyn Clock>,
    tx: mpsc::Sender<EsMsg>,
    order: VecDeque<String>,
    seen: HashSet<String>,
}

/// Starts the session task; it ends when `tx`'s receiver is dropped or the
/// task is aborted.
pub(crate) fn spawn(
    settings: EventSubSettings,
    plan: SubscriptionPlan,
    api: Arc<dyn LiveApi>,
    connector: Arc<dyn WsConnector>,
    clock: Arc<dyn Clock>,
    tx: mpsc::Sender<EsMsg>,
) -> JoinHandle<()> {
    let session = Session {
        settings,
        plan,
        api,
        connector,
        clock,
        tx,
        order: VecDeque::new(),
        seen: HashSet::new(),
    };
    tokio::spawn(session.run())
}

impl Session {
    async fn state(&self, state: EventSubState) -> bool {
        self.tx.send(EsMsg::State(state)).await.is_ok()
    }

    async fn run(mut self) {
        let mut attempt: u32 = 0;
        loop {
            if !self.state(EventSubState::Connecting).await {
                return;
            }
            let url = self.settings.url.clone();
            let end = match self.connector.connect(&url).await {
                Ok(conn) => self.session(conn).await,
                Err(e) => End::Lost {
                    reason: e.to_string(),
                    healthy: false,
                },
            };
            let (reason, healthy) = match end {
                End::Gone => return,
                End::Fatal(reason) => {
                    let _ = self.state(EventSubState::Stopped { reason }).await;
                    return;
                }
                End::Lost { reason, healthy } => (reason, healthy),
            };
            attempt = if healthy { 1 } else { attempt + 1 };
            let delay = self.settings.backoff.delay(attempt);
            let state = EventSubState::Reconnecting {
                attempt,
                delay,
                reason,
            };
            if !self.state(state).await {
                return;
            }
            self.clock.sleep(delay).await;
        }
    }

    async fn wait(&self, conn: &mut dyn WsConnection, timeout: Duration) -> Wait {
        let frame = tokio::select! {
            biased;
            frame = conn.recv_text() => frame,
            () = self.clock.sleep(timeout) => return Wait::Timeout,
        };
        match frame {
            None => Wait::Closed("connection closed by Twitch".to_owned()),
            Some(Err(e)) => Wait::Closed(e.to_string()),
            Some(Ok(text)) => Wait::Text(text),
        }
    }

    async fn welcome(&self, conn: &mut dyn WsConnection) -> Result<(String, Duration), End> {
        loop {
            match self.wait(conn, self.settings.welcome_timeout).await {
                Wait::Text(text) => {
                    if let Ok(EventSubFrame::Welcome {
                        session_id,
                        keepalive_timeout,
                    }) = parse_frame(&text)
                    {
                        let keepalive =
                            keepalive_timeout.unwrap_or(self.settings.default_keepalive);
                        return Ok((session_id, keepalive));
                    }
                }
                Wait::Closed(reason) => {
                    return Err(End::Lost {
                        reason,
                        healthy: false,
                    });
                }
                Wait::Timeout => {
                    return Err(End::Lost {
                        reason: "no session_welcome from Twitch".to_owned(),
                        healthy: false,
                    });
                }
            }
        }
    }

    async fn subscribe(&self, session_id: &str) -> Result<Coverage, End> {
        let mut cov = Coverage::default();
        for sub in &self.plan.subs {
            // `channel.update` and `stream.offline` only make sense with
            // `stream.online` of the same channel.
            if sub.kind != SUB_ONLINE && !cov.online.contains(&sub.user_id) {
                continue;
            }
            match self.api.subscribe(sub.request(session_id)).await {
                Ok(_) | Err(LiveError::Conflict) => {}
                Err(LiveError::Unauthorized(m)) => {
                    return Err(End::Fatal(format!("Twitch sign-in is not valid: {m}")));
                }
                Err(LiveError::BudgetExceeded | LiveError::RateLimited(_)) => {
                    cov.budget_hit = true;
                    break;
                }
                // This channel stays on polling.
                Err(_) => continue,
            }
            cov.cost += 1;
            let set = match sub.kind {
                SUB_ONLINE => &mut cov.online,
                SUB_UPDATE => &mut cov.update,
                SUB_OFFLINE => &mut cov.offline,
                _ => continue,
            };
            set.insert(sub.user_id.clone());
        }
        Ok(cov)
    }

    fn first_time(&mut self, message_id: &str) -> bool {
        if message_id.is_empty() {
            return true;
        }
        if !self.seen.insert(message_id.to_owned()) {
            return false;
        }
        self.order.push_back(message_id.to_owned());
        if self.order.len() > DEDUP_WINDOW
            && let Some(old) = self.order.pop_front()
        {
            self.seen.remove(&old);
        }
        true
    }

    async fn session(&mut self, conn: Box<dyn WsConnection>) -> End {
        let mut conn = conn;
        let (session_id, mut keepalive) = match self.welcome(conn.as_mut()).await {
            Ok(w) => w,
            Err(end) => {
                conn.close().await;
                return end;
            }
        };
        let coverage = match self.subscribe(&session_id).await {
            Ok(c) => c,
            Err(end) => {
                conn.close().await;
                return end;
            }
        };
        if !self.state(EventSubState::Connected(coverage)).await {
            return End::Gone;
        }
        loop {
            let timeout = keepalive + self.settings.keepalive_grace;
            let text = match self.wait(conn.as_mut(), timeout).await {
                Wait::Text(t) => t,
                Wait::Closed(reason) => {
                    conn.close().await;
                    return End::Lost {
                        reason,
                        healthy: true,
                    };
                }
                Wait::Timeout => {
                    conn.close().await;
                    return End::Lost {
                        reason: "keepalive timeout".to_owned(),
                        healthy: true,
                    };
                }
            };
            let Ok(frame) = parse_frame(&text) else {
                continue;
            };
            match frame {
                EventSubFrame::Notification { message_id, .. } => {
                    if !self.first_time(&message_id) {
                        continue;
                    }
                    if let Ok(Some(event)) = parse_eventsub_message(text.as_bytes())
                        && self.tx.send(EsMsg::Event(event)).await.is_err()
                    {
                        return End::Gone;
                    }
                }
                EventSubFrame::Reconnect { reconnect_url } => {
                    // Subscriptions move with the session: open the new
                    // URL, wait for its welcome, then drop the old socket.
                    let moved = match self.connector.connect(&reconnect_url).await {
                        Ok(mut next) => match self.welcome(next.as_mut()).await {
                            Ok((_, k)) => Ok((next, k)),
                            Err(end) => Err(end),
                        },
                        Err(e) => Err(End::Lost {
                            reason: e.to_string(),
                            healthy: true,
                        }),
                    };
                    conn.close().await;
                    match moved {
                        Ok((next, k)) => {
                            conn = next;
                            keepalive = k;
                        }
                        Err(End::Lost { reason, .. }) => {
                            return End::Lost {
                                reason,
                                healthy: true,
                            };
                        }
                        Err(end) => return end,
                    }
                }
                EventSubFrame::Revocation {
                    subscription_type,
                    status,
                } => {
                    if status == "authorization_revoked" {
                        conn.close().await;
                        return End::Fatal(format!("Twitch revoked {subscription_type}: {status}"));
                    }
                    // Other revocations (a channel was banned or renamed):
                    // polling still covers the channel.
                }
                EventSubFrame::Welcome { .. }
                | EventSubFrame::Keepalive
                | EventSubFrame::Unknown(_) => {}
            }
        }
    }
}
