// SPDX-License-Identifier: MPL-2.0
//! Playback against a Plex Media Server (NET-013, ADR-0134): what the
//! player tells the server while it plays, the user's "watched" marks and
//! Continue Watching, and the server's transcoder for the rare file this
//! device should not play as is.
//!
//! - **Timeline** ([`Server::timeline`]): `GET /:/timeline` with the
//!   item, its state (`playing`, `paused`, `stopped`) and the position.
//!   This is what makes Plex Web show the item under "Continue Watching"
//!   at our position; the server marks an item watched by itself when a
//!   timeline goes past its threshold (90 % by default).
//! - **Watched marks**: [`Server::scrobble`] and [`Server::unscrobble`]
//!   (`GET /:/scrobble|unscrobble`).
//! - **Continue Watching**: [`Server::remove_from_continue_watching`]
//!   (`PUT /actions/removeFromContinueWatching`, with `/:/progress` at 0
//!   for servers without it).
//! - **Transcode on request** ([`Server::transcode_url`]): the universal
//!   transcoder's HLS (`/video/:/transcode/universal/start.m3u8`) for the
//!   player, and [`Server::stop_transcode`] when it closes. Direct play
//!   (the file's bytes through the stream proxy) stays the default.
//!
//! Every URL here carries the token in a header, except the transcode
//! URL, which libmpv opens itself and therefore holds it as
//! `X-Plex-Token`: never log it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::http::{MAX_JSON, Method, check, encode};
use crate::model::{Envelope, Item};
use crate::server::Server;
use crate::{Error, Result};

/// `identifier` of the library's items in the playback API.
pub const LIBRARY_IDENTIFIER: &str = "com.plexapp.plugins.library";

/// State of the player in a timeline report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    /// Playing.
    Playing,
    /// Paused.
    Paused,
    /// Closed, or moved on to another item.
    Stopped,
}

impl PlayState {
    /// `playing`, `paused` or `stopped`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Playing => "playing",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
        }
    }
}

/// One timeline report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timeline {
    /// The item (`ratingKey`).
    pub rating_key: String,
    /// What the player does.
    pub state: PlayState,
    /// Position, ms.
    pub position_ms: i64,
    /// Duration, ms, if known.
    pub duration_ms: Option<i64>,
    /// The transcode session, when the player plays the server's HLS.
    pub session: Option<String>,
}

/// What the transcoder makes of an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscodeRequest {
    /// The item (`ratingKey`).
    pub rating_key: String,
    /// Its version (`mediaIndex`).
    pub media_index: usize,
    /// The file of the version (`partIndex`).
    pub part_index: usize,
    /// Upper bound of the video bit rate, kbit/s; `None`: the source's.
    pub max_bitrate_kbps: Option<u32>,
    /// Largest picture, width × height; `None`: the source's.
    pub resolution: Option<(u32, u32)>,
    /// Allow copying the video when it fits (remux only); `false` makes
    /// the server re-encode it.
    pub direct_stream: bool,
    /// Session id ([`new_session_id`]), also used to stop it.
    pub session: String,
}

/// A new transcode session id (32 hex digits, unique in this process and
/// practically unique across devices).
#[must_use]
pub fn new_session_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let mixed = (u128::from(std::process::id()) << 64) ^ u128::from(n).rotate_left(40) ^ nanos;
    format!("{mixed:032x}")
}

