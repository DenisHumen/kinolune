// SPDX-License-Identifier: MPL-2.0
//! SRV-003 "done when": brute force locks, refresh tokens rotate and a reused
//! one ends the session, revoking a device kills its tokens. Plus the paths
//! around them: first-start setup, device sign-in with the paired key, PIN
//! profiles on a paired device, protected administration.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;

use futures_util::{SinkExt, StreamExt};
use pa_pairing::pair::{Joiner, PairMessage, PairingCode};
use pa_pairing::{DeviceIdentity, DeviceKind, Family};
use pa_server::{Config, Server};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message as WsMessage;

const ADMIN_PASSWORD: &str = "correct horse battery";

struct Running {
    addr: SocketAddr,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Result<(), pa_server::Error>>,
    pairing: pa_server::pairing::Pairing,
}

impl Running {
    async fn start(data: &Path) -> Self {
        let config = Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            tls: pa_server::config::TlsConfig {
                enabled: false,
                ..Default::default()
            },
            ..Config::default()
        };
        let server = Server::bind(&config, data).await.unwrap();
        assert!(!server.auth().has_admin());
        let addr = server.local_addr().unwrap();
        let pairing = server.hub().pairing().clone();
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(server.run(async {
            let _ = stopped.await;
        }));
        Self {
            addr,
            stop,
            task,
            pairing,
        }
    }

    async fn stop(self) {
        self.stop.send(()).unwrap();
        self.task.await.unwrap().unwrap();
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> Reply {
        http(self.addr, method, path, token, body).await
    }

    /// Creates the first administrator from loopback; returns its tokens.
    async fn setup_admin(&self) -> Value {
        let r = self
            .call(
                "POST",
                "/api/v1/auth/setup",
                None,
                Some(json!({"name": "Admin", "password": ADMIN_PASSWORD})),
            )
            .await;
        assert_eq!(r.status, 201, "{}", r.body);
        assert_eq!(r.json()["user"]["role"], "admin");
        r.json()["tokens"].clone()
    }

    async fn login(&self, name: &str, password: &str) -> Reply {
        self.call(
            "POST",
            "/api/v1/auth/login",
            None,
            Some(json!({"name": name, "password": password})),
        )
        .await
    }
}

struct Reply {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("{e}: {}", self.body))
    }
}

