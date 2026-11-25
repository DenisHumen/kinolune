// SPDX-License-Identifier: MPL-2.0
//! Manual checks against real Twitch (TWI-014 follow-up,
//! `docs/dev/twitch-live-checks.md`). Never run in CI: every test is
//! `#[ignore]` and also returns early unless `PA_TWITCH_LIVE_TEST=1`.
//!
//! Nothing Twitch-specific is compiled in. The GQL client-id and the
//! persisted-query hashes come only from the environment at run time and go
//! into an unsigned test [`RemoteConfig`] built here:
//!
//! - `PA_TWITCH_GQL_CLIENT_ID`: public web-player client-id;
//! - `PA_TWITCH_HASH_PLAYBACK_ACCESS_TOKEN`, `PA_TWITCH_HASH_CHAPTERS`
//!   (`VideoPlayer_ChapterSelectButtonVideo`), optional
//!   `PA_TWITCH_HASH_VIDEO_METADATA`;
//! - `PA_TWITCH_VOD_ID`: a public VOD;
//! - optional `PA_TWITCH_PROXY_HOLD_SECS`: keep the loopback proxy running
//!   this long and print its URL, for a libmpv check from outside.
//!
//! No login, no Helix credentials, no ad or restriction bypass: requests
//! are those of the app. Output lists variants and chapters, never tokens,
//! signatures or media URLs.
//!
//! ```sh
//! PA_TWITCH_LIVE_TEST=1 PA_TWITCH_GQL_CLIENT_ID=… PA_TWITCH_HASH_…=… \
//!   PA_TWITCH_VOD_ID=… cargo test -p pa-twitch --features pa-flavor/full \
//!   --test real_twitch -- --ignored --nocapture --test-threads=1
//! ```
#![cfg(all(pa_twitch_native, feature = "reqwest"))]

use std::sync::Arc;
use std::time::Duration;

use pa_remote_config::{ClientContext, RemoteConfig};
use pa_twitch::chapters::{
    CHAPTERS_OPERATION, ChapterResolver, ChaptersGql, ChaptersGqlOff, VIDEO_METADATA_OPERATION,
    VodChaptersRequest,
};
use pa_twitch::playback::live::{LiveMode, LiveProxy, MediaPlaylist, ReqwestUpstream};
use pa_twitch::playback::{
    DeviceCodecs, Flavor, NativePlayback, PLAYBACK_ACCESS_TOKEN_OPERATION, PlaybackDecision,
    PlaybackTarget, QualityPreference, TwitchFeatures, TwitchPlayback,
};
use pa_twitch::testing::FixtureTransport;
use pa_twitch::{
    AccessToken, ClientId, HelixClient, HelixConfig, ReqwestTransport, Video, parse_rfc3339_ms,
};

