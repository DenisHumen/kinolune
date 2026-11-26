// SPDX-License-Identifier: MPL-2.0
//! Making a clip of a live stream (TWI-040) against recorded responses
//! (`tests/fixtures/clips_create/`). No network: the fixture transports fail
//! on any unplanned call. All tokens here are fake.

use std::sync::Arc;
use std::time::Duration;

use pa_secrets::{MemoryStore, SecretKey, SecretStore, SecretString, redact};
use pa_twitch::testing::{FixtureTransport, ManualClock};
use pa_twitch::{
    AuthConfig, AuthorizedHelix, CLIP_POLL_INTERVAL, CLIP_READY_TIMEOUT, ClientId, ClipError,
    HelixClient, HelixConfig, Method, SCOPE_CLIPS_EDIT, TwitchAuth,
};

const START: u64 = 1_800_000_000;
const CLIENT_ID: &str = "testclientid0000000000000000000";
const USER_ID: &str = "424242";
const ACCESS_1: &str = "fakeaccess1000000000000000000";
const REFRESH_1: &str = "fakerefresh1000000000000000000000000000000000000";
const ACCESS_3: &str = "fakeaccess3000000000000000000";
const CLIP_ID: &str = "FiercePlayfulOtterKappa-AbCdEf123";

macro_rules! fx {
    ($name:literal) => {
        include_str!(concat!("fixtures/clips_create/", $name, ".json"))
    };
}

macro_rules! auth_fx {
    ($name:literal) => {
        include_str!(concat!("fixtures/auth/", $name, ".json"))
    };
}

struct Harness {
    api: AuthorizedHelix,
    id: Arc<FixtureTransport>,
    helix: Arc<FixtureTransport>,
    clock: Arc<ManualClock>,
}

/// Signed in from stored tokens with `scopes`.
async fn signed_in(scopes: &[&str]) -> Harness {
    let id = Arc::new(FixtureTransport::new());
    let helix_transport = Arc::new(FixtureTransport::new());
    let clock = Arc::new(ManualClock::at_unix(START));
    let store = Arc::new(MemoryStore::new());
    let doc = serde_json::json!({
        "v": 1,
        "access_token": ACCESS_1,
        "refresh_token": REFRESH_1,
        "expires_at": START + 14_400,
        "scopes": scopes,
        "user_id": USER_ID,
        "login": "test_viewer",
    });
    store
        .set(
            &SecretKey::twitch_tokens(USER_ID).unwrap(),
            &SecretString::new(doc.to_string()),
        )
        .unwrap();
    let mut config = AuthConfig::new(ClientId::new(CLIENT_ID).unwrap());
    config.id_base_url = "https://id.test/oauth2".to_owned();
    let auth = Arc::new(TwitchAuth::with_clock(
        config,
        id.clone(),
        clock.clone(),
        store,
    ));
    auth.restore(USER_ID).await.unwrap().unwrap();
    let mut helix_config = HelixConfig::new(ClientId::new(CLIENT_ID).unwrap());
    helix_config.base_url = "https://helix.test/helix".to_owned();
    helix_config.max_retries = 2;
    let helix = HelixClient::with_clock(helix_config, helix_transport.clone(), clock.clone());
    Harness {
        api: AuthorizedHelix::new(auth, Arc::new(helix)),
        id,
        helix: helix_transport,
        clock,
    }
}

impl Harness {
    fn helix_calls(&self) -> Vec<(Method, String)> {
        self.helix
            .requests()
            .into_iter()
            .map(|r| (r.method, r.url.replace("https://helix.test/helix/", "")))
            .collect()
    }
}

