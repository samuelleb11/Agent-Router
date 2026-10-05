//! A refused change is answered with an error status and the reason in words, never with a
//! 200 that carries an error — see `refusal` for why, and for what each status means.
//!
//! Each test drives the app `serve` builds (the same routes, the guard in front) over a
//! store of its own in a temporary directory, as a page on this machine would. Only
//! refusals are exercised, and only ones decided before the router acts on anything: no
//! Keychain, no ~/.claude, no ~/.agent-router, no launchctl, no network and no APNs. That is
//! why the login item, the wrapper's automatic switch and the push test are refused here
//! by a body they cannot read: their other refusals come from launchctl, a store that will
//! not write, or the secrets file in ~/.agent-router.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::proxy::AppState;

/// A router over a store of its own, built as `serve` builds one.
struct Lab {
    app: axum::Router,
    state: Arc<AppState>,
    dir: PathBuf,
    /// What a hosted router asks of every caller, loopback included.
    password: Option<&'static str>,
}

impl Lab {
    fn local(name: &str) -> Self {
        Self::new(name, false)
    }

    /// A hosted router, its password pinned as a deployment pins it at every start.
    async fn hosted(name: &str) -> Self {
        const PASSWORD: &str = "the deployment's own";
        let mut lab = Self::new(name, true);
        lab.state.auth.pin(&lab.state.config.db_path, PASSWORD).await.unwrap();
        lab.password = Some(PASSWORD);
        lab
    }

    fn new(name: &str, server: bool) -> Self {
        // As `serve` does before anything builds a TLS client.
        rustls::crypto::ring::default_provider().install_default().ok();
        let dir = std::env::temp_dir().join(format!("ar-refusals-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("agent-router.db");
        crate::db::Db::open(&db_path).unwrap();
        // Built by hand rather than by `Config::load`, which settles the data directory for
        // the rest of the process — and every other test in it would then share this one.
        let config = crate::config::Config {
            server,
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            upstream: "https://upstream.invalid".into(),
            db_path,
            body_cap: 64 * 1024,
            dump_headers: 0,
            allow_hosts: Vec::new(),
            public_url: server.then(|| "https://router.invalid".to_string()),
            push_keys_dir: None,
            drain: Duration::from_secs(1),
        };
        let (capture, _) = tokio::sync::mpsc::channel(1);
        // The other half of the switch is dropped, never supervised: a refusal stops before
        // the socket, and anything that got that far would fail at once rather than wait.
        let (network, _) = crate::network::switch();
        let state = AppState::new(config, capture, network).unwrap();
        Self { app: crate::app(state.clone()), state, dir, password: None }
    }

    /// POST `body` as JSON, from this machine, and read the answer.
    async fn post(&self, path: &str, body: &str) -> (StatusCode, Value) {
        self.send(path, Some("application/json"), body).await
    }

    async fn send(&self, path: &str, content_type: Option<&str>, body: &str) -> (StatusCode, Value) {
        let mut req = Request::post(path).header(header::HOST, "127.0.0.1");
        if let Some(t) = content_type {
            req = req.header(header::CONTENT_TYPE, t);
        }
        if let Some(p) = self.password {
            req = req.header(header::AUTHORIZATION, format!("Bearer {p}"));
        }
        let mut req = req.body(Body::from(body.to_string())).unwrap();
        // What `serve` hands the guard about every connection: here, one from loopback.
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50_000))));
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| panic!("{path} answered {status} with no JSON: {}", String::from_utf8_lossy(&bytes)));
        (status, body)
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The words of a refusal answered with `status`, once it is known to have the shape every
/// client reads: `ok: false`, and the words under `error`.
fn words((got, body): (StatusCode, Value), status: StatusCode) -> String {
    assert_eq!(got, status, "{body}");
    assert!(got.is_client_error(), "a refusal is a 4xx: {got}");
    assert_eq!(body["ok"], json!(false), "{body}");
    let words = body["error"].as_str().unwrap_or_else(|| panic!("no words under `error`: {body}"));
    assert!(!words.trim().is_empty(), "{body}");
    words.to_string()
}

/// The wrapper switches' words: under `reason` as they always were, and under `error` too.
fn reason(answer: (StatusCode, Value), status: StatusCode) -> String {
    let body = answer.1.clone();
    let words = words(answer, status);
    assert_eq!(body["reason"].as_str(), Some(words.as_str()), "{body}");
    words
}

