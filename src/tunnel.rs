//! What a machine agent and a hosted router say to each other. See
//! `docs/hosting-machine-agent.md`.
//!
//! The agent holds one WebSocket open to the router (`PATH`), and the router sends it the
//! live requests for the sessions on its machine: follow this one, answer that prompt.
//! The agent runs each against the same `live` handlers a local router serves, in
//! process, and streams the response back. One socket carries many requests at once, so
//! every frame names the request it belongs to.
//!
//! A frame is one binary WebSocket message: a kind byte, the request id as a big-endian
//! `u32`, then the payload. Heads travel as JSON; bodies as raw bytes, cut into pieces
//! so that one long event stream never holds the socket for long.

use serde::{Deserialize, Serialize};

/// Where an agent connects on a hosted router.
pub const PATH: &str = "/_router/agent";

/// agent -> router, first: who is connecting.
pub const HELLO: u8 = 1;
/// router -> agent: run this request.
pub const REQUEST: u8 = 2;
/// agent -> router: the response's status and headers.
pub const HEAD: u8 = 3;
/// agent -> router: the next piece of a response body.
pub const BODY: u8 = 4;
/// agent -> router: the response is complete.
pub const END: u8 = 5;
/// router -> agent: nobody wants this response any more (a viewer closed the page).
pub const CANCEL: u8 = 6;
/// agent -> router: the request failed before it had a head.
pub const FAIL: u8 = 7;

/// The most a `BODY` frame carries.
pub const CHUNK: usize = 64 * 1024;
/// The largest message either side accepts. A wrapper frame is up to 1 MiB, so a request
/// that carries one is too, and base64 makes it a third larger again.
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;

/// `[kind][id: u32 big-endian][payload]`.
pub fn encode(kind: u8, id: u32, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(kind);
    frame.extend_from_slice(&id.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// The kind, the id and the payload — `None` for anything shorter than a header.
pub fn decode(frame: &[u8]) -> Option<(u8, u32, &[u8])> {
    let (head, payload) = frame.split_at_checked(5)?;
    let id = u32::from_be_bytes([head[1], head[2], head[3], head[4]]);
    Some((head[0], id, payload))
}

/// Who is connecting: sent once, first.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    /// A random id the agent made on its first run and keeps: it survives a rename.
    pub machine: String,
    /// What to call the machine — its host name unless told otherwise.
    pub label: String,
    /// `macos`, `windows`, `linux`.
    pub platform: String,
    pub version: String,
    pub build: String,
}

/// A request for the agent to run.
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub method: String,
    /// Path and query, as the router received them.
    pub path: String,
    pub headers: Vec<(String, String)>,
    /// Base64, so the frame stays one JSON document.
    #[serde(default)]
    pub body: String,
}

/// A response's status and headers; its body follows in `BODY` frames.
#[derive(Debug, Serialize, Deserialize)]
pub struct Head {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

/// Standard base64 with padding. The decoder is `auth::base64_decode`.
pub fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            out.push(if i <= chunk.len() { A[((n >> (18 - 6 * i)) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// Request and response headers worth carrying across: what the handlers read and what
/// a browser needs back. Hop-by-hop headers belong to each connection, and the router's
/// own credential never leaves it.
pub fn carried(name: &str) -> bool {
    !matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
            | "host"
            | "authorization"
            | "cookie"
            | "content-length"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_reads_back_as_it_was_written() {
        for (kind, id, payload) in [(HELLO, 0u32, &b"{}"[..]), (BODY, u32::MAX, &b""[..]), (CANCEL, 7, &[0u8, 255][..])] {
            let frame = encode(kind, id, payload);
            assert_eq!(decode(&frame), Some((kind, id, payload)));
        }
        assert_eq!(decode(&[1, 0, 0, 0]), None, "shorter than a header");
    }

    #[test]
    fn base64_round_trips() {
        for bytes in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0, 255, 128, 7][..]] {
            assert_eq!(crate::auth::base64_decode(&base64(bytes)).unwrap(), bytes);
        }
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b"fo"), "Zm8=");
    }

    #[test]
    fn credentials_and_hop_by_hop_headers_stay_behind() {
        for h in ["authorization", "cookie", "host", "connection", "transfer-encoding", "content-length"] {
            assert!(!carried(h), "{h}");
        }
        for h in ["accept", "content-type", "last-event-id", "cache-control", "x-accel-buffering"] {
            assert!(carried(h), "{h}");
        }
    }
}
