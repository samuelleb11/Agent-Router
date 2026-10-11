//! Parsing of what crossed the wire. Runs on the capture task, never on the hot path.
//!
//! Two directions:
//!   * `SseScanner` consumes the (decompressed) response stream incrementally and
//!     extracts usage, model, stop reason and tool calls. Incremental because the
//!     final token counts arrive in `message_delta` at the very end of a stream that
//!     may be larger than anything we want to hold in memory.
//!   * `analyze_request` walks the request JSON to work out what filled the context
//!     window: system prompt, tool definitions, conversation history, tool results.

use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub thinking_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_5m_tokens: i64,
    pub cache_creation_1h_tokens: i64,
}

impl Usage {
    /// Every token the model had to read for this request, cached or not.
    pub fn prompt_tokens(&self) -> i64 {
        self.input_tokens + self.cache_read_tokens + self.cache_creation_tokens()
    }

    pub fn cache_creation_tokens(&self) -> i64 {
        self.cache_creation_5m_tokens + self.cache_creation_1h_tokens
    }

    /// This usage as Anthropic counts it, whichever provider reported it, so every rollup,
    /// the context panels and the cost view read one meaning. Anthropic's `input_tokens`
    /// leaves out what was read from cache; Kimi Code's holds it (measured 2026-10-10,
    /// docs/sources-and-models.md), so its fresh input is what is left once the cache reads
    /// are taken out.
    pub fn in_anthropic_terms(mut self, provider: crate::accounts::Provider) -> Self {
        if provider == crate::accounts::Provider::Kimi {
            self.input_tokens = (self.input_tokens - self.cache_read_tokens).max(0);
        }
        self
    }