#[tokio::test]
async fn a_password_too_short_to_keep_is_refused_rather_than_set() {
    let lab = Lab::local("auth");
    let refused = lab.post("/api/auth", r#"{"password":"short"}"#).await;
    assert_eq!(words(refused, StatusCode::BAD_REQUEST), "use at least 8 characters");
    assert!(!lab.state.auth.configured().await, "nothing was set");

    let refused = lab.post("/api/auth", r#"{"password":12345678}"#).await;
    assert!(words(refused, StatusCode::UNPROCESSABLE_ENTITY).contains("password"));
}

#[tokio::test]
async fn the_network_switch_is_refused_without_a_password_and_for_an_address_it_cannot_use() {
    let lab = Lab::local("network");
    let refused = lab.post("/api/network", r#"{"enable":true}"#).await;
    assert_eq!(
        words(refused, StatusCode::CONFLICT),
        "set a password first — until one exists, callers from other machines are refused"
    );

    // With a password set, what is left to refuse is the address itself.
    lab.state.auth.set(&lab.state.config.db_path, "correct horse").await.unwrap();
    let refused = lab.post("/api/network", r#"{"enable":true,"address":"the office"}"#).await;
    assert_eq!(words(refused, StatusCode::BAD_REQUEST), r#""the office" is not an IP address"#);
    let refused = lab.post("/api/network", r#"{"enable":true,"address":"127.0.0.1"}"#).await;
    assert_eq!(words(refused, StatusCode::BAD_REQUEST), "127.0.0.1 is loopback, which is what this turns off");
    assert_eq!(lab.state.network.current(), None, "the socket never moved");
}

/// Its only other refusals are launchctl's and the file system's.
#[tokio::test]
async fn the_login_item_switch_refuses_a_body_it_cannot_read_in_words() {
    let lab = Lab::local("service");
    let refused = lab.post("/api/setup/service", "{}").await;
    assert!(words(refused, StatusCode::UNPROCESSABLE_ENTITY).contains("missing field `enable`"));
    let refused = lab.post("/api/setup/service", "on").await;
    assert!(words(refused, StatusCode::BAD_REQUEST).contains("JSON"));
    let refused = lab.send("/api/setup/service", None, r#"{"enable":true}"#).await;
    assert!(words(refused, StatusCode::UNSUPPORTED_MEDIA_TYPE).contains("Content-Type"));
}

#[tokio::test]
async fn routing_a_project_that_is_not_a_directory_is_refused() {
    let lab = Lab::local("config");
    let missing = lab.dir.join("no-such-project");
    let body = json!({ "enable": true, "project": missing.to_str().unwrap() }).to_string();
    let refused = lab.post("/api/setup/config", &body).await;
    assert_eq!(words(refused, StatusCode::BAD_REQUEST), format!("{} is not a directory", missing.display()));
}

/// The listing a path is checked against is only read, and a path in a fresh temporary
/// directory is in it on no machine.
#[tokio::test]
async fn the_wrapper_switches_refuse_with_their_reason_and_the_same_words_as_error() {
    let lab = Lab::local("shim");
    let stranger = lab.dir.join("claude");
    let body = json!({ "path": stranger.to_str().unwrap(), "action": "replace" }).to_string();
    let refused = lab.post("/api/setup/shim", &body).await;
    assert_eq!(reason(refused, StatusCode::BAD_REQUEST), r#"unknown action "replace""#);

    let body = json!({ "path": stranger.to_str().unwrap(), "action": "install" }).to_string();
    let refused = lab.post("/api/setup/shim", &body).await;
    assert_eq!(reason(refused, StatusCode::CONFLICT), "Not a known extension binary.");
    assert!(!stranger.exists(), "nothing was written there");

    // The automatic switch's only other refusal is the store failing.
    let refused = lab.post("/api/setup/shim/auto", r#"{"enabled":"yes"}"#).await;
    assert!(words(refused, StatusCode::UNPROCESSABLE_ENTITY).contains("enabled"));
    assert!(!crate::shim::auto_enabled(&lab.state.config.db_path), "the switch never moved");
}

#[tokio::test]
async fn a_push_key_that_is_incomplete_or_does_not_sign_is_refused_rather_than_saved() {
    let lab = Lab::local("push-config");
    let refused = lab.post("/api/push/config", r#"{"team_id":"ABCDE12345","key_id":"","p8":""}"#).await;
    assert_eq!(words(refused, StatusCode::BAD_REQUEST), "team id, key id and the .p8 key are all needed");
    let refused = lab.post("/api/push/config", r#"{"team_id":"ABCDE12345","key_id":"ABC123DEF4","p8":"not a key"}"#).await;
    assert!(words(refused, StatusCode::UNPROCESSABLE_ENTITY).starts_with("that key does not sign: "));

    let refused = lab.post("/api/push/config", r#"{"platform":"fcm"}"#).await;
    assert_eq!(
        words(refused, StatusCode::BAD_REQUEST),
        "paste the service-account JSON the Firebase console downloads"
    );
    let file = json!({ "platform": "fcm", "service_account": r#"{"type":"authorized_user"}"# }).to_string();
    let refused = lab.post("/api/push/config", &file).await;
    assert!(words(refused, StatusCode::BAD_REQUEST).starts_with("that is not a service account: "));
    let file = json!({
        "platform": "fcm",
        "service_account": json!({
            "type": "service_account", "project_id": "p", "client_email": "push@p.iam.gserviceaccount.com",
            "private_key": "not a key", "private_key_id": "k1",
        }).to_string(),
    })
    .to_string();
    let refused = lab.post("/api/push/config", &file).await;
    assert!(words(refused, StatusCode::UNPROCESSABLE_ENTITY).starts_with("that key does not sign: "));
}

/// Its other refusal, no credentials set, is decided by the secrets file on the machine.
#[tokio::test]
async fn a_push_test_refuses_a_body_it_cannot_read_in_words() {
    let lab = Lab::local("push-test");
    let refused = lab.post("/api/push/test", "test").await;
    assert!(words(refused, StatusCode::BAD_REQUEST).contains("JSON"));
    let refused = lab.post("/api/push/test", r#"{"session":42}"#).await;
    assert!(words(refused, StatusCode::UNPROCESSABLE_ENTITY).contains("session"));
}

#[tokio::test]
async fn a_usage_key_that_is_missing_or_not_a_session_key_is_refused() {
    let lab = Lab::local("usage-token");
    let refused = lab.post("/api/usage-token", r#"{"account_id":"default","token":"  "}"#).await;
    assert_eq!(words(refused, StatusCode::BAD_REQUEST), "no token supplied");
    let refused = lab.post("/api/usage-token", r#"{"account_id":"default","token":"sk-ant-api03-abc"}"#).await;
    assert_eq!(
        words(refused, StatusCode::BAD_REQUEST),
        "that does not look like a claude.ai session key (expected sk-ant-sid…)"
    );
}

#[tokio::test]
async fn a_sign_in_with_no_name_or_while_another_runs_is_refused() {
    let lab = Lab::local("login");
    let refused = lab.post("/api/login/start", r#"{"label":"   "}"#).await;
    assert_eq!(words(refused, StatusCode::BAD_REQUEST), "give the account a name first");

    lab.state.login.lock().unwrap().phase = Some(crate::login::Phase::AwaitingSignIn);
    let refused = lab.post("/api/login/start", r#"{"label":"work"}"#).await;
    assert_eq!(words(refused, StatusCode::CONFLICT), "a sign-in is already in progress");
}

/// A sign-in whose `claude` cannot be started ends Failed, saying why, and the next start is
/// taken. It once stood Starting for good, which reads as a sign-in still running, so every
/// later start was refused as one until the router restarted.
///
/// Not a refusal but the router failing (500), and it acts: under test a sign-in runs a
/// program no machine has (`login::CLAUDE`), in a profile directory of the test process's
/// own (`accounts::profiles_root`), never ~/.agent-router. Unix only, as signing in is:
/// elsewhere it stops before it starts, at the `$BROWSER` stand-in it cannot write.
#[cfg(unix)]
#[tokio::test]
async fn a_sign_in_that_cannot_start_ends_failed_and_the_next_start_is_taken() {
    let lab = Lab::local("sign-in-cannot-start");
    for attempt in ["the first start", "the next start"] {
        let (status, body) = lab.post("/api/login/start", r#"{"label":"Cannot Start"}"#).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{attempt} is not refused as one running: {body}");
        assert_eq!(body["ok"], json!(false), "{body}");
        let why = body["error"].as_str().unwrap_or_default().to_string();
        assert!(why.contains("starting `claude setup-token`"), "{attempt} says why: {body}");

        // What `GET /api/login/status` answers: ended, in the same words.
        let flow = lab.state.login.lock().unwrap().to_json();
        assert_eq!(flow["phase"], "failed", "{attempt}: {flow}");
        assert_eq!(flow["error"], why.as_str(), "{attempt}: {flow}");
        assert_eq!(flow["label"], "Cannot Start", "{attempt}: {flow}");
    }
    let _ = std::fs::remove_dir_all(crate::accounts::profiles_root().unwrap().join("cannot-start"));
}

/// A hosted router answers the same paths from `hosted`, and refused in this shape before
/// the local ones did: 409 and `{ok: false, error}`. Pinned here so the two stay one shape
/// for the clients that talk to both.
#[tokio::test]
async fn a_hosted_router_refuses_what_would_act_on_its_machine_in_the_same_shape() {
    let lab = Lab::hosted("hosted").await;
    for (path, body) in [
        ("/api/setup/service", r#"{"enable":true}"#),
        ("/api/setup/config", r#"{"enable":true}"#),
        ("/api/setup/shim", r#"{"path":"/x/claude","action":"install"}"#),
        ("/api/setup/shim/auto", r#"{"enabled":true}"#),
        ("/api/network", r#"{"enable":true}"#),
    ] {
        let why = words(lab.post(path, body).await, StatusCode::CONFLICT);
        assert!(why.starts_with("this router is hosted"), "{path}: {why}");
    }
    let refused = lab.post("/api/login/start", r#"{"label":"work"}"#).await;
    assert!(words(refused, StatusCode::CONFLICT).starts_with("signing in runs Claude Code's own login"));
    for (path, body) in [
        ("/api/accounts/probe", r#"{"id":"work"}"#),
        ("/api/accounts", r#"{"label":"work","source":"profile"}"#),
    ] {
        let why = words(lab.post(path, body).await, StatusCode::CONFLICT);
        assert!(why.starts_with("a profile account is a Claude Code login directory"), "{path}: {why}");
    }
    let refused = lab.post("/api/auth", r#"{"password":"a different one"}"#).await;
    assert!(words(refused, StatusCode::CONFLICT).starts_with("the password is set by the deployment"));
}

/// A hosted router reads `POST /api/accounts`'s body itself, to refuse anything but a pasted
/// token before the add sees it (`hosted::accounts_add`). A body it cannot read was refused
/// there in axum's plain text, which no client reads. It is refused as every other is now:
/// the status a router on a machine gives the same body, and its `{ok: false, error}`, word
/// for word.
#[tokio::test]
async fn hosted_accounts_body_refusal_is_the_one_a_router_on_a_machine_gives() {
    let hosted = Lab::hosted("accounts-body-hosted").await;
    let local = Lab::local("accounts-body-local");
    let json = Some("application/json");

    // Not JSON at all: what any body read through `refusal::JsonBody` gets, on any path.
    let there = hosted.send("/api/accounts", json, "hosted-body-work").await;
    assert!(words(there.clone(), StatusCode::BAD_REQUEST).contains("JSON"), "{there:?}");
    assert_eq!(local.send("/api/accounts", json, "hosted-body-work").await, there);
    assert_eq!(local.send("/api/accounts/edit", json, "hosted-body-work").await, there);

    // JSON of the wrong shape, and JSON not said to be.
    let token = r#"{"label":"hosted-body-work","source":"token","token":"sk-ant-oat01-abc"}"#;
    for (content_type, body, status, about) in [
        (json, r#"{"label":"hosted-body-work"}"#, StatusCode::UNPROCESSABLE_ENTITY, "source"),
        (None, token, StatusCode::UNSUPPORTED_MEDIA_TYPE, "Content-Type"),
    ] {
        let there = hosted.send("/api/accounts", content_type, body).await;
        assert!(words(there.clone(), status).contains(about), "{body}: {there:?}");
        assert_eq!(local.send("/api/accounts", content_type, body).await, there, "{body}");
    }
    assert_eq!(held(&hosted, "hosted-body-work"), None, "nothing was added");
    assert_eq!(held(&local, "hosted-body-work"), None);
}

// ---------------------------------------------------------------------------
// Push registration, and the accounts writes
// ---------------------------------------------------------------------------
//
// The accounts tests keep a token where the router keeps one, in its secrets file. Under
// test that file is in a directory of the test process's own (`accounts::secrets`), never
// in ~/.agent-router, and every id here is used by one test only.

/// A device token of each kind, in the shapes that occur.
const APNS: &str = "740f4707bebcf74f9b7c25d48e3358945f6aa01da5ddb387462c7eaf61bb78ad";
const FCM: &str = "cjE9vT2zRk6mQq1sLp0aBc:APA91bH-x_9Kd3fQz7YwVn2pRt5uXsA0bCdEfGhIjKlMnOpQrStUvWxYz01234567890abcdefghijklmnopqrstuvwxyzAB";

/// Only the phones call these. A refusal was a 200 carrying `error` until it was not.
#[tokio::test]
async fn push_registration_refusal_is_an_error_status_in_words() {
    let lab = Lab::local("push-register");
    let junk = [
        r#"{"token":"hello"}"#.to_string(),
        r#"{"token":"   "}"#.to_string(),
        // A service the router does not speak, and a token that is not what it claims.
        json!({ "token": APNS, "platform": "webpush" }).to_string(),
        json!({ "token": FCM, "platform": "apns" }).to_string(),
    ];
    for body in &junk {
        let refused = lab.post("/api/push/register", body).await;
        assert_eq!(words(refused, StatusCode::BAD_REQUEST), "that is not a device token", "{body}");
    }
    let refused = lab.post("/api/push/register", r#"{"name":"a phone"}"#).await;
    assert!(words(refused, StatusCode::UNPROCESSABLE_ENTITY).contains("token"));
    let refused = lab.post("/api/push/register", "register this phone").await;
    assert!(words(refused, StatusCode::BAD_REQUEST).contains("JSON"));
    let refused = lab.send("/api/push/register", None, &json!({ "token": APNS }).to_string()).await;
    assert!(words(refused, StatusCode::UNSUPPORTED_MEDIA_TYPE).contains("Content-Type"));
    assert!(lab.state.push.devices().is_empty(), "no phone was registered");

    let refused = lab.post("/api/push/unregister", r#"{"token":"  "}"#).await;
    assert_eq!(words(refused, StatusCode::BAD_REQUEST), "no device token supplied");
    let refused = lab.post("/api/push/unregister", r#"{"token":42}"#).await;
    assert!(words(refused, StatusCode::UNPROCESSABLE_ENTITY).contains("token"));
    let refused = lab.post("/api/push/unregister", "forget me").await;
    assert!(words(refused, StatusCode::BAD_REQUEST).contains("JSON"));
}

/// An account as far as signing goes: its name, where its credential comes from, its
/// profile directory, and the token stored under its id.
#[derive(Debug, Clone, PartialEq)]
struct Held {
    label: String,
    source: String,
    profile_dir: Option<String>,
    token: Option<String>,
}

/// What the account with this id is now, if there is one.
fn held(lab: &Lab, id: &str) -> Option<Held> {
    use rusqlite::OptionalExtension;
    let conn = rusqlite::Connection::open(&lab.state.config.db_path).unwrap();
    let (label, source, profile_dir) = conn
        .query_row("SELECT label, source, profile_dir FROM accounts WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()
        .unwrap()?;
    // Read as the router reads a token account's credential to sign with.
    let as_token = crate::accounts::Account {
        id: id.to_string(),
        label: String::new(),
        source: crate::accounts::Source::Token,
        profile_dir: None,
        email: None,
    };
    let token = crate::accounts::resolve(&as_token).ok().map(|r| r.token);
    Some(Held { label, source, profile_dir, token })
}

/// An account already in the store, as an earlier add would have left it.
fn existing(lab: &Lab, id: &str, label: &str, source: &str, profile_dir: Option<&str>, token: Option<&str>) -> Held {
    let conn = rusqlite::Connection::open(&lab.state.config.db_path).unwrap();
    conn.execute(
        "INSERT INTO accounts (id, label, source, profile_dir, active, created_at) VALUES (?1, ?2, ?3, ?4, 0, 1)",
        rusqlite::params![id, label, source, profile_dir],
    )
    .unwrap();
    if let Some(token) = token {
        crate::accounts::store_token(id, token).unwrap();
    }
    let now = held(lab, id).unwrap();
    assert_eq!(now.token.as_deref(), token, "the store holds what the test put there");
    now
}

/// The token an account already has, and the one a clashing add brings. Neither is a
/// credential.
const THEIRS: &str = "sk-ant-oat01-theirs-QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8";
const OURS: &str = "sk-ant-oat01-ours-JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9JX8QZ9";

/// A refused add's words name the account that has the id: its id and its name.
fn names(why: &str, id: &str, holder: &str) {
    assert!(why.contains(&format!("{id:?}")), "the id is said: {why}");
    assert!(why.contains(&format!("{holder:?}")), "the account is named: {why}");
}

#[tokio::test]
async fn account_name_clash_refuses_a_token_account_and_leaves_the_one_there() {
    let lab = Lab::local("clash-token");
    let before = existing(&lab, "clash-token-work", "Clash token work", "token", None, Some(THEIRS));
    let body = json!({ "label": "CLASH Token-Work", "source": "token", "token": OURS }).to_string();
    let why = words(lab.post("/api/accounts", &body).await, StatusCode::CONFLICT);
    names(&why, "clash-token-work", "Clash token work");
    assert_eq!(held(&lab, "clash-token-work"), Some(before), "its name, source and token are its own");

    // The built-in account: a new "Default" would have made it a token account.
    let before = existing(&lab, "default", "Claude Code (default profile)", "keychain", None, None);
    let body = json!({ "label": "Default", "source": "token", "token": OURS }).to_string();
    let why = words(lab.post("/api/accounts", &body).await, StatusCode::CONFLICT);
    names(&why, "default", "Claude Code (default profile)");
    assert_eq!(held(&lab, "default"), Some(before), "still Claude Code's own login, with no token of ours");
}

#[tokio::test]
async fn account_name_clash_refuses_a_profile_account_and_leaves_the_one_there() {
    let lab = Lab::local("clash-profile");
    let before = existing(&lab, "clash-profile-work", "Clash profile work", "token", None, Some(THEIRS));
    let body = json!({ "label": "Clash Profile Work", "source": "profile" }).to_string();
    let why = words(lab.post("/api/accounts", &body).await, StatusCode::CONFLICT);
    names(&why, "clash-profile-work", "Clash profile work");
    assert_eq!(held(&lab, "clash-profile-work"), Some(before));

    // A profile account, whose credential is its directory: neither kind of add takes it.
    let dir = lab.dir.join("accounts").join("clash-profile-home");
    let before = existing(&lab, "clash-profile-home", "Clash profile home", "profile", dir.to_str(), None);
    for body in [
        json!({ "label": "clash profile HOME", "source": "token", "token": OURS }),
        json!({ "label": "Clash Profile Home", "source": "profile" }),
    ] {
        let why = words(lab.post("/api/accounts", &body.to_string()).await, StatusCode::CONFLICT);
        names(&why, "clash-profile-home", "Clash profile home");
        assert_eq!(held(&lab, "clash-profile-home").as_ref(), Some(&before), "{body}");
    }
}

/// Refused before `claude setup-token` is run at all: its token would have been stored
/// over this account's when the reader finished signing in.
#[tokio::test]
async fn account_name_clash_refuses_a_sign_in_before_it_starts() {
    let lab = Lab::local("clash-sign-in");
    let before = existing(&lab, "clash-signin-work", "Clash sign-in work", "token", None, Some(THEIRS));
    let why = words(lab.post("/api/login/start", r#"{"label":"Clash Signin Work"}"#).await, StatusCode::CONFLICT);
    names(&why, "clash-signin-work", "Clash sign-in work");
    assert_eq!(lab.state.login.lock().unwrap().phase, None, "no sign-in was started");
    assert_eq!(held(&lab, "clash-signin-work"), Some(before));
}

/// 400 for what is wrong whatever the store holds, 409 for what the store's state stands
/// in the way of — never the 500 every one of these once was.
#[tokio::test]
async fn accounts_bad_input_refusal_is_400_and_a_conflict_with_the_store_is_409() {
    let lab = Lab::local("accounts-writes");
    let default = existing(&lab, "default", "Claude Code (default profile)", "keychain", None, None);
    let work = existing(&lab, "bad-input-work", "Bad input work", "token", None, None);

    for (body, said) in [
        (r#"{"label":"x","source":"keychain"}"#, "the default Keychain account is discovered automatically and cannot be added"),
        (r#"{"label":"x","source":"oauth"}"#, r#"unknown account source "oauth""#),
        (r#"{"label":"x","source":"token"}"#, "a token is required for this account type"),
        (r#"{"label":"x","source":"token","token":"   "}"#, "a token is required for this account type"),
    ] {
        assert_eq!(words(lab.post("/api/accounts", body).await, StatusCode::BAD_REQUEST), said, "{body}");
    }

    for (body, status, said) in [
        (r#"{"id":"bad-input-work","email":"me at work"}"#, StatusCode::BAD_REQUEST, r#""me at work" is not an email address"#),
        (
            r#"{"id":"bad-input-work","token":"sk-ant-sid01-abc"}"#,
            StatusCode::BAD_REQUEST,
            "that is a claude.ai session key — it belongs in the session-key field, and cannot sign API requests",
        ),
        (r#"{"id":"bad-input-work","token":"hunter2"}"#, StatusCode::BAD_REQUEST, "expected a token starting sk-ant- (from `claude setup-token`)"),
        (r#"{"id":"nobody","label":"x"}"#, StatusCode::CONFLICT, r#"no account with id "nobody""#),
        (
            r#"{"id":"default","token":"sk-ant-oat01-abc"}"#,
            StatusCode::CONFLICT,
            "a keychain account takes its credential from Claude Code, so there is nothing here to replace — log in again instead",
        ),
    ] {
        assert_eq!(words(lab.post("/api/accounts/edit", body).await, status), said, "{body}");
    }

    let refused = lab.post("/api/accounts/activate", r#"{"id":"nobody"}"#).await;
    assert_eq!(words(refused, StatusCode::CONFLICT), r#"no account with id "nobody""#);
    let refused = lab.post("/api/accounts/remove", r#"{"id":"default"}"#).await;
    assert_eq!(
        words(refused, StatusCode::BAD_REQUEST),
        "the default account is discovered automatically and cannot be removed"
    );
    let refused = lab.post("/api/accounts/probe", r#"{"id":"nobody"}"#).await;
    assert_eq!(words(refused, StatusCode::CONFLICT), r#"no account with id "nobody""#);
    let refused = lab.post("/api/accounts/probe", r#"{"id":"bad-input-work"}"#).await;
    assert!(words(refused, StatusCode::CONFLICT).starts_with(r#""bad-input-work" is not a profile account"#));

    // A body they cannot read, in the same shape as every other refusal.
    for path in ["/api/accounts/edit", "/api/accounts/activate", "/api/accounts/remove", "/api/accounts/probe"] {
        assert!(words(lab.post(path, "default").await, StatusCode::BAD_REQUEST).contains("JSON"), "{path}");
        assert!(words(lab.post(path, "{}").await, StatusCode::UNPROCESSABLE_ENTITY).contains("id"), "{path}");
    }

    assert_eq!(held(&lab, "default"), Some(default), "nothing refused changed anything");
    assert_eq!(held(&lab, "bad-input-work"), Some(work));
    assert_eq!(held(&lab, "x"), None);
    assert_eq!(held(&lab, "nobody"), None);
}

/// A hosted router signs only with a pasted token, so a Claude Code login that came with
/// a copied store is the state in the way of switching to it.
#[tokio::test]
async fn accounts_bad_input_refusal_on_a_hosted_router_names_the_login_it_cannot_sign_with() {
    let lab = Lab::hosted("accounts-hosted").await;
    existing(&lab, "default", "Claude Code (default profile)", "keychain", None, None);
    let refused = lab.post("/api/accounts/activate", r#"{"id":"default"}"#).await;
    assert_eq!(words(refused, StatusCode::CONFLICT), crate::accounts::NOT_ON_A_SERVER);
    let refused = lab.post("/api/accounts/activate", r#"{"id":"nobody"}"#).await;
    assert_eq!(words(refused, StatusCode::CONFLICT), r#"no account with id "nobody""#);
}
