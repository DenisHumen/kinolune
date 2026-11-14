// SPDX-License-Identifier: MPL-2.0
//! Which channels get EventSub subscriptions and which are polled.
//!
//! A WebSocket session authorised by a user token has a cost budget
//! (`max_total_cost`, 10 at the time of writing): every subscription whose
//! condition is a broadcaster that did not authorise our app costs 1 —
//! `stream.online`, `stream.offline` and `channel.update` all do. So
//! EventSub covers about ten channels' `stream.online`, and polling covers
//! the rest (research 06 §1.2). The plan spends the budget in passes:
//!
//! 1. `stream.online` for channels with notifications on, in the given
//!    order (the app puts pinned and recently watched first), then for the
//!    other channels (their timelines feed chapters, TWI-008);
//! 2. `channel.update` for channels with `stream.online` (exact chapter
//!    boundaries instead of the poll interval);
//! 3. `stream.offline` for the same channels.
//!
//! Twitch may still answer `429` earlier (another session of the same user
//! and app spends the same budget); the session then stops subscribing and
//! the channels left without `stream.online` are polled.

use std::collections::{BTreeMap, BTreeSet};

use pa_twitch::EventSubSubscriptionRequest;

/// Default `max_total_cost` of a WebSocket session.
pub const DEFAULT_MAX_TOTAL_COST: u32 = 10;

/// `stream.online` v1.
pub const SUB_ONLINE: &str = "stream.online";
/// `channel.update` v2.
pub const SUB_UPDATE: &str = "channel.update";
/// `stream.offline` v1.
pub const SUB_OFFLINE: &str = "stream.offline";

/// A channel the monitor watches (a favourite or a follow).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedChannel {
    /// Broadcaster id.
    pub user_id: String,
    /// Login (for the notification's deep link).
    pub login: String,
    /// Display name (for the notification text).
    pub display_name: String,
    /// "X is live" notifications for this channel.
    pub notify: bool,
}

impl WatchedChannel {
    /// A channel with notifications on.
    pub fn new(user_id: &str, login: &str, display_name: &str) -> Self {
        Self {
            user_id: user_id.to_owned(),
            login: login.to_owned(),
            display_name: display_name.to_owned(),
            notify: true,
        }
    }

    /// The same channel with notifications set to `notify`.
    pub fn with_notify(mut self, notify: bool) -> Self {
        self.notify = notify;
        self
    }
}

/// One planned subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSub {
    /// Broadcaster id.
    pub user_id: String,
    /// Subscription type.
    pub kind: &'static str,
    /// Subscription version.
    pub version: &'static str,
}

impl PlannedSub {
    /// The request for a session.
    pub fn request(&self, session_id: &str) -> EventSubSubscriptionRequest {
        EventSubSubscriptionRequest {
            kind: self.kind.to_owned(),
            version: self.version.to_owned(),
            condition: BTreeMap::from([(
                "broadcaster_user_id".to_owned(),
                self.user_id.clone(),
            )]),
            session_id: session_id.to_owned(),
        }
    }
}

/// Subscriptions of one EventSub session, in the order they are created.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubscriptionPlan {
    /// Subscriptions, most important first.
    pub subs: Vec<PlannedSub>,
}

impl SubscriptionPlan {
    /// Spends `budget` (cost 1 per subscription) on `channels`.
    pub fn new(channels: &[WatchedChannel], budget: u32) -> Self {
        let mut seen = BTreeSet::new();
        let ordered: Vec<&WatchedChannel> = channels
            .iter()
            .filter(|c| c.notify)
            .chain(channels.iter().filter(|c| !c.notify))
            .filter(|c| !c.user_id.is_empty() && seen.insert(c.user_id.as_str()))
            .collect();
        let budget = budget as usize;
        let mut subs = Vec::new();
        let online: Vec<&str> = ordered
            .iter()
            .take(budget)
            .map(|c| c.user_id.as_str())
            .collect();
        for (kind, version) in [(SUB_ONLINE, "1"), (SUB_UPDATE, "2"), (SUB_OFFLINE, "1")] {
            for id in &online {
                if subs.len() == budget {
                    return Self { subs };
                }
                subs.push(PlannedSub {
                    user_id: (*id).to_owned(),
                    kind,
                    version,
                });
            }
        }
        Self { subs }
    }

    /// Channels whose `stream.online` is planned.
    pub fn online_channels(&self) -> BTreeSet<String> {
        self.channels_with(SUB_ONLINE)
    }

    /// Channels with a planned subscription of `kind`.
    pub fn channels_with(&self, kind: &str) -> BTreeSet<String> {
        self.subs
            .iter()
            .filter(|s| s.kind == kind)
            .map(|s| s.user_id.clone())
            .collect()
    }

    /// Total cost.
    pub fn cost(&self) -> u32 {
        u32::try_from(self.subs.len()).unwrap_or(u32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channels(n: usize) -> Vec<WatchedChannel> {
        (0..n)
            .map(|i| {
                WatchedChannel::new(&format!("{}", 100 + i), &format!("c{i}"), &format!("C{i}"))
            })
            .collect()
    }

    #[test]
    fn few_channels_get_all_three_subscriptions() {
        let plan = SubscriptionPlan::new(&channels(3), DEFAULT_MAX_TOTAL_COST);
        assert_eq!(plan.cost(), 9);
        assert_eq!(plan.online_channels().len(), 3);
        assert_eq!(plan.channels_with(SUB_UPDATE).len(), 3);
        assert_eq!(plan.channels_with(SUB_OFFLINE).len(), 3);
        assert_eq!(plan.subs[0].kind, SUB_ONLINE);
    }

    #[test]
    fn four_channels_spend_the_rest_on_updates() {
        let plan = SubscriptionPlan::new(&channels(4), DEFAULT_MAX_TOTAL_COST);
        assert_eq!(plan.cost(), 10);
        assert_eq!(plan.online_channels().len(), 4);
        assert_eq!(plan.channels_with(SUB_UPDATE).len(), 4);
        assert_eq!(plan.channels_with(SUB_OFFLINE).len(), 2);
    }

    #[test]
    fn many_channels_only_online_within_budget() {
        let plan = SubscriptionPlan::new(&channels(25), DEFAULT_MAX_TOTAL_COST);
        assert_eq!(plan.cost(), 10);
        assert_eq!(plan.online_channels().len(), 10);
        assert!(plan.channels_with(SUB_UPDATE).is_empty());
    }

    #[test]
    fn notify_channels_first_and_duplicates_dropped() {
        let mut list = channels(12);
        for c in &mut list[..4] {
            c.notify = false;
        }
        list.push(list[5].clone());
        let plan = SubscriptionPlan::new(&list, DEFAULT_MAX_TOTAL_COST);
        let online = plan.online_channels();
        // The 8 notify channels plus the first 2 silent ones.
        for c in &list[4..12] {
            assert!(online.contains(&c.user_id), "{}", c.user_id);
        }
        assert!(online.contains("100") && online.contains("101"));
        assert!(!online.contains("102"));
        assert_eq!(plan.cost(), 10);
    }

    #[test]
    fn request_has_broadcaster_condition() {
        let plan = SubscriptionPlan::new(&channels(1), 1);
        let r = plan.subs[0].request("sess");
        assert_eq!(r.kind, "stream.online");
        assert_eq!(r.version, "1");
        assert_eq!(r.session_id, "sess");
        assert_eq!(r.condition["broadcaster_user_id"], "100");
    }

    #[test]
    fn zero_budget_plans_nothing() {
        assert!(SubscriptionPlan::new(&channels(3), 0).subs.is_empty());
    }
}