#[tokio::test]
async fn clip_is_created_and_ready_after_waiting() {
    let h = signed_in(&["user:read:follows", SCOPE_CLIPS_EDIT]).await;
    assert!(h.api.can_make_clips());
    h.helix
        .push_fixture(fx!("created"))
        .push_fixture(fx!("pending"))
        .push_fixture(fx!("pending"))
        .push_fixture(fx!("ready"));

    let (created, clip) = h.api.make_clip("1001", true).await.unwrap();
    assert_eq!(created.id, CLIP_ID);
    assert_eq!(
        created.edit_url,
        format!("https://clips.twitch.tv/{CLIP_ID}/edit")
    );
    assert_eq!(clip.id, CLIP_ID);
    assert_eq!(clip.url, format!("https://clips.twitch.tv/{CLIP_ID}"));
    assert!(clip.embed_url.starts_with("https://clips.twitch.tv/embed"));
    let get = (Method::Get, format!("clips?id={CLIP_ID}"));
    assert_eq!(
        h.helix_calls(),
        [
            (Method::Post, "clips?broadcaster_id=1001&has_delay=true".to_owned()),
            get.clone(),
            get.clone(),
            get,
        ]
    );
    // One pause before each check, nothing after the clip appeared.
    assert_eq!(h.clock.sleeps(), [CLIP_POLL_INTERVAL; 3]);
    let post = &h.helix.requests()[0];
    assert_eq!(
        post.header("authorization"),
        Some(format!("Bearer {ACCESS_1}").as_str())
    );
    assert!(post.body.is_none());
}

#[tokio::test]
async fn clip_that_never_appears_fails_after_15_seconds() {
    let h = signed_in(&[SCOPE_CLIPS_EDIT]).await;
    h.helix.push_fixture(fx!("created"));
    for _ in 0..15 {
        h.helix.push_fixture(fx!("pending"));
    }
    let err = h.api.make_clip("1001", false).await.unwrap_err();
    assert_eq!(err, ClipError::NotReady);
    assert_eq!(h.helix.remaining(), 0);
    let total: Duration = h.clock.sleeps().iter().sum();
    assert_eq!(total, CLIP_READY_TIMEOUT);
    assert_eq!(
        h.helix_calls()[0],
        (Method::Post, "clips?broadcaster_id=1001".to_owned())
    );
}

#[tokio::test]
async fn channel_without_clips_is_reported_with_the_reason() {
    let h = signed_in(&[SCOPE_CLIPS_EDIT]).await;
    h.helix.push_fixture(fx!("disabled"));
    let err = h.api.make_clip("1001", false).await.unwrap_err();
    assert_eq!(
        err,
        ClipError::Disabled {
            message: "The specified broadcaster has not enabled clips on their channel".to_owned()
        }
    );
    assert_eq!(h.helix.requests().len(), 1);
}

#[tokio::test]
async fn offline_channel_is_reported() {
    let h = signed_in(&[SCOPE_CLIPS_EDIT]).await;
    h.helix.push_fixture(fx!("offline"));
    let err = h.api.make_clip("1001", false).await.unwrap_err();
    assert_eq!(err, ClipError::Offline);
}

#[tokio::test]
async fn without_the_scope_nothing_is_sent_and_a_new_sign_in_adds_it() {
    let h = signed_in(&["user:read:follows", "user:read:chat"]).await;
    assert!(!h.api.can_make_clips());
    let err = h.api.make_clip("1001", false).await.unwrap_err();
    assert_eq!(err, ClipError::ScopeRequired);
    assert!(h.helix.requests().is_empty());

    // Repeated Device Code Flow asking for `clips:edit` on top of the
    // default scopes.
    h.id.push_fixture(auth_fx!("device"))
        .push_fixture(fx!("token_with_clips"))
        .push_fixture(auth_fx!("validate"));
    let device = h
        .api
        .auth()
        .start_device_login_with(&[SCOPE_CLIPS_EDIT])
        .await
        .unwrap();
    h.api.auth().complete_device_login(device).await.unwrap();
    let body = String::from_utf8(h.id.requests()[0].body.clone().unwrap()).unwrap();
    assert!(
        body.contains("scopes=user%3Aread%3Afollows%20user%3Aread%3Achat%20clips%3Aedit"),
        "{body}"
    );
    assert!(h.api.can_make_clips());

    h.helix
        .push_fixture(fx!("created"))
        .push_fixture(fx!("ready"));
    let (_, clip) = h.api.make_clip("1001", false).await.unwrap();
    assert_eq!(clip.id, CLIP_ID);
    assert_eq!(
        h.helix.requests()[0].header("authorization"),
        Some(format!("Bearer {ACCESS_3}").as_str())
    );
}

