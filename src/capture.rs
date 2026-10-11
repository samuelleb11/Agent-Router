//! The capture task: everything that happens *off* the hot path.
//!
//! The proxy hands finished requests to this task over a bounded channel. If the
//! channel is full the event is dropped and counted — capture must never be able to
//! slow down or stall a live coding session, so backpressure is not an option.

use std::io::Write;
use std::path::PathBuf;

use flate2::write::{GzDecoder, ZlibDecoder};
use tokio::sync::mpsc;

use crate::analyze::{RequestShape, Usage};
use crate::db::Db;
use crate::transcript::Resolver;

#[derive(Debug, Default)]
pub struct StoredBody {
    pub stored: Vec<u8>,
    pub truncated: bool,
    pub true_bytes: i64,
}

impl StoredBody {
    /// For a body capped as it streamed in: `stored` is already no longer than the cap,
    /// and `true_bytes` was counted separately as the bytes went past.
    ///
    /// Both numbers have to be carried, because the buffer cannot report on itself. A
    /// constructor that measures what it is handed against the cap always finds them
    /// equal — the buffer stopped growing there — and so calls every truncated response
    /// complete, with the cap itself as the true size. That is exactly what happened:
    /// 6,082 rows in the live store were cut and recorded as whole.
    pub fn streamed(stored: Vec<u8>, true_bytes: i64) -> Self {
        Self { truncated: true_bytes > stored.len() as i64, stored, true_bytes }
    }
}


/// Quota headroom, straight off the response. On a subscription this matters more
/// than a dollar figure: it is the scarce resource actually being spent.
#[derive(Debug, Default)]
pub struct RateLimit {
    pub status: Option<String>,
    pub utilization_5h: Option<f64>,
    pub utilization_7d: Option<f64>,
    pub reset_5h: Option<i64>,
    pub reset_7d: Option<i64>,
}

impl RateLimit {
    pub fn from_headers(h: &axum::http::HeaderMap) -> Self {
        let get = |name: &str| h.get(name).and_then(|v| v.to_str().ok());
        Self {
            status: get("anthropic-ratelimit-unified-status").map(str::to_owned),
            utilization_5h: get("anthropic-ratelimit-unified-5h-utilization")
                .and_then(|v| v.parse().ok()),
            utilization_7d: get("anthropic-ratelimit-unified-7d-utilization")
                .and_then(|v| v.parse().ok()),
            reset_5h: get("anthropic-ratelimit-unified-5h-reset").and_then(|v| v.parse().ok()),
            reset_7d: get("anthropic-ratelimit-unified-7d-reset").and_then(|v| v.parse().ok()),
        }
    }
}

#[derive(Debug)]
pub struct CaptureEvent {
    pub request_id: Option<String>,
    pub message_id: Option<String>,
    pub session_id: Option<String>,
    pub account_email: Option<String>,
    /// Present only on subagent (Task tool) traffic — Claude Code labels it with
    /// `x-claude-code-agent-id`, which the main thread does not send. This is what
    /// separates subagent token burn from its parent session.
    pub agent_id: Option<String>,
    pub account_id: String,
    /// The provider of the account that served it.
    pub provider: crate::accounts::Provider,
    pub entrypoint: Option<String>,
    pub model: Option<String>,
    pub method: String,
    pub path: String,
    pub started_at_ms: i64,
    pub ttfb_ms: i64,
    pub duration_ms: i64,
    pub status: i64,
    pub streaming: bool,
    pub stop_reason: Option<String>,
    pub service_tier: Option<String>,
    pub error: Option<String>,
    pub rl: RateLimit,
    pub usage: Usage,
    pub tool_calls: Vec<String>,
    pub shape: Option<RequestShape>,
    /// No request body: the conversation inside it is indexed as blocks instead, in
    /// full rather than capped, and `request_bytes` still records how big it was. See
    /// `Db::record`.
    pub response_body: StoredBody,
    pub request_bytes: i64,
    pub response_bytes: i64,
}