    fn merge_from(&mut self, u: &Value) {
        let n = |k: &str| u.get(k).and_then(Value::as_i64).unwrap_or(0);
        // message_start carries the prompt side; message_delta carries the final
        // output count. Take the larger of each so ordering cannot lose a value.
        self.input_tokens = self.input_tokens.max(n("input_tokens"));
        self.output_tokens = self.output_tokens.max(n("output_tokens"));
        self.cache_read_tokens = self.cache_read_tokens.max(n("cache_read_input_tokens"));
        self.thinking_tokens = self.thinking_tokens.max(
            u.get("output_tokens_details")
                .and_then(|d| d.get("thinking_tokens"))
                .and_then(Value::as_i64)
                .unwrap_or(0),
        );
        if let Some(cc) = u.get("cache_creation") {
            let c = |k: &str| cc.get(k).and_then(Value::as_i64).unwrap_or(0);
            self.cache_creation_5m_tokens = self
                .cache_creation_5m_tokens
                .max(c("ephemeral_5m_input_tokens"));
            self.cache_creation_1h_tokens = self
                .cache_creation_1h_tokens
                .max(c("ephemeral_1h_input_tokens"));
        } else {
            // Older shape: a single undifferentiated cache_creation_input_tokens.
            let total = n("cache_creation_input_tokens");
            if total > 0 && self.cache_creation_tokens() == 0 {
                self.cache_creation_5m_tokens = total;
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct SseScanner {
    partial: Vec<u8>,
    pub message_id: Option<String>,
    pub model: Option<String>,
    pub stop_reason: Option<String>,
    pub service_tier: Option<String>,
    pub usage: Usage,
    pub tool_calls: Vec<String>,
    pub error: Option<String>,
    pub saw_any_event: bool,
}

impl SseScanner {
    /// Feed decompressed response bytes. Safe to call with arbitrary chunk splits —
    /// a partial trailing line is held until the rest of it arrives.
    pub fn feed(&mut self, chunk: &[u8]) {
        self.partial.extend_from_slice(chunk);
        while let Some(pos) = self.partial.iter().position(|&b| b == b'\n') {
            let line = self.partial.drain(..=pos).collect::<Vec<u8>>();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\r', '\n']);
            if let Some(payload) = line.strip_prefix("data:") {
                if let Ok(v) = serde_json::from_str::<Value>(payload.trim()) {
                    self.absorb(&v);
                }
            }
        }
        // Guard against a pathological stream with no newline at all.
        if self.partial.len() > 1 << 20 {
            self.partial.clear();
        }
    }

    /// A non-streaming JSON response body, parsed whole.
    pub fn absorb_json_body(&mut self, body: &[u8]) {
        if let Ok(v) = serde_json::from_slice::<Value>(body) {
            if let Some(detail) = v.get("detail").and_then(Value::as_str) {
                self.error = Some(detail.to_owned());
                return;
            }
            if crate::responses::absorb(self, &v) {
                return;
            }
            self.absorb_message(&v);
            self.absorb(&v);
            // Kimi Code's error bodies are `{"error": {"message", "type"}}`, with no
            // top-level `"type": "error"` for `absorb` to know them by.
            if self.error.is_none() {
                if let Some(message) = v.get("error").and_then(|e| e.get("message")).and_then(Value::as_str) {
                    self.error = Some(message.to_string());
                }
            }
        }
    }

    fn absorb(&mut self, v: &Value) {
        self.saw_any_event = true;
        if crate::responses::absorb(self, v) {
            return;
        }
        match v.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(m) = v.get("message") {
                    self.absorb_message(m);
                }
            }
            Some("message_delta") => {
                if let Some(u) = v.get("usage") {
                    self.usage.merge_from(u);
                }
                if let Some(d) = v.get("delta") {
                    if let Some(sr) = d.get("stop_reason").and_then(Value::as_str) {
                        self.stop_reason = Some(sr.to_string());
                    }
                }
            }
            Some("content_block_start") => {
                if let Some(b) = v.get("content_block") {
                    if b.get("type").and_then(Value::as_str) == Some("tool_use") {
                        if let Some(name) = b.get("name").and_then(Value::as_str) {
                            self.tool_calls.push(name.to_string());
                        }
                    }
                }
            }
            Some("error") => {
                self.error = Some(
                    v.get("error")
                        .and_then(|e| e.get("message"))
                        .or_else(|| v.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                        .to_string(),
                );
            }
            _ => {}
        }
    }

    fn absorb_message(&mut self, m: &Value) {
        if let Some(id) = m.get("id").and_then(Value::as_str) {
            self.message_id = Some(id.to_string());
        }
        if let Some(model) = m.get("model").and_then(Value::as_str) {
            self.model = Some(model.to_string());
        }
        if let Some(sr) = m.get("stop_reason").and_then(Value::as_str) {
            self.stop_reason = Some(sr.to_string());
        }
        if let Some(u) = m.get("usage") {
            self.usage.merge_from(u);
            if let Some(t) = u.get("service_tier").and_then(Value::as_str) {
                self.service_tier = Some(t.to_string());
            }
        }
        // Non-streaming bodies carry their tool calls inline.
        if let Some(content) = m.get("content").and_then(Value::as_array) {
            for b in content {
                if b.get("type").and_then(Value::as_str) == Some("tool_use") {
                    if let Some(name) = b.get("name").and_then(Value::as_str) {
                        self.tool_calls.push(name.to_string());
                    }
                }
            }
        }
    }
}

/// One tool result observed in the request's conversation history.
///
/// Keyed by `tool_use_id`, which is stable across requests — every subsequent
/// request in a session replays the same result, so the store must deduplicate on
/// this id or a long session will count the same tool output dozens of times.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub name: Option<String>,
    pub result_chars: i64,
    pub is_error: bool,
}

#[derive(Debug, Default)]
pub struct RequestShape {
    pub model: Option<String>,
    /// Explicit route attribution, retained when a registered alias differs from
    /// the upstream model. These values describe this request at send time.
    pub service_id: Option<String>,
    pub upstream_model: Option<String>,
    pub streaming: bool,
    pub message_count: i64,
    pub system_chars: i64,
    pub tools_chars: i64,
    pub tools_defined: i64,
    /// History excluding tool results, which are counted separately.
    pub history_chars: i64,
    pub tool_result_chars: i64,
    pub tool_results: Vec<ToolResult>,
    /// Every distinct piece of text in this request's conversation. The search corpus.
    pub blocks: Vec<Block>,
    /// The working directory and git branch Claude Code told the model it runs in. Read
    /// when no transcript can say: a hosted router, a session on another machine, one
    /// the CLI never wrote to disk. See `environment_of`.
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    /// The session a Kimi Code request names in its body, `metadata.user_id`, which is
    /// `session_<uuid>` on every request, its subagents' included (docs/kimi-client.md).
    /// Claude Code's own `metadata.user_id` is another shape and is never taken.
    pub body_session: Option<String>,
}

/// The client a User-Agent names, as the dashboards label it. `None` for any other caller.
pub fn client_of(user_agent: &str) -> Option<&'static str> {
    let ua = user_agent.trim();
    if ua.starts_with("codex_cli_rs/") || ua.starts_with("codex_exec/") || ua.starts_with("codex/") || ua.starts_with("codex-cli/") {
        Some("codex-cli")
    } else if ua.starts_with("codex_vscode/") || ua.starts_with("codex-vscode/") {
        Some("codex-vscode")
    } else if ua.starts_with("opencode/") {
        Some("opencode")
    } else if let Some(rest) = ua.strip_prefix("kimi-code-cli/") {
        Some(if rest.contains("(web)") { "kimi-code-web" } else { "kimi-code-cli" })
    } else if ua.starts_with("kimi-code-vscode/") {
        Some("kimi-code-vscode")
    } else if ua.starts_with("claude-cli/") {
        Some(if ua.contains("claude-vscode") { "claude-code-vscode" } else { "claude-code-cli" })
    } else {
        None
    }
}

/// One content block out of the conversation, as the search index stores it.
///
/// `hash` is the identity: the same text in the same kind of block is the same block,
/// however many requests replay it. `ordinal` and `role` describe where this copy sat,
/// and travel to `block_sites` rather than into the hash.
#[derive(Debug, Clone)]
pub struct Block {
    pub hash: [u8; 32],
    pub kind: &'static str,
    pub role: String,
    /// Index in this request's `messages` array.
    pub ordinal: i64,
    pub text: String,
    /// Length before `MAX_BLOCK_CHARS` was applied, so a clipped block still says so.
    pub chars: i64,
}

/// A single block wider than this is clipped. Nothing legitimate comes close — the
/// largest real ones are whole-file tool results — and it bounds what one pathological
/// request can put into the index.
const MAX_BLOCK_CHARS: usize = 1 << 20;

pub fn analyze_request(body: &[u8]) -> Option<RequestShape> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let (cwd, git_branch) = environment_of(&v);
    let mut shape = RequestShape {
        model: v.get("model").and_then(Value::as_str).map(str::to_owned),
        streaming: v.get("stream").and_then(Value::as_bool).unwrap_or(false),
        cwd,
        git_branch,
        body_session: v
            .get("metadata")
            .and_then(|m| m.get("user_id"))
            .and_then(Value::as_str)
            .filter(|id| id.starts_with("session_") && id.len() <= 128)
            .map(str::to_owned),
        ..Default::default()
    };

