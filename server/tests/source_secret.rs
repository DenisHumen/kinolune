// SPDX-License-Identifier: MPL-2.0
//! SYNC-010: a `source.upsert` with a sealed password goes through the
//! user's server hub. The hub stores and relays it (the other device opens
//! the password), but neither its state files (journal, library, roster,
//! keys) nor its log output ever hold the password in clear text; every
//! byte of the data folder is scanned after a restart.

use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use pa_server::{Config, Server};
use pa_sync::proto::{self, Hello, Message, OpFrame, ProtoRange, Pull, Role};
use pa_sync::seal::{self, SealKey};
use pa_sync::{Hlc, Op, OpBody, OpId};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const PASSWORD: &str = "hub-never-sees-Th1s";
const WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Running {
    addr: SocketAddr,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<(), pa_server::Error>>,
}

async fn start(data: &Path) -> Running {
    let mut config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        tls: pa_server::config::TlsConfig {
            enabled: false,
            ..Default::default()
        },
        ..Config::default()
    };
    config.sync.require_pairing = false;
    let server = Server::bind(&config, data).await.unwrap();
    let addr = server.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let task = tokio::spawn(server.run(async {
        let _ = stopped.await;
    }));
    Running { addr, stop, task }
}

async fn send(ws: &mut Ws, msg: &Message) {
    let frame = proto::encode(msg).unwrap();
    ws.send(WsMessage::Text(frame.into())).await.unwrap();
}

async fn recv(ws: &mut Ws) -> Option<Message> {
    loop {
        let msg = tokio::time::timeout(WAIT, ws.next())
            .await
            .expect("in time")?;
        match msg {
            Ok(WsMessage::Text(text)) => return Some(proto::decode(text.as_str()).unwrap()),
            Ok(WsMessage::Close(_)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

async fn connect(addr: SocketAddr, device: &str) -> Ws {
    let url = format!("ws://{addr}{}", pa_server::sync::PATH);
    let mut ws = tokio_tungstenite::connect_async(url).await.unwrap().0;
    send(
        &mut ws,
        &Message::Hello(Hello {
            proto: ProtoRange::LOCAL,
            device_id: device.to_owned(),
            role: Role::Device,
            cursor: None,
            head: None,
            nonce: None,
        }),
    )
    .await;
    assert!(matches!(recv(&mut ws).await, Some(Message::Hello(_))));
    ws
}

fn files_under(dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_under(&path, out);
        } else if let Ok(bytes) = std::fs::read(&path) {
            out.push((path.display().to_string(), bytes));
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_hub_relays_a_sealed_password_but_never_stores_or_logs_it() {
    let logs = LogBuffer::default();
    let writer = logs.clone();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || writer.clone())
            .finish(),
    )
    .unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let hub = start(tmp.path()).await;
    let mut phone = connect(hub.addr, "phone").await;
    let mut tv = connect(hub.addr, "tv").await;

    // The family key is on the phone and the TV, not on this hub.
    let family = SealKey::new("family-x", 1, &[0x5a; 32]);
    let op = Op {
        op_id: OpId::new_v7(),
        hlc: Hlc {
            wall_ms: 1_763_000_000_000,
            counter: 0,
            device: "phone".into(),
        },
        body: OpBody::SourceUpsert {
            source: "0193a1b2c3d4".into(),
            config: r#"{"kind":"smb","name":"NAS","uri":"smb://nas.local/media","username":"anna","scan_into_library":true,"auto_scan":true,"enabled":true,"sync":"password"}"#.into(),
            secret: Some(seal::seal(&family, "source:0193a1b2c3d4", PASSWORD.as_bytes()).unwrap()),
        },
    };
    send(&mut phone, &Message::Op(OpFrame::plain(&op).unwrap())).await;
    assert!(matches!(recv(&mut phone).await, Some(Message::Ack(_))));
    let Some(Message::Op(fanned)) = recv(&mut tv).await else {
        panic!("no fan-out")
    };
    let OpBody::SourceUpsert { secret, .. } = fanned.open().unwrap().body else {
        panic!("not a source op")
    };
    let opened = seal::open(&family, "source:0193a1b2c3d4", &secret.unwrap()).unwrap();
    assert_eq!(&*opened, PASSWORD.as_bytes());

    // Restart: the op is in the hub's journal on disk and comes back.
    let stopping = tokio::spawn(async move {
        hub.stop.send(()).unwrap();
        hub.task.await.unwrap().unwrap();
    });
    assert!(recv(&mut phone).await.is_none());
    stopping.await.unwrap();
    let hub = start(tmp.path()).await;
    let mut laptop = connect(hub.addr, "laptop").await;
    assert!(matches!(recv(&mut laptop).await, Some(Message::Change(_))));
    send(
        &mut laptop,
        &Message::Pull(Pull {
            cursor: None,
            limit: 100,
        }),
    )
    .await;
    let Some(Message::Op(stored)) = recv(&mut laptop).await else {
        panic!("not journaled")
    };
    assert_eq!(stored.op_id, op.op_id);
    hub.stop.send(()).unwrap();
    hub.task.await.unwrap().unwrap();

    let mut files = Vec::new();
    files_under(tmp.path(), &mut files);
    assert!(!files.is_empty());
    let journal = files
        .iter()
        .filter(|(p, _)| p.contains("sync"))
        .map(|(_, b)| b.as_slice())
        .any(|b| find(b, b"source.upsert"));
    assert!(journal, "the op is in the hub's journal files");
    for (path, bytes) in &files {
        assert!(
            !find(bytes, PASSWORD.as_bytes()),
            "password in clear text in {path}"
        );
    }
    let logged = logs.0.lock().unwrap().clone();
    assert!(!logged.is_empty(), "the log capture works");
    assert!(!find(&logged, PASSWORD.as_bytes()), "password in the logs");
}
