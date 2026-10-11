//! The developer machines whose agents are connected to a hosted router, and the live
//! requests the router relays to them. See `docs/hosting-machine-agent.md`; the frames
//! are `tunnel`'s.
//!
//! A hosted router cannot see a session: its transcript, its registry record and its
//! wrapper socket are on the machine running it, and the router cannot connect in. The
//! agent there connects out instead, and everything the live view needs travels as
//! requests down that connection. The router keeps only what it must to route them —
//! which machine is connected, and which machine each session was last listed on — and
//! derives nothing itself: each machine's activity is computed on that machine's clock.

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Notify};

use crate::tunnel::{self, Head, Hello};

/// How long a relayed request waits for its head. A side question (`/btw`) answers only
/// once its fork has, which the agent allows three minutes; everything else answers at
/// once.
const HEAD_WAIT: Duration = Duration::from_secs(200);
/// How long the merged list and the notifier wait for each machine. One slow machine
/// must not hold up everyone else's sessions.
const ASK_WAIT: Duration = Duration::from_secs(4);
/// The router pings this often, and gives up on a connection silent for `IDLE_LIMIT`.
const PING_EVERY: Duration = Duration::from_secs(20);
const IDLE_LIMIT: Duration = Duration::from_secs(60);
/// A revoked or expired access token must release its already-connected machine too.
const TOKEN_CHECK_EVERY: Duration = Duration::from_secs(5);
/// A collected answer (a list, the notifier's evidence) is never bigger than this.
const MAX_COLLECTED: usize = 16 * 1024 * 1024;

/// One connected agent.
pub struct Machine {
    pub hello: Hello,
    pub connected_at_ms: i64,
    out: mpsc::UnboundedSender<Vec<u8>>,
    next: AtomicU32,
    pending: Mutex<HashMap<u32, Pending>>,
}

/// A request sent and not finished: who waits for its head, and where its body goes.
struct Pending {
    head: Option<oneshot::Sender<Result<Head, String>>>,
    body: Option<mpsc::UnboundedSender<Bytes>>,
}

/// Why a request could not be relayed.
#[derive(Debug)]
pub enum Relay {
    /// No connected agent lists the session.
    Unknown,
    /// The machine holding it is not connected now.
    Offline(String),
    /// The agent took too long, or went away mid-request.
    Failed(String),
}

impl Relay {
    pub fn into_response(self) -> Response {
        let (status, why) = match self {
            Relay::Unknown => (StatusCode::NOT_FOUND, "No such session.".to_string()),
            Relay::Offline(label) => (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("the machine running this session ({label}) is offline"),
            ),
            Relay::Failed(why) => (StatusCode::BAD_GATEWAY, why),
        };
        (status, axum::Json(json!({ "ok": false, "error": why }))).into_response()
    }
}

impl Machine {
    fn send(&self, kind: u8, id: u32, payload: &[u8]) -> bool {
        self.out.send(tunnel::encode(kind, id, payload)).is_ok()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u32, Pending>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// One frame from the agent, handed to whoever waits on its request.
    fn dispatch(&self, frame: &[u8]) {
        let Some((kind, id, payload)) = tunnel::decode(frame) else {
            return;
        };
        let mut pending = self.lock();
        match kind {
            tunnel::HEAD => {
                let head = serde_json::from_slice::<Head>(payload).map_err(|e| format!("a malformed head: {e}"));
                if let Some(tx) = pending.get_mut(&id).and_then(|p| p.head.take()) {
                    let _ = tx.send(head);
                }
            }
            tunnel::BODY => {
                if let Some(tx) = pending.get(&id).and_then(|p| p.body.as_ref()) {
                    // A reader that went away is the stream's guard's business: it sends
                    // the cancel as it drops.
                    let _ = tx.send(Bytes::copy_from_slice(payload));
                }
            }
            tunnel::END => {
                pending.remove(&id);
            }
            tunnel::FAIL => {
                if let Some(mut p) = pending.remove(&id) {
                    if let Some(tx) = p.head.take() {
                        let _ = tx.send(Err(String::from_utf8_lossy(payload).into_owned()));
                    }
                }
            }
            _ => {}
        }
    }

    /// Everything in flight fails: the connection it would have come back on is gone.
    fn fail_all(&self, why: &str) {
        for (_, mut p) in self.lock().drain() {
            if let Some(tx) = p.head.take() {
                let _ = tx.send(Err(why.to_string()));
            }
        }
    }

    /// Run a request on the agent and hand its response back as it streams. Dropping the
    /// response's body — a viewer closing the page — tells the agent to stop.
    pub async fn request(
        self: &Arc<Self>,
        method: &Method,
        path: &str,
        headers: Vec<(String, String)>,
        body: &[u8],
        wait: Duration,
    ) -> Result<Response, Relay> {
        let id = loop {
            let id = self.next.fetch_add(1, Ordering::Relaxed);
            if id != 0 {
                break id;
            }
        };
        let (head_tx, head_rx) = oneshot::channel();
        let (body_tx, body_rx) = mpsc::unbounded_channel();
        self.lock().insert(id, Pending { head: Some(head_tx), body: Some(body_tx) });
        // From here on, however this ends, the agent hears about it if it should.
        let guard = Abandon { machine: self.clone(), id };

        let request = tunnel::Request {
            method: method.to_string(),
            path: path.to_string(),
            headers,
            body: tunnel::base64(body),
        };
        let encoded = serde_json::to_vec(&request).map_err(|e| Relay::Failed(e.to_string()))?;
        if !self.send(tunnel::REQUEST, id, &encoded) {
            return Err(Relay::Offline(self.hello.label.clone()));
        }
        let head = match tokio::time::timeout(wait, head_rx).await {
            Ok(Ok(Ok(head))) => head,
            Ok(Ok(Err(why))) => return Err(Relay::Failed(format!("{}: {why}", self.hello.label))),
            Ok(Err(_)) => return Err(Relay::Offline(self.hello.label.clone())),
            Err(_) => return Err(Relay::Failed(format!("{} did not answer in time", self.hello.label))),
        };

        let status = StatusCode::from_u16(head.status).unwrap_or(StatusCode::BAD_GATEWAY);
        let event_stream = head
            .headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v.contains("event-stream"));
        let stream = futures_util::stream::unfold((body_rx, guard), |(mut rx, guard)| async move {
            rx.recv().await.map(|bytes| (Ok::<_, Infallible>(bytes), (rx, guard)))
        });
        let mut response = Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        let out = response.headers_mut();
        for (name, value) in head.headers {
            if !tunnel::carried(&name.to_ascii_lowercase()) {
                continue;
            }
            if let (Ok(name), Ok(value)) = (HeaderName::try_from(name), HeaderValue::try_from(value)) {
                out.append(name, value);
            }
        }
        if event_stream {
            // An ingress that buffers would deliver a live view in lumps, or not at all.
            out.insert("x-accel-buffering", HeaderValue::from_static("no"));
        }
        Ok(response)
    }

