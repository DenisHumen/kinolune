// SPDX-License-Identifier: MPL-2.0
//! Headless "X is live" watcher for a desktop session or a home server
//! (TWI-009): prints a line per notification with its delay.
//!
//! ```text
//! # Made-up Twitch in process (no network): Moonlit Pancake goes live
//! # after 30 s, Quiet Harbor FM two minutes later.
//! cargo run -p pa-twitch-live --example live_watch -- --demo
//!
//! # Real Twitch: a public client id and a user token of that client
//! # (Device Code Flow, TWI-002); channel ids as arguments.
//! KINOLUNE_TWITCH_CLIENT_ID=… KINOLUNE_TWITCH_TOKEN=… \
//!   cargo run -p pa-twitch-live --example live_watch -- 12345 67890
//! ```
//!
//! Options: `--battery` / `--saver` (polling interval), `--poll-only` (no
//! EventSub).

use std::sync::Arc;
use std::time::Duration;

use pa_twitch::{AccessToken, ClientId, HelixClient, HelixConfig, SystemClock};
use pa_twitch_chat::TungsteniteConnector;
use pa_twitch_live::demo::{FAKE_EVENTSUB_URL, FakeTwitch, spawn_demo_script};
use pa_twitch_live::{
    EventSubState, LiveApi, LiveMonitor, MonitorConfig, MonitorDeps, MonitorEvent, NoStore,
    PowerState, WatchedChannel,
};
use pa_twitch_chat::WsConnector;

/// Helix, EventSub connector, channels and the demo script task.
type Setup = (
    Arc<dyn LiveApi>,
    Arc<dyn WsConnector>,
    Vec<WatchedChannel>,
    Option<tokio::task::JoinHandle<()>>,
);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let (demo, poll_only) = (has("--demo"), has("--poll-only"));
    let power = if has("--saver") {
        PowerState::Saver
    } else if has("--battery") {
        PowerState::Battery
    } else {
        PowerState::Plugged
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async move {
        let clock = Arc::new(SystemClock);
        let mut config = MonitorConfig {
            use_eventsub: !poll_only,
            ..MonitorConfig::default()
        };
        let (api, connector, channels, _script): Setup = if demo {
            let fake = FakeTwitch::demo(clock.clone());
            config.eventsub.url = FAKE_EVENTSUB_URL.to_owned();
            let script = spawn_demo_script(fake.clone(), Duration::from_secs(30));
            (
                Arc::new(fake.clone()),
                Arc::new(fake),
                FakeTwitch::demo_channels(),
                Some(script),
            )
        } else {
            let (Ok(id), Ok(token)) = (
                std::env::var("KINOLUNE_TWITCH_CLIENT_ID"),
                std::env::var("KINOLUNE_TWITCH_TOKEN"),
            ) else {
                eprintln!("set KINOLUNE_TWITCH_CLIENT_ID and KINOLUNE_TWITCH_TOKEN, or use --demo");
                std::process::exit(2);
            };
            let client_id = ClientId::new(id).expect("client id");
            let helix = HelixClient::with_default_transport(HelixConfig::new(client_id))
                .expect("Helix client");
            helix.set_access_token(AccessToken::new(token));
            let channels = args
                .iter()
                .filter(|a| !a.starts_with("--"))
                .map(|id| WatchedChannel::new(id, id, id))
                .collect();
            (
                Arc::new(helix),
                Arc::new(TungsteniteConnector::new().expect("TLS")),
                channels,
                None,
            )
        };
        let monitor = LiveMonitor::spawn(
            config,
            channels,
            power,
            MonitorDeps {
                api,
                connector: Some(connector),
                clock,
                store: Box::new(NoStore),
            },
        );
        while let Some(event) = monitor.next_event().await {
            match event {
                MonitorEvent::Alert(a) => println!(
                    "LIVE {} — {} [{}] via {} after {:.1} s",
                    a.display_name,
                    a.title.as_deref().unwrap_or_default(),
                    a.game_name.as_deref().unwrap_or_default(),
                    a.source.as_str(),
                    a.delay().as_secs_f64(),
                ),
                MonitorEvent::Status(s) => {
                    let es = match &s.eventsub {
                        EventSubState::Connected(c) => format!("eventsub {} ch, cost {}", c.online.len(), c.cost),
                        other => format!("{other:?}"),
                    };
                    println!(
                        "status: {es}; polled only {}; next poll in {} s; errors {}",
                        s.polled_only,
                        s.next_poll_in.as_secs(),
                        s.poll_errors
                    );
                }
                MonitorEvent::Timeline(change) => {
                    let t = change.timeline();
                    println!(
                        "timeline {} {}: {} segment(s)",
                        t.user_id,
                        t.stream_id,
                        t.segments.len()
                    );
                }
            }
        }
    });
}
