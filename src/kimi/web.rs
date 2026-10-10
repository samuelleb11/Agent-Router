//! Hosted Kimi Code sessions (AGEN-0111): sessions the router starts and drives, through
//! Kimi Code's own server, `kimi web`. The agent runs that server on loopback, on the owner's
//! Kimi Code home, so a hosted session is a Kimi Code session like any other, also
//! resumable in the terminal later. Sessions started anywhere else are only watched
//! (`session`): Kimi Code keeps each in its own process, and a second process resuming one
//! interleaves its wire (measured), so the router never drives a session it did not start.
//!
//! What the server was measured to do (Kimi Code 2.1.1, fixtures/kimi/web/):
//! - its bearer is `$KIMI_CODE_HOME/server.token`, and every answer is
//!   `{"code": 0, "msg", "data", "request_id"}`, another `code` saying why not;
//! - `POST /api/v1/sessions` with the directory starts a session, but keeps no model: the
//!   model, like the permission mode, is set with `POST …/profile`;
//! - `POST …/prompts` sends a prompt and names it; `POST …/prompts/<prompt>:abort` interrupts it;
//! - `GET …/approvals?status=pending` lists what waits on a person, and
//!   `POST …/approvals/<id>` answers it.
//!
//! The server is started on first use and stopped once nothing it hosts has been used for
//! `IDLE`. Its port and process are kept beside the router's data, so an agent that replaced
//! the one that started it takes it over rather than leaving it running unowned.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use axum::http::{header, Method};
use axum::body::Bytes;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::library::sync::Router;

/// How long the server may sit with nothing it hosts in use before it is stopped.
pub const IDLE: Duration = Duration::from_secs(30 * 60);

/// How long a started server has to answer.
const STARTING: Duration = Duration::from_secs(30);

/// How long one request to the server may take.
const ASKING: Duration = Duration::from_secs(30);

/// Kimi Code's server, as a client of its API.
pub struct Server {
    api: Router,
}

/// An approval waiting on a person.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Approval {
    pub approval_id: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    /// What it would do, in Kimi Code's words: "Running: echo hello".
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

impl Server {
    /// The server at `base` (`http://127.0.0.1:<port>`), with its bearer.
    pub fn new(base: &str, token: &str) -> Result<Self> {
        Ok(Self { api: Router::new(base, token)? })
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let headers = if body.is_some() { vec![(header::CONTENT_TYPE, "application/json".to_string())] } else { Vec::new() };
        let body = body.map(|b| Bytes::from(b.to_string())).unwrap_or_default();
        let (status, _, answer) = tokio::time::timeout(ASKING, self.api.send(method, path, headers, body))
            .await
            .map_err(|_| anyhow!("Kimi Code's server did not answer {path} in time"))??;
        let answer: Value = serde_json::from_slice(&answer).with_context(|| format!("Kimi Code's server answered {path} with {status}"))?;
        match answer.get("code").and_then(Value::as_i64) {
            Some(0) if status.is_success() => Ok(answer.get("data").cloned().unwrap_or(Value::Null)),
            _ => {
                let why = answer.get("msg").or_else(|| answer.get("message")).and_then(Value::as_str).unwrap_or("no reason given");
                bail!("Kimi Code's server refused {path} ({status}): {why}")
            }
        }
    }

    /// Start a session in `cwd` on `model`, asking before each tool runs. Says its id.
    pub async fn start(&self, cwd: &str, model: &str) -> Result<String> {
        let body = json!({ "metadata": { "cwd": cwd }, "agent_config": { "permission_mode": "manual" } });
        let made = self.call(Method::POST, "/api/v1/sessions", Some(body)).await?;
        let id = made.get("id").and_then(Value::as_str).ok_or_else(|| anyhow!("Kimi Code's server named no session"))?.to_string();
        // A model given when the session is made is not kept (measured): it is set after.
        self.profile(&id, Some(model), None).await?;
        Ok(id)
    }