/// Whether `uri` points into a private network (LAN), for the
/// transcoder's `location`: plain private IPs, `localhost` and the
/// `a-b-c-d.<hash>.plex.direct` names of private IPs.
#[must_use]
pub fn is_lan_address(uri: &str) -> bool {
    let rest = uri.split_once("://").map_or(uri, |(_, r)| r);
    let hostport = rest.split('/').next().unwrap_or("");
    let host = match hostport.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => hostport.split(':').next().unwrap_or(""),
    };
    let host = host.to_ascii_lowercase();
    if host == "localhost" {
        return true;
    }
    // `192-168-1-2.abc.plex.direct` or `fd00--1.abc.plex.direct`.
    let candidate = match host.strip_suffix(".plex.direct") {
        Some(name) => {
            let first = name.split('.').next().unwrap_or("");
            if first.contains("--") || first.matches('-').count() > 3 {
                first.replace('-', ":")
            } else {
                first.replace('-', ".")
            }
        }
        None => host,
    };
    match candidate.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        Ok(std::net::IpAddr::V6(ip)) => {
            let first = ip.segments()[0];
            ip.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

/// Whether a file of `bitrate_kbps` should go through the transcoder:
/// always when asked (`force`), or when it is above the user's bound
/// `max_kbps`. Everything else plays directly: libmpv decodes nearly every
/// codec and container (ADR-0134).
#[must_use]
pub fn needs_transcode(bitrate_kbps: Option<i64>, force: bool, max_kbps: Option<u32>) -> bool {
    force || matches!((bitrate_kbps, max_kbps), (Some(b), Some(m)) if m > 0 && b > i64::from(m))
}

impl Server {
    fn call(&self, method: Method, path_query: &str) -> Result<()> {
        check(self.send_as(method, path_query, None, MAX_JSON)?).map(|_| ())
    }

    /// Reports the player's state of an item: Plex keeps the position
    /// (Continue Watching on every client of the user) and marks the item
    /// watched once past its threshold.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] without an item; network errors.
    pub fn timeline(&self, t: &Timeline) -> Result<()> {
        let key = rating_key(&t.rating_key)?;
        let mut q = format!(
            "/:/timeline?ratingKey={key}&key={}&identifier={LIBRARY_IDENTIFIER}&state={}&time={}",
            encode(&format!("/library/metadata/{}", t.rating_key.trim())),
            t.state.as_str(),
            t.position_ms.max(0)
        );
        if let Some(d) = t.duration_ms.filter(|d| *d > 0) {
            q.push_str(&format!("&duration={d}"));
        }
        if let Some(s) = &t.session {
            q.push_str(&format!("&X-Plex-Session-Identifier={}", encode(s)));
        }
        self.call(Method::Get, &q)
    }

    /// Marks an item watched (its view count goes up, the position is
    /// cleared).
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] without an item; network errors.
    pub fn scrobble(&self, rating_key_: &str) -> Result<()> {
        let key = rating_key(rating_key_)?;
        self.call(
            Method::Get,
            &format!("/:/scrobble?key={key}&identifier={LIBRARY_IDENTIFIER}"),
        )
    }

    /// Marks an item not watched.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] without an item; network errors.
    pub fn unscrobble(&self, rating_key_: &str) -> Result<()> {
        let key = rating_key(rating_key_)?;
        self.call(
            Method::Get,
            &format!("/:/unscrobble?key={key}&identifier={LIBRARY_IDENTIFIER}"),
        )
    }

    /// Sets the user's position of an item without a playing session.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] without an item; network errors.
    pub fn set_progress(&self, rating_key_: &str, position_ms: i64) -> Result<()> {
        let key = rating_key(rating_key_)?;
        self.call(
            Method::Get,
            &format!(
                "/:/progress?key={key}&identifier={LIBRARY_IDENTIFIER}&time={}&state=stopped",
                position_ms.max(0)
            ),
        )
    }

    /// Takes an item out of the user's Continue Watching; on a server
    /// without that action the position is set to 0, which does the same.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] without an item; network errors.
    pub fn remove_from_continue_watching(&self, rating_key_: &str) -> Result<()> {
        let key = rating_key(rating_key_)?;
        match self.call(
            Method::Put,
            &format!("/actions/removeFromContinueWatching?ratingKey={key}"),
        ) {
            Err(Error::NotFound | Error::Status { .. }) => self.set_progress(rating_key_, 0),
            other => other,
        }
    }

    /// The item `rating_key` as the server has it now (with the user's
    /// position, view count and last view); `None` if it is gone.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] without an item; network errors.
    pub fn metadata(&self, rating_key_: &str) -> Result<Option<Item>> {
        let key = rating_key(rating_key_)?;
        let r = match check(self.send(&format!("/library/metadata/{key}"), None, MAX_JSON)?) {
            Err(Error::NotFound) => return Ok(None),
            r => r?,
        };
        let e: Envelope<Item> = crate::http::parse(&r.body)?;
        Ok(e.container.metadata.into_iter().next())
    }

    /// The HLS URL of the universal transcoder for `r` on the connection
    /// in use, for the player. It carries the token and the `X-Plex-*`
    /// values as query parameters (the player sends no headers of ours):
    /// never log it.
    #[must_use]
    pub fn transcode_url(&self, r: &TranscodeRequest) -> String {
        let base = self.active_address();
        let info = self.info();
        let mut q: Vec<(&str, String)> = vec![
            ("path", format!("/library/metadata/{}", r.rating_key.trim())),
            ("mediaIndex", r.media_index.to_string()),
            ("partIndex", r.part_index.to_string()),
            ("protocol", "hls".into()),
            ("fastSeek", "1".into()),
            ("copyts", "1".into()),
            ("directPlay", "0".into()),
            ("directStream", u8::from(r.direct_stream).to_string()),
            ("directStreamAudio", "1".into()),
            ("subtitles", "auto".into()),
            ("videoQuality", "100".into()),
            (
                "location",
                if is_lan_address(base) { "lan" } else { "wan" }.into(),
            ),
        ];
        if let Some(b) = r.max_bitrate_kbps.filter(|b| *b > 0) {
            q.push(("maxVideoBitrate", b.to_string()));
        }
        if let Some((w, h)) = r.resolution.filter(|(w, h)| *w > 0 && *h > 0) {
            q.push(("videoResolution", format!("{w}x{h}")));
        }
        q.extend([
            ("session", r.session.clone()),
            ("X-Plex-Session-Identifier", r.session.clone()),
            ("X-Plex-Client-Profile-Name", "Generic".into()),
            (
                "X-Plex-Client-Profile-Extra",
                "add-transcode-target(type=videoProfile&context=streaming&protocol=hls\
                 &container=mpegts&videoCodec=h264&audioCodec=aac,mp3)"
                    .into(),
            ),
            ("X-Plex-Product", info.product.clone()),
            ("X-Plex-Version", info.version.clone()),
            ("X-Plex-Client-Identifier", info.client_id.clone()),
            ("X-Plex-Platform", info.platform.clone()),
            ("X-Plex-Device", info.device.clone()),
            ("X-Plex-Token", self.token().expose().to_owned()),
        ]);
        let query: Vec<String> = q
            .iter()
            .map(|(k, v)| format!("{k}={}", encode(v)))
            .collect();
        format!(
            "{base}/video/:/transcode/universal/start.m3u8?{}",
            query.join("&")
        )
    }

    /// Stops transcode session `session` (the player closed).
    ///
    /// # Errors
    ///
    /// Network errors.
    pub fn stop_transcode(&self, session: &str) -> Result<()> {
        self.call(
            Method::Get,
            &format!(
                "/video/:/transcode/universal/stop?session={}",
                encode(session)
            ),
        )
    }
}