    /// A request whose whole answer is wanted as JSON: a list, the notifier's evidence.
    pub async fn get_json(self: &Arc<Self>, path: &str, wait: Duration) -> Result<Value, Relay> {
        let response = self.request(&Method::GET, path, Vec::new(), &[], wait).await?;
        if !response.status().is_success() {
            return Err(Relay::Failed(format!("{} answered {}", self.hello.label, response.status())));
        }
        let bytes = tokio::time::timeout(wait, axum::body::to_bytes(response.into_body(), MAX_COLLECTED))
            .await
            .map_err(|_| Relay::Failed(format!("{} did not finish in time", self.hello.label)))?
            .map_err(|e| Relay::Failed(e.to_string()))?;
        serde_json::from_slice(&bytes).map_err(|e| Relay::Failed(e.to_string()))
    }
}

/// Drops with the response it belongs to. If the request is still open then — the head
/// never came, or a reader left before the end — the agent is told to stop working on it.
struct Abandon {
    machine: Arc<Machine>,
    id: u32,
}

impl Drop for Abandon {
    fn drop(&mut self) {
        if self.machine.lock().remove(&self.id).is_some() {
            self.machine.send(tunnel::CANCEL, self.id, &[]);
        }
    }
}

/// A machine as `/api/machines` reports it.
#[derive(Debug, Clone, Serialize)]
pub struct Seen {
    pub id: String,
    pub label: String,
    pub platform: String,
    pub version: String,
    pub build: String,
    pub online: bool,
    /// When it connected, or when it went away.
    pub since_ms: i64,
}

/// What every machine says is waiting on a person, for the notifier.
#[derive(Default)]
pub struct Evidence {
    /// (dedupe key, session, headline, detail), as `notify::waiting_in` gives them.
    pub waiting: Vec<(String, String, String, String)>,
    /// session -> (project, title), to name a banner.
    pub labels: HashMap<String, (String, Option<String>)>,
    /// Keys from machines heard from for the first time since they connected: already
    /// waiting then, so not news (a rollout, or a laptop opened again).
    pub first: HashSet<String>,
}

#[derive(Default)]
pub struct Machines {
    online: Mutex<HashMap<String, Arc<Machine>>>,
    seen: Mutex<HashMap<String, Seen>>,
    /// session -> machine id, as the machines last listed them.
    sessions: Mutex<HashMap<String, String>>,
    /// Machines the notifier has taken evidence from since they connected.
    heard: Mutex<HashSet<String>>,
    /// The router is stopping: its agents are let go, and none is taken on (`stop`).
    stopping: AtomicBool,
    stopped: Notify,
}

fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Machines {
    /// Let every agent go, and take no more. A rolling update stops this router once its
    /// replacement is serving, and an agent can only be reached through the router it
    /// dialled: closed now, each one reconnects within a second or two, and the load
    /// balancer sends it to the replacement. Held until the drain ends, they would leave
    /// the replacement with no machines for as long.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.stopped.notify_waiters();
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    /// Resolves once `stop` has been called, whenever that was.
    async fn until_stopped(&self) {
        loop {
            let notified = self.stopped.notified();
            tokio::pin!(notified);
            // Registered before the flag is read, so a `stop` between the two still wakes it.
            notified.as_mut().enable();
            if self.is_stopping() {
                return;
            }
            notified.await;
        }
    }

    /// One agent's connection, for as long as it lasts.
    pub async fn serve(self: Arc<Self>, socket: WebSocket) {
        self.serve_authorized(socket, None).await;
    }

    pub async fn serve_with_token(
        self: Arc<Self>,
        socket: WebSocket,
        db: std::path::PathBuf,
        token: crate::auth::tokens::Token,
    ) {
        self.serve_authorized(socket, Some((db, token))).await;
    }

    async fn serve_authorized(
        self: Arc<Self>,
        socket: WebSocket,
        token: Option<(std::path::PathBuf, crate::auth::tokens::Token)>,
    ) {
        if self.is_stopping() {
            return;
        }
        let invalidated = token_invalidated(token);
        tokio::pin!(invalidated);
        let (mut sink, mut stream) = socket.split();
        let first = tokio::select! {
            first = tokio::time::timeout(Duration::from_secs(10), stream.next()) => first,
            () = &mut invalidated => return,
            () = self.until_stopped() => return,
        };
        let hello = match first {
            Ok(Some(Ok(Message::Binary(data)))) => match tunnel::decode(&data) {
                Some((tunnel::HELLO, _, payload)) => serde_json::from_slice::<Hello>(payload).ok(),
                _ => None,
            },
            _ => None,
        };
        let Some(hello) = hello.filter(|h| !h.machine.is_empty()) else {
            tracing::warn!("an agent connected without saying which machine it is; closing");
            return;
        };

        let (out, mut outgoing) = mpsc::unbounded_channel::<Vec<u8>>();
        let machine = Arc::new(Machine {
            hello: hello.clone(),
            connected_at_ms: crate::db::now_ms(),
            out,
            next: AtomicU32::new(1),
            pending: Mutex::new(HashMap::new()),
        });
        if let Some(previous) = locked(&self.online).insert(hello.machine.clone(), machine.clone())
        {
            previous.fail_all("the machine reconnected");
        }
        locked(&self.heard).remove(&hello.machine);
        self.note(&machine, true);
        tracing::info!(machine = %hello.machine, label = %hello.label, platform = %hello.platform, build = %hello.build, "machine agent connected");

        let writer = tokio::spawn(async move {
            let mut ping = tokio::time::interval(PING_EVERY);
            loop {
                tokio::select! {
                    frame = outgoing.recv() => match frame {
                        Some(frame) => if sink.send(Message::Binary(frame.into())).await.is_err() { break },
                        None => break,
                    },
                    _ = ping.tick() => if sink.send(Message::Ping(Bytes::new())).await.is_err() { break },
                }
            }
            let _ = sink.close().await;
        });
        loop {
            let next = tokio::select! {
                next = tokio::time::timeout(IDLE_LIMIT, stream.next()) => next,
                () = self.until_stopped() => break,
                () = &mut invalidated => break,
            };
            match next {
                Ok(Some(Ok(Message::Binary(data)))) => machine.dispatch(&data),
                Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
                Ok(Some(Ok(_))) => {}
            }
        }
        writer.abort();

        // Unregistered only if still the registered connection: a reconnect may already
        // have taken its place.
        let current = {
            let mut online = locked(&self.online);
            let current = online
                .get(&hello.machine)
                .is_some_and(|m| Arc::ptr_eq(m, &machine));
            if current {
                online.remove(&hello.machine);
            }
            current
        };
        machine.fail_all("the machine went offline");
        if current {
            self.note(&machine, false);
        }
        tracing::info!(machine = %hello.machine, label = %hello.label, "machine agent disconnected");
    }

    fn note(&self, machine: &Machine, online: bool) {
        let h = &machine.hello;
        locked(&self.seen).insert(
            h.machine.clone(),
            Seen {
                id: h.machine.clone(),
                label: h.label.clone(),
                platform: h.platform.clone(),
                version: h.version.clone(),
                build: h.build.clone(),
                online,
                since_ms: if online { machine.connected_at_ms } else { crate::db::now_ms() },
            },
        );
    }

    pub(crate) fn connected(&self) -> Vec<Arc<Machine>> {
        locked(&self.online).values().cloned().collect()
    }

    /// The machine with this id, if it is connected now.
    pub(crate) fn online(&self, id: &str) -> Option<Arc<Machine>> {
        locked(&self.online).get(id).cloned()
    }

    /// Every machine seen since the router started, connected ones first.
    pub fn seen(&self) -> Vec<Seen> {
        let mut all: Vec<Seen> = locked(&self.seen).values().cloned().collect();
        all.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| a.label.cmp(&b.label)));
        all
    }

    /// Every connected machine's sessions, merged, each naming its machine, with the
    /// router's own shelf applied. A machine that does not answer in time is left out
    /// rather than holding up the rest.
    pub async fn sessions(&self, shelf: &HashMap<String, i64>) -> Vec<Value> {
        let machines = self.connected();
        let answers = futures_util::future::join_all(
            machines.iter().map(|m| async move { (m, m.get_json("/api/live/sessions", ASK_WAIT).await) }),
        )
        .await;
        let mut rows = Vec::new();
        let mut index = HashMap::new();
        for (machine, answer) in answers {
            let list = match answer {
                Ok(Value::Array(list)) => list,
                Ok(_) => continue,
                Err(why) => {
                    tracing::warn!(machine = %machine.hello.label, "no session list: {why:?}");
                    continue;
                }
            };
            for mut row in list {
                let Some(id) = row.get("session_id").and_then(Value::as_str).map(str::to_owned) else {
                    continue;
                };
                index.insert(id.clone(), machine.hello.machine.clone());
                if let Some(obj) = row.as_object_mut() {
                    obj.insert(
                        "machine".into(),
                        json!({ "id": machine.hello.machine, "label": machine.hello.label }),
                    );
                    // The router's shelf decides for the sessions it holds an answer for;
                    // the machine's own (from before it was hosted) for the rest.
                    if shelf.contains_key(&id) {
                        let last = obj.get("last_activity_ms").and_then(Value::as_i64).unwrap_or(0);
                        let active = obj.get("active").and_then(Value::as_bool).unwrap_or(false);
                        let shelved = crate::live::is_shelved(shelf, &id, last) && !active;
                        obj.insert("archived".into(), shelved.into());
                    }
                }
                rows.push(row);
            }
        }
        locked(&self.sessions).extend(index);
        rows.sort_by_key(|r| std::cmp::Reverse(r.get("last_activity_ms").and_then(Value::as_i64).unwrap_or(0)));
        rows
    }

    /// The machine a session was last listed on: connected, offline, or never heard of.
    fn lookup(&self, session: &str) -> Option<Result<Arc<Machine>, String>> {
        let machine = locked(&self.sessions).get(session).cloned()?;
        if let Some(m) = locked(&self.online).get(&machine) {
            return Some(Ok(m.clone()));
        }
        let label = locked(&self.seen).get(&machine).map(|s| s.label.clone()).unwrap_or(machine);
        Some(Err(label))
    }

    /// The connected machine holding a session, asking every machine once if it is not
    /// known yet (a session that started since the last list).
    pub async fn holder(&self, session: &str) -> Result<Arc<Machine>, Relay> {
        if let Some(Ok(m)) = self.lookup(session) {
            return Ok(m);
        }
        let _ = self.sessions(&HashMap::new()).await;
        match self.lookup(session) {
            Some(Ok(m)) => Ok(m),
            Some(Err(label)) => Err(Relay::Offline(label)),
            None => Err(Relay::Unknown),
        }
    }

    /// The same request to every connected machine, each answer as JSON.
    pub async fn ask_all(&self, path: &str) -> Vec<Value> {
        let machines = self.connected();
        futures_util::future::join_all(machines.iter().map(|m| m.get_json(path, ASK_WAIT)))
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect()
    }

    /// What is waiting on a person, from every connected machine.
    pub async fn evidence(&self, mark_first: bool) -> Evidence {
        let machines = self.connected();
        let answers = futures_util::future::join_all(
            machines.iter().map(|m| async move { (m, m.get_json("/agent/waiting", ASK_WAIT).await) }),
        )
        .await;
        let mut ev = Evidence::default();
        let mut index = HashMap::new();
        for (machine, answer) in answers {
            let Ok(Value::Array(rows)) = answer else {
                continue;
            };
            let first = mark_first && locked(&self.heard).insert(machine.hello.machine.clone());
            for row in rows {
                let field = |k: &str| row.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                let (key, session) = (field("key"), field("session"));
                if key.is_empty() || session.is_empty() {
                    continue;
                }
                index.insert(session.clone(), machine.hello.machine.clone());
                let project = row.get("project").and_then(Value::as_str).unwrap_or("session").to_string();
                let title = row.get("title").and_then(Value::as_str).map(str::to_owned);
                ev.labels.insert(session.clone(), (project, title));
                if first {
                    ev.first.insert(key.clone());
                }
                ev.waiting.push((key, session, field("head"), field("detail")));
            }
        }
        locked(&self.sessions).extend(index);
        ev
    }

    /// Of these announced prompts, missing from the last evidence, the ones still waiting.
    /// A prompt on a machine that is offline stays up: nothing has said it was answered,
    /// and taking its banner down would lose it until the machine came back.
    pub async fn still_waiting(&self, candidates: Vec<(String, String)>) -> HashSet<String> {
        let mut still = HashSet::new();
        let mut asks = Vec::new();
        for (key, session) in candidates {
            match self.lookup(&session) {
                Some(Err(_offline)) => {
                    still.insert(key);
                }
                Some(Ok(machine)) => asks.push((key, session, machine)),
                // Never listed by any machine since the router started: nothing to ask.
                None => {}
            }
        }
        let answers = futures_util::future::join_all(asks.into_iter().map(|(key, session, machine)| async move {
            let path = format!(
                "/agent/still-waiting?key={}&session={}",
                crate::machines::encode(&key),
                crate::machines::encode(&session)
            );
            let waiting = machine
                .get_json(&path, ASK_WAIT)
                .await
                .ok()
                .and_then(|v| v.get("waiting").and_then(Value::as_bool));
            // No answer is not an answer: keep the banner.
            (key, waiting.unwrap_or(true))
        }))
        .await;
        still.extend(answers.into_iter().filter(|(_, waiting)| *waiting).map(|(key, _)| key));
        still
    }
}

