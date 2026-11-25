// SPDX-License-Identifier: MPL-2.0
//! SRV-019 "done when": `GET /api/v1/system/activity` — idle → not busy; a
//! `playing` heartbeat → busy until its TTL runs out; running work
//! (recording, scan) → busy until it ends; a request that is not from the
//! machine itself → refused.

use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use pa_server::activity::{ActivityKind, PATH};
use pa_server::{Config, Server};
use pa_sync::Hlc;
use pa_sync::proto::{self, Hello, Message, PlayState, PlaybackUpdate, ProtoRange, Role};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message as WsMessage;

const WAIT: Duration = Duration::from_secs(5);

fn config(listen: &str) -> Config {
    let mut config = Config {
        listen: listen.parse().unwrap(),
        // Plain HTTP: raw requests (TLS: tls_mdns.rs).
        tls: pa_server::config::TlsConfig {
            enabled: false,
            ..Default::default()
        },
        ..Config::default()
    };
    // Unpaired test device: pairing is covered by tests/pairing.rs.
    config.sync.require_pairing = false;
    config.discovery.enabled = false;
    config
}

/// Minimal HTTP/1.1 GET to `addr` with extra header lines.
async fn get(addr: SocketAddr, path: &str, extra: &str) -> (u16, Value) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n{extra}Connection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    (status, serde_json::from_str(body).unwrap())
}

async fn report(addr: SocketAddr) -> Value {
    let (status, body) = get(addr, PATH, "").await;
    assert_eq!(status, 200, "{body}");
    body
}

fn idle() -> Value {
    json!({"busy": false, "sessions": 0, "reason": ""})
}

/// Polls until the report equals `want` (at most [`WAIT`]).
async fn wait_for(addr: SocketAddr, want: &Value) -> Duration {
    let started = Instant::now();
    loop {
        let got = report(addr).await;
        if &got == want {
            return started.elapsed();
        }
        assert!(started.elapsed() < WAIT, "still {got}, want {want}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn pb(state: PlayState, wall_ms: u64, ttl_ms: Option<u64>) -> Message {
    Message::PbUpdate(PlaybackUpdate {
        aliases: Vec::new(),
        profile: "default".to_owned(),
        key: "tmdb:movie:603".parse().unwrap(),
        position_ms: 60_000,
        duration_ms: Some(8_160_000),
        rate: 1.0,
        state,
        hlc: Hlc {
            wall_ms,
            counter: 0,
            device: "tv".to_owned(),
        },
        ttl_ms,
    })
}

/// Starts the server, runs `check`, stops it.
async fn with_server<F, Fut>(config: &Config, check: F)
where
    F: FnOnce(SocketAddr, pa_server::activity::Activity) -> Fut,
    Fut: Future<Output = ()>,
{
    let tmp = tempfile::tempdir().unwrap();
    let server = Server::bind(config, tmp.path()).await.unwrap();
    let addr = server.local_addr().unwrap();
    let activity = server.activity().clone();
    let (stop, stopped) = oneshot::channel::<()>();
    let task = tokio::spawn(server.run(async {
        let _ = stopped.await;
    }));
    check(addr, activity).await;
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playing_heartbeat_is_busy_until_its_ttl() {
    with_server(&config("127.0.0.1:0"), |addr, _| async move {
        assert_eq!(report(addr).await, idle());

        let url = format!("ws://{addr}{}", pa_server::sync::PATH);
        let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        let hello = Message::Hello(Hello {
            proto: ProtoRange::LOCAL,
            device_id: "tv".to_owned(),
            role: Role::Device,
            cursor: None,
            head: None,
            nonce: None,
        });
        let send = |msg: &Message| WsMessage::Text(proto::encode(msg).unwrap().into());
        ws.send(send(&hello)).await.unwrap();
        // The hub's hello.
        let first = tokio::time::timeout(WAIT, ws.next()).await.unwrap();
        assert!(matches!(first, Some(Ok(WsMessage::Text(_)))));

        // Playing with a short TTL: busy, then idle once it runs out.
        ws.send(send(&pb(PlayState::Playing, 1_000, Some(1_500))))
            .await
            .unwrap();
        let busy = json!({"busy": true, "sessions": 1, "reason": "playback"});
        wait_for(addr, &busy).await;
        let gone = wait_for(addr, &idle()).await;
        assert!(
            gone >= Duration::from_millis(500),
            "idle too early: {gone:?}"
        );

        // A fresh heartbeat makes it busy again; pause ends it at once.
        ws.send(send(&pb(PlayState::Playing, 2_000, None)))
            .await
            .unwrap();
        wait_for(addr, &busy).await;
        ws.send(send(&pb(PlayState::Paused, 3_000, Some(600_000))))
            .await
            .unwrap();
        wait_for(addr, &idle()).await;
        ws.close(None).await.unwrap();
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_work_is_busy_until_it_ends() {
    with_server(&config("127.0.0.1:0"), |addr, activity| async move {
        let scan = activity.begin(ActivityKind::Scan);
        assert_eq!(
            report(addr).await,
            json!({"busy": true, "sessions": 0, "reason": "scan"})
        );
        let recording = activity.begin(ActivityKind::Recording);
        assert_eq!(
            report(addr).await,
            json!({"busy": true, "sessions": 0, "reason": "recording"})
        );
        let stream = activity.begin(ActivityKind::Playback);
        assert_eq!(
            report(addr).await,
            json!({"busy": true, "sessions": 1, "reason": "playback"})
        );
        drop((scan, recording, stream));
        assert_eq!(report(addr).await, idle());
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_through_a_proxy() {
    with_server(&config("127.0.0.1:0"), |addr, _| async move {
        for header in [
            "X-Forwarded-For: 192.168.1.10\r\n",
            "Forwarded: for=192.168.1.10\r\n",
            "X-Real-IP: 192.168.1.10\r\n",
        ] {
            let (status, body) = get(addr, PATH, header).await;
            assert_eq!(status, 403, "{header}");
            assert!(body["error"].is_string());
        }
    })
    .await;
}

/// An address of this machine on a network interface (not loopback), if
/// it has one. Connecting a UDP socket sends nothing; it only picks the
/// source address of the route.
fn lan_address() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_from_the_network() {
    let Some(lan) = lan_address() else {
        // The pure check is covered by the unit test of `activity.rs`.
        eprintln!("skipped: no network interface besides loopback");
        return;
    };
    with_server(&config("0.0.0.0:0"), |addr, _| async move {
        // Connecting to our own LAN address: the peer address is the LAN
        // address too, as for another device of the home network.
        let remote = SocketAddr::new(lan, addr.port());
        let (status, body) = get(remote, PATH, "").await;
        assert_eq!(status, 403, "{body}");
        assert!(body["error"].is_string());
        // The same server still answers the machine itself.
        let local = SocketAddr::new("127.0.0.1".parse().unwrap(), addr.port());
        assert_eq!(report(local).await, idle());
    })
    .await;
}