/// `rating_key`, trimmed and encoded; [`Error::Invalid`] when empty.
fn rating_key(rating_key: &str) -> Result<String> {
    let k = rating_key.trim();
    if k.is_empty() {
        return Err(Error::Invalid("no item".into()));
    }
    Ok(encode(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_addresses() {
        for lan in [
            "http://192.168.1.2:32400",
            "https://192-168-1-2.abc.plex.direct:32400",
            "https://10-0-0-5.abc.plex.direct:32400",
            "http://[fd00::1]:32400",
            "https://fd00--1.abc.plex.direct:32400",
            "http://127.0.0.1:1",
            "http://localhost:32400",
        ] {
            assert!(is_lan_address(lan), "{lan}");
        }
        for wan in [
            "https://1-2-3-4.abc.plex.direct:32400",
            "https://138-201-1-2.abc.plex.direct:8443",
            "http://8.8.8.8:32400",
            "https://plex.example.org",
        ] {
            assert!(!is_lan_address(wan), "{wan}");
        }
    }

    #[test]
    fn transcodes_only_when_asked_or_too_big() {
        assert!(!needs_transcode(Some(80_000), false, None));
        assert!(!needs_transcode(None, false, Some(4000)));
        assert!(!needs_transcode(Some(3000), false, Some(4000)));
        assert!(needs_transcode(Some(8000), false, Some(4000)));
        assert!(needs_transcode(None, true, None));
    }

    #[test]
    fn session_ids_differ() {
        let a = new_session_id();
        let b = new_session_id();
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