/// Polling also covers expiration and revocation from another router process sharing
/// the store. No raw credential is retained after the HTTP upgrade.
async fn token_invalidated(token: Option<(std::path::PathBuf, crate::auth::tokens::Token)>) {
    let Some((db, token)) = token else {
        return std::future::pending().await;
    };
    loop {
        match crate::auth::tokens::is_active(&db, &token.id).await {
            Ok(true) => tokio::time::sleep(TOKEN_CHECK_EVERY).await,
            Ok(false) => return,
            Err(error) => {
                tracing::warn!(%error, "machine access token could not be checked; closing the connection");
                return;
            }
        }
    }
}

/// Percent-encoding for a query value.
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The routes a hosted router answers by relaying to machines.
pub mod api {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::ws::WebSocketUpgrade;
    use axum::extract::{Extension, FromRequest, Request, State};
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Response};
    use axum::Json;
    use serde::Deserialize;
    use serde_json::{json, Value};

    use super::{Relay, HEAD_WAIT};
    use crate::proxy::AppState;
    use crate::refusal::{JsonBody, LiveBody};

    /// Requests carry their session in the query (`?session=`) or the JSON body, as
    /// the local routes read it. 1 MiB is the most a wrapper frame carries, and a body
    /// that holds one is a little more.
    const MAX_BODY: usize = 2 * 1024 * 1024;

    /// `GET /_router/agent`: an agent connecting. A stopping router turns it away, and the
    /// agent tries again, through the load balancer, to the router replacing it.
    pub async fn connect(
        State(state): State<Arc<AppState>>,
        token: Option<Extension<crate::auth::tokens::Token>>,
        ws: WebSocketUpgrade,
    ) -> Response {
        if state.machines.is_stopping() {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "agent-router: this router is stopping; the one replacing it takes the machines\n",
            )
                .into_response();
        }
        let machines = state.machines.clone();
        let db = state.config.db_path.clone();
        ws.max_message_size(crate::tunnel::MAX_MESSAGE)
            .max_frame_size(crate::tunnel::MAX_MESSAGE)
            .on_upgrade(move |socket| async move {
                match token {
                    Some(Extension(token)) => machines.serve_with_token(socket, db, token).await,
                    None => machines.serve(socket).await,
                }
            })
    }

    /// `GET /api/machines`: each machine, with where its Library stands (`library::push`),
    /// from what it last said and what is live now, and the clients it last reported
    /// (`clients`).
    pub async fn machines(State(state): State<Arc<AppState>>) -> Json<Value> {
        let standings = crate::library::push::standings(&state).await;
        let clients = crate::places::clients(&state).await;
        let rows = state
            .machines
            .seen()
            .into_iter()
            .map(|m| {
                let library = standings.get(&m.id).cloned().unwrap_or_else(crate::library::push::Standing::not_yet);
                let mut row = json!(m);
                row["library"] = json!(library);
                row["clients"] = clients.get(&m.id).cloned().unwrap_or_else(|| json!([]));
                row
            })
            .collect();
        Json(Value::Array(rows))
    }

    /// `GET /api/live/sessions`: every machine's sessions, merged.
    pub async fn sessions(State(state): State<Arc<AppState>>) -> Response {
        let shelf = tokio::task::spawn_blocking(crate::live::archived_set).await.unwrap_or_default();
        let rows = state.machines.sessions(&shelf).await;
        ([(axum::http::header::CACHE_CONTROL, "no-store")], Json(rows)).into_response()
    }

    fn carried_headers(req: &Request) -> Vec<(String, String)> {
        req.headers()
            .iter()
            .filter(|(name, _)| crate::tunnel::carried(name.as_str()))
            .filter(|(name, _)| {
                !matches!(
                    name.as_str(),
                    "x-api-key" | "x-agent-router-password" | "x-agent-router-token"
                )
            })
            .filter_map(|(name, value)| {
                Some((name.as_str().to_string(), value.to_str().ok()?.to_string()))
            })
            .collect()
    }

    fn path_of(req: &Request) -> String {
        req.uri().path_and_query().map(|pq| pq.as_str().to_string()).unwrap_or_else(|| req.uri().path().to_string())
    }

    fn session_in_query(req: &Request) -> Option<String> {
        req.uri().query()?.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == "session").then(|| v.to_string())
        })
    }

    /// A request that names its session in the query: follow, stats, prompt.
    pub async fn by_query(State(state): State<Arc<AppState>>, req: Request) -> Response {
        let Some(session) = session_in_query(&req) else {
            return Relay::Unknown.into_response();
        };
        let machine = match state.machines.holder(&session).await {
            Ok(m) => m,
            Err(why) => return why.into_response(),
        };
        let (method, path, headers) = (req.method().clone(), path_of(&req), carried_headers(&req));
        match machine.request(&method, &path, headers, &[], HEAD_WAIT).await {
            Ok(response) => response,
            Err(why) => why.into_response(),
        }
    }

    /// What a relayed body must name, read the way the machine reads its whole body.
    #[derive(Deserialize)]
    struct Named {
        session: String,
    }

    /// A request that names its session in its JSON body: send, answer, stop, model,
    /// effort, mode, side question.
    ///
    /// The session is read by the rules the machine's handler reads the whole body by
    /// (`refusal::LiveBody`), so a body that is not JSON, is not said to be, or names no
    /// session as a string is refused here as the machine would refuse it, with axum's
    /// status and words. Anything else goes to the machine as it came.
    pub async fn by_body(State(state): State<Arc<AppState>>, req: Request) -> Response {
        let (method, path, headers) = (req.method().clone(), path_of(&req), carried_headers(&req));
        let (parts, body) = req.into_parts();
        let body = match axum::body::to_bytes(body, MAX_BODY).await {
            Ok(b) => b,
            Err(_) => {
                return (StatusCode::PAYLOAD_TOO_LARGE, Json(json!({ "ok": false, "error": "that message is too long to send" })))
                    .into_response()
            }
        };
        let named = Request::from_parts(parts, Body::from(body.clone()));
        let session = match <LiveBody<Named> as FromRequest<()>>::from_request(named, &()).await {
            Ok(JsonBody(Named { session })) => session,
            Err(refused) => return refused,
        };
        let machine = match state.machines.holder(&session).await {
            Ok(m) => m,
            Err(why) => return why.into_response(),
        };
        match machine.request(&method, &path, headers, &body, HEAD_WAIT).await {
            Ok(response) => response,
            Err(why) => why.into_response(),
        }
    }

    /// What a request that starts something must name: the machine to start it on.
    #[derive(Deserialize)]
    struct OnMachine {
        machine: String,
    }

    /// A request that names its machine in its JSON body: a hosted Kimi Code session started
    /// there (`kimi::live::start`). Refused here as the machine would refuse a body it cannot
    /// read, and as offline when that machine is not connected.
    pub async fn by_machine(State(state): State<Arc<AppState>>, req: Request) -> Response {
        let (method, path, headers) = (req.method().clone(), path_of(&req), carried_headers(&req));
        let (parts, body) = req.into_parts();
        let Ok(body) = axum::body::to_bytes(body, MAX_BODY).await else {
            return (StatusCode::PAYLOAD_TOO_LARGE, Json(json!({ "ok": false, "error": "that request is too long" }))).into_response();
        };
        let named = Request::from_parts(parts, Body::from(body.clone()));
        let id = match <LiveBody<OnMachine> as FromRequest<()>>::from_request(named, &()).await {
            Ok(JsonBody(OnMachine { machine })) => machine,
            Err(refused) => return refused,
        };
        let Some(machine) = state.machines.online(&id) else {
            let label = state.machines.seen().into_iter().find(|m| m.id == id).map(|m| m.label);
            return match label {
                Some(label) => Relay::Offline(label).into_response(),
                None => (StatusCode::NOT_FOUND, Json(json!({ "ok": false, "error": "No such machine." }))).into_response(),
            };
        };
        match machine.request(&method, &path, headers, &body, HEAD_WAIT).await {
            Ok(response) => response,
            Err(why) => why.into_response(),
        }
    }

    /// `GET /api/live/project`: one working directory's totals, from every machine that
    /// has sessions there, added up.
    pub async fn project(State(state): State<Arc<AppState>>, req: Request) -> Response {
        let answers = state.machines.ask_all(&path_of(&req)).await;
        let mut total = serde_json::Map::new();
        for answer in answers {
            let Value::Object(fields) = answer else { continue };
            for (key, value) in fields {
                let sum = match (total.get(&key), &value) {
                    (Some(Value::Number(a)), Value::Number(b)) => match (a.as_i64(), b.as_i64()) {
                        (Some(a), Some(b)) => json!(a + b),
                        _ => json!(a.as_f64().unwrap_or(0.0) + b.as_f64().unwrap_or(0.0)),
                    },
                    (None, _) => value,
                    (Some(existing), _) => existing.clone(),
                };
                total.insert(key, sum);
            }
        }
        Json(Value::Object(total)).into_response()
    }

    /// `POST /api/live/archive`: the router's own shelf, for a session some machine lists. A
    /// body it cannot read is refused as a router on a machine refuses it (`refusal::LiveBody`).
    pub async fn archive(State(state): State<Arc<AppState>>, JsonBody(body): LiveBody<crate::live::ArchiveBody>) -> Response {
        if let Err(why) = state.machines.holder(&body.session).await {
            if matches!(why, Relay::Unknown) {
                return why.into_response();
            }
        }
        let done = tokio::task::spawn_blocking(move || -> std::io::Result<usize> {
            let mut set = crate::live::archived_set();
            if body.archived {
                set.insert(body.session, crate::db::now_ms());
            } else {
                set.remove(&body.session);
            }
            crate::live::write_archived(&set)
        })
        .await;
        match done {
            Ok(Ok(n)) => Json(json!({ "ok": true, "archived": n })).into_response(),
            Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "ok": false, "error": e.to_string() }))).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "ok": false, "error": e.to_string() }))).into_response(),
        }
    }

    /// `GET /api/push/pending`: what the phones should still show, from the machines.
    pub async fn pending(State(state): State<Arc<AppState>>) -> Json<Value> {
        let ev = state.machines.evidence(false).await;
        let mut keys: std::collections::HashSet<String> = ev.waiting.iter().map(|(k, ..)| k.clone()).collect();
        let missing = state.push.missing_from(&ev.waiting);
        keys.extend(state.machines.still_waiting(missing).await);
        let mut out: Vec<String> = keys.into_iter().collect();
        out.sort();
        Json(json!({ "waiting": out }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn machine() -> (Arc<Machine>, mpsc::UnboundedReceiver<Vec<u8>>) {
        let (out, rx) = mpsc::unbounded_channel();
        let m = Arc::new(Machine {
            hello: Hello {
                machine: "m-1".into(),
                label: "laptop".into(),
                platform: "macos".into(),
                version: "0".into(),
                build: "tree:x".into(),
            },
            connected_at_ms: 0,
            out,
            next: AtomicU32::new(1),
            pending: Mutex::new(HashMap::new()),
        });
        (m, rx)
    }

    /// A relayed response streams the agent's body, and the request leaves nothing behind.
    #[tokio::test]
    async fn a_relayed_response_streams_back_and_closes() {
        let (m, mut sent) = machine();
        let task = {
            let m = m.clone();
            tokio::spawn(async move { m.request(&Method::GET, "/api/live/stats?session=s", vec![], &[], Duration::from_secs(5)).await })
        };
        let frame = sent.recv().await.unwrap();
        let (kind, id, payload) = tunnel::decode(&frame).unwrap();
        assert_eq!(kind, tunnel::REQUEST);
        let req: tunnel::Request = serde_json::from_slice(payload).unwrap();
        assert_eq!((req.method.as_str(), req.path.as_str()), ("GET", "/api/live/stats?session=s"));

        let head = serde_json::to_vec(&Head { status: 200, headers: vec![("content-type".into(), "text/event-stream".into())] }).unwrap();
        m.dispatch(&tunnel::encode(tunnel::HEAD, id, &head));
        m.dispatch(&tunnel::encode(tunnel::BODY, id, b"data: 1\n\n"));
        m.dispatch(&tunnel::encode(tunnel::BODY, id, b"data: 2\n\n"));
        m.dispatch(&tunnel::encode(tunnel::END, id, b""));

        let response = task.await.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-accel-buffering"], "no");
        let body = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"data: 1\n\ndata: 2\n\n");
        assert!(m.lock().is_empty());
        assert!(sent.try_recv().is_err(), "a finished request sends no cancel");
    }

    /// A reader that leaves early tells the agent to stop.
    #[tokio::test]
    async fn leaving_early_cancels_on_the_agent() {
        let (m, mut sent) = machine();
        let task = {
            let m = m.clone();
            tokio::spawn(async move { m.request(&Method::GET, "/api/live/stream?session=s", vec![], &[], Duration::from_secs(5)).await })
        };
        let (_, id, _) = tunnel::decode(&sent.recv().await.unwrap()).map(|(k, i, p)| (k, i, p.to_vec())).unwrap();
        let head = serde_json::to_vec(&Head { status: 200, headers: vec![] }).unwrap();
        m.dispatch(&tunnel::encode(tunnel::HEAD, id, &head));
        let response = task.await.unwrap().unwrap();
        drop(response);
        let frame = sent.recv().await.unwrap();
        assert_eq!(tunnel::decode(&frame).map(|(k, i, _)| (k, i)), Some((tunnel::CANCEL, id)));
        assert!(m.lock().is_empty());
    }

    /// An agent that goes away fails what was in flight, as an offline machine.
    #[tokio::test]
    async fn a_machine_that_leaves_fails_its_requests() {
        let (m, mut sent) = machine();
        let task = {
            let m = m.clone();
            tokio::spawn(async move { m.request(&Method::POST, "/api/live/send", vec![], b"{}", Duration::from_secs(5)).await })
        };
        let _ = sent.recv().await.unwrap();
        m.fail_all("the machine went offline");
        match task.await.unwrap() {
            Err(Relay::Failed(why)) => assert!(why.contains("offline"), "{why}"),
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn query_values_are_encoded() {
        assert_eq!(encode("req:abc-1_2.3~"), "req%3Aabc-1_2.3~");
        assert_eq!(encode("a b&c"), "a%20b%26c");
    }

    /// A hosted router relays a start to the machine its body names, as that machine's
    /// agent answers it; a machine it does not know is refused.
    #[tokio::test]
    async fn the_hosted_router_relays_a_start_to_its_machine() {
        let lab = crate::api_refusals::Lab::hosted("relays-start").await;
        let (m, mut sent) = machine();
        locked(&lab.state.machines.online).insert("m-1".into(), m.clone());
        lab.state.machines.note(&m, true);
        let body = json!({ "machine": "m-1", "cwd": "/Users/u/code/demo", "model": "k3" });
        let agent = async {
            let frame = sent.recv().await.unwrap();
            let (kind, id, payload) = tunnel::decode(&frame).unwrap();
            assert_eq!(kind, tunnel::REQUEST);
            let req: tunnel::Request = serde_json::from_slice(payload).unwrap();
            assert_eq!((req.method.as_str(), req.path.as_str()), ("POST", "/api/live/start"));
            let head = serde_json::to_vec(&Head { status: 200, headers: vec![("content-type".into(), "application/json".into())] }).unwrap();
            m.dispatch(&tunnel::encode(tunnel::HEAD, id, &head));
            m.dispatch(&tunnel::encode(tunnel::BODY, id, br#"{"ok":true,"session":"session_new"}"#));
            m.dispatch(&tunnel::encode(tunnel::END, id, b""));
        };
        let body = body.to_string();
        let ((status, answer), ()) = tokio::join!(lab.post("/api/live/start", &body), agent);
        assert_eq!(status, StatusCode::OK, "{answer}");
        assert_eq!(answer["session"], "session_new");

        let elsewhere = json!({ "machine": "m-2", "cwd": "/Users/u", "model": "k3" });
        let (status, _) = lab.post("/api/live/start", &elsewhere.to_string()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// The router's credential belongs to the router, whichever header carried it.
    /// An agent still receives the headers its handler needs to stream a response.
    #[tokio::test]
    async fn credentials_stay_out_of_requests_relayed_to_a_machine() {
        let lab = crate::api_refusals::Lab::hosted("relay-credentials").await;
        let (m, mut sent) = machine();
        locked(&lab.state.machines.online).insert("m-1".into(), m.clone());
        let request = axum::http::Request::post("/api/live/start")
            .header("authorization", "Bearer SECRET-BEARER")
            .header("cookie", "router=SECRET-COOKIE")
            .header("x-api-key", "SECRET-API-KEY")
            .header("x-agent-router-password", "SECRET-PASSWORD")
            .header("x-agent-router-token", "SECRET-TOKEN")
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .header("last-event-id", "event-7")
            .body(Body::from(r#"{"machine":"m-1","cwd":"/tmp","model":"k3"}"#))
            .unwrap();
        let agent = async {
            let frame = sent.recv().await.unwrap();
            let (kind, id, payload) = tunnel::decode(&frame).unwrap();
            assert_eq!(kind, tunnel::REQUEST);
            let req: tunnel::Request = serde_json::from_slice(payload).unwrap();
            assert!(
                req.headers
                    .iter()
                    .all(|(_, value)| !value.contains("SECRET")),
                "router credentials must not travel to its agents"
            );
            for (name, value) in [
                ("content-type", "application/json"),
                ("accept", "text/event-stream"),
                ("last-event-id", "event-7"),
            ] {
                assert!(
                    req.headers.contains(&(name.to_string(), value.to_string())),
                    "{name} must still reach the agent"
                );
            }
            let head = serde_json::to_vec(&Head {
                status: 200,
                headers: vec![],
            })
            .unwrap();
            m.dispatch(&tunnel::encode(tunnel::HEAD, id, &head));
            m.dispatch(&tunnel::encode(tunnel::END, id, b""));
        };
        let (response, ()) = tokio::join!(
            api::by_machine(axum::extract::State(lab.state.clone()), request),
            agent
        );
        assert_eq!(response.status(), StatusCode::OK);
    }

    async fn invalidated_token_disconnects_machine(expire: bool) {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let lab = crate::api_refusals::Lab::hosted(if expire {
            "machine-token-expiry"
        } else {
            "machine-token-revoke"
        })
        .await;
        let (status, made) = lab
            .post(
                "/api/auth/tokens",
                r#"{"name":"test machine","scope":"machine"}"#,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
        let (id, token) = (
            made["id"].as_str().unwrap(),
            made["token"].as_str().unwrap(),
        );
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let app = lab
            .app
            .clone()
            .into_make_service_with_connect_info::<std::net::SocketAddr>();
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        let mut request = format!("ws://{addr}{}", tunnel::PATH)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (mut socket, _) = tokio_tungstenite::client_async(request, tcp).await.unwrap();
        let hello = Hello {
            machine: "token-machine".into(),
            label: "test machine".into(),
            platform: "test".into(),
            version: "test".into(),
            build: "test".into(),
        };
        socket
            .send(tokio_tungstenite::tungstenite::Message::Binary(
                tunnel::encode(tunnel::HELLO, 0, &serde_json::to_vec(&hello).unwrap()).into(),
            ))
            .await
            .unwrap();
        let machine = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(machine) = lab.state.machines.online(&hello.machine) {
                    break machine;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the valid token connects a machine");
        let pending = tokio::spawn(async move {
            machine
                .request(
                    &Method::GET,
                    "/api/live/stats?session=s",
                    vec![],
                    &[],
                    Duration::from_secs(20),
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(frame))) =
                    socket.next().await
                {
                    if tunnel::decode(&frame).is_some_and(|(kind, _, _)| kind == tunnel::REQUEST) {
                        break;
                    }
                }
            }
        })
        .await
        .expect("the machine has an outstanding request");

        if expire {
            rusqlite::Connection::open(&lab.state.config.db_path)
                .unwrap()
                .execute(
                    "UPDATE auth_tokens SET expires_at = ?2 WHERE id = ?1",
                    rusqlite::params![id, crate::db::now_ms() - 1],
                )
                .unwrap();
        } else {
            let mut request = axum::http::Request::delete(format!("/api/auth/tokens/{id}"))
                .header("host", "127.0.0.1")
                .header("authorization", format!("Bearer {}", lab.password.unwrap()))
                .body(Body::empty())
                .unwrap();
            request.extensions_mut().insert(axum::extract::ConnectInfo(
                std::net::SocketAddr::from(([127, 0, 0, 1], 50_000)),
            ));
            let response = lab.app.clone().oneshot(request).await.unwrap();
            assert!(response.status().is_success(), "revocation must succeed");
        }
        let failed = tokio::time::timeout(TOKEN_CHECK_EVERY + Duration::from_secs(3), pending)
            .await
            .expect("invalidating the token releases its existing connection")
            .unwrap();
        assert!(
            matches!(failed, Err(Relay::Failed(ref why)) if why.contains("offline")),
            "the outstanding request must fail immediately: {failed:?}"
        );
        assert!(lab.state.machines.online(&hello.machine).is_none());
        assert!(lab
            .state
            .machines
            .seen()
            .iter()
            .any(|m| m.id == hello.machine && !m.online));
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match socket.next().await {
                    None
                    | Some(Err(_))
                    | Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) => break,
                    _ => {}
                }
            }
        })
        .await
        .expect("the old WebSocket must close");
        server.abort();
    }

    #[tokio::test]
    async fn revoking_a_machine_token_closes_its_existing_connection() {
        invalidated_token_disconnects_machine(false).await;
    }

    #[tokio::test]
    async fn expiring_a_machine_token_closes_its_existing_connection() {
        invalidated_token_disconnects_machine(true).await;
    }

    /// The clients a machine reports with its places come back on `/api/machines`; a report
    /// that does not name them (an older agent's) leaves them as they were.
    #[tokio::test]
    async fn machines_report_their_clients() {
        let lab = crate::api_refusals::Lab::hosted("machines-clients").await;
        let (m, _sent) = machine();
        lab.state.machines.note(&m, true);
        let clients = json!([
            { "client": "claude-code", "cli": { "path": "/u/.local/bin/claude", "version": "2.1.267" },
              "extensions": [{ "editor": "vscode", "version": "2.1.296" }], "home": "/u/.claude" },
            { "client": "kimi-code", "cli": { "path": "/u/.kimi-code/bin/kimi", "version": "2.1.1" },
              "extensions": [], "home": "/u/.kimi-code" },
        ]);
        let (status, _) = lab.post("/api/machines/places", &json!({ "machine": "m-1", "clients": clients }).to_string()).await;
        assert_eq!(status, StatusCode::OK);
        let (status, rows) = lab.get("/api/machines").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(rows[0]["id"], "m-1");
        assert_eq!(rows[0]["clients"], clients);

        let (status, _) = lab.post("/api/machines/places", &json!({ "machine": "m-1" }).to_string()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(lab.get("/api/machines").await.1[0]["clients"], clients, "an older agent's report keeps them");
    }
}
