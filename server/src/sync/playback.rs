// SPDX-License-Identifier: MPL-2.0
//! The last `pb.update` per `(profile, key)`, kept in memory for a few
//! minutes so a device that just connected knows the fresh position at once
//! (spec §4.5, §7). Never written to disk.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::extract::ws::Utf8Bytes;
use pa_model::MediaKey;
use pa_sync::Hlc;
use pa_sync::handoff::DEFAULT_TTL_MS;
use pa_sync::proto::{PlayState, PlaybackUpdate};

struct Entry {
    hlc: Hlc,
    seen: Instant,
    frame: Utf8Bytes,
    /// `playing` or `paused`, for "someone is watching" (SRV-019).
    watching: bool,
    /// How long the state stays current after `seen` (`ttl_ms`).
    live: Duration,
}

pub(super) struct PlaybackCache {
    ttl: Duration,
    max: usize,
    entries: HashMap<(String, MediaKey), Entry>,
}

impl PlaybackCache {
    pub(super) fn new(ttl: Duration, max: usize) -> Self {
        Self {
            ttl,
            max: max.max(1),
            entries: HashMap::new(),
        }
    }

    /// Keeps `frame` (the encoded `update`) unless a newer update by HLC is
    /// already kept for the same `(profile, key)`.
    pub(super) fn put(&mut self, update: &PlaybackUpdate, frame: Utf8Bytes, now: Instant) {
        let key = (update.profile.clone(), update.key.clone());
        if let Some(old) = self.entries.get(&key)
            && old.hlc >= update.hlc
            && now.duration_since(old.seen) < self.ttl
        {
            return;
        }
        if !self.entries.contains_key(&key) && self.entries.len() >= self.max {
            self.prune(now);
            if self.entries.len() >= self.max
                && let Some(oldest) = self
                    .entries
                    .iter()
                    .min_by_key(|(_, e)| e.seen)
                    .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(
            key,
            Entry {
                hlc: update.hlc.clone(),
                seen: now,
                frame,
                // `stopped` ends the session; an unknown newer state is
                // treated as `paused` (spec §2).
                watching: update.state != PlayState::Stopped,
                live: Duration::from_millis(update.ttl_ms.unwrap_or(DEFAULT_TTL_MS)),
            },
        );
    }

    /// Frames still within the TTL, oldest first; drops the expired ones.
    pub(super) fn fresh(&mut self, now: Instant) -> Vec<Utf8Bytes> {
        self.prune(now);
        let mut kept: Vec<&Entry> = self.entries.values().collect();
        kept.sort_by_key(|e| e.seen);
        kept.into_iter().map(|e| e.frame.clone()).collect()
    }

    /// Titles someone is watching right now: the last update is `playing`
    /// or `paused` (a paused movie must not be rebooted away) and younger
    /// than its own TTL (`ttl_ms`, measured from when the hub got it, so a
    /// device with a wrong clock does not matter) and than the replay time.
    pub(super) fn watching(&self, now: Instant) -> usize {
        self.entries
            .values()
            .filter(|e| {
                let age = now.saturating_duration_since(e.seen);
                e.watching && age < e.live && age < self.ttl
            })
            .count()
    }

    fn prune(&mut self, now: Instant) {
        let ttl = self.ttl;
        self.entries
            .retain(|_, e| now.saturating_duration_since(e.seen) < ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(key: &str, wall_ms: u64, device: &str) -> PlaybackUpdate {
        PlaybackUpdate {
            aliases: Vec::new(),
            profile: "default".to_owned(),
            key: key.parse().unwrap(),
            position_ms: wall_ms,
            duration_ms: None,
            rate: 1.0,
            state: PlayState::Playing,
            hlc: Hlc {
                wall_ms,
                counter: 0,
                device: device.to_owned(),
            },
            ttl_ms: None,
        }
    }

    fn put(cache: &mut PlaybackCache, u: &PlaybackUpdate, now: Instant) {
        cache.put(u, format!("{}@{}", u.hlc.device, u.hlc.wall_ms).into(), now);
    }

    #[test]
    fn keeps_newest_per_title_until_ttl() {
        let t0 = Instant::now();
        let mut cache = PlaybackCache::new(Duration::from_secs(60), 100);
        put(&mut cache, &update("tmdb:movie:603", 2_000, "phone"), t0);
        // Older by HLC (a late frame from another device): ignored.
        put(&mut cache, &update("tmdb:movie:603", 1_000, "tv"), t0);
        put(&mut cache, &update("tmdb:movie:604", 1_500, "tv"), t0);
        let fresh = cache.fresh(t0);
        assert_eq!(fresh.len(), 2);
        assert!(fresh.iter().any(|f| f.as_str() == "phone@2000"));
        assert!(fresh.iter().all(|f| f.as_str() != "tv@1000"));
        assert!(cache.fresh(t0 + Duration::from_secs(61)).is_empty());
    }

    #[test]
    fn bounded() {
        let t0 = Instant::now();
        let mut cache = PlaybackCache::new(Duration::from_secs(60), 2);
        for (i, id) in [1, 2, 3].into_iter().enumerate() {
            let key = format!("tmdb:movie:{id}");
            put(
                &mut cache,
                &update(&key, 1_000, "tv"),
                t0 + Duration::from_secs(i as u64),
            );
        }
        let fresh = cache.fresh(t0 + Duration::from_secs(3));
        assert_eq!(fresh.len(), 2);
    }

    #[test]
    fn playing_until_its_ttl() {
        let t0 = Instant::now();
        let mut cache = PlaybackCache::new(Duration::from_secs(600), 100);
        assert_eq!(cache.watching(t0), 0);
        let mut u = update("tmdb:movie:603", 1_000, "tv");
        u.ttl_ms = Some(15_000);
        put(&mut cache, &u, t0);
        assert_eq!(cache.watching(t0 + Duration::from_secs(14)), 1);
        assert_eq!(cache.watching(t0 + Duration::from_secs(15)), 0);
        // Without `ttl_ms`: the protocol default (15 s).
        let t1 = t0 + Duration::from_secs(20);
        let mut u = update("tmdb:movie:604", 1_000, "tv");
        u.ttl_ms = None;
        put(&mut cache, &u, t1);
        assert_eq!(cache.watching(t1 + Duration::from_secs(14)), 1);
        assert_eq!(cache.watching(t1 + Duration::from_secs(15)), 0);
    }

    #[test]
    fn paused_counts_until_its_ttl_stopped_does_not() {
        let t0 = Instant::now();
        let mut cache = PlaybackCache::new(Duration::from_secs(600), 100);
        let mut u = update("tmdb:movie:603", 1_000, "tv");
        put(&mut cache, &u, t0);
        assert_eq!(cache.watching(t0), 1);
        u.state = PlayState::Paused;
        u.ttl_ms = Some(600_000);
        u.hlc.wall_ms = 2_000;
        put(&mut cache, &u, t0);
        assert_eq!(cache.watching(t0 + Duration::from_secs(599)), 1);
        assert_eq!(cache.watching(t0 + Duration::from_secs(600)), 0);
        u.state = PlayState::Stopped;
        u.hlc.wall_ms = 3_000;
        put(&mut cache, &u, t0);
        assert_eq!(cache.watching(t0), 0);
    }
}