fn enabled() -> bool {
    let on = std::env::var("PA_TWITCH_LIVE_TEST").as_deref() == Ok("1");
    if !on {
        eprintln!("skipped: set PA_TWITCH_LIVE_TEST=1 (see the module docs)");
    }
    on
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

fn ctx() -> ClientContext {
    ClientContext {
        app_version: "0.1.0".into(),
        platform: "linux".into(),
    }
}

/// An unsigned test config with GQL on and the values from the environment.
fn config() -> RemoteConfig {
    let mut queries = serde_json::Map::new();
    for (op, var) in [
        (
            PLAYBACK_ACCESS_TOKEN_OPERATION,
            "PA_TWITCH_HASH_PLAYBACK_ACCESS_TOKEN",
        ),
        (CHAPTERS_OPERATION, "PA_TWITCH_HASH_CHAPTERS"),
        (VIDEO_METADATA_OPERATION, "PA_TWITCH_HASH_VIDEO_METADATA"),
    ] {
        if let Ok(hash) = std::env::var(var) {
            queries.insert(op.into(), hash.into());
        }
    }
    let json = serde_json::json!({
        "schema": 1,
        "serial": 1,
        "flags": {
            "twitch.gql_playback": true,
            "twitch.chapters_gql": true,
            "twitch.chat_replay": false,
            "twitch.archive": false,
        },
        "kill_switches": [],
        "twitch": {
            "gql_client_id": env("PA_TWITCH_GQL_CLIENT_ID"),
            "persisted_queries": queries,
            "player": "native",
        },
    });
    RemoteConfig::parse(json.to_string().as_bytes()).unwrap()
}

fn transport() -> Arc<ReqwestTransport> {
    Arc::new(ReqwestTransport::new(Duration::from_secs(20)).unwrap())
}

fn vod() -> PlaybackTarget {
    PlaybackTarget::vod(&env("PA_TWITCH_VOD_ID")).unwrap()
}

async fn resolve(codecs: DeviceCodecs) -> NativePlayback {
    let features = TwitchFeatures::resolve_for(&config(), &ctx(), Flavor::Full);
    let playback = TwitchPlayback::new(features, transport());
    match playback.resolve(&vod(), codecs).await {
        Ok(PlaybackDecision::Native(n)) => n,
        other => panic!("expected native playback, got {other:?}"),
    }
}

fn print_variants(n: &NativePlayback) {
    for v in &n.playlist.variants {
        eprintln!(
            "  {:<11} group={:<10} source={:<5} audio_only={:<5} bw={:>8} res={:<10} fps={:<7} codecs={}",
            v.quality,
            v.group_id,
            v.is_source,
            v.audio_only,
            v.bandwidth,
            v.resolution.map_or("-".into(), |(w, h)| format!("{w}x{h}")),
            v.frame_rate.map_or("-".into(), |f| format!("{f:.3}")),
            v.codecs.join(","),
        );
    }
}

/// TWI-004: token → usher master playlist → variants, for H.264 only and
/// for a device that also decodes AV1 and HEVC.
#[tokio::test]
#[ignore = "real Twitch; PA_TWITCH_LIVE_TEST=1 and env values"]
async fn real_vod_resolve() {
    if !enabled() {
        return;
    }
    for codecs in [
        DeviceCodecs::H264_ONLY,
        DeviceCodecs {
            av1: true,
            hevc: true,
        },
    ] {
        let n = resolve(codecs).await;
        let token: serde_json::Value = serde_json::from_str(&n.access_token.value).unwrap();
        eprintln!(
            "supported_codecs={} variants={} token: forbidden={} max_resolution={} https_required={}",
            codecs.supported_codecs(),
            n.playlist.variants.len(),
            token["authorization"]["forbidden"],
            token["maximum_resolution"],
            token["https_required"],
        );
        print_variants(&n);
        assert!(n.master_url.contains("supported_codecs="));
        assert!(n.playlist.source().is_some_and(|v| v.is_source));
        assert!(n.playlist.audio_only().is_some());
        if codecs == DeviceCodecs::H264_ONLY {
            assert!(
                n.playlist
                    .variants
                    .iter()
                    .filter(|v| !v.audio_only)
                    .all(|v| v.codecs.iter().any(|c| c.starts_with("avc1."))),
                "H.264 only was asked for"
            );
        }
    }
}

/// TWI-014: the VOD through the loopback proxy (our FFmpeg has no TLS).
/// Reads the playlist and the first, middle and last segment through the
/// proxy, at the lowest video quality; with `PA_TWITCH_PROXY_HOLD_SECS`
/// keeps serving for a player.
#[tokio::test]
#[ignore = "real Twitch; PA_TWITCH_LIVE_TEST=1 and env values"]
async fn real_vod_through_the_proxy() {
    if !enabled() {
        return;
    }
    let n = resolve(DeviceCodecs::H264_ONLY).await;
    let quality = std::env::var("PA_TWITCH_MAX_HEIGHT")
        .ok()
        .and_then(|h| h.parse().ok())
        .unwrap_or(160);
    let variant = n
        .playlist
        .select(QualityPreference::AtMost(quality))
        .unwrap();
    eprintln!("variant {}", variant.quality);
    let proxy = LiveProxy::start(
        Arc::new(ReqwestUpstream::new().unwrap()),
        &variant.url,
        LiveMode::Stable,
    )
    .unwrap();
    let client = reqwest::Client::new();
    let url = proxy.playlist_url();
    let bytes = client
        .get(&url)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let playlist = MediaPlaylist::parse(&text).unwrap();
    let total: f64 = playlist.segments.iter().map(|s| s.duration).sum();
    eprintln!(
        "segments={} ended={} total={total:.3}s",
        playlist.segments.len(),
        playlist.ended
    );
    assert!(playlist.ended, "a VOD playlist ends");
    let base = &url[..url.rfind('/').unwrap() + 1];
    let n_seg = playlist.segments.len();
    for i in [0, n_seg / 2, n_seg - 1] {
        let s = &playlist.segments[i];
        let r = client.get(format!("{base}{}", s.uri)).send().await.unwrap();
        let status = r.status().as_u16();
        let bytes = r.bytes().await.unwrap();
        eprintln!(
            "segment #{} ({}) status={status} bytes={}",
            s.sequence,
            s.uri,
            bytes.len()
        );
        assert_eq!(status, 200, "segment {i} through the proxy");
        assert_eq!(bytes.first(), Some(&0x47), "MPEG-TS sync byte");
    }
    if let Some(secs) = std::env::var("PA_TWITCH_PROXY_HOLD_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        eprintln!("PROXY_URL {url}");
        tokio::time::sleep(Duration::from_secs(secs)).await;
        let t = proxy.report(0.0, 1.0).totals;
        eprintln!("served segments={} bytes={}", t.segments, t.bytes);
    }
}

/// The VOD as Helix would describe it, from the GQL `VideoMetadata`
/// answer (we have no Helix credentials).
async fn video_from_metadata() -> Video {
    // Installs the ring provider for the plain reqwest client below.
    let _ = transport();
    let hash = env("PA_TWITCH_HASH_VIDEO_METADATA");
    let id = env("PA_TWITCH_VOD_ID");
    let body = serde_json::json!({
        "operationName": VIDEO_METADATA_OPERATION,
        "variables": { "channelLogin": "", "videoID": id },
        "extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } },
    });
    let bytes = reqwest::Client::new()
        .post("https://gql.twitch.tv/gql")
        .header("Client-ID", env("PA_TWITCH_GQL_CLIENT_ID"))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let answer: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let v = &answer["data"]["video"];
    let secs = v["lengthSeconds"].as_u64().unwrap();
    eprintln!(
        "VideoMetadata: length={secs}s created={} type={} game={}",
        v["createdAt"], v["broadcastType"], v["game"]["displayName"]
    );
    serde_json::from_value(serde_json::json!({
        "id": id,
        "user_id": v["owner"]["id"],
        "user_login": v["owner"]["login"],
        "user_name": v["owner"]["displayName"],
        "created_at": v["createdAt"],
        "type": v["broadcastType"].as_str().unwrap().to_ascii_lowercase(),
        "duration": format!("{}h{}m{}s", secs / 3600, secs / 60 % 60, secs % 60),
    }))
    .unwrap()
}

/// A Helix client that fails any request: the fallback must not need one
/// for an old VOD.
fn offline_helix() -> (HelixClient, Arc<FixtureTransport>) {
    let transport = Arc::new(FixtureTransport::new());
    let helix = HelixClient::new(
        HelixConfig::new(ClientId::new("testclientid0000000000000000000").unwrap()),
        transport.clone(),
    );
    helix.set_access_token(AccessToken::new("test-user-token-not-real"));
    (helix, transport)
}

/// TWI-008: chapters from GQL behind the config flag, without and with the
/// `VideoMetadata` hash, and the fallback with GQL off.
#[tokio::test]
#[ignore = "real Twitch; PA_TWITCH_LIVE_TEST=1 and env values"]
async fn real_vod_chapters() {
    if !enabled() {
        return;
    }
    let video = video_from_metadata().await;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let age_h = (now_ms - parse_rfc3339_ms(&video.created_at).unwrap()) / 3_600_000;
    let request = VodChaptersRequest {
        video: &video,
        timeline: None,
        stored: None,
        now_ms,
    };
    let mut no_metadata = config();
    no_metadata
        .twitch
        .persisted_queries
        .remove(VIDEO_METADATA_OPERATION);
    for (name, cfg) in [
        ("moments only", no_metadata),
        ("with VideoMetadata", config()),
    ] {
        let gql = ChaptersGql::resolve_for(&cfg, &ctx(), Flavor::Full);
        assert!(gql.is_on());
        let (helix, offline) = offline_helix();
        let resolved = ChapterResolver::new(gql, transport())
            .resolve(&helix, request)
            .await
            .unwrap();
        eprintln!("GQL {name}: failure={:?}", resolved.gql_failure);
        for c in resolved.chapters.iter().flatten() {
            eprintln!(
                "  {}..{:?} ms {} ({}) source={}",
                c.start_ms,
                c.end_ms,
                c.game.name,
                c.game.id,
                c.source.as_str()
            );
        }
        assert_eq!(resolved.gql_failure, None);
        assert!(offline.requests().is_empty(), "no Helix call");
    }
    // GQL off: only the fallback, which needs Helix and a fresh archive.
    let (helix, offline) = offline_helix();
    let resolved = ChapterResolver::new(ChaptersGql::Off(ChaptersGqlOff::FlagOff), transport())
        .resolve(&helix, request)
        .await;
    eprintln!(
        "GQL off: VOD age {age_h} h, fallback {:?}, Helix requests {}",
        resolved.as_ref().map(|r| r.chapters.as_ref().map(Vec::len)),
        offline.requests().len()
    );
}

/// TWI-016: the chat of a public VOD by offset around 1:00 and the page
/// after it. Needs `PA_TWITCH_HASH_COMMENTS` (`VideoCommentsByOffsetOrCursor`).
/// Prints counts only, never message texts or user names.
#[tokio::test]
#[ignore = "real Twitch; PA_TWITCH_LIVE_TEST=1 and env values"]
async fn real_twitch_chat_replay() {
    use pa_twitch::chat_replay::{COMMENTS_OPERATION, ChatReplayGql, OnlineReplay};
    if !enabled() {
        return;
    }
    let mut queries = serde_json::Map::new();
    queries.insert(
        COMMENTS_OPERATION.into(),
        env("PA_TWITCH_HASH_COMMENTS").into(),
    );
    let json = serde_json::json!({
        "schema": 1,
        "serial": 1,
        "flags": { "twitch.gql_playback": true, "twitch.chat_replay": true },
        "kill_switches": [],
        "twitch": {
            "gql_client_id": env("PA_TWITCH_GQL_CLIENT_ID"),
            "persisted_queries": queries,
            "player": "native",
        },
    });
    let config = RemoteConfig::parse(json.to_string().as_bytes()).unwrap();
    let gql = ChatReplayGql::resolve_for(&config, &ctx(), Flavor::Full);
    let replay = OnlineReplay::new(&gql, &env("PA_TWITCH_VOD_ID"), transport()).unwrap();
    let first = replay.ensure(60_000).await.unwrap();
    let ahead = replay
        .ensure(first.to_ms.unwrap_or(60_000) + 1)
        .await
        .unwrap();
    let stats = replay.stats();
    eprintln!(
        "chat replay: {first:?} then {ahead:?}; {} pages, {} comments, {} deleted",
        stats.pages, stats.comments, stats.deleted
    );
    assert!(stats.pages >= 1);
}
