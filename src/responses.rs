//! Native OpenAI Responses requests and response events projected into the router's
//! existing context, usage and search model. The original wire bytes are never changed.

use std::collections::HashMap;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::analyze::{Block, RequestShape, SseScanner, ToolResult, Usage};

/// OpenAI includes cached input in input_tokens and reasoning in output_tokens. Keep
/// those subsets separate without charging or counting them a second time.
pub fn usage(v: &Value) -> Usage {
    let n = |key: &str| v.get(key).and_then(Value::as_i64).unwrap_or(0).max(0);
    let cached = v
        .get("input_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0)
        .min(n("input_tokens"));
    Usage {
        input_tokens: n("input_tokens") - cached,
        cache_read_tokens: cached,
        output_tokens: n("output_tokens"),
        thinking_tokens: v
            .get("output_tokens_details")
            .and_then(|d| d.get("reasoning_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0)
            .min(n("output_tokens")),
        ..Default::default()
    }
}

/// Absorb one native Responses event (or a complete non-streaming response). Returns
/// whether it recognized the event, for SseScanner to share its existing SSE framing.
pub fn absorb(scanner: &mut SseScanner, v: &Value) -> bool {
    let kind = v
        .get("type")
        .or_else(|| v.get("object"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !kind.starts_with("response.") && kind != "response" {
        return false;
    }
    scanner.saw_any_event = true;
    if matches!(
        kind,
        "response.output_item.added" | "response.output_item.done"
    ) {
        // Added is emitted once per item; the terminal response below supplies the
        // authoritative complete list, including calls whose added event was absent.
        if kind.ends_with(".added") {
            if let Some(name) = v.get("item").and_then(tool_name) {
                scanner.tool_calls.push(name.to_owned());
            }
        }
    }
    let response = v
        .get("response")
        .or_else(|| (kind == "response").then_some(v));
    let Some(response) = response else {
        return true;
    };
    if let Some(id) = response.get("id").and_then(Value::as_str) {
        scanner.message_id = Some(id.to_owned());
    }
    if let Some(model) = response.get("model").and_then(Value::as_str) {
        scanner.model = Some(model.to_owned());
    }
    if let Some(tier) = response.get("service_tier").and_then(Value::as_str) {
        scanner.service_tier = Some(tier.to_owned());
    }
    if let Some(u) = response.get("usage").filter(|u| u.is_object()) {
        scanner.usage = usage(u);
    }
    let terminal = matches!(
        kind,
        "response.completed" | "response.failed" | "response.incomplete" | "response"
    );
    if terminal {
        if let Some(output) = response.get("output").and_then(Value::as_array) {
            scanner.tool_calls = output
                .iter()
                .filter_map(tool_name)
                .map(str::to_owned)
                .collect();
        }
        scanner.stop_reason = response
            .get("status")
            .and_then(Value::as_str)
            .map(|status| match status {
                "completed" if !scanner.tool_calls.is_empty() => "tool_use".to_owned(),
                "completed" => "end_turn".to_owned(),
                "incomplete" => match response
                    .get("incomplete_details")
                    .and_then(|d| d.get("reason"))
                    .and_then(Value::as_str)
                {
                    Some("max_output_tokens") => "max_tokens".to_owned(),
                    Some(reason) => reason.to_owned(),
                    None => "incomplete".to_owned(),
                },
                other => other.to_owned(),
            });
    }
    if let Some(error) = response.get("error").filter(|e| !e.is_null()) {
        scanner.error = Some(
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("OpenAI response failed")
                .to_owned(),
        );
    }
    true
}

fn tool_name(item: &Value) -> Option<&str> {
    match item.get("type").and_then(Value::as_str) {
        Some("function_call" | "custom_tool_call") => item.get("name").and_then(Value::as_str),
        _ => None,
    }
}

pub fn analyze_request(body: &[u8]) -> Option<RequestShape> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let mut shape = RequestShape {
        model: v.get("model").and_then(Value::as_str).map(str::to_owned),
        streaming: v.get("stream").and_then(Value::as_bool).unwrap_or(false),
        ..Default::default()
    };
    if let Some(instructions) = v.get("instructions").and_then(Value::as_str) {
        shape.system_chars += instructions.chars().count() as i64;
        block(
            &mut shape.blocks,
            "system",
            "system",
            -1,
            instructions.to_owned(),
        );
    }
    if let Some(tools) = v.get("tools").and_then(Value::as_array) {
        shape.tools_chars = tools.iter().map(chars).sum();
        shape.tools_defined = tools
            .iter()
            .map(|tool| {
                if tool.get("type").and_then(Value::as_str) == Some("namespace") {
                    tool.get("tools")
                        .and_then(Value::as_array)
                        .map_or(0, |tools| tools.len() as i64)
                } else {
                    1
                }
            })
            .sum();
    }
    let input = match v.get("input") {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::String(text)) => vec![serde_json::json!({ "role": "user", "content": text })],
        _ => Vec::new(),
    };
    let names: HashMap<&str, &str> = input
        .iter()
        .filter_map(|item| {
            Some((
                item.get("call_id").and_then(Value::as_str)?,
                tool_name(item)?,
            ))
        })
        .collect();
    let mut opening = true;
    for (ordinal, item) in input.iter().enumerate() {
        let ordinal = ordinal as i64;
        let kind = item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("message");
        let role = item
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("assistant");
        if role == "assistant" || matches!(kind, "function_call" | "custom_tool_call" | "reasoning")
        {
            opening = false;
        }
        match kind {
            "message" => {
                shape.message_count += 1;
                let text = item.get("content").map(text).unwrap_or_default();
                if matches!(role, "system" | "developer") {
                    shape.system_chars += text.chars().count() as i64;
                    block(&mut shape.blocks, "system", role, ordinal, text.clone());
                } else {
                    shape.history_chars += item.get("content").map(chars).unwrap_or(0);
                    block(&mut shape.blocks, "text", role, ordinal, text.clone());
                }
                if opening && shape.cwd.is_none() {
                    shape.cwd = cwd_in(&text);
                }
            }
            "function_call" | "custom_tool_call" => {
                shape.history_chars += chars(item);
                let args = item
                    .get("arguments")
                    .or_else(|| item.get("input"))
                    .map(text)
                    .unwrap_or_default();
                block(
                    &mut shape.blocks,
                    "tool_use",
                    "assistant",
                    ordinal,
                    format!("{} {}", tool_name(item).unwrap_or_default(), args),
                );
            }
            "function_call_output" | "custom_tool_call_output" => {
                let output = item.get("output").map(text).unwrap_or_default();
                let result_chars = output.chars().count() as i64;
                let id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                shape.tool_result_chars += result_chars;
                shape.tool_results.push(ToolResult {
                    tool_use_id: id.to_owned(),
                    name: names.get(id).map(|name| (*name).to_owned()),
                    result_chars,
                    is_error: item
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                });
                block(&mut shape.blocks, "tool_result", "tool", ordinal, output);
            }
            "reasoning" => {
                let summary = item.get("summary").map(text).unwrap_or_default();
                shape.history_chars += summary.chars().count() as i64;
                block(&mut shape.blocks, "thinking", "assistant", ordinal, summary);
            }
            _ => {}
        }
    }
    Some(shape)
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(text)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(_) if v.get("text").and_then(Value::as_str).is_some() => {
            v["text"].as_str().unwrap_or_default().to_owned()
        }
        // Binary images/files and encrypted reasoning are not searchable text.
        Value::Object(_)
            if matches!(
                v.get("type").and_then(Value::as_str),
                Some("input_image" | "input_file")
            ) =>
        {
            String::new()
        }
        Value::Null => String::new(),
        _ => v.to_string(),
    }
}

fn chars(v: &Value) -> i64 {
    match v {
        Value::String(s) => s.chars().count() as i64,
        _ => v.to_string().len() as i64,
    }
}

fn block(blocks: &mut Vec<Block>, kind: &'static str, role: &str, ordinal: i64, text: String) {
    if text.trim().is_empty() {
        return;
    }
    let chars = text.chars().count() as i64;
    let text = text.chars().take(1 << 20).collect::<String>();
    let mut hash = Sha256::new();
    hash.update(kind.as_bytes());
    hash.update([0]);
    hash.update(text.as_bytes());
    blocks.push(Block {
        hash: hash.finalize().into(),
        kind,
        role: role.to_owned(),
        ordinal,
        text,
        chars,
    });
}

fn cwd_in(text: &str) -> Option<String> {
    let context = text
        .split_once("<environment_context>")?
        .1
        .split_once("</environment_context>")?
        .0;
    let cwd = context
        .split_once("<cwd>")?
        .1
        .split_once("</cwd>")?
        .0
        .trim();
    let bytes = cwd.as_bytes();
    let absolute = cwd.starts_with('/')
        || cwd.starts_with("\\\\")
        || (bytes.len() > 2
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'));
    (absolute && cwd.len() <= 4096 && !cwd.chars().any(char::is_control)).then(|| cwd.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn terminal_usage_counts_cached_and_reasoning_tokens_once() {
        let mut scanner = SseScanner::default();
        assert!(absorb(
            &mut scanner,
            &json!({ "type": "response.output_item.added",
            "item": { "type": "function_call", "name": "shell" } })
        ));
        let completed = json!({ "type": "response.completed", "response": {
            "id": "resp_test", "model": "gpt-test", "status": "completed", "service_tier": "default",
            "output": [{ "type": "function_call", "name": "shell" }],
            "usage": { "input_tokens": 100, "input_tokens_details": { "cached_tokens": 60 },
                "output_tokens": 30, "output_tokens_details": { "reasoning_tokens": 20 } }
        } });
        absorb(&mut scanner, &completed);
        assert_eq!(scanner.usage.prompt_tokens(), 100);
        assert_eq!(scanner.usage.input_tokens, 40);
        assert_eq!(scanner.usage.cache_read_tokens, 60);
        assert_eq!(scanner.usage.output_tokens, 30);
        assert_eq!(scanner.usage.thinking_tokens, 20);
        assert_eq!(scanner.tool_calls, ["shell"]);
        assert_eq!(scanner.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(scanner.message_id.as_deref(), Some("resp_test"));
        assert_eq!(scanner.model.as_deref(), Some("gpt-test"));
    }

    #[test]
    fn native_request_context_and_search_use_the_shared_model() {
        let request = json!({ "model": "gpt-test", "stream": true, "instructions": "Develop software",
            "tools": [{ "type": "namespace", "name": "functions", "tools": [{ "type": "function", "name": "shell" }] }],
            "input": [
                { "role": "user", "content": [{ "type": "input_text", "text": "<environment_context><cwd>/work/project</cwd></environment_context>\nFix it" }] },
                { "type": "function_call", "name": "shell", "call_id": "call_1", "arguments": "{\"cmd\":\"ls\"}" },
                { "type": "function_call_output", "call_id": "call_1", "output": "one file" },
                { "type": "reasoning", "summary": [{ "type": "summary_text", "text": "Consider the result" }], "encrypted_content": "unreadable" }
            ] });
        let shape = analyze_request(request.to_string().as_bytes()).unwrap();
        assert_eq!(shape.cwd.as_deref(), Some("/work/project"));
        assert_eq!(shape.model.as_deref(), Some("gpt-test"));
        assert_eq!(shape.tools_defined, 1);
        assert_eq!(shape.tool_results[0].name.as_deref(), Some("shell"));
        assert_eq!(shape.tool_results[0].tool_use_id, "call_1");
        assert_eq!(shape.tool_result_chars, 8);
        assert!(shape
            .blocks
            .iter()
            .any(|b| b.kind == "thinking" && b.text == "Consider the result"));
        assert!(!shape.blocks.iter().any(|b| b.text.contains("unreadable")));
    }

    #[test]
    fn incomplete_and_failed_responses_remain_distinguishable() {
        let mut scanner = SseScanner::default();
        absorb(
            &mut scanner,
            &json!({ "object": "response", "status": "incomplete",
            "incomplete_details": { "reason": "max_output_tokens" }, "output": [] }),
        );
        assert_eq!(scanner.stop_reason.as_deref(), Some("max_tokens"));
        absorb(
            &mut scanner,
            &json!({ "type": "response.failed", "response": {
            "status": "failed", "error": { "message": "provider failure" } } }),
        );
        assert_eq!(scanner.error.as_deref(), Some("provider failure"));
    }
}
