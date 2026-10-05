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