/// Incremental content-decoder. Responses are relayed downstream compressed and
/// byte-faithful; this decodes a *copy* so the capture task can read the SSE.
pub enum Decoder {
    Identity,
    Gzip(Box<GzDecoder<Vec<u8>>>),
    Zlib(Box<ZlibDecoder<Vec<u8>>>),
    Unsupported,
}

impl Decoder {
    pub fn for_encoding(encoding: Option<&str>) -> Self {
        match encoding.map(str::trim) {
            None | Some("") | Some("identity") => Decoder::Identity,
            Some("gzip") | Some("x-gzip") => Decoder::Gzip(Box::new(GzDecoder::new(Vec::new()))),
            Some("deflate") => Decoder::Zlib(Box::new(ZlibDecoder::new(Vec::new()))),
            Some(other) => {
                tracing::warn!(
                    encoding = other,
                    "unsupported content-encoding; usage will not be parsed for this request"
                );
                Decoder::Unsupported
            }
        }
    }

    /// Feed compressed bytes, get back whatever plaintext became available.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<u8> {
        match self {
            Decoder::Identity => chunk.to_vec(),
            Decoder::Gzip(d) => {
                let _ = d.write_all(chunk);
                std::mem::take(d.get_mut())
            }
            Decoder::Zlib(d) => {
                let _ = d.write_all(chunk);
                std::mem::take(d.get_mut())
            }
            Decoder::Unsupported => Vec::new(),
        }
    }

    /// Flush any bytes the decoder is still holding at end of stream.
    pub fn finish(&mut self) -> Vec<u8> {
        match self {
            Decoder::Gzip(d) => {
                let _ = d.try_finish();
                std::mem::take(d.get_mut())
            }
            Decoder::Zlib(d) => {
                let _ = d.try_finish();
                std::mem::take(d.get_mut())
            }
            _ => Vec::new(),
        }
    }
}

/// Everything known about a request before its response body starts flowing.
pub struct Pending {
    pub request_id: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub account_id: String,
    /// The identity that actually served this request. Recorded per request because
    /// an account row's identity can change under it — `/login` rewrites the Keychain
    /// entry the default account reads.
    pub account_email: Option<String>,
    pub provider: crate::accounts::Provider,
    pub entrypoint: Option<String>,
    pub method: String,
    pub path: String,
    pub started_at_ms: i64,
    pub ttfb_ms: i64,
    pub status: i64,
    pub streaming_requested: bool,
    pub rl: RateLimit,
    pub shape: Option<RequestShape>,
    pub request_bytes: i64,
}

/// Accumulates the response as it streams past, and emits the capture event **when
/// dropped** rather than when the stream ends.
///
/// Emitting at the end of the stream loses whole classes of request. A response with
/// an exact `content-length` — which is every error body Anthropic returns, since
/// they are small and unzipped — is satisfied once the consumer has read that many
/// bytes, and the body is dropped without a final poll. Streaming and gzipped replies
/// have no content-length and must be polled to completion, so they were captured and
/// errors silently were not. The same gap swallowed any request whose client
/// disconnected mid-stream. `Drop` runs in all three cases.
pub struct Tee {
    state: std::sync::Arc<crate::proxy::AppState>,
    pending: Option<Pending>,
    decoder: Decoder,
    scanner: crate::analyze::SseScanner,
    raw: Vec<u8>,
    body_cap: usize,
    /// Every decoded byte that arrived, whether or not it fitted under the cap. The
    /// length of `raw` cannot stand in for this — it stops growing at the cap.
    plain_bytes: i64,
    wire_bytes: i64,
    is_sse: bool,
    started: std::time::Instant,
    transport_error: Option<String>,
    upstream_protocol: Option<crate::translate::Protocol>,
}