#[tokio::test]
async fn http_401_missing_scope_asks_for_a_new_sign_in() {
    // The stored scopes claim `clips:edit`, Twitch disagrees: one refresh,
    // still 401 → sign in again with the scope.
    let h = signed_in(&[SCOPE_CLIPS_EDIT]).await;
    h.helix
        .push_fixture(fx!("missing_scope"))
        .push_fixture(fx!("missing_scope"));
    h.id.push_fixture(auth_fx!("token_refreshed"));
    let err = h.api.make_clip("1001", false).await.unwrap_err();
    assert_eq!(err, ClipError::ScopeRequired);
    assert_eq!(h.helix.requests().len(), 2);
    assert_eq!(h.id.requests().len(), 1);
}

#[tokio::test]
async fn http_429_retries_then_reports_the_wait() {
    let h = signed_in(&[SCOPE_CLIPS_EDIT]).await;
    for _ in 0..3 {
        h.helix
            .push_fixture(include_str!("fixtures/rate_limited.json"));
    }
    let err = h.api.make_clip("1001", false).await.unwrap_err();
    assert!(matches!(err, ClipError::RateLimited { .. }), "{err:?}");
    assert_eq!(h.helix.requests().len(), 3);

    // A single 429 is waited out and the clip still gets made.
    let h = signed_in(&[SCOPE_CLIPS_EDIT]).await;
    h.helix
        .push_fixture(include_str!("fixtures/rate_limited.json"))
        .push_fixture(fx!("created"))
        .push_fixture(fx!("ready"));
    let (created, _) = h.api.make_clip("1001", false).await.unwrap();
    assert_eq!(created.id, CLIP_ID);
    assert_eq!(h.clock.sleeps()[0], Duration::from_secs(5));
}

#[tokio::test]
async fn signed_out_user_is_sent_to_sign_in() {
    let id = Arc::new(FixtureTransport::new());
    let clock = Arc::new(ManualClock::at_unix(START));
    let auth = Arc::new(TwitchAuth::with_clock(
        AuthConfig::new(ClientId::new(CLIENT_ID).unwrap()),
        id,
        clock.clone(),
        Arc::new(MemoryStore::new()),
    ));
    let helix_transport = Arc::new(FixtureTransport::new());
    let helix = HelixClient::with_clock(
        HelixConfig::new(ClientId::new(CLIENT_ID).unwrap()),
        helix_transport.clone(),
        clock,
    );
    let api = AuthorizedHelix::new(auth, Arc::new(helix));
    assert!(!api.can_make_clips());
    assert_eq!(
        api.make_clip("1001", false).await.unwrap_err(),
        ClipError::SignedOut
    );
    assert!(helix_transport.requests().is_empty());
}

#[tokio::test]
async fn tokens_and_scopes_stay_out_of_errors_and_logs() {
    let h = signed_in(&[SCOPE_CLIPS_EDIT]).await;
    h.helix
        .push_fixture(fx!("missing_scope"))
        .push_fixture(fx!("missing_scope"));
    h.id.push_fixture(auth_fx!("token_refreshed"));
    let scope_err = h.api.make_clip("1001", false).await.unwrap_err();

    let mut dumps = vec![
        scope_err.to_string(),
        format!("{scope_err:?}"),
        ClipError::SignedOut.to_string(),
        ClipError::ReauthRequired.to_string(),
        ClipError::NotReady.to_string(),
    ];
    dumps.extend(h.helix.requests().iter().map(|r| format!("{r:?}")));
    for dump in &dumps {
        assert!(!dump.contains(ACCESS_1), "token leaked in {dump}");
        assert!(!dump.contains(REFRESH_1), "refresh token leaked in {dump}");
    }
    // What the app logs about a failed clip: the error text, without the
    // scope list of the account.
    for err in [&scope_err, &ClipError::SignedOut, &ClipError::NotReady] {
        assert!(!err.to_string().contains(SCOPE_CLIPS_EDIT), "{err}");
    }
    let line = format!("clip failed: {scope_err} with {ACCESS_1}");
    assert!(!redact(&line).contains(ACCESS_1));
}