    shape.system_chars = match v.get("system") {
        Some(Value::String(s)) => s.chars().count() as i64,
        Some(other) => json_chars(other),
        None => 0,
    };
    if let Some(tools) = v.get("tools").and_then(Value::as_array) {
        shape.tools_defined = tools.len() as i64;
        shape.tools_chars = tools.iter().map(json_chars).sum();
    }

    // tool_use blocks name the call; the matching tool_result arrives in a later
    // user message. Walk once to name them, then attribute result sizes.
    let mut names: std::collections::HashMap<String, String> = Default::default();
    let messages = v.get("messages").and_then(Value::as_array);
    if let Some(messages) = messages {
        shape.message_count = messages.len() as i64;
        for m in messages {
            if let Some(blocks) = m.get("content").and_then(Value::as_array) {
                for b in blocks {
                    if b.get("type").and_then(Value::as_str) == Some("tool_use") {
                        if let (Some(id), Some(name)) = (
                            b.get("id").and_then(Value::as_str),
                            b.get("name").and_then(Value::as_str),
                        ) {
                            names.insert(id.to_string(), name.to_string());
                        }
                    }
                }
            }
        }
        for m in messages {
            match m.get("content") {
                Some(Value::String(s)) => shape.history_chars += s.chars().count() as i64,
                Some(Value::Array(blocks)) => {
                    for b in blocks {
                        let chars = json_chars(b);
                        if b.get("type").and_then(Value::as_str) == Some("tool_result") {
                            let id = b
                                .get("tool_use_id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            shape.tool_result_chars += chars;
                            shape.tool_results.push(ToolResult {
                                name: names.get(&id).cloned(),
                                tool_use_id: id,
                                result_chars: chars,
                                is_error: b
                                    .get("is_error")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false),
                            });
                        } else {
                            shape.history_chars += chars;
                        }
                    }
                }
                _ => {}
            }
        }
        shape.blocks = extract_blocks(messages);
    }
    // The system prompt carries the project's own instructions, so it is worth finding
    // by its text and not only by its size. It sits ahead of the conversation, at
    // negative ordinals, because it is not a turn in it.
    //
    // Nearly every request in a project repeats the same system prompt word for word,
    // so this costs one block per distinct version however many requests carry it.
    // Tool definitions are left out deliberately: they are client boilerplate, the same
    // in every session of a given Claude Code build, so searching them would match
    // everything or nothing — and `context` already records their size and count.
    match v.get("system") {
        Some(Value::String(text)) => push_block(&mut shape.blocks, "system", "system", -1, text.clone()),
        Some(Value::Array(parts)) => {
            for (i, part) in parts.iter().enumerate() {
                let text = match part {
                    Value::String(t) => t.clone(),
                    other => str_field(other, "text"),
                };
                push_block(&mut shape.blocks, "system", "system", -(i as i64) - 1, text);
            }
        }
        _ => {}
    }
    Some(shape)
}