/// Minimal HTTP/1.1 client.
async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Reply {
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(token) = token {
        request.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(&body);
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let status = line.split(' ').nth(1).unwrap().parse().unwrap();
    let mut headers = HashMap::new();
    loop {
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let mut body = String::new();
    reader.read_to_string(&mut body).await.unwrap();
    Reply {
        status,
        headers,
        body,
    }
}

fn token<'a>(tokens: &'a Value, field: &str) -> &'a str {
    tokens[field].as_str().unwrap()
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

async fn recv(ws: &mut Ws) -> PairMessage {
    loop {
        if let WsMessage::Text(text) = ws.next().await.unwrap().unwrap() {
            return PairMessage::decode(text.as_str()).unwrap();
        }
    }
}

/// Pairs a new device with an invitation of the server (SYNC-005).
async fn pair_device(server: &Running) -> Family {
    let invite = server.pairing.invite(false).unwrap();
    let code = PairingCode::parse(&invite.code).unwrap();
    let identity = DeviceIdentity::generate("living-room-tv").unwrap();
    let url = format!("ws://{}{}", server.addr, pa_server::pairing::PAIR_PATH);
    let mut ws = tokio_tungstenite::connect_async(url).await.unwrap().0;
    let (mut joiner, hello) =
        Joiner::start(&identity, "Living room TV", DeviceKind::Tv, &code, None).unwrap();
    ws.send(WsMessage::Text(hello.encode().into()))
        .await
        .unwrap();
    let theirs = recv(&mut ws).await;
    let intro = joiner.intro(&identity, &theirs).unwrap();
    ws.send(WsMessage::Text(intro.encode().into()))
        .await
        .unwrap();
    let welcome = recv(&mut ws).await;
    joiner
        .finish(&welcome)
        .unwrap()
        .into_family(identity)
        .unwrap()
}

/// Signs a paired device in with its key; returns the device tokens.
async fn device_login(server: &Running, family: &Family) -> Reply {
    let challenge = server
        .call("POST", "/api/v1/auth/challenge", None, None)
        .await;
    assert_eq!(challenge.status, 200, "{}", challenge.body);
    let challenge = challenge.json();
    let nonce = pa_pairing::auth::nonce().unwrap();
    let sig = pa_pairing::auth::prove(
        family.identity(),
        challenge["server_id"].as_str().unwrap(),
        &nonce,
        challenge["challenge"].as_str().unwrap(),
    );
    server
        .call(
            "POST",
            "/api/v1/auth/device",
            None,
            Some(json!({
                "device_id": family.identity().device_id(),
                "nonce": nonce,
                "challenge": challenge["challenge"],
                "sig": sig,
            })),
        )
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn brute_force_locks_the_account_then_the_address() {
    let tmp = tempfile::tempdir().unwrap();
    let server = Running::start(tmp.path()).await;
    let admin = server.setup_admin().await;
    let bearer = token(&admin, "access_token");
    let kid = server
        .call(
            "POST",
            "/api/v1/users",
            Some(bearer),
            Some(json!({"name": "Kid", "password": "kid's own password"})),
        )
        .await;
    assert_eq!(kid.status, 201, "{}", kid.body);

    // Five wrong passwords lock the account…
    for _ in 0..5 {
        let r = server.login("admin", "guess-guess").await;
        assert_eq!(r.status, 401, "{}", r.body);
        assert_eq!(
            r.headers.get("www-authenticate").map(String::as_str),
            Some("Bearer")
        );
    }
    // …even for the right password now, with a Retry-After.
    let locked = server.login("Admin", ADMIN_PASSWORD).await;
    assert_eq!(locked.status, 429, "{}", locked.body);
    let wait: u64 = locked.headers["retry-after"].parse().unwrap();
    assert!((1..=30).contains(&wait), "Retry-After {wait}");

    // Other accounts still work from this address (5 of 20 failures).
    let r = server.login("kid", "kid's own password").await;
    assert_eq!(r.status, 200, "{}", r.body);

    // Spraying many names from one address locks the address.
    for i in 0..15 {
        let r = server.login(&format!("nobody-{i}"), "guess-guess").await;
        assert_eq!(r.status, 401, "{}", r.body);
    }
    let r = server.login("kid", "kid's own password").await;
    assert_eq!(r.status, 429, "{}", r.body);

    // The administrator's existing session is not affected.
    let me = server
        .call("GET", "/api/v1/auth/me", Some(bearer), None)
        .await;
    assert_eq!(me.status, 200, "{}", me.body);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_rotates_and_reuse_revokes_the_session() {
    let tmp = tempfile::tempdir().unwrap();
    let server = Running::start(tmp.path()).await;
    let first = server.setup_admin().await;
    assert_eq!(first["token_type"], "Bearer");
    assert_eq!(first["expires_in"], 15 * 60);
    assert!(token(&first, "access_token").starts_with("v4.public."));

    // Rotation: a new pair, and the new access token works.
    let r = server
        .call(
            "POST",
            "/api/v1/auth/refresh",
            None,
            Some(json!({"refresh_token": first["refresh_token"]})),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let second = r.json();
    assert_ne!(second["refresh_token"], first["refresh_token"]);
    let me = server
        .call(
            "GET",
            "/api/v1/auth/me",
            Some(token(&second, "access_token")),
            None,
        )
        .await;
    assert_eq!(me.status, 200, "{}", me.body);
    assert_eq!(me.json()["method"], "password");
    assert_eq!(me.json()["user"]["name"], "Admin");

    // The old refresh token again: reuse. The whole session ends.
    let r = server
        .call(
            "POST",
            "/api/v1/auth/refresh",
            // A stale access token in the header does not matter here.
            Some(token(&first, "access_token")),
            Some(json!({"refresh_token": first["refresh_token"]})),
        )
        .await;
    assert_eq!(r.status, 401, "{}", r.body);
    assert!(r.json()["error"].as_str().unwrap().contains("reused"));
    for access in [
        token(&first, "access_token"),
        token(&second, "access_token"),
    ] {
        let me = server
            .call("GET", "/api/v1/auth/me", Some(access), None)
            .await;
        assert_eq!(me.status, 401, "{}", me.body);
    }
    let r = server
        .call(
            "POST",
            "/api/v1/auth/refresh",
            None,
            Some(json!({"refresh_token": second["refresh_token"]})),
        )
        .await;
    assert_eq!(r.status, 401, "{}", r.body);

    // Sign-out ends a session as well.
    let r = server.login("admin", ADMIN_PASSWORD).await;
    assert_eq!(r.status, 200, "{}", r.body);
    let third = r.json();
    let r = server
        .call(
            "POST",
            "/api/v1/auth/logout",
            None,
            Some(json!({"refresh_token": third["refresh_token"]})),
        )
        .await;
    assert_eq!(r.status, 204, "{}", r.body);
    let me = server
        .call(
            "GET",
            "/api/v1/auth/me",
            Some(token(&third, "access_token")),
            None,
        )
        .await;
    assert_eq!(me.status, 401, "{}", me.body);

    // Garbage tokens.
    for bad in ["v4.public.AAAA", "nonsense"] {
        let me = server.call("GET", "/api/v1/auth/me", Some(bad), None).await;
        assert_eq!(me.status, 401, "{}", me.body);
    }
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_pin_profiles_and_revocation() {
    let tmp = tempfile::tempdir().unwrap();
    let server = Running::start(tmp.path()).await;
    let admin = server.setup_admin().await;
    let admin_bearer = token(&admin, "access_token");

    // Administration is protected once an administrator exists.
    let r = server.call("POST", "/api/v1/pair/invite", None, None).await;
    assert_eq!(r.status, 401, "{}", r.body);
    let r = server.call("GET", "/api/v1/devices", None, None).await;
    assert_eq!(r.status, 401, "{}", r.body);
    let r = server
        .call("POST", "/api/v1/pair/invite", Some(admin_bearer), None)
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let r = server
        .call(
            "POST",
            "/api/v1/auth/setup",
            None,
            Some(json!({"name": "Again", "password": ADMIN_PASSWORD})),
        )
        .await;
    assert_eq!(r.status, 409, "{}", r.body);

    // A household member with a PIN.
    let r = server
        .call(
            "POST",
            "/api/v1/users",
            Some(admin_bearer),
            Some(json!({"name": "Kid", "pin": "2468"})),
        )
        .await;
    assert_eq!(r.status, 201, "{}", r.body);
    let kid_id = r.json()["id"].as_str().unwrap().to_owned();
    assert_eq!(r.json()["has_pin"], true);
    assert_eq!(r.json()["has_password"], false);

    // The TV pairs (SYNC-005) and signs in with its device key.
    let family = pair_device(&server).await;
    let device_id = family.identity().device_id().to_owned();
    let r = device_login(&server, &family).await;
    assert_eq!(r.status, 200, "{}", r.body);
    let device = r.json();
    assert_eq!(device["scope"], "device");
    let device_bearer = token(&device, "access_token");

    // A stranger's key is refused.
    let stranger = Family::found(
        DeviceIdentity::generate("stranger").unwrap(),
        "Stranger",
        DeviceKind::Phone,
        1,
    )
    .unwrap();
    let r = device_login(&server, &stranger).await;
    assert_eq!(r.status, 401, "{}", r.body);

    // Profiles to pick, then the PIN.
    let r = server
        .call("GET", "/api/v1/auth/profiles", Some(device_bearer), None)
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(
        r.json()["users"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["name"] == "Kid")
    );
    let pin = |p: &'static str| json!({"user_id": kid_id, "pin": p});
    let r = server
        .call(
            "POST",
            "/api/v1/auth/pin",
            Some(device_bearer),
            Some(pin("0000")),
        )
        .await;
    assert_eq!(r.status, 401, "{}", r.body);
    // Not without a device…
    let r = server
        .call("POST", "/api/v1/auth/pin", None, Some(pin("2468")))
        .await;
    assert_eq!(r.status, 401, "{}", r.body);
    // …nor from a session that is not bound to one.
    let r = server
        .call(
            "POST",
            "/api/v1/auth/pin",
            Some(admin_bearer),
            Some(pin("2468")),
        )
        .await;
    assert_eq!(r.status, 403, "{}", r.body);
    let r = server
        .call(
            "POST",
            "/api/v1/auth/pin",
            Some(device_bearer),
            Some(pin("2468")),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let kid = r.json();
    let kid_bearer = token(&kid, "access_token");
    let me = server
        .call("GET", "/api/v1/auth/me", Some(kid_bearer), None)
        .await;
    assert_eq!(me.status, 200, "{}", me.body);
    assert_eq!(me.json()["method"], "pin");
    assert_eq!(me.json()["device_id"], device_id.as_str());
    assert_eq!(me.json()["user"]["name"], "Kid");

    // A member cannot administer.
    let r = server
        .call("GET", "/api/v1/users", Some(kid_bearer), None)
        .await;
    assert_eq!(r.status, 403, "{}", r.body);
    let r = server
        .call("GET", "/api/v1/devices", Some(device_bearer), None)
        .await;
    assert_eq!(r.status, 403, "{}", r.body);

    // The administrator revokes the TV: every token bound to it dies at once.
    let r = server
        .call(
            "DELETE",
            &format!("/api/v1/devices/{device_id}"),
            Some(admin_bearer),
            None,
        )
        .await;
    assert_eq!(r.status, 204, "{}", r.body);
    for access in [device_bearer, kid_bearer] {
        let me = server
            .call("GET", "/api/v1/auth/me", Some(access), None)
            .await;
        assert_eq!(me.status, 401, "{}", me.body);
    }
    for tokens in [&device, &kid] {
        let r = server
            .call(
                "POST",
                "/api/v1/auth/refresh",
                None,
                Some(json!({"refresh_token": tokens["refresh_token"]})),
            )
            .await;
        assert_eq!(r.status, 401, "{}", r.body);
    }
    // The revoked key cannot sign in again.
    let r = device_login(&server, &family).await;
    assert_eq!(r.status, 401, "{}", r.body);
    // The administrator, not bound to the TV, still works.
    let me = server
        .call("GET", "/api/v1/auth/me", Some(admin_bearer), None)
        .await;
    assert_eq!(me.status, 200, "{}", me.body);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accounts_credentials_and_deletion() {
    let tmp = tempfile::tempdir().unwrap();
    let server = Running::start(tmp.path()).await;
    let admin = server.setup_admin().await;
    let admin_bearer = token(&admin, "access_token");
    let r = server
        .call(
            "POST",
            "/api/v1/users",
            Some(admin_bearer),
            Some(json!({"name": "Mom", "password": "mom's password"})),
        )
        .await;
    assert_eq!(r.status, 201, "{}", r.body);
    let mom_id = r.json()["id"].as_str().unwrap().to_owned();
    for (body, status) in [
        (json!({"name": "mom", "password": "another one"}), 409),
        (json!({"name": "Dad"}), 400),
        (json!({"name": "Dad", "role": "admin", "pin": "1234"}), 400),
        (json!({"name": "Dad", "password": "short"}), 400),
        (json!({"name": "Dad", "pin": "12"}), 400),
    ] {
        let r = server
            .call("POST", "/api/v1/users", Some(admin_bearer), Some(body))
            .await;
        assert_eq!(r.status, status, "{}", r.body);
    }

    // Mom has two sessions; changing her password keeps the one that did it.
    let one = server.login("mom", "mom's password").await.json();
    let two = server.login("mom", "mom's password").await.json();
    let r = server
        .call(
            "PATCH",
            &format!("/api/v1/users/{mom_id}"),
            Some(token(&one, "access_token")),
            Some(json!({"password": "a new password"})),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let me = |t: &Value| {
        let t = token(t, "access_token").to_owned();
        let server = &server;
        async move {
            server
                .call("GET", "/api/v1/auth/me", Some(&t), None)
                .await
                .status
        }
    };
    assert_eq!(me(&one).await, 200);
    assert_eq!(me(&two).await, 401);
    assert_eq!(server.login("mom", "mom's password").await.status, 401);
    assert_eq!(server.login("mom", "a new password").await.status, 200);
    // She cannot change somebody else.
    let admin_id = server
        .call("GET", "/api/v1/auth/me", Some(admin_bearer), None)
        .await
        .json()["user"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let r = server
        .call(
            "PATCH",
            &format!("/api/v1/users/{admin_id}"),
            Some(token(&one, "access_token")),
            Some(json!({"password": "taken over!"})),
        )
        .await;
    assert_eq!(r.status, 403, "{}", r.body);

    // The last administrator stays; a deleted user's sessions end.
    let r = server
        .call(
            "DELETE",
            &format!("/api/v1/users/{admin_id}"),
            Some(admin_bearer),
            None,
        )
        .await;
    assert_eq!(r.status, 409, "{}", r.body);
    let r = server
        .call(
            "DELETE",
            &format!("/api/v1/users/{mom_id}"),
            Some(admin_bearer),
            None,
        )
        .await;
    assert_eq!(r.status, 204, "{}", r.body);
    assert_eq!(me(&one).await, 401);
    let r = server
        .call("GET", "/api/v1/users", Some(admin_bearer), None)
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["users"].as_array().unwrap().len(), 1);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accounts_and_keys_survive_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let server = Running::start(tmp.path()).await;
    assert!(tmp.path().join(pa_server::auth::SETUP_CODE_FILE).exists());
    let admin = server.setup_admin().await;
    assert!(!tmp.path().join(pa_server::auth::SETUP_CODE_FILE).exists());
    server.stop().await;

    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        tls: pa_server::config::TlsConfig {
            enabled: false,
            ..Default::default()
        },
        ..Config::default()
    };
    let server = Server::bind(&config, tmp.path()).await.unwrap();
    assert!(server.auth().has_admin());
    let addr = server.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let task = tokio::spawn(server.run(async {
        let _ = stopped.await;
    }));
    // The access token of the first run still verifies (same key, same
    // session), and loopback no longer administers without signing in.
    let me = http(
        addr,
        "GET",
        "/api/v1/auth/me",
        Some(token(&admin, "access_token")),
        None,
    )
    .await;
    assert_eq!(me.status, 200, "{}", me.body);
    let r = http(addr, "GET", "/api/v1/users", None, None).await;
    assert_eq!(r.status, 401, "{}", r.body);
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
}