    /// Set the session's model, its permission mode (`manual`, `auto`, `yolo`), or both.
    pub async fn profile(&self, session: &str, model: Option<&str>, mode: Option<&str>) -> Result<()> {
        let mut config = serde_json::Map::new();
        if let Some(m) = model {
            config.insert("model".into(), json!(m));
        }
        if let Some(m) = mode {
            config.insert("permission_mode".into(), json!(m));
        }
        self.call(Method::POST, &format!("/api/v1/sessions/{session}/profile"), Some(json!({ "agent_config": config }))).await?;
        Ok(())
    }

    /// Send a prompt. Says its id, which an interrupt names.
    pub async fn prompt(&self, session: &str, text: &str) -> Result<String> {
        let body = json!({ "content": [{ "type": "text", "text": text }] });
        let sent = self.call(Method::POST, &format!("/api/v1/sessions/{session}/prompts"), Some(body)).await?;
        sent.get("prompt_id").and_then(Value::as_str).map(str::to_owned).ok_or_else(|| anyhow!("Kimi Code's server named no prompt"))
    }

    /// Interrupt the prompt `prompt`. Says whether it was running.
    pub async fn interrupt(&self, session: &str, prompt: &str) -> Result<bool> {
        let done = self.call(Method::POST, &format!("/api/v1/sessions/{session}/prompts/{prompt}:abort"), Some(json!({}))).await?;
        Ok(done.get("aborted").and_then(Value::as_bool).unwrap_or(false))
    }

    /// What waits on a person.
    pub async fn pending(&self, session: &str) -> Result<Vec<Approval>> {
        let listed = self.call(Method::GET, &format!("/api/v1/sessions/{session}/approvals?status=pending"), None).await?;
        Ok(serde_json::from_value(listed.get("items").cloned().unwrap_or_else(|| json!([])))?)
    }

    /// Answer an approval: run it, or not.
    pub async fn answer(&self, session: &str, approval: &str, approve: bool) -> Result<()> {
        let decision = if approve { "approved" } else { "rejected" };
        self.call(Method::POST, &format!("/api/v1/sessions/{session}/approvals/{approval}"), Some(json!({ "decision": decision })))
            .await?;
        Ok(())
    }

    /// The session as the server has it: `busy`, `pending_interaction`, its model.
    pub async fn session(&self, session: &str) -> Result<Value> {
        self.call(Method::GET, &format!("/api/v1/sessions/{session}"), None).await
    }

    /// What the session runs on now: `model`, `permission`, `busy`.
    #[cfg_attr(not(test), expect(dead_code, reason = "scripts/check-kimi-hosting.sh reads it"))]
    pub async fn status(&self, session: &str) -> Result<Value> {
        self.call(Method::GET, &format!("/api/v1/sessions/{session}/status"), None).await
    }

    async fn shutdown(&self) -> Result<()> {
        self.call(Method::POST, "/api/v1/shutdown", Some(json!({}))).await.map(|_| ())
    }
}

/// The server process the agent started, and how to reach it. No child when it was started
/// by the agent this one replaced, or handed over by a test.
struct Running {
    child: Option<tokio::process::Child>,
    server: std::sync::Arc<Server>,
}

impl Running {
    /// Whether it still answers.
    async fn alive(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => child.try_wait().is_ok_and(|done| done.is_none()),
            None => self.server.call(Method::GET, "/api/v1/sessions?limit=1", None).await.is_ok(),
        }
    }
}

/// What the agent keeps of the sessions it hosts.
#[derive(Default)]
struct Hosting {
    running: Option<Running>,
    /// Each hosted session's last prompt, which an interrupt names.
    prompts: HashMap<String, String>,
    used: Option<Instant>,
}

/// The server the agent started, kept so the agent after it can take it over.
#[derive(serde::Serialize, Deserialize)]
struct Started {
    port: u16,
}