/// Where the session runs, as Claude Code tells the model: its environment block names
/// the directory (` - Primary working directory: <path>`, or `Working directory: <path>`
/// from older builds) and its git status the branch (`Current branch: <name>`).
///
/// Looked for only where Claude Code puts them — the `system` field, then the opening
/// turn, the messages before the model's first reply: newer builds carry the same blocks
/// in the first user message, and since 2.1.292 the environment in a message of its own
/// with the `system` role — and only as whole lines of those exact shapes. Anywhere later
/// in a conversation the same words are quoted text: a tool result, a file being edited,
/// this very comment. A later `system` message, "# Environment update", says the
/// directory moved ("<new> (was <old>)"), which is not where the session started. The
/// format is Claude Code's and undocumented, so a miss is a session with no project, as
/// it was before; a transcript, where there is one, still decides.
fn environment_of(v: &Value) -> (Option<String>, Option<String>) {
    let mut texts: Vec<&str> = Vec::new();
    match v.get("system") {
        Some(Value::String(t)) => texts.push(t),
        Some(Value::Array(parts)) => texts.extend(parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str))),
        _ => {}
    }
    let opening = v
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take_while(|m| m.get("role").and_then(Value::as_str) != Some("assistant"));
    for m in opening {
        match m.get("content") {
            Some(Value::String(t)) => texts.push(t),
            Some(Value::Array(blocks)) => texts.extend(
                blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(Value::as_str)),
            ),
            _ => {}
        }
    }
    environment_in(&texts)
}

/// The lines themselves, in texts already known to be the opening's: a request's, or
/// the opening blocks the store kept of a session recorded before they were read
/// (`db::backfill_session_cwds`).
pub fn environment_in(texts: &[&str]) -> (Option<String>, Option<String>) {
    // A note that the directory changed is not where it started, in either shape.
    let lines = || texts.iter().flat_map(|t| t.lines()).map(str::trim_start).filter(|l| !l.contains(" (was "));
    let primary = lines().find_map(|l| {
        let l = l.strip_prefix("- ").unwrap_or(l);
        l.strip_prefix("Primary working directory: ").and_then(as_path)
    });
    // The older line, only when the newer one is not there.
    let cwd = primary.or_else(|| lines().find_map(|l| l.strip_prefix("Working directory: ").and_then(as_path)));
    // Kimi Code says it in a sentence of its own: "The current working directory is `<path>`".
    let cwd = cwd.or_else(|| {
        texts.iter().find_map(|t| {
            let (_, rest) = t.split_once("The current working directory is `")?;
            as_path(rest.split_once('`')?.0)
        })
    });
    let branch = lines().find_map(|l| {
        let name = l.strip_prefix("Current branch: ")?.trim();
        (!name.is_empty() && name.len() <= 255 && !name.contains(char::is_whitespace)).then(|| name.to_string())
    });
    (cwd, branch)
}

/// An absolute path as Claude Code prints one: POSIX, a Windows drive (`C:\…`), or a
/// UNC share. Anything else — a relative path, a sentence — is not taken.
fn as_path(raw: &str) -> Option<String> {
    let p = raw.trim();
    let b = p.as_bytes();
    let absolute = p.starts_with('/')
        || p.starts_with("\\\\")
        || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'));
    (absolute && p.len() <= 4096 && !p.chars().any(char::is_control)).then(|| p.to_string())
}

/// Pull the conversation apart into indexable text.
///
/// This runs here, on the full request body, because this is the only place the whole
/// conversation exists in memory. What reaches the store is capped at `body_cap`, and
/// since `messages` is the first field of the request JSON that cap keeps the *oldest*
/// part of a conversation and drops the newest — so the index cannot be built from
/// what was captured, only from what crossed the wire.
///
/// Every request replays the entire history, so a block here is re-extracted once per
/// request in the session. Deduplication happens at the point of insert, by hash, in
/// `Db::record`: on a real store the replayed bytes outweigh the distinct ones by about
/// three hundred to one, which is what makes indexing the uncapped text cheaper than
/// storing the capped bodies already is.
///
/// One block per *content block*, not per message: a turn that thinks, writes and then
/// calls three tools is five rows, each searchable on its own.
fn extract_blocks(messages: &[Value]) -> Vec<Block> {
    let mut out = Vec::new();
    for (i, m) in messages.iter().enumerate() {
        let role = m.get("role").and_then(Value::as_str).unwrap_or("unknown").to_string();
        let ordinal = i as i64;
        match m.get("content") {
            Some(Value::String(s)) => push_block(&mut out, "text", &role, ordinal, s.clone()),
            Some(Value::Array(blocks)) => {
                for b in blocks {
                    let (kind, text) = match b.get("type").and_then(Value::as_str) {
                        Some("text") => ("text", str_field(b, "text")),
                        Some("thinking") => ("thinking", str_field(b, "thinking")),
                        // The call's arguments are the searchable part: a shell command,
                        // a path, a pattern. The name alone would match every Bash call.
                        Some("tool_use") => (
                            "tool_use",
                            format!(
                                "{} {}",
                                b.get("name").and_then(Value::as_str).unwrap_or(""),
                                b.get("input").map(flatten).unwrap_or_default()
                            ),
                        ),
                        Some("tool_result") => {
                            ("tool_result", b.get("content").map(flatten).unwrap_or_default())
                        }
                        // Images, documents and redacted thinking carry no text to index.
                        _ => continue,
                    };
                    push_block(&mut out, kind, &role, ordinal, text);
                }
            }
            _ => {}
        }
    }
    out
}

