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
            self.absorb_message(&v);
            self.absorb(&v);
        }
    }

    fn absorb(&mut self, v: &Value) {
        self.saw_any_event = true;
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
    pub streaming: bool,
    pub message_count: i64,
    pub system_chars: i64,
    pub tools_chars: i64,
    pub tools_defined: i64,
    /// History excluding tool results, which are counted separately.
    pub history_chars: i64,
    pub tool_result_chars: i64,
    pub tool_results: Vec<ToolResult>,
}

pub fn analyze_request(body: &[u8]) -> Option<RequestShape> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let mut shape = RequestShape {
        model: v.get("model").and_then(Value::as_str).map(str::to_owned),
        streaming: v.get("stream").and_then(Value::as_bool).unwrap_or(false),
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
    }
    Some(shape)
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