/// The agent's hosted sessions, and the `kimi web` that runs them.
pub struct Host {
    kimi: PathBuf,
    home: PathBuf,
    /// Set for the server besides what the agent has: `KIMI_CODE_HOME`, and for a check,
    /// `HOME` and the rest of a scratch home.
    env: Vec<(String, String)>,
    /// Where the sessions it started are kept, and beside it the server it started.
    kept: PathBuf,
    /// The sessions it started, kept at `kept` across restarts. Read without waiting, for
    /// the list.
    hosted: std::sync::Mutex<HashSet<String>>,
    state: Mutex<Hosting>,
}

impl Host {
    /// Sessions hosted by Kimi Code's `kimi` on the home `home`, the ones it started kept at
    /// `kept`.
    pub fn new(kimi: PathBuf, home: PathBuf, env: Vec<(String, String)>, kept: PathBuf) -> Self {
        let hosted = std::fs::read(&kept).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { kimi, home, env, kept, hosted: std::sync::Mutex::new(hosted), state: Mutex::new(Hosting::default()) }
    }

    /// Hosting through a server already running, which it never stops: for tests.
    #[cfg(test)]
    pub fn serving(server: Server, kept: PathBuf) -> Self {
        let host = Self::new(PathBuf::from("/nonexistent/kimi"), PathBuf::from("/nonexistent"), Vec::new(), kept);
        host.state.try_lock().unwrap().running = Some(Running { child: None, server: std::sync::Arc::new(server) });
        host
    }

    /// This machine's: its Kimi Code, its home, and the sessions kept in the router's data.
    pub fn here() -> Option<Self> {
        let kimi = crate::clients::kimi_command()?;
        let home = super::home()?;
        let kept = crate::config::data_dir().ok()?.join("kimi-hosted.json");
        let env = vec![("KIMI_CODE_HOME".to_string(), home.display().to_string())];
        Some(Self::new(kimi, home, env, kept))
    }

    /// Whether the router started this session, and so may drive it.
    pub fn hosts(&self, session: &str) -> bool {
        self.hosted.lock().unwrap_or_else(|p| p.into_inner()).contains(session)
    }

    fn started_path(&self) -> PathBuf {
        self.kept.with_file_name("kimi-web.json")
    }

    /// The server, started if it is not running: the one this agent started, else the one
    /// the agent before it started, else a new one.
    async fn server(&self) -> Result<std::sync::Arc<Server>> {
        let mut state = self.state.lock().await;
        state.used = Some(Instant::now());
        if let Some(running) = state.running.as_mut() {
            if running.alive().await {
                return Ok(running.server.clone());
            }
        }
        let running = match self.adopt().await {
            Some(running) => running,
            None => self.spawn().await?,
        };
        let server = running.server.clone();
        state.running = Some(running);
        Ok(server)
    }

    /// The server the agent before this one started, if it still answers with this home's
    /// token.
    async fn adopt(&self) -> Option<Running> {
        let started: Started = serde_json::from_slice(&std::fs::read(self.started_path()).ok()?).ok()?;
        let token = std::fs::read_to_string(self.home.join("server.token")).ok()?;
        let server = Server::new(&format!("http://127.0.0.1:{}", started.port), token.trim()).ok()?;
        let mut running = Running { child: None, server: std::sync::Arc::new(server) };
        running.alive().await.then_some(running)
    }

