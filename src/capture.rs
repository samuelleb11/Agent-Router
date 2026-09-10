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
    pub fn new(bytes: &[u8], cap: usize) -> Self {
        let truncated = bytes.len() > cap;
        Self {
            stored: bytes[..bytes.len().min(cap)].to_vec(),
            truncated,
            true_bytes: bytes.len() as i64,
        }
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
    pub request_body: StoredBody,
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
    pub entrypoint: Option<String>,
    pub method: String,
    pub path: String,
    pub started_at_ms: i64,
    pub ttfb_ms: i64,
    pub status: i64,
    pub streaming_requested: bool,
    pub rl: RateLimit,
    pub shape: Option<RequestShape>,
    pub request_body: StoredBody,
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
    wire_bytes: i64,
    is_sse: bool,
    started: std::time::Instant,
    transport_error: Option<String>,
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
            wire_bytes: 0,
            is_sse,
            started,
            transport_error: None,
        }
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
            entrypoint: p.entrypoint,
            model: self
                .scanner
                .model
                .clone()
                .or_else(|| p.shape.as_ref().and_then(|s| s.model.clone())),
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
            usage: self.scanner.usage.clone(),
            tool_calls: self.scanner.tool_calls.clone(),
            request_body: p.request_body,
            response_body: StoredBody::new(&self.raw, self.body_cap),
            request_bytes: p.request_bytes,
            response_bytes: self.wire_bytes,
            shape: p.shape,
        };
        self.state.offer(ev);
    }
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
                let info = resolver.resolve(session);
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

    #[test]
    fn body_truncation_records_the_true_size() {
        let b = StoredBody::new(&vec![b'x'; 100], 10);
        assert_eq!(b.stored.len(), 10);
        assert!(b.truncated);
        assert_eq!(b.true_bytes, 100);

        let b = StoredBody::new(b"short", 10);
        assert!(!b.truncated);
        assert_eq!(b.true_bytes, 5);
    }
}
