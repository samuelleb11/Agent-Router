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

/// Start the capture task and return the sender the proxy tees into.
pub fn spawn(db_path: PathBuf) -> mpsc::Sender<CaptureEvent> {
    // Bounded: a burst deeper than this is dropped rather than queued, so memory
    // cannot grow without limit and the hot path is never asked to wait.
    let (tx, mut rx) = mpsc::channel::<CaptureEvent>(256);

    tokio::spawn(async move {
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
    });

    tx
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