impl Tee {
    pub fn new(
        state: std::sync::Arc<crate::proxy::AppState>,
        pending: Pending,
        encoding: Option<&str>,
        is_sse: bool,
        body_cap: usize,
        started: std::time::Instant,
    ) -> Self {
        Self {
            state,
            pending: Some(pending),
            decoder: Decoder::for_encoding(encoding),
            scanner: Default::default(),
            raw: Vec::new(),
            body_cap,
            plain_bytes: 0,
            wire_bytes: 0,
            is_sse,
            started,
            transport_error: None,
            upstream_protocol: None,
        }
    }

    pub fn set_upstream_protocol(&mut self, protocol: crate::translate::Protocol) {
        self.upstream_protocol = Some(protocol);
    }

    /// Feed bytes exactly as they came off the wire, still compressed.
    pub fn feed(&mut self, chunk: &[u8]) {
        self.wire_bytes += chunk.len() as i64;
        let plain = self.decoder.feed(chunk);
        self.absorb(&plain);
    }

    pub fn fail(&mut self, err: String) {
        self.transport_error = Some(err);
    }

    fn absorb(&mut self, plain: &[u8]) {
        if plain.is_empty() {
            return;
        }
        self.plain_bytes += plain.len() as i64;
        if self.raw.len() < self.body_cap {
            let room = self.body_cap - self.raw.len();
            self.raw.extend_from_slice(&plain[..plain.len().min(room)]);
        }
        if self.is_sse {
            self.scanner.feed(plain);
        }
    }
}

impl Drop for Tee {
    fn drop(&mut self) {
        let Some(p) = self.pending.take() else { return };
        let tail = self.decoder.finish();
        self.absorb(&tail);
        if !self.is_sse && !self.raw.is_empty() {
            let raw = std::mem::take(&mut self.raw);
            self.scanner.absorb_json_body(&raw);
            self.raw = raw;
        }
        let ev = CaptureEvent {
            request_id: p.request_id,
            message_id: self.scanner.message_id.clone(),
            session_id: p.session_id,
            agent_id: p.agent_id,
            account_id: p.account_id,
            account_email: p.account_email,
            provider: p.provider,
            entrypoint: p.entrypoint,
            model: if p.shape.as_ref().is_some_and(|shape| shape.service_id.is_some()) {
                p.shape.as_ref().and_then(|shape| shape.model.clone())
            } else {
                self.scanner.model.clone().or_else(|| p.shape.as_ref().and_then(|s| s.model.clone()))
            },
            method: p.method,
            path: p.path,
            started_at_ms: p.started_at_ms,
            ttfb_ms: p.ttfb_ms,
            duration_ms: self.started.elapsed().as_millis() as i64,
            status: p.status,
            streaming: self.is_sse || p.streaming_requested,
            stop_reason: self.scanner.stop_reason.clone(),
            service_tier: self.scanner.service_tier.clone(),
            error: self.transport_error.clone().or_else(|| self.scanner.error.clone()),
            rl: p.rl,
            usage: if self.upstream_protocol == Some(crate::translate::Protocol::Responses) {
                self.scanner.usage.clone()
            } else {
                self.scanner.usage.clone().in_anthropic_terms(p.provider)
            },
            tool_calls: self.scanner.tool_calls.clone(),
            response_body: StoredBody::streamed(std::mem::take(&mut self.raw), self.plain_bytes),
            request_bytes: p.request_bytes,
            response_bytes: self.wire_bytes,
            shape: p.shape,
        };
        self.state.offer(ev);
    }
}

/// Whether a request's own environment says where its session runs: only a turn of the
/// main thread does. A subagent's (`x-claude-code-agent-id`) may run in a worktree of its
/// own. A request that offers the model no tools is not a turn of the conversation at
/// all, though it carries the session's id — a title, a summary, the auto-mode
/// classifier — and what it carries is the conversation quoted back, where any line can
/// look like an environment: a session editing this very parser once had its project
/// set to one of the parser's test strings that way.
fn speaks_for_its_session(agent_id: Option<&str>, shape: &RequestShape) -> bool {
    agent_id.is_none() && shape.tools_defined > 0
}