    async fn spawn(&self) -> Result<Running> {
        // A port of the kernel's choosing, let go just before the server takes it.
        let port = std::net::TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port();
        let mut cmd = tokio::process::Command::new(&self.kimi);
        cmd.args(["web", "--port", &port.to_string(), "--no-open"]);
        cmd.envs(self.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        cmd.kill_on_drop(true);
        let mut child = cmd.spawn().with_context(|| format!("could not start {} web", self.kimi.display()))?;
        let base = format!("http://127.0.0.1:{port}");
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                bail!("Kimi Code's server stopped as it started ({status})");
            }
            // Any answer says it is up; its token is written by then.
            let probe = Server::new(&base, "")?;
            if probe.api.send(Method::GET, "/api/v1/meta", Vec::new(), Bytes::new()).await.is_ok() {
                break;
            }
            if started.elapsed() > STARTING {
                let _ = child.kill().await;
                bail!("Kimi Code's server did not answer within {}s", STARTING.as_secs());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let token = std::fs::read_to_string(self.home.join("server.token")).context("Kimi Code's server wrote no token")?;
        let server = std::sync::Arc::new(Server::new(&base, token.trim())?);
        crate::setup::write_atomic(&self.started_path(), serde_json::to_vec(&Started { port })?)?;
        Ok(Running { child: Some(child), server })
    }

    /// Start a hosted session in `cwd` on `model`. Says its id.
    pub async fn start(&self, cwd: &Path, model: &str) -> Result<String> {
        let server = self.server().await?;
        let id = server.start(&cwd.display().to_string(), model).await?;
        let kept = {
            let mut hosted = self.hosted.lock().unwrap_or_else(|p| p.into_inner());
            hosted.insert(id.clone());
            let mut kept: Vec<String> = hosted.iter().cloned().collect();
            kept.sort();
            kept
        };
        crate::setup::write_atomic(&self.kept, serde_json::to_vec(&kept)?)?;
        Ok(id)
    }

    /// The server, for a session the router hosts; refused for any other.
    async fn driving(&self, session: &str) -> Result<std::sync::Arc<Server>> {
        if !self.hosts(session) {
            bail!("the router did not start this Kimi Code session, so it only watches it");
        }
        self.server().await
    }

    pub async fn prompt(&self, session: &str, text: &str) -> Result<()> {
        let prompt = self.driving(session).await?.prompt(session, text).await?;
        self.state.lock().await.prompts.insert(session.to_string(), prompt);
        Ok(())
    }

    /// Interrupt the session's turn. Says whether one was running.
    pub async fn interrupt(&self, session: &str) -> Result<bool> {
        let server = self.driving(session).await?;
        let Some(prompt) = self.state.lock().await.prompts.get(session).cloned() else { return Ok(false) };
        server.interrupt(session, &prompt).await
    }

    pub async fn pending(&self, session: &str) -> Result<Vec<Approval>> {
        self.driving(session).await?.pending(session).await
    }

    pub async fn answer(&self, session: &str, approval: &str, approve: bool) -> Result<()> {
        self.driving(session).await?.answer(session, approval, approve).await
    }

    pub async fn set_model(&self, session: &str, model: &str) -> Result<()> {
        self.driving(session).await?.profile(session, Some(model), None).await
    }

    /// Set the permission mode: `manual`, `auto` or `yolo`.
    pub async fn set_mode(&self, session: &str, mode: &str) -> Result<()> {
        if !matches!(mode, "manual" | "auto" | "yolo") {
            bail!("Kimi Code has no permission mode {mode:?}");
        }
        self.driving(session).await?.profile(session, None, Some(mode)).await
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "scripts/check-kimi-hosting.sh reads it"))]
    pub async fn session(&self, session: &str) -> Result<Value> {
        self.driving(session).await?.session(session).await
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "scripts/check-kimi-hosting.sh reads it"))]
    pub async fn status(&self, session: &str) -> Result<Value> {
        self.driving(session).await?.status(session).await
    }

    /// Stop the server if nothing it hosts has been used for `idle` and none is busy.
    pub async fn stop_if_idle(&self, idle: Duration) -> Result<bool> {
        let server = {
            let state = self.state.lock().await;
            match (&state.running, state.used) {
                (Some(r), Some(used)) if used.elapsed() >= idle => r.server.clone(),
                _ => return Ok(false),
            }
        };
        let hosted: Vec<String> = self.hosted.lock().unwrap_or_else(|p| p.into_inner()).iter().cloned().collect();
        for session in &hosted {
            if server.session(session).await.is_ok_and(|s| s.get("busy").and_then(Value::as_bool) == Some(true)) {
                return Ok(false);
            }
        }
        self.stop().await;
        Ok(true)
    }

    /// Stop the server: asked first, ended if it does not go.
    pub async fn stop(&self) {
        let Some(running) = self.state.lock().await.running.take() else { return };
        let _ = running.server.shutdown().await;
        let _ = std::fs::remove_file(self.started_path());
        if let Some(mut child) = running.child {
            if tokio::time::timeout(Duration::from_secs(5), child.wait()).await.is_err() {
                let _ = child.kill().await;
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::extract::{Path as UrlPath, State};
    use axum::routing::{get, post};
    use axum::Json;
    use std::sync::Arc;

    /// A stand-in of Kimi Code's server: the answers recorded from the real one, and every
    /// request it was sent.
    #[derive(Clone, Default)]
    pub(crate) struct Standin {
        heard: Arc<std::sync::Mutex<Vec<(String, String, Value, Option<String>)>>>,
    }

    fn recorded(name: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/kimi/web").join(name);
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    pub(crate) async fn standin() -> (String, Standin) {
        let s = Standin::default();
        let note = |s: &Standin, method: &str, path: String, body: Value, auth: Option<String>| {
            s.heard.lock().unwrap().push((method.into(), path, body, auth));
        };
        let auth = |h: &axum::http::HeaderMap| h.get("authorization").and_then(|v| v.to_str().ok()).map(str::to_owned);
        let app = axum::Router::new()
            .route(
                "/api/v1/sessions",
                post(move |State(s): State<Standin>, h: axum::http::HeaderMap, Json(b): Json<Value>| async move {
                    note(&s, "POST", "/api/v1/sessions".into(), b, auth(&h));
                    Json(recorded("created.json"))
                })
                // Asked only to see the server is up, so not noted.
                .get(|| async { Json(recorded("sessions.json")) }),
            )
            .route(
                "/api/v1/sessions/{id}/profile",
                post(move |State(s): State<Standin>, UrlPath(id): UrlPath<String>, h: axum::http::HeaderMap, Json(b): Json<Value>| async move {
                    note(&s, "POST", format!("/api/v1/sessions/{id}/profile"), b, auth(&h));
                    Json(recorded("session.json"))
                }),
            )
            .route(
                "/api/v1/sessions/{id}/prompts",
                post(move |State(s): State<Standin>, UrlPath(id): UrlPath<String>, h: axum::http::HeaderMap, Json(b): Json<Value>| async move {
                    note(&s, "POST", format!("/api/v1/sessions/{id}/prompts"), b, auth(&h));
                    Json(json!({ "code": 0, "msg": "success", "data": { "prompt_id": "msg_01PROMPT", "status": "running" }, "request_id": "r" }))
                }),
            )
            .route(
                "/api/v1/sessions/{id}/prompts/{tail}",
                post(move |State(s): State<Standin>, UrlPath((id, tail)): UrlPath<(String, String)>, h: axum::http::HeaderMap| async move {
                    note(&s, "POST", format!("/api/v1/sessions/{id}/prompts/{tail}"), Value::Null, auth(&h));
                    Json(json!({ "code": 0, "msg": "success", "data": { "aborted": true }, "request_id": "r" }))
                }),
            )
            .route(
                "/api/v1/sessions/{id}/approvals",
                get(move |State(s): State<Standin>, UrlPath(id): UrlPath<String>, h: axum::http::HeaderMap| async move {
                    note(&s, "GET", format!("/api/v1/sessions/{id}/approvals"), Value::Null, auth(&h));
                    Json(recorded("approvals-pending.json"))
                }),
            )
            .route(
                "/api/v1/sessions/{id}/approvals/{approval}",
                post(
                    move |State(s): State<Standin>, UrlPath((id, approval)): UrlPath<(String, String)>, h: axum::http::HeaderMap, Json(b): Json<Value>| async move {
                        note(&s, "POST", format!("/api/v1/sessions/{id}/approvals/{approval}"), b, auth(&h));
                        Json(json!({ "code": 0, "msg": "success", "data": { "resolved": true, "resolved_at": "now" }, "request_id": "r" }))
                    },
                ),
            )
            .with_state(s.clone());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, s)
    }

    impl Standin {
        pub(crate) fn heard(&self) -> Vec<(String, String, Value, Option<String>)> {
            std::mem::take(&mut *self.heard.lock().unwrap())
        }
    }

    const SESSION: &str = "session_fd31bc41-dc2f-432b-8be3-2ae7f81240a6";

    #[tokio::test]
    async fn a_hosted_kimi_session_is_started_in_a_directory() {
        let (base, standin) = standin().await;
        let server = Server::new(&base, "the server's token").unwrap();
        let id = server.start("/Users/u/code/demo", "agent-router/k3").await.unwrap();
        assert_eq!(id, recorded("created.json")["data"]["id"].as_str().unwrap());

        let heard = standin.heard();
        assert_eq!(heard.len(), 2, "{heard:?}");
        let (method, path, body, auth) = &heard[0];
        assert_eq!((method.as_str(), path.as_str()), ("POST", "/api/v1/sessions"));
        assert_eq!(body["metadata"]["cwd"], "/Users/u/code/demo");
        assert_eq!(body["agent_config"]["permission_mode"], "manual", "each tool asks first");
        assert_eq!(auth.as_deref(), Some("Bearer the server's token"));
        // The model, set after: the server does not keep one given with the session.
        let (_, path, body, _) = &heard[1];
        assert_eq!(path, &format!("/api/v1/sessions/{id}/profile"));
        assert_eq!(body["agent_config"]["model"], "agent-router/k3");

        // A Host keeps the session it started, and drives no other.
        let kept = std::env::temp_dir().join(format!("ar-kimi-hosted-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&kept);
        let host = Host::new(PathBuf::from("/nonexistent/kimi"), PathBuf::from("/nonexistent"), Vec::new(), kept.clone());
        assert!(!host.hosts(SESSION));
        let refused = host.prompt(SESSION, "hello").await.unwrap_err();
        assert!(refused.to_string().contains("only watches it"), "{refused}");
        let _ = std::fs::remove_file(&kept);
    }

    #[tokio::test]
    async fn a_hosted_kimi_session_takes_a_prompt_an_answer_an_interrupt_and_a_model() {
        let (base, standin) = standin().await;
        let server = Server::new(&base, "t").unwrap();

        assert_eq!(server.prompt(SESSION, "Run echo, please.").await.unwrap(), "msg_01PROMPT");
        let pending = server.pending(SESSION).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].tool_name.as_deref(), Some("Bash"));
        assert_eq!(pending[0].action.as_deref(), Some("Running: echo hello-from-standin"));
        server.answer(SESSION, &pending[0].approval_id, true).await.unwrap();
        assert!(server.interrupt(SESSION, "msg_01PROMPT").await.unwrap());
        server.profile(SESSION, Some("agent-router/k3-256k"), None).await.unwrap();
        server.profile(SESSION, None, Some("auto")).await.unwrap();

        let heard: Vec<(String, String, Value)> = standin.heard().into_iter().map(|(m, p, b, _)| (m, p, b)).collect();
        let at = |p: &str| format!("/api/v1/sessions/{SESSION}{p}");
        assert_eq!(
            heard,
            [
                ("POST".into(), at("/prompts"), json!({ "content": [{ "type": "text", "text": "Run echo, please." }] })),
                ("GET".into(), at("/approvals"), Value::Null),
                ("POST".into(), at(&format!("/approvals/{}", pending[0].approval_id)), json!({ "decision": "approved" })),
                ("POST".into(), at("/prompts/msg_01PROMPT:abort"), Value::Null),
                ("POST".into(), at("/profile"), json!({ "agent_config": { "model": "agent-router/k3-256k" } })),
                ("POST".into(), at("/profile"), json!({ "agent_config": { "permission_mode": "auto" } })),
            ]
        );

        // A refusal is said in the server's own words.
        let refused = server.call(Method::GET, "/api/v1/nowhere", None).await.unwrap_err();
        assert!(refused.to_string().contains("/api/v1/nowhere"), "{refused}");
    }

    /// The real `kimi web`, started and driven by `Host` on a scratch home with a stand-in
    /// model: run by scripts/check-kimi-hosting.sh, which sets the scratch up.
    #[tokio::test]
    #[ignore = "runs the real kimi web: scripts/check-kimi-hosting.sh"]
    async fn the_real_kimi_web_hosts_a_session() {
        let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} is set by scripts/check-kimi-hosting.sh"));
        let (home, os_home, work) = (PathBuf::from(var("AR_KIMI_HOME")), var("AR_KIMI_OS_HOME"), PathBuf::from(var("AR_KIMI_WORK")));
        let env = vec![
            ("HOME".to_string(), os_home),
            ("KIMI_CODE_HOME".to_string(), home.display().to_string()),
            ("KIMI_CODE_NO_AUTO_UPDATE".to_string(), "1".to_string()),
            ("KIMI_DISABLE_TELEMETRY".to_string(), "1".to_string()),
        ];
        let host = Host::new(PathBuf::from(var("AR_KIMI_BIN")), home.clone(), env, work.join("hosted.json"));
        let id = host.start(&work, "agent-router/k3").await.unwrap();
        assert!(host.hosts(&id));
        println!("started {id}");

        async fn until<F, Fut>(what: &str, mut check: F)
        where
            F: FnMut() -> Fut,
            Fut: std::future::Future<Output = bool>,
        {
            let start = Instant::now();
            while !check().await {
                assert!(start.elapsed() < Duration::from_secs(60), "waited a minute for {what}");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        let busy = |s: Value| s.get("busy").and_then(Value::as_bool) == Some(true);

        host.prompt(&id, "Run echo, please.").await.unwrap();
        until("the approval", || async { !host.pending(&id).await.unwrap().is_empty() }).await;
        let approval = host.pending(&id).await.unwrap().remove(0);
        assert_eq!(approval.tool_name.as_deref(), Some("Bash"));
        host.answer(&id, &approval.approval_id, true).await.unwrap();
        until("the turn's end", || async { !busy(host.session(&id).await.unwrap()) }).await;
        println!("prompted, its approval answered");

        host.set_model(&id, "agent-router/k3-256k").await.unwrap();
        host.set_mode(&id, "auto").await.unwrap();
        let status = host.status(&id).await.unwrap();
        assert_eq!(status["model"], "agent-router/k3-256k");
        assert_eq!(status["permission"], "auto");
        println!("model and mode switched");

        host.set_mode(&id, "manual").await.unwrap();
        host.prompt(&id, "Run echo again.").await.unwrap();
        until("the second approval", || async { !host.pending(&id).await.unwrap().is_empty() }).await;
        assert!(host.interrupt(&id).await.unwrap());
        until("the interrupted turn's end", || async { !busy(host.session(&id).await.unwrap()) }).await;
        assert_eq!(host.session(&id).await.unwrap()["last_turn_reason"], "cancelled");
        println!("interrupted");

        // What it did is in its files, as the watched sessions read them.
        let dir = super::super::session::sessions_in(&home).into_iter().find(|d| d.ends_with(&id)).unwrap();
        let read = super::super::session::read(&dir).unwrap();
        assert!(read.feed().iter().any(|i| i.kind == "tool_result" && i.text.as_deref() == Some("hello-from-standin\n")));

        host.stop().await;
        println!("stopped");
    }
}