/// Blocks out of a body that may have been cut short, for indexing what was captured
/// before there was an index. Complete bodies take the ordinary path.
pub fn blocks_from_stored_body(body: &[u8]) -> Vec<Block> {
    match analyze_request(body) {
        Some(shape) => shape.blocks,
        None => extract_blocks(&recover_messages(body)),
    }
}

/// Recover whatever complete messages a truncated request body still holds.
///
/// A body captured under `body_cap` is not valid JSON — it stops mid-token — so
/// `serde_json` refuses the whole of it. But the cut is at the end and `messages` is
/// the first field, so what survives is a run of complete message objects followed by
/// one partial one. This walks the array counting braces and hands back every element
/// that closed before the bytes ran out, dropping the partial tail.
///
/// Braces inside strings are text, not structure, so the scan tracks string state and
/// its escapes; without that a `{` in a shell command or a file path would throw the
/// depth off and swallow the rest of the conversation.
pub fn recover_messages(body: &[u8]) -> Vec<Value> {
    let Some(start) = messages_array_at(body) else { return Vec::new() };
    let tail = &body[start..];
    let mut out = Vec::new();
    let (mut depth, mut in_string, mut escaped) = (0i32, false, false);
    let mut elem_start = None;
    for (i, &b) in tail.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    elem_start = Some(i);
                }
                depth += 1;
            }
            b'[' => depth += 1,
            b'}' | b']' => {
                depth -= 1;
                if depth < 0 {
                    break; // the messages array itself closed: nothing was truncated
                }
                if depth == 0 {
                    if let Some(from) = elem_start.take() {
                        if let Ok(v) = serde_json::from_slice(&tail[from..=i]) {
                            out.push(v);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Offset of the first element of the top-level `messages` array.
///
/// A plain search for the key, because in a request body it sits at the top level
/// within the first few bytes. A conversation that quotes `"messages":[` inside its own
/// text could mislead it, but only for a body that failed to parse whole — and then the
/// worst case is recovering from the wrong offset, not corrupting anything.
fn messages_array_at(body: &[u8]) -> Option<usize> {
    let key = br#""messages""#;
    let mut from = 0;
    while let Some(hit) = body[from..]
        .windows(key.len())
        .position(|w| w == key)
        .map(|p| from + p)
    {
        let mut i = hit + key.len();
        while body.get(i).is_some_and(|b| b.is_ascii_whitespace()) {
            i += 1;
        }
        if body.get(i) == Some(&b':') {
            i += 1;
            while body.get(i).is_some_and(|b| b.is_ascii_whitespace()) {
                i += 1;
            }
            if body.get(i) == Some(&b'[') {
                return Some(i + 1);
            }
        }
        from = hit + key.len();
    }
    None
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Text out of a value that may be a string, a list of blocks, or arbitrary JSON.
/// Tool results take all three shapes depending on the tool.
fn flatten(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => {
            items.iter().map(flatten).collect::<Vec<_>>().join("\n")
        }
        Value::Object(map) => match map.get("text").and_then(Value::as_str) {
            Some(t) => t.to_string(),
            None => serde_json::to_string(v).unwrap_or_default(),
        },
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn push_block(out: &mut Vec<Block>, kind: &'static str, role: &str, ordinal: i64, text: String) {
    if text.trim().is_empty() {
        return;
    }
    let chars = text.chars().count() as i64;
    let text = match text.char_indices().nth(MAX_BLOCK_CHARS) {
        Some((cut, _)) => text[..cut].to_string(),
        None => text,
    };
    let mut h = Sha256::new();
    h.update(kind.as_bytes());
    h.update([0u8]);
    h.update(text.as_bytes());
    out.push(Block {
        hash: h.finalize().into(),
        kind,
        role: role.to_string(),
        ordinal,
        text,
        chars,
    });
}

/// Serialized size of a JSON value, as a stand-in for its token weight. Used only
/// for the *relative* split between context components; the exact total comes from
/// the API and is apportioned across these shares.
fn json_chars(v: &Value) -> i64 {
    match v {
        Value::String(s) => s.chars().count() as i64,
        other => serde_json::to_string(other).map(|s| s.len() as i64).unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_codex_exec_requests_are_attributed_to_the_cli() {
        assert_eq!(client_of("codex_exec/0.162.0-alpha.17.2 (Mac OS 26.6.2; arm64) unknown (codex_exec; 0.162.0-alpha.17.2)"), Some("codex-cli"));
        assert_eq!(client_of("codex_cli_rs/0.162.0"), Some("codex-cli"));
        assert_eq!(client_of("codex_vscode/0.162.0"), Some("codex-vscode"));
        assert_eq!(client_of("some_other_codex_exec/0.162.0"), None);
    }
    use serde_json::json;

    fn env_of(v: Value) -> (Option<String>, Option<String>) {
        environment_of(&v)
    }

    /// Both places Claude Code puts its environment, in the shapes it writes them.
    #[test]
    fn the_request_says_where_the_session_runs() {
        let env = "# Environment\nYou have been invoked in the following environment: \n \
                   - Primary working directory: /Users/u/code/project\n - Is a git repository: true\n";
        let git = "gitStatus: ...\n\nCurrent branch: feature/x\n\nMain branch (you will usually use this for PRs): main";
        let in_system = json!({ "system": [{ "type": "text", "text": env }, { "type": "text", "text": git }], "messages": [] });
        assert_eq!(
            env_of(in_system),
            (Some("/Users/u/code/project".into()), Some("feature/x".into()))
        );
        let in_first_message = json!({
            "system": "You are Claude Code.",
            "messages": [{ "role": "user", "content": [
                { "type": "text", "text": format!("<system-reminder>{env}</system-reminder>") },
                { "type": "text", "text": git },
                { "type": "text", "text": "fix the tests" },
            ]}],
        });
        assert_eq!(
            env_of(in_first_message),
            (Some("/Users/u/code/project".into()), Some("feature/x".into()))
        );
        let windows = json!({ "system": " - Primary working directory: C:\\Users\\u\\proj\n", "messages": [] });
        assert_eq!(env_of(windows).0.as_deref(), Some("C:\\Users\\u\\proj"));
        let older = json!({ "system": "<env>\nWorking directory: /home/u/p\nIs directory a git repo: Yes\n</env>", "messages": [] });
        assert_eq!(env_of(older).0.as_deref(), Some("/home/u/p"));
    }

    /// Claude Code 2.1.292 sends the environment as a message of its own with the
    /// `system` role, after the first user message; a later one says the directory moved.
    #[test]
    fn a_system_message_in_the_opening_turn_says_where_the_session_runs() {
        let env = "# Environment\nYou have been invoked in the following environment: \n \
                   - Primary working directory: /home/u/code/project\n - Is a git repository: true\n";
        let moved = "# Environment update\n - Primary working directory: /home/u/code/project/sub (was /home/u/code/project)";
        let opening = json!({
            "system": "You are Claude Code.",
            "messages": [
                { "role": "user", "content": [
                    { "type": "text", "text": "<system-reminder>gitStatus: ...\n\nCurrent branch: dev\n</system-reminder>" },
                    { "type": "text", "text": "fix the tests" },
                ]},
                { "role": "system", "content": [{ "type": "text", "text": env }] },
                { "role": "assistant", "content": "On it." },
                { "role": "system", "content": moved },
            ],
        });
        assert_eq!(env_of(opening), (Some("/home/u/code/project".into()), Some("dev".into())));

        let only_later = json!({
            "system": "You are Claude Code.",
            "messages": [
                { "role": "user", "content": "hello" },
                { "role": "assistant", "content": "Hi." },
                { "role": "system", "content": env },
            ],
        });
        assert_eq!(env_of(only_later), (None, None), "after the first reply it is not the opening");
        assert_eq!(environment_in(&[moved]), (None, None), "a move is not where it started");
    }

    /// The same words anywhere else are text about a directory, not this session's.
    #[test]
    fn quoted_or_malformed_lines_are_not_taken() {
        let later = json!({
            "system": "You are Claude Code.",
            "messages": [
                { "role": "user", "content": "hello" },
                { "role": "assistant", "content": "Primary working directory: /assistant/said" },
                { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t", "content":
                    " - Primary working directory: /from/a/tool\nCurrent branch: tool-branch" }] },
            ],
        });
        assert_eq!(env_of(later), (None, None));
        for bad in [
            " - Primary working directory: relative/path",
            " - Primary working directory: ",
            "The Primary working directory: /in/a/sentence",
            "Current branch: two words",
        ] {
            let v = json!({ "system": bad, "messages": [] });
            assert_eq!(env_of(v), (None, None), "{bad:?}");
        }
        let moved = json!({ "system": "Working directory: /new (was /old)", "messages": [] });
        assert_eq!(env_of(moved).0, None, "a note that it moved is not where it started");
    }

    /// One block per content block, carrying where it sat and what kind it was — and
    /// nothing for the blocks that hold no text.
    #[test]
    fn extraction_splits_a_turn_into_its_blocks() {
        let body = br#"{"model":"m","messages":[
            {"role":"user","content":[{"type":"text","text":"why is body_cap 64 KB"}]},
            {"role":"assistant","content":[
              {"type":"thinking","thinking":"the cap predates the index"},
              {"type":"text","text":"because nothing read the bodies back"},
              {"type":"tool_use","name":"Bash","input":{"command":"grep body_cap src/"}}]},
            {"role":"user","content":[
              {"type":"tool_result","content":[{"type":"text","text":"src/config.rs:12"}]},
              {"type":"image","source":{"data":"..."}}]}
          ]}"#;
        let blocks = analyze_request(body).unwrap().blocks;

        let got: Vec<_> = blocks.iter().map(|b| (b.kind, b.ordinal, b.role.as_str())).collect();
        assert_eq!(
            got,
            vec![
                ("text", 0, "user"),
                ("thinking", 1, "assistant"),
                ("text", 1, "assistant"),
                ("tool_use", 1, "assistant"),
                ("tool_result", 2, "user"),
            ],
            "the image carries no text and should not be indexed"
        );
        // A tool call is searchable by its arguments, not just its name.
        assert!(blocks[3].text.contains("grep body_cap src/"));
        assert_eq!(blocks[4].text, "src/config.rs:12", "a tool result flattens to its text");
    }

    /// The system prompt is searchable, sits ahead of the conversation, and costs one
    /// block however many requests repeat it.
    #[test]
    fn the_system_prompt_is_indexed_ahead_of_the_turns() {
        let body = br#"{"messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}],
            "system":[{"type":"text","text":"You are a Claude agent."},
                      {"type":"text","text":"Always read CLAUDE.md first."}],
            "tools":[{"name":"Bash","description":"runs a command"}]}"#;
        let blocks = analyze_request(body).unwrap().blocks;
        let got: Vec<_> = blocks.iter().map(|b| (b.kind, b.ordinal)).collect();
        assert_eq!(got, vec![("text", 0), ("system", -1), ("system", -2)]);
        assert!(
            !blocks.iter().any(|b| b.text.contains("runs a command")),
            "tool definitions are client boilerplate and are not indexed"
        );
        // Two requests carrying the same system prompt are the same block.
        let again = analyze_request(body).unwrap().blocks;
        assert_eq!(blocks[1].hash, again[1].hash);
    }

    /// The same text is the same block wherever it turns up, which is what lets a
    /// replayed conversation cost nothing the second time. Different kinds are not.
    #[test]
    fn identical_text_hashes_identically() {
        let one = analyze_request(
            br#"{"messages":[{"role":"user","content":[{"type":"text","text":"the kestrel"}]}]}"#,
        )
        .unwrap()
        .blocks;
        let two = analyze_request(
            br#"{"messages":[
                 {"role":"assistant","content":[{"type":"text","text":"elsewhere"}]},
                 {"role":"user","content":[{"type":"text","text":"the kestrel"}]}]}"#,
        )
        .unwrap()
        .blocks;
        assert_eq!(one[0].hash, two[1].hash, "same text, same block, different position");
        assert_ne!(one[0].hash, two[0].hash);

        let as_result = analyze_request(
            br#"{"messages":[{"role":"user","content":[
                 {"type":"tool_result","content":"the kestrel"}]}]}"#,
        )
        .unwrap()
        .blocks;
        assert_ne!(
            one[0].hash, as_result[0].hash,
            "a tool result is not the same thing as someone typing it"
        );
    }

    /// The store is full of bodies cut mid-token by `body_cap`. Everything that closed
    /// before the cut is still indexable; only the partial tail is lost.
    #[test]
    fn a_truncated_body_gives_up_the_messages_that_survived_it() {
        let whole = br#"{"model":"m","messages":[
            {"role":"user","content":[{"type":"text","text":"first turn"}]},
            {"role":"assistant","content":[{"type":"text","text":"second turn"}]},
            {"role":"user","content":[{"type":"text","text":"third turn"}]}
          ],"system":[{"type":"text","text":"the standing instructions"}]}"#;

        // Cut in the middle of the third message, as the cap would.
        let cut = &whole[..whole.len() - 90];
        assert!(analyze_request(cut).is_none(), "it must really be broken JSON");

        let texts: Vec<_> =
            blocks_from_stored_body(cut).into_iter().map(|b| b.text).collect();
        assert_eq!(
            texts,
            vec!["first turn", "second turn"],
            "the partial third turn is dropped, and the system prompt past it was never in these bytes"
        );

        // Intact, the same body takes the ordinary path and gives everything in it —
        // the three turns, and the system prompt that sat beyond where the cut fell.
        let texts: Vec<_> =
            blocks_from_stored_body(whole).into_iter().map(|b| b.text).collect();
        assert_eq!(
            texts,
            vec!["first turn", "second turn", "third turn", "the standing instructions"]
        );
    }

    /// Braces inside a tool call are text. Counting them as structure would throw the
    /// depth off and swallow every message after the first one that contains a shell
    /// command — which is most of them.
    #[test]
    fn braces_and_quotes_inside_message_text_do_not_confuse_the_scan() {
        let body = br#"{"messages":[
            {"role":"user","content":[{"type":"text","text":"run awk '{print $1}' and \"quote\" it"}]},
            {"role":"assistant","content":[{"type":"text","text":"done ]}"}]},
            {"role":"user","content":[{"type":"text","text":"tru"#;
        let texts: Vec<_> =
            blocks_from_stored_body(body).into_iter().map(|b| b.text).collect();
        assert_eq!(texts, vec![r#"run awk '{print $1}' and "quote" it"#, "done ]}"]);
    }

    #[test]
    fn scans_usage_across_split_chunks() {
        let mut s = SseScanner::default();
        let stream = concat!(
            "event: message_start\n",
            r#"data: {"type":"message_start","message":{"id":"msg_1","model":"claude-opus-5","usage":{"input_tokens":2,"cache_read_input_tokens":24661,"cache_creation":{"ephemeral_1h_input_tokens":30132,"ephemeral_5m_input_tokens":0},"service_tier":"standard"}}}"#,
            "\n\nevent: content_block_start\n",
            r#"data: {"type":"content_block_start","content_block":{"type":"tool_use","name":"Bash"}}"#,
            "\n\nevent: message_delta\n",
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":175,"output_tokens_details":{"thinking_tokens":40}}}"#,
            "\n\n",
        );
        // Feed one byte at a time: the scanner must survive arbitrary chunking.
        for b in stream.as_bytes() {
            s.feed(&[*b]);
        }
        assert_eq!(s.message_id.as_deref(), Some("msg_1"));
        assert_eq!(s.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(s.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(s.service_tier.as_deref(), Some("standard"));
        assert_eq!(s.tool_calls, vec!["Bash"]);
        assert_eq!(s.usage.input_tokens, 2);
        assert_eq!(s.usage.output_tokens, 175);
        assert_eq!(s.usage.thinking_tokens, 40);
        assert_eq!(s.usage.cache_read_tokens, 24661);
        assert_eq!(s.usage.cache_creation_1h_tokens, 30132);
        assert_eq!(s.usage.prompt_tokens(), 2 + 24661 + 30132);
    }

    #[test]
    fn splits_context_and_names_tool_results() {
        let body = br#"{
          "model":"claude-opus-5","stream":true,
          "system":[{"type":"text","text":"SYSTEM PROMPT"}],
          "tools":[{"name":"Bash","description":"run"},{"name":"Read","description":"read"}],
          "messages":[
            {"role":"user","content":"hello"},
            {"role":"assistant","content":[{"type":"tool_use","id":"tu_1","name":"Bash","input":{}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"tu_1","content":"OUTPUT"}]}
          ]}"#;
        let shape = analyze_request(body).expect("parses");
        assert_eq!(shape.model.as_deref(), Some("claude-opus-5"));
        assert!(shape.streaming);
        assert_eq!(shape.message_count, 3);
        assert_eq!(shape.tools_defined, 2);
        assert!(shape.system_chars > 0);
        assert_eq!(shape.tool_results.len(), 1);
        assert_eq!(shape.tool_results[0].name.as_deref(), Some("Bash"));
        assert_eq!(shape.tool_results[0].tool_use_id, "tu_1");
        // The tool result is attributed to tools, not to plain history.
        assert!(shape.tool_result_chars > 0);
    }
}