/// Start the capture task and return the sender the proxy tees into.
pub fn spawn(db_path: PathBuf) -> (mpsc::Sender<CaptureEvent>, tokio::task::JoinHandle<()>) {
    // Bounded: a burst deeper than this is dropped rather than queued, so memory
    // cannot grow without limit and the hot path is never asked to wait.
    let (tx, mut rx) = mpsc::channel::<CaptureEvent>(256);

    let task = tokio::spawn(async move {
        let mut db = match Db::open(&db_path) {
            Ok(db) => db,
            Err(err) => {
                tracing::error!(error = %err, "capture disabled: could not open the database");
                return;
            }
        };
        tracing::info!(db = %db_path.display(), "capture started");

        let mut resolver = Resolver::new();
        while let Some(ev) = rx.recv().await {
            if let Some(session) = &ev.session_id {
                // The transcript decides where there is one. A hosted router never has
                // one to look for; elsewhere it is missing for sessions on other
                // machines and ones the CLI never wrote to disk. Either way the request
                // said where it runs, and that fills whatever the transcript did not.
                let mut info = if crate::config::is_server() {
                    Default::default()
                } else {
                    resolver.resolve(session)
                };
                if let Some(shape) = ev.shape.as_ref().filter(|s| speaks_for_its_session(ev.agent_id.as_deref(), s)) {
                    if info.cwd.is_none() {
                        info.cwd = shape.cwd.clone();
                    }
                    if info.git_branch.is_none() {
                        info.git_branch = shape.git_branch.clone();
                    }
                }
                if let Err(err) = db.touch_session(
                    session,
                    info.cwd.as_deref(),
                    info.git_branch.as_deref(),
                    ev.entrypoint.as_deref(),
                ) {
                    tracing::warn!(error = %err, "could not record session");
                }
            }
            if let Err(err) = db.record(&ev) {
                tracing::warn!(error = %err, "could not record request");
            }
        }
        // The channel closed, which happens only once every sender is gone — i.e.
        // after the server has drained. Everything queued has now been written.
        tracing::info!("capture drained");
    });

    (tx, task)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;

    #[test]
    fn gzip_decoder_survives_arbitrary_chunking() {
        let plain = b"data: {\"type\":\"message_delta\"}\n\n".repeat(50);
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&plain).unwrap();
        let compressed = enc.finish().unwrap();

        let mut d = Decoder::for_encoding(Some("gzip"));
        let mut out = Vec::new();
        for chunk in compressed.chunks(7) {
            out.extend(d.feed(chunk));
        }
        out.extend(d.finish());
        assert_eq!(out, plain);
    }

    /// A response longer than the cap must say so. It is capped as it streams, so the
    /// buffer alone cannot tell — which is why 6,082 rows in the live store claimed to
    /// be complete at exactly the cap before this was counted separately.
    #[test]
    fn a_streamed_body_reports_the_size_it_had_before_the_cap() {
        let cut = StoredBody::streamed(vec![b'x'; 64], 4096);
        assert!(cut.truncated);
        assert_eq!(cut.true_bytes, 4096, "the size before the cap, not after it");
        assert_eq!(cut.stored.len(), 64);

        let whole = StoredBody::streamed(vec![b'x'; 64], 64);
        assert!(!whole.truncated, "a body that fitted is not truncated");
        assert_eq!(whole.true_bytes, 64);
    }

    /// Only the main thread's turns say where the session runs: not a subagent's, and not
    /// a request with no tools that quotes the conversation back.
    #[test]
    fn only_the_main_threads_turns_say_where_the_session_runs() {
        let env = "# Environment\nYou have been invoked in the following environment: \n - Primary working directory: /home/u/code/p\n";
        let turn = crate::analyze::analyze_request(
            serde_json::json!({
                "tools": [{ "name": "Bash", "input_schema": { "type": "object" } }],
                "messages": [
                    { "role": "user", "content": "fix it" },
                    { "role": "system", "content": env },
                ],
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(turn.cwd.as_deref(), Some("/home/u/code/p"));
        assert!(speaks_for_its_session(None, &turn));
        assert!(!speaks_for_its_session(Some("a2fc94267e771da5e"), &turn), "a subagent's");

        let quoted = format!("Judge this action. The transcript so far:\n{env}\nsed -n 1,9p src/analyze.rs\n - Primary working directory: /in/a/test\\n\";");
        let classifier = crate::analyze::analyze_request(
            serde_json::json!({ "messages": [{ "role": "user", "content": quoted }] }).to_string().as_bytes(),
        )
        .unwrap();
        assert!(classifier.cwd.is_some(), "it does look like an environment");
        assert!(!speaks_for_its_session(None, &classifier), "but it offers no tools");
    }

    /// Kimi Code's `input_tokens` holds its cache reads, so a request recorded as it came
    /// would count them twice: once as input and once as cache reads, and price both.
    #[tokio::test]
    async fn kimi_cached_tokens_are_counted_once() {
        let dir = std::env::temp_dir().join(format!("ar-kimi-usage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("agent-router.db");
        let mut db = crate::db::Db::open(&db_path).unwrap();
        crate::pricing::ensure_table(&rusqlite::Connection::open(&db_path).unwrap()).unwrap();
        rustls::crypto::ring::default_provider().install_default().ok();
        let config = crate::config::Config {
            server: false,
            bind: std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
            upstream: "https://upstream.invalid".into(),
            db_path: db_path.clone(),
            body_cap: 64 * 1024,
            dump_headers: 0,
            allow_hosts: Vec::new(),
            public_url: None,
            push_keys_dir: None,
            agents_dir: None,
            drain: std::time::Duration::from_secs(1),
            stop_delay: std::time::Duration::ZERO,
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let (network, _) = crate::network::switch();
        let state = crate::proxy::AppState::new(config, tx, network).unwrap();

        // The last `k3` request of AGEN-0097's session, as Kimi Code streamed its usage.
        let sse = concat!(
            "event:message_start\n",
            r#"data:{"type":"message_start","message":{"id":"msg_k","type":"message","role":"assistant","content":[],"model":"k3","usage":{"input_tokens":18880,"cache_creation_input_tokens":0,"cache_read_input_tokens":18432,"output_tokens":0}}}"#,
            "\n\nevent:message_delta\n",
            r#"data:{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":18880,"cache_creation_input_tokens":0,"cache_read_input_tokens":18432,"output_tokens":50}}"#,
            "\n\nevent:message_stop\n",
            r#"data:{"type":"message_stop"}"#,
            "\n\n",
        );
        let mut events = Vec::new();
        for (id, provider) in [("req-kimi", crate::accounts::Provider::Kimi), ("req-claude", crate::accounts::Provider::Anthropic)] {
            let pending = Pending {
                request_id: Some(id.into()),
                session_id: None,
                agent_id: None,
                account_id: provider.as_str().into(),
                account_email: None,
                provider,
                entrypoint: None,
                method: "POST".into(),
                path: "/v1/messages?beta=true".into(),
                started_at_ms: 1_760_000_000_000,
                ttfb_ms: 1,
                status: 200,
                streaming_requested: true,
                rl: Default::default(),
                shape: None,
                request_bytes: 10,
            };
            let mut tee = Tee::new(state.clone(), pending, None, true, 64 * 1024, std::time::Instant::now());
            tee.feed(sse.as_bytes());
            drop(tee);
            events.push(rx.recv().await.expect("the capture event"));
        }
        for ev in &events {
            db.record(ev).unwrap();
        }

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let row = |id: &str| -> (i64, i64, i64, f64) {
            conn.query_row(
                "SELECT u.input_tokens, u.cache_read_tokens, u.prompt_tokens, c.cost
                 FROM requests r JOIN usage u ON u.request_id = r.id JOIN request_cost c ON c.request_id = r.id
                 WHERE r.request_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap()
        };
        let (input, cached, prompt, cost) = row("req-kimi");
        assert_eq!((input, cached, prompt), (448, 18_432, 18_880), "each token of the 18.9K context once");
        let k3 = 448.0 * 3.0 + 18_432.0 * 0.30 + 50.0 * 15.0;
        assert!((cost - k3 / 1_000_000.0).abs() < 1e-9, "priced once at K3's rates: {cost}");
        // Anthropic's input_tokens already leaves its cache reads out: recorded as it came.
        let (input, cached, prompt, _) = row("req-claude");
        assert_eq!((input, cached, prompt), (18_880, 18_432, 37_312));
        drop(conn);
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Kimi Code request as it reaches the router: no Claude Code session header, the
    /// session in `metadata.user_id`, the directory in a sentence of the system prompt.
    fn kimi_body(session: &str, cwd: &str) -> String {
        serde_json::json!({
            "model": "k3", "max_tokens": 32000, "stream": true,
            "metadata": { "user_id": session },
            "system": [{ "type": "text", "text": format!("You are Kimi Code.\nThe current working directory is `{cwd}`. Use it."), "cache_control": { "type": "ephemeral" } }],
            "tools": [{ "name": "Bash", "input_schema": { "type": "object" } }],
            "messages": [{ "role": "user", "content": "hi" }],
        })
        .to_string()
    }

    /// Run the capture task over a store of its own with one event, as the proxy hands it on.
    async fn captured(tag: &str, ev: CaptureEvent) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ar-kimi-capture-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("agent-router.db");
        crate::db::Db::open(&db).unwrap();
        crate::pricing::ensure_table(&rusqlite::Connection::open(&db).unwrap()).unwrap();
        let (tx, task) = spawn(db.clone());
        tx.send(ev).await.unwrap();
        drop(tx);
        task.await.unwrap();
        db
    }

    fn kimi_event(session: Option<String>, body: &str, user_agent: &str) -> CaptureEvent {
        CaptureEvent {
            request_id: Some(format!("req-{}", user_agent.len())),
            message_id: None,
            session_id: session,
            account_email: None,
            agent_id: None,
            account_id: "kimi".into(),
            provider: crate::accounts::Provider::Kimi,
            entrypoint: Some(user_agent.into()),
            model: Some("k3".into()),
            method: "POST".into(),
            path: "/v1/messages?beta=true".into(),
            started_at_ms: 1_760_000_000_000,
            ttfb_ms: 1,
            duration_ms: 2,
            status: 200,
            streaming: true,
            stop_reason: None,
            service_tier: None,
            error: None,
            rl: Default::default(),
            usage: Default::default(),
            tool_calls: Vec::new(),
            shape: crate::analyze::analyze_request(body.as_bytes()),
            response_body: StoredBody { stored: Vec::new(), truncated: false, true_bytes: 0 },
            request_bytes: body.len() as i64,
            response_bytes: 0,
        }
    }

    #[tokio::test]
    async fn a_kimi_request_is_filed_under_its_session() {
        let session = "session_6f708192-0000-4000-8000-0000000000aa";
        let body = kimi_body(session, "/Users/u/code/kimi-project");
        let shape = crate::analyze::analyze_request(body.as_bytes());
        // No Claude Code header: the body names the session. A header, when there is one, wins.
        let filed = crate::proxy::session_of(None, shape.as_ref());
        assert_eq!(filed.as_deref(), Some(session));
        assert_eq!(crate::proxy::session_of(Some("claude-1".into()), shape.as_ref()).as_deref(), Some("claude-1"));
        // Claude Code's own `metadata.user_id` is another shape, and never taken.
        let claude = serde_json::json!({ "model": "claude-opus-5-5", "metadata": { "user_id": "user_abc_account_def_session_ghi" }, "messages": [] });
        assert_eq!(crate::analyze::analyze_request(claude.to_string().as_bytes()).unwrap().body_session, None);

        let db = captured("filed", kimi_event(filed, &body, "kimi-code-cli/2.1.1")).await;
        let conn = rusqlite::Connection::open(&db).unwrap();
        let (requests, cwd): (i64, Option<String>) = conn
            .query_row(
                "SELECT (SELECT count(*) FROM requests WHERE session_id = ?1), (SELECT cwd FROM sessions WHERE id = ?1)",
                [session],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((requests, cwd.as_deref()), (1, Some("/Users/u/code/kimi-project")), "filed under its session, in its directory");
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn a_kimi_requests_directory_is_read_from_its_system_prompt() {
        let body = kimi_body("session_x", "/Users/u/code/kimi-project");
        let shape = crate::analyze::analyze_request(body.as_bytes()).unwrap();
        assert_eq!(shape.cwd.as_deref(), Some("/Users/u/code/kimi-project"));
        // Claude Code's line still wins where both are, and a relative path is not taken.
        let both = serde_json::json!({ "system": " - Primary working directory: /claude/dir\nThe current working directory is `/kimi/dir`.", "messages": [] });
        assert_eq!(crate::analyze::analyze_request(both.to_string().as_bytes()).unwrap().cwd.as_deref(), Some("/claude/dir"));
        let relative = serde_json::json!({ "system": "The current working directory is `code/here`.", "messages": [] });
        assert_eq!(crate::analyze::analyze_request(relative.to_string().as_bytes()).unwrap().cwd, None);
        // Quoted later in the conversation it is someone's text, not the environment.
        let quoted = serde_json::json!({ "system": "You are Kimi Code.", "messages": [
            { "role": "user", "content": "hi" },
            { "role": "assistant", "content": "ok" },
            { "role": "user", "content": "The current working directory is `/quoted/later`." }] });
        assert_eq!(crate::analyze::analyze_request(quoted.to_string().as_bytes()).unwrap().cwd, None);
    }

    #[tokio::test]
    async fn a_kimi_request_is_labelled_with_its_client() {
        for (ua, client) in [
            ("kimi-code-cli/2.1.1", Some("kimi-code-cli")),
            ("kimi-code-cli/2.1.1 (web)", Some("kimi-code-web")),
            ("kimi-code-vscode/0.8.1", Some("kimi-code-vscode")),
            ("claude-cli/2.1.267 (external, cli)", Some("claude-code-cli")),
            ("claude-cli/2.1.267 (external, claude-vscode)", Some("claude-code-vscode")),
            ("opencode/1.19.0 ai-sdk/provider-utils/4.0.46 runtime/bun/1.3.14", Some("opencode")),
            ("curl/8.7.1", None),
        ] {
            assert_eq!(crate::analyze::client_of(ua), client, "{ua}");
        }
        // The session the request was filed under carries it, on /api/sessions.
        let session = "session_6f708192-0000-4000-8000-0000000000bb";
        let body = kimi_body(session, "/Users/u/code/kimi-project");
        let db = captured("client", kimi_event(Some(session.into()), &body, "kimi-code-vscode/0.8.1")).await;
        let reads = crate::api::ReadPool::new(&db);
        let everything = crate::api::Filter { project: None, session: None, account: None, since: None, until: None, tz: None, provider: None };
        let list = crate::api::sessions_of(&reads.get().unwrap(), &everything).unwrap();
        let row = list.as_array().unwrap().iter().find(|s| s["id"] == session).unwrap().clone();
        assert_eq!((&row["client"], &row["entrypoint"]), (&serde_json::json!("kimi-code-vscode"), &serde_json::json!("kimi-code-vscode/0.8.1")));
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }
}
