//! Portable Chat Completions ingress, normalized through the Responses adapters.

use anyhow::{bail, Context, Result};
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderValue, Request, Response, StatusCode};
use axum::response::IntoResponse;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

const MAX_BODY: usize = 64 * 1024 * 1024;

/// Capture the client endpoint and byte count while the provider pipeline receives
/// a canonical Responses request. This never contains a credential.
#[derive(Clone)]
pub(crate) struct Original {
    pub path: String,
    pub request_bytes: i64,
}

fn unsupported(feature: &str) -> anyhow::Error {
    anyhow::anyhow!("Chat Completions {feature} is unavailable in portable routing")
}

fn text(value: &Value, field: &str) -> Result<String> {
    value[field]
        .as_str()
        .map(str::to_owned)
        .with_context(|| format!("{field} must be text"))
}

fn content(value: &Value, role: &str) -> Result<Vec<Value>> {
    if let Some(text) = value.as_str() {
        return Ok(vec![
            json!({"type":if role=="assistant" {"output_text"} else {"input_text"},"text":text}),
        ]);
    }
    let parts = value
        .as_array()
        .context("message content must be text or a content array")?;
    parts.iter().map(|part| match part["type"].as_str() {
        Some("text") => Ok(json!({"type":if role=="assistant" {"output_text"} else {"input_text"},"text":text(part,"text")?})),
        Some("image_url") if role=="user" => {
            let mut image = json!({"type":"input_image","image_url":text(&part["image_url"],"url")?});
            if let Some(detail) = part["image_url"].get("detail").filter(|v|!v.is_null()) {
                if detail!="auto" { return Err(unsupported("image detail other than auto")); }
                image["detail"] = detail.clone();
            }
            Ok(image)
        },
        _ => Err(unsupported("message content type")),
    }).collect()
}

fn request(value: &Value) -> Result<Value> {
    let object = value
        .as_object()
        .context("Chat Completions request must be an object")?;
    let allowed: HashSet<&str> = [
        "model",
        "messages",
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "stream",
        "stream_options",
        "max_tokens",
        "max_completion_tokens",
        "temperature",
        "top_p",
        "response_format",
        "reasoning_effort",
        "metadata",
        "store",
        "n",
        "stop",
        "frequency_penalty",
        "presence_penalty",
        "logprobs",
        "top_logprobs",
        "logit_bias",
        "modalities",
        "audio",
    ]
    .into_iter()
    .collect();
    for (key, v) in object {
        if !allowed.contains(key.as_str()) && !v.is_null() {
            return Err(unsupported(&format!("option {key}")));
        }
    }
    if value.get("n").is_some_and(|v| !v.is_null() && v != 1) {
        return Err(unsupported("multiple choices (n must be 1)"));
    }
    for key in ["frequency_penalty", "presence_penalty", "top_logprobs"] {
        if value
            .get(key)
            .is_some_and(|v| !v.is_null() && v.as_f64() != Some(0.0))
        {
            return Err(unsupported(key));
        }
    }
    for key in ["logprobs", "store"] {
        if value.get(key).is_some_and(|v| !v.is_null() && v != false) {
            return Err(unsupported(key));
        }
    }
    if value
        .get("stop")
        .is_some_and(|v| !v.is_null() && !v.as_array().is_some_and(Vec::is_empty))
    {
        return Err(unsupported("stop sequences"));
    }
    if value
        .get("logit_bias")
        .is_some_and(|v| !v.is_null() && !v.as_object().is_some_and(serde_json::Map::is_empty))
    {
        return Err(unsupported("logit_bias"));
    }
    if value.get("audio").is_some_and(|v| !v.is_null())
        || value
            .get("modalities")
            .is_some_and(|v| !v.is_null() && v != &json!(["text"]))
    {
        return Err(unsupported("audio/modalities"));
    }
    if let Some(options) = value.get("stream_options").filter(|v| !v.is_null()) {
        let options = options
            .as_object()
            .context("stream_options must be an object")?;
        for (key, v) in options {
            if key != "include_usage" && !v.is_null() {
                return Err(unsupported(&format!("stream_options.{key}")));
            }
            if key == "include_usage" && !v.is_boolean() {
                bail!("stream_options.include_usage must be boolean");
            }
        }
    }
    if value
        .get("stream")
        .is_some_and(|v| !v.is_null() && !v.is_boolean())
    {
        bail!("stream must be boolean");
    }
    let model = text(value, "model")?;
    if model.trim().is_empty() {
        bail!("a model is required");
    }
    let messages = value["messages"]
        .as_array()
        .context("messages must be an array")?;
    if messages.is_empty() {
        bail!("messages must not be empty");
    }
    let mut input = Vec::new();
    let mut conversation = false;
    let mut called = HashSet::new();
    let mut answered = HashSet::new();
    for message in messages {
        let fields = message
            .as_object()
            .context("messages must contain objects")?;
        for (key, v) in fields {
            if ![
                "role",
                "content",
                "tool_calls",
                "tool_call_id",
                "refusal",
                "reasoning_content",
            ]
            .contains(&key.as_str())
                && !v.is_null()
            {
                return Err(unsupported(&format!("message {key}")));
            }
        }
        let role = text(message, "role")?;
        if !["system", "developer", "user", "assistant", "tool"].contains(&role.as_str()) {
            return Err(unsupported("message role"));
        }
        for key in ["function_call", "audio", "name", "reasoning_details"] {
            if message.get(key).is_some_and(|v| !v.is_null()) {
                return Err(unsupported(&format!("message {key}")));
            }
        }
        if role == "system" || role == "developer" {
            if conversation {
                return Err(unsupported(
                    "system/developer updates after conversation history",
                ));
            }
        } else {
            conversation = true;
        }
        if role == "tool" {
            let call_id = text(message, "tool_call_id")?;
            if !called.contains(&call_id) || !answered.insert(call_id.clone()) {
                bail!("tool result requires one preceding function call with its tool_call_id");
            }
            let output = if message["content"].is_string() {
                message["content"].clone()
            } else {
                json!(content(&message["content"], "tool")?)
            };
            input.push(json!({"type":"function_call_output","call_id":call_id,"output":output}));
            continue;
        }
        if let Some(reasoning) = message.get("reasoning_content").filter(|v| !v.is_null()) {
            if role != "assistant" {
                return Err(unsupported("reasoning_content on non-assistant messages"));
            }
            let reasoning = reasoning
                .as_str()
                .context("reasoning_content must be text")?;
            input.push(
                json!({"type":"reasoning","summary":[{"type":"summary_text","text":reasoning}]}),
            );
        }
        let mut blocks = if message["content"].is_null() {
            if role != "assistant" {
                bail!("message content is required");
            }
            Vec::new()
        } else {
            content(&message["content"], &role)?
        };
        if let Some(refusal) = message.get("refusal").filter(|v| !v.is_null()) {
            if role != "assistant" {
                return Err(unsupported("refusal on non-assistant messages"));
            }
            blocks.push(json!({"type":"refusal","refusal":refusal.as_str().context("refusal must be text")?}));
        }
        if !blocks.is_empty() {
            input.push(json!({"type":"message","role":role,"content":blocks}));
        }
        if let Some(calls) = message.get("tool_calls").filter(|v| !v.is_null()) {
            if role != "assistant" {
                return Err(unsupported("tool calls on non-assistant messages"));
            }
            for call in calls.as_array().context("tool_calls must be an array")? {
                if call["type"] != "function" {
                    return Err(unsupported("non-function tool calls"));
                }
                let arguments = text(&call["function"], "arguments")?;
                let parsed: Value = serde_json::from_str(&arguments)
                    .context("tool-call arguments must contain JSON")?;
                if !parsed.is_object() {
                    bail!("portable function arguments must be a JSON object");
                }
                let call_id = text(call, "id")?;
                if call_id.is_empty() || !called.insert(call_id.clone()) {
                    bail!("function call IDs must be nonempty and unique");
                }
                input.push(json!({"type":"function_call","call_id":call_id,"name":text(&call["function"],"name")?,"arguments":arguments}));
            }
        }
    }
    let mut out = json!({"model":model,"input":input,"stream":value["stream"].as_bool().unwrap_or(false),"store":false});
    for key in ["temperature", "top_p", "metadata", "parallel_tool_calls"] {
        if let Some(v) = value.get(key).filter(|v| !v.is_null()) {
            if ["temperature", "top_p"].contains(&key) && !v.is_number() {
                bail!("{key} must be a number");
            }
            if key == "parallel_tool_calls" && !v.is_boolean() {
                bail!("parallel_tool_calls must be boolean");
            }
            out[key] = v.clone();
        }
    }
    if value.get("max_tokens").is_some_and(|v| !v.is_null())
        && value
            .get("max_completion_tokens")
            .is_some_and(|v| !v.is_null())
    {
        bail!("choose max_tokens or max_completion_tokens");
    }
    if let Some(max) = value
        .get("max_completion_tokens")
        .filter(|v| !v.is_null())
        .or_else(|| value.get("max_tokens").filter(|v| !v.is_null()))
    {
        if max.as_u64().is_none_or(|n| n == 0) {
            bail!("max tokens must be a positive integer");
        }
        out["max_output_tokens"] = max.clone();
    }
    if let Some(tools) = value.get("tools").filter(|v| !v.is_null()) {
        let mut converted = Vec::new();
        let mut names = HashSet::new();
        for tool in tools.as_array().context("tools must be an array")? {
            if tool["type"] != "function" {
                return Err(unsupported("non-function tools"));
            }
            let function = tool["function"]
                .as_object()
                .context("function tool must be an object")?;
            let name = text(&tool["function"], "name")?;
            if name.is_empty() || !names.insert(name) {
                bail!("function tool names must be nonempty and unique");
            }
            let mut tool = Value::Object(function.clone());
            tool["type"] = json!("function");
            if tool["strict"].is_null() {
                tool["strict"] = json!(false);
            }
            converted.push(tool);
        }
        out["tools"] = json!(converted);
    }
    if let Some(choice) = value.get("tool_choice").filter(|v| !v.is_null()) {
        out["tool_choice"] = if choice.is_string() {
            if !["auto", "none", "required"].contains(&choice.as_str().unwrap()) {
                return Err(unsupported("tool_choice"));
            }
            choice.clone()
        } else if choice["type"] == "function" {
            json!({"type":"function","name":text(&choice["function"],"name")?})
        } else {
            return Err(unsupported("tool_choice"));
        };
    }
    if let Some(effort) = value.get("reasoning_effort").filter(|v| !v.is_null()) {
        if !effort.as_str().is_some_and(|v| {
            ["none", "minimal", "low", "medium", "high", "xhigh", "max"].contains(&v)
        }) {
            bail!("reasoning_effort is invalid");
        }
        out["reasoning"] = json!({"effort":effort});
    }
    if let Some(format) = value.get("response_format").filter(|v| !v.is_null()) {
        out["text"]["format"] = match format["type"].as_str() {
            Some("text") => json!({"type":"text"}),
            Some("json_object") => json!({"type":"json_object"}),
            Some("json_schema") => {
                let mut schema = format["json_schema"]
                    .as_object()
                    .context("json_schema must be an object")?
                    .clone();
                if !schema.get("schema").is_some_and(Value::is_object) {
                    bail!("json_schema.schema must be an object");
                }
                text(&format["json_schema"], "name")?;
                schema.insert("type".into(), json!("json_schema"));
                Value::Object(schema)
            }
            _ => return Err(unsupported("response_format")),
        };
    }
    Ok(out)
}

fn usage(response: &Value) -> Value {
    let u = &response["usage"];
    let input = u["input_tokens"].as_u64().unwrap_or(0);
    let output = u["output_tokens"].as_u64().unwrap_or(0);
    json!({"prompt_tokens":input,"completion_tokens":output,"total_tokens":input.saturating_add(output),
        "prompt_tokens_details":{"cached_tokens":u["input_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0).min(input)},
        "completion_tokens_details":{"reasoning_tokens":u["output_tokens_details"]["reasoning_tokens"].as_u64().unwrap_or(0).min(output)}})
}

fn finish_reason(response: &Value, tools: bool) -> &'static str {
    if response["status"] == "incomplete" {
        return if response["incomplete_details"]["reason"] == "content_filter" {
            "content_filter"
        } else {
            "length"
        };
    }
    if tools {
        "tool_calls"
    } else {
        "stop"
    }
}

fn message(response: &Value) -> Result<Value> {
    let mut text = String::new();
    let mut refusal = String::new();
    let mut reasoning = String::new();
    let mut tools = Vec::new();
    for item in response["output"]
        .as_array()
        .context("upstream Responses output must be an array")?
    {
        match item["type"].as_str() {
            Some("message") => for part in item["content"].as_array().context("response message content must be an array")? {
                match part["type"].as_str() {
                    Some("output_text") => text.push_str(&self::text(part,"text")?),
                    Some("refusal") => refusal.push_str(&self::text(part,"refusal")?),
                    _ => return Err(unsupported("upstream output content")),
                }
            },
            Some("function_call") => tools.push(json!({"id":self::text(item,"call_id")?,"type":"function",
                "function":{"name":self::text(item,"name")?,"arguments":self::text(item,"arguments")?}})),
            Some("reasoning") => if let Some(summary) = item["summary"].as_array() {
                for part in summary { if let Some(text) = part["text"].as_str() { reasoning.push_str(text); } }
            },
            _ => return Err(unsupported("upstream output item")),
        }
    }
    let mut out =
        json!({"role":"assistant","content":if text.is_empty() {Value::Null} else {json!(text)}});
    if !tools.is_empty() {
        out["tool_calls"] = json!(tools);
    }
    if !refusal.is_empty() {
        out["refusal"] = json!(refusal);
    }
    if !reasoning.is_empty() {
        out["reasoning_content"] = json!(reasoning);
    }
    Ok(out)
}

fn response(response: &Value, model: &str) -> Result<Value> {
    if response.get("error").is_some_and(|v| !v.is_null()) {
        return Ok(json!({"error":response["error"]}));
    }
    let message = message(response)?;
    Ok(
        json!({"id":response["id"].as_str().unwrap_or("chatcmpl-router"),"object":"chat.completion",
        "created":response["created_at"].as_i64().unwrap_or_else(||crate::db::now_ms()/1000),"model":model,
        "choices":[{"index":0,"message":message,"finish_reason":finish_reason(response,message.get("tool_calls").is_some()),"logprobs":null}],"usage":usage(response)}),
    )
}

#[derive(Default)]
struct ToolStream {
    index: usize,
    id: String,
    name: String,
    arguments: String,
}

struct ChatStream {
    pending: Vec<u8>,
    id: String,
    model: String,
    created: i64,
    started: bool,
    done: bool,
    include_usage: bool,
    text: BTreeMap<usize, String>,
    reasoning: BTreeMap<usize, String>,
    refusal: BTreeMap<usize, String>,
    tools: BTreeMap<usize, ToolStream>,
    item_indices: BTreeMap<String, usize>,
}

impl ChatStream {
    fn new(model: String, include_usage: bool) -> Self {
        Self {
            pending: Vec::new(),
            id: "chatcmpl-router".into(),
            model,
            created: crate::db::now_ms() / 1000,
            started: false,
            done: false,
            include_usage,
            text: BTreeMap::new(),
            reasoning: BTreeMap::new(),
            refusal: BTreeMap::new(),
            tools: BTreeMap::new(),
            item_indices: BTreeMap::new(),
        }
    }

    fn emit(&self, delta: Value, finish: Option<&str>, out: &mut Vec<u8>) {
        let mut value = json!({"id":self.id,"object":"chat.completion.chunk","created":self.created,"model":self.model,
            "choices":[{"index":0,"delta":delta,"finish_reason":finish,"logprobs":null}]});
        if self.include_usage {
            value["usage"] = Value::Null;
        }
        out.extend(format!("data: {value}\n\n").bytes());
    }

    fn start(&mut self, out: &mut Vec<u8>) {
        if !self.started {
            self.started = true;
            self.emit(json!({"role":"assistant","content":""}), None, out);
        }
    }

    fn suffix(complete: &str, sent: &str) -> Result<String> {
        complete
            .strip_prefix(sent)
            .map(str::to_owned)
            .context("upstream terminal content disagrees with its streamed deltas")
    }

    fn item(&mut self, index: usize, item: &Value, out: &mut Vec<u8>) -> Result<()> {
        self.start(out);
        if let Some(id) = item["id"].as_str() {
            self.item_indices.insert(id.into(), index);
        }
        if let Some(id) = item["call_id"].as_str() {
            self.item_indices.insert(format!("call:{id}"), index);
        }
        match item["type"].as_str() {
            Some("message") => {
                let mut content = String::new();
                let mut refusal = String::new();
                if let Some(parts) = item["content"].as_array() {
                    for part in parts {
                        match part["type"].as_str() {
                            Some("output_text") => content.push_str(&text(part, "text")?),
                            Some("refusal") => refusal.push_str(&text(part, "refusal")?),
                            _ => return Err(unsupported("upstream output content")),
                        }
                    }
                }
                for (field, complete) in [("content", content), ("refusal", refusal)] {
                    let map = if field == "content" {
                        &mut self.text
                    } else {
                        &mut self.refusal
                    };
                    let sent = map.entry(index).or_default();
                    let suffix = Self::suffix(&complete, sent)?;
                    *sent = complete;
                    if !suffix.is_empty() {
                        self.emit(json!({field:suffix}), None, out);
                    }
                }
            }
            Some("function_call") => {
                let id = text(item, "call_id")?;
                let name = text(item, "name")?;
                if !self.tools.contains_key(&index) {
                    let tool_index = self.tools.len();
                    self.tools.insert(
                        index,
                        ToolStream {
                            index: tool_index,
                            id: id.clone(),
                            name: name.clone(),
                            arguments: String::new(),
                        },
                    );
                    self.emit(json!({"tool_calls":[{"index":tool_index,"id":id,"type":"function","function":{"name":name,"arguments":""}}]}),None,out);
                }
                let tool = self.tools.get_mut(&index).unwrap();
                if tool.id != id || tool.name != name {
                    bail!("upstream changed a streamed tool's identity");
                }
                let arguments = item["arguments"].as_str().unwrap_or_default();
                let suffix = Self::suffix(arguments, &tool.arguments)?;
                let tool_index = tool.index;
                tool.arguments = arguments.into();
                if !suffix.is_empty() {
                    self.emit(json!({"tool_calls":[{"index":tool_index,"function":{"arguments":suffix}}]}),None,out);
                }
            }
            Some("reasoning") => {
                let complete: String = item["summary"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p["text"].as_str())
                    .collect();
                let sent = self.reasoning.entry(index).or_default();
                let suffix = Self::suffix(&complete, sent)?;
                *sent = complete;
                if !suffix.is_empty() {
                    self.emit(json!({"reasoning_content":suffix}), None, out);
                }
            }
            _ => return Err(unsupported("upstream output item")),
        }
        Ok(())
    }

    fn event(&mut self, value: &Value, out: &mut Vec<u8>) -> Result<()> {
        if self.done {
            return Ok(());
        }
        if let Some(response) = value.get("response") {
            if !self.started {
                if let Some(id) = response["id"].as_str() {
                    self.id = id.into();
                }
                if let Some(at) = response["created_at"].as_i64() {
                    self.created = at;
                }
            }
        }
        let index = value["output_index"]
            .as_u64()
            .map(|n| n as usize)
            .or_else(|| {
                value["item_id"]
                    .as_str()
                    .and_then(|id| self.item_indices.get(id).copied())
            })
            .unwrap_or(0);
        match value["type"].as_str() {
            Some("response.created") => self.start(out),
            Some("response.output_item.added" | "response.output_item.done") => {
                self.item(index, &value["item"], out)?
            }
            Some(
                "response.output_text.delta"
                | "response.refusal.delta"
                | "response.reasoning_summary_text.delta"
                | "response.reasoning_text.delta",
            ) => {
                self.start(out);
                let delta = text(value, "delta")?;
                let (field, map) = match value["type"].as_str().unwrap() {
                    "response.output_text.delta" => ("content", &mut self.text),
                    "response.refusal.delta" => ("refusal", &mut self.refusal),
                    _ => ("reasoning_content", &mut self.reasoning),
                };
                map.entry(index).or_default().push_str(&delta);
                self.emit(json!({field:delta}), None, out);
            }
            Some("response.function_call_arguments.delta") => {
                let delta = text(value, "delta")?;
                let tool = self
                    .tools
                    .get_mut(&index)
                    .context("function delta arrived without its tool definition")?;
                tool.arguments.push_str(&delta);
                let tool_index = tool.index;
                self.emit(
                    json!({"tool_calls":[{"index":tool_index,"function":{"arguments":delta}}]}),
                    None,
                    out,
                );
            }
            Some("response.completed" | "response.incomplete") => {
                self.complete(&value["response"], out)?
            }
            Some("response.failed" | "error") => {
                let error = value
                    .get("error")
                    .or_else(|| value["response"].get("error"))
                    .cloned()
                    .unwrap_or_else(
                        || json!({"type":"upstream_error","message":"upstream response failed"}),
                    );
                out.extend(format!("data: {}\n\ndata: [DONE]\n\n", json!({"error":error})).bytes());
                self.done = true;
            }
            _ => {}
        }
        Ok(())
    }

    fn complete(&mut self, response: &Value, out: &mut Vec<u8>) -> Result<()> {
        self.start(out);
        for (position, item) in response["output"]
            .as_array()
            .context("upstream Responses output must be an array")?
            .iter()
            .enumerate()
        {
            let index = item["id"]
                .as_str()
                .and_then(|id| self.item_indices.get(id).copied())
                .or_else(|| {
                    item["call_id"]
                        .as_str()
                        .and_then(|id| self.item_indices.get(&format!("call:{id}")).copied())
                })
                .unwrap_or(position);
            self.item(index, item, out)?;
        }
        self.emit(
            json!({}),
            Some(finish_reason(response, !self.tools.is_empty())),
            out,
        );
        if self.include_usage {
            out.extend(format!("data: {}\n\n",json!({"id":self.id,"object":"chat.completion.chunk","created":self.created,"model":self.model,"choices":[],"usage":usage(response)})).bytes());
        }
        out.extend(b"data: [DONE]\n\n");
        self.done = true;
        Ok(())
    }

    fn push(&mut self, bytes: &[u8]) -> Result<Vec<u8>> {
        self.pending.extend(bytes);
        if self.pending.len() > MAX_BODY {
            bail!("upstream SSE frame exceeds the body limit");
        }
        let mut out = Vec::new();
        loop {
            let boundary = [
                self.pending
                    .windows(2)
                    .position(|w| w == b"\n\n")
                    .map(|p| (p, 2)),
                self.pending
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|p| (p, 4)),
            ]
            .into_iter()
            .flatten()
            .min_by_key(|(p, _)| *p);
            let Some((end, width)) = boundary else {
                break;
            };
            let frame: Vec<u8> = self.pending.drain(..end + width).collect();
            let frame = std::str::from_utf8(&frame).context("upstream SSE frame is not UTF-8")?;
            let data = frame
                .lines()
                .filter_map(|line| {
                    line.strip_prefix("data:")
                        .map(|s| s.strip_prefix(' ').unwrap_or(s))
                })
                .collect::<Vec<_>>()
                .join("\n");
            if !data.is_empty() && data != "[DONE]" {
                self.event(
                    &serde_json::from_str(&data).context("invalid upstream SSE JSON")?,
                    &mut out,
                )?;
            }
        }
        Ok(out)
    }

    fn finish(&self) -> Result<()> {
        if !self.done {
            bail!("upstream Responses stream ended before a terminal event");
        }
        Ok(())
    }
}

fn failure(status: StatusCode, message: &str) -> axum::response::Response {
    (status,axum::Json(json!({"error":{"type":if status==StatusCode::BAD_REQUEST {"invalid_request_error"} else {"upstream_error"},"message":format!("agent-router: {message}")}}))).into_response()
}

pub async fn handle(
    State(state): State<Arc<crate::proxy::AppState>>,
    req: Request<Body>,
) -> axum::response::Response {
    let (mut parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_BODY).await {
        Ok(bytes) => bytes,
        Err(_) => return failure(StatusCode::BAD_REQUEST, "invalid or oversized request body"),
    };
    let original: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return failure(StatusCode::BAD_REQUEST, "invalid request JSON"),
    };
    let converted = match request(&original) {
        Ok(value) => value,
        Err(error) => return failure(StatusCode::BAD_REQUEST, &error.to_string()),
    };
    let model = converted["model"].as_str().unwrap().to_owned();
    let streaming = converted["stream"].as_bool().unwrap();
    let include_usage = original["stream_options"]["include_usage"]
        .as_bool()
        .unwrap_or(false);
    parts.extensions.insert(Original {
        path: parts
            .uri
            .path_and_query()
            .map(|p| p.as_str())
            .unwrap_or("/v1/chat/completions")
            .into(),
        request_bytes: bytes.len() as i64,
    });
    parts.uri = "/v1/responses".parse().unwrap();
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.insert(
        header::ACCEPT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    let upstream = crate::proxy::handle(
        State(state),
        Request::from_parts(parts, Body::from(converted.to_string())),
    )
    .await;
    let (mut parts, body) = upstream.into_parts();
    let encoding = parts
        .headers
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let sse = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("event-stream"));
    for header in [
        header::CONTENT_LENGTH,
        header::CONTENT_ENCODING,
        header::ETAG,
    ] {
        parts.headers.remove(header);
    }
    if !parts.status.is_success() || !streaming || !sse {
        let bytes = match axum::body::to_bytes(body, MAX_BODY).await {
            Ok(bytes) => bytes,
            Err(_) => return failure(StatusCode::BAD_GATEWAY, "could not read upstream reply"),
        };
        let mut decoder = crate::capture::Decoder::for_encoding(encoding.as_deref());
        let mut plain = decoder.feed(&bytes);
        plain.extend(decoder.finish());
        if !parts.status.is_success() {
            return Response::from_parts(parts, Body::from(plain));
        }
        let value: Value = match serde_json::from_slice(&plain) {
            Ok(value) => value,
            Err(_) => return failure(StatusCode::BAD_GATEWAY, "upstream reply was not JSON"),
        };
        if value["status"] == "failed" || value.get("error").is_some_and(|v| !v.is_null()) {
            return (StatusCode::BAD_GATEWAY,axum::Json(json!({"error":value.get("error").cloned().unwrap_or_else(||json!({"type":"upstream_error","message":"upstream response failed"}))}))).into_response();
        }
        if streaming {
            let mut stream = ChatStream::new(model, include_usage);
            let mut output = Vec::new();
            if let Some(id) = value["id"].as_str() {
                stream.id = id.into();
            }
            if let Err(error) = stream.complete(&value, &mut output) {
                return failure(StatusCode::BAD_GATEWAY, &error.to_string());
            }
            parts.headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream"),
            );
            Response::from_parts(parts, Body::from(output))
        } else {
            match response(&value, &model) {
                Ok(value) => {
                    parts.headers.insert(
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("application/json"),
                    );
                    Response::from_parts(parts, Body::from(value.to_string()))
                }
                Err(error) => failure(StatusCode::BAD_GATEWAY, &error.to_string()),
            }
        }
    } else {
        parts.headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        let output: std::pin::Pin<
            Box<dyn futures_util::Stream<Item = std::io::Result<bytes::Bytes>> + Send>,
        > = Box::pin(async_stream::try_stream! {
            let mut stream = ChatStream::new(model,include_usage);
            let mut decoder = crate::capture::Decoder::for_encoding(encoding.as_deref());
            let mut incoming = body.into_data_stream();
            while let Some(chunk)=incoming.next().await {
                let chunk=chunk.map_err(|error|std::io::Error::other(error.to_string()))?;
                let output=stream.push(&decoder.feed(&chunk)).map_err(|error|std::io::Error::other(error.to_string()))?;
                if !output.is_empty(){yield bytes::Bytes::from(output);}
            }
            let output=stream.push(&decoder.finish()).map_err(|error|std::io::Error::other(error.to_string()))?;
            if !output.is_empty(){yield bytes::Bytes::from(output);}
            stream.finish().map_err(|error|std::io::Error::other(error.to_string()))?;
        });
        Response::from_parts(parts, Body::from_stream(output))
    }
}

#[cfg(test)]
mod tests {
    use crate::accounts::Provider;
    use crate::api_refusals::Lab;
    use crate::proxy::{Active, AppState};
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Request, Response, StatusCode};
    use axum::routing::post;
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use tower::ServiceExt;

    async fn ask(
        app: &axum::Router,
        path: &str,
        token: Option<&str>,
        body: Value,
    ) -> axum::response::Response {
        let mut req = Request::post(path)
            .header("host", "localhost")
            .header("content-type", "application/json")
            .header("thread-id", "chat-session");
        if let Some(token) = token {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        let mut req = req.body(Body::from(body.to_string())).unwrap();
        req.extensions_mut()
            .insert(ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                50_000,
            ))));
        app.clone().oneshot(req).await.unwrap()
    }

    fn upstream(model: &str, provider: Provider, stream: bool) -> String {
        let responses = json!({"id":"resp_chat","object":"response","model":model,"status":"completed","output":[
            {"id":"msg_chat","type":"message","role":"assistant","content":[{"type":"output_text","text":"Chat ✓","annotations":[]}]},
            {"id":"fc_chat","type":"function_call","call_id":"call_chat","name":"read","arguments":"{\"path\":\"next\"}","status":"completed"}
        ],"usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":60},"output_tokens":13,"output_tokens_details":{"reasoning_tokens":3},"total_tokens":113}});
        let input = if provider == Provider::Kimi { 100 } else { 40 };
        let messages = json!({"id":"msg_chat","type":"message","role":"assistant","model":model,
            "content":[{"type":"text","text":"Chat ✓"},{"type":"tool_use","id":"call_chat","name":"read","input":{"path":"next"}}],
            "stop_reason":"tool_use","usage":{"input_tokens":input,"cache_read_input_tokens":60,"output_tokens":13}});
        if !stream {
            return if provider == Provider::OpenAI {
                responses
            } else {
                messages
            }
            .to_string();
        }
        let frames = if provider == Provider::OpenAI {
            vec![
                json!({"type":"response.created","response":{"id":"resp_chat","model":model,"status":"in_progress","output":[]}}),
                json!({"type":"response.output_item.added","output_index":0,"item":{"id":"msg_chat","type":"message","role":"assistant","content":[]}}),
                json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"Chat ✓"}),
                json!({"type":"response.output_item.done","output_index":0,"item":responses["output"][0]}),
                json!({"type":"response.output_item.added","output_index":1,"item":{"id":"fc_chat","type":"function_call","call_id":"call_chat","name":"read","arguments":""}}),
                json!({"type":"response.function_call_arguments.delta","output_index":1,"item_id":"fc_chat","delta":"{\"path\":"}),
                json!({"type":"response.function_call_arguments.delta","output_index":1,"item_id":"fc_chat","delta":"\"next\"}"}),
                json!({"type":"response.output_item.done","output_index":1,"item":responses["output"][1]}),
                json!({"type":"response.completed","response":responses}),
            ]
        } else {
            vec![
                json!({"type":"message_start","message":{"id":"msg_chat","model":model,"content":[],"usage":{"input_tokens":input,"cache_read_input_tokens":60,"output_tokens":0}}}),
                json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Chat ✓"}}),
                json!({"type":"content_block_stop","index":0}),
                json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call_chat","name":"read","input":{}}}),
                json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"next\"}"}}),
                json!({"type":"content_block_stop","index":1}),
                json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":13}}),
                json!({"type":"message_stop"}),
            ]
        };
        frames.iter().map(|v| format!("data: {v}\n\n")).collect()
    }

    async fn chat_case(provider: Provider, client_streams: bool, upstream_streams: bool) {
        let model = match provider {
            Provider::Anthropic => "claude-chat-test",
            Provider::OpenAI => "gpt-chat-test",
            Provider::Kimi => "k3",
        };
        let (observed, mut requests) = tokio::sync::mpsc::channel(2);
        let output = upstream(model, provider, upstream_streams);
        let handler = move |req: Request<Body>| {
            let observed = observed.clone();
            let output = output.clone();
            async move {
                let (parts, body) = req.into_parts();
                let body = axum::body::to_bytes(body, 1_000_000).await.unwrap();
                observed
                    .send((parts.uri.path().to_owned(), parts.headers, body))
                    .await
                    .unwrap();
                let chunks: Vec<_> = output
                    .as_bytes()
                    .chunks(7)
                    .map(|b| Ok::<_, std::io::Error>(bytes::Bytes::copy_from_slice(b)))
                    .collect();
                Response::builder()
                    .header(
                        "content-type",
                        if upstream_streams {
                            "text/event-stream"
                        } else {
                            "application/json"
                        },
                    )
                    .header("x-request-id", "req_chat")
                    .body(Body::from_stream(futures_util::stream::iter(chunks)))
                    .unwrap()
            }
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new()
                    .route("/v1/messages", post(handler.clone()))
                    .route("/v1/responses", post(handler)),
            )
            .await
            .unwrap();
        });
        let tag = format!(
            "chat-{}-{client_streams}-{upstream_streams}",
            provider.as_str()
        );
        let lab = Lab::local(&tag);
        let conn = rusqlite::Connection::open(&lab.state.config.db_path).unwrap();
        conn.execute("INSERT INTO accounts(id,label,source,provider,active,created_at) VALUES('chat','Chat','token',?1,1,1)",[provider.as_str()]).unwrap();
        let config = crate::config::Config {
            server: false,
            bind: ([127, 0, 0, 1], 0).into(),
            upstream: base.clone(),
            db_path: lab.state.config.db_path.clone(),
            body_cap: 65536,
            dump_headers: 0,
            allow_hosts: vec![],
            public_url: None,
            push_keys_dir: None,
            agents_dir: None,
            drain: std::time::Duration::from_secs(1),
            stop_delay: std::time::Duration::ZERO,
        };
        let (capture, mut captured) = tokio::sync::mpsc::channel(2);
        let (network, _) = crate::network::switch();
        let mut state = AppState::new(config, capture, network).unwrap();
        let tls = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .build();
        Arc::get_mut(&mut state).unwrap().client = Client::builder(TokioExecutor::new()).build(tls);
        state.credentials.write().await.insert(
            "chat".into(),
            Active {
                account_id: "chat".into(),
                label: "Chat".into(),
                identity: None,
                token: "TEST-CHAT-UPSTREAM".into(),
                expires_at_ms: 0,
                provider,
                base_url: Some(base),
                openai_chatgpt: false,
                chatgpt_account_id: None,
            },
        );
        let app = crate::app(state);
        let created = ask(
            &app,
            "/api/auth/tokens",
            None,
            json!({"name":"Chat client","scope":"client"}),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let created: Value = serde_json::from_slice(
            &axum::body::to_bytes(created.into_body(), 8192)
                .await
                .unwrap(),
        )
        .unwrap();
        let token = created["token"].as_str().unwrap();
        let body = json!({"model":model,"stream":client_streams,"stream_options":{"include_usage":true},"max_completion_tokens":100,
            "messages":[{"role":"system","content":"Be helpful"},{"role":"user","content":[{"type":"text","text":"Read fixture"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AA=="}}]},
                {"role":"assistant","content":null,"tool_calls":[{"id":"call_prior","type":"function","function":{"name":"read","arguments":"{\"path\":\"fixture\"}"}}]},
                {"role":"tool","tool_call_id":"call_prior","content":"PRIOR_RESULT"},{"role":"user","content":"Continue"}],
            "tools":[{"type":"function","function":{"name":"read","description":"Read a file","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}}],
            "response_format":{"type":"json_schema","json_schema":{"name":"answer","schema":{"type":"object","properties":{"ok":{"type":"boolean"}}},"strict":true}}});
        let original_size = body.to_string().len() as i64;
        let response = ask(&app, "/v1/chat/completions", Some(token), body).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "portable Chat Completions must route {}",
            provider.as_str()
        );
        assert_eq!(
            response.headers()["content-type"],
            if client_streams {
                "text/event-stream"
            } else {
                "application/json"
            }
        );
        let bytes = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        if client_streams {
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(text.ends_with("data: [DONE]\n\n"));
            let frames: Vec<Value> = text
                .lines()
                .filter_map(|l| l.strip_prefix("data: "))
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            assert_eq!(
                frames
                    .iter()
                    .filter_map(|f| f["choices"][0]["delta"]["content"].as_str())
                    .collect::<String>(),
                "Chat ✓"
            );
            assert!(frames
                .iter()
                .any(|f| f["choices"][0]["delta"]["tool_calls"][0]["function"]["name"] == "read"));
            assert_eq!(
                frames
                    .iter()
                    .filter_map(|f| f["choices"][0]["delta"]["tool_calls"][0]["function"]
                        ["arguments"]
                        .as_str())
                    .collect::<String>(),
                "{\"path\":\"next\"}"
            );
            assert!(frames
                .iter()
                .any(|f| f["choices"][0]["finish_reason"] == "tool_calls"));
            assert!(frames.iter().any(|f| f["usage"]["prompt_tokens"] == 100
                && f["usage"]["completion_tokens"] == 13
                && f["usage"]["prompt_tokens_details"]["cached_tokens"] == 60));
        } else {
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["object"], "chat.completion");
            assert_eq!(value["model"], model);
            assert_eq!(value["choices"][0]["message"]["content"], "Chat ✓");
            assert_eq!(
                value["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
                "{\"path\":\"next\"}"
            );
            assert_eq!(value["choices"][0]["finish_reason"], "tool_calls");
            assert_eq!(value["usage"]["prompt_tokens"], 100);
            assert_eq!(value["usage"]["completion_tokens"], 13);
            assert_eq!(value["usage"]["prompt_tokens_details"]["cached_tokens"], 60);
        }
        let (path, headers, bytes) = requests.recv().await.unwrap();
        assert_eq!(
            path,
            if provider == Provider::OpenAI {
                "/v1/responses"
            } else {
                "/v1/messages"
            }
        );
        assert!(headers
            .values()
            .all(|v| v.to_str().is_ok_and(|v| !v.contains(token))));
        let sent: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(sent["model"], model);
        assert!(sent.to_string().contains("PRIOR_RESULT"));
        assert!(sent.to_string().contains(if provider == Provider::OpenAI {
            "input_image"
        } else {
            "image"
        }));
        assert!(sent.to_string().contains("json_schema"));
        let event = captured.recv().await.unwrap();
        assert_eq!(event.provider, provider);
        assert_eq!(event.path, "/v1/chat/completions");
        assert_eq!(event.request_bytes, original_size);
        assert_eq!(event.usage.prompt_tokens(), 100);
        assert_eq!(event.usage.output_tokens, 13);
        assert_eq!(event.session_id.as_deref(), Some("codex:chat-session"));
        server.abort();
    }

    #[tokio::test]
    async fn portable_chat_completions_cross_provider_matrix() {
        for provider in [Provider::Anthropic, Provider::OpenAI, Provider::Kimi] {
            for client_streams in [false, true] {
                for upstream_streams in [false, true] {
                    chat_case(provider, client_streams, upstream_streams).await;
                }
            }
        }
    }

    #[tokio::test]
    async fn unsupported_chat_options_are_refused_before_a_provider_is_contacted() {
        let lab = Lab::local("chat-unsupported");
        let base = json!({"model":"gpt-test","messages":[{"role":"user","content":"Hi"}]});
        let cases = [
            ("n", json!(2)),
            ("stop", json!(["STOP"])),
            ("frequency_penalty", json!(0.5)),
            ("presence_penalty", json!(1)),
            ("logprobs", json!(true)),
            ("top_logprobs", json!(2)),
            ("logit_bias", json!({"123":1})),
            ("store", json!(true)),
            ("modalities", json!(["text", "audio"])),
            ("audio", json!({"voice":"alloy","format":"wav"})),
            ("functions", json!([{"name":"legacy"}])),
            ("seed", json!(42)),
            ("parallel_tool_calls", json!("yes")),
            ("stream_options", json!({"include_obfuscation":true})),
            (
                "tools",
                json!([{"type":"custom","custom":{"name":"patch"}}]),
            ),
            (
                "messages",
                json!([{"role":"user","content":"Hi"},{"role":"system","content":"Late update"}]),
            ),
            (
                "messages",
                json!([{"role":"function","name":"legacy","content":"value"}]),
            ),
            (
                "messages",
                json!([{"role":"assistant","content":null,"reasoning_details":[{"encrypted_content":"opaque"}]}]),
            ),
            (
                "messages",
                json!([{"role":"user","content":[{"type":"input_audio","input_audio":{"data":"AA==","format":"wav"}}]}]),
            ),
            (
                "messages",
                json!([{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.invalid/image.png","detail":"low"}}]}]),
            ),
            (
                "messages",
                json!([{"role":"tool","tool_call_id":"unknown","content":"orphan result"}]),
            ),
        ];
        for (key, value) in cases {
            let mut body = base.clone();
            body[key] = value;
            let response = ask(
                &lab.app,
                "/v1/chat/completions",
                Some("unused-local-client-key"),
                body,
            )
            .await;
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "{key} must be refused without provider accounts or outbound requests"
            );
            let error: Value = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 8192)
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert!(error["error"]["message"]
                .as_str()
                .unwrap()
                .starts_with("agent-router:"));
        }
    }

    #[test]
    fn null_new_budget_retains_legacy_budget_and_tool_outputs_are_input_content() {
        let converted=super::request(&json!({"model":"gpt-test","max_completion_tokens":null,"max_tokens":32,
            "messages":[{"role":"assistant","content":null,"tool_calls":[{"id":"call","type":"function","function":{"name":"read","arguments":"{}"}}]},
                {"role":"tool","tool_call_id":"call","content":[{"type":"text","text":"tool result"}]}],
            "tools":[{"type":"function","function":{"name":"read","parameters":{"type":"object"}}}],"response_format":{"type":"json_object"}})).unwrap();
        assert_eq!(converted["max_output_tokens"], 32);
        assert_eq!(converted["input"][1]["output"][0]["type"], "input_text");
        assert_eq!(
            converted["tools"][0]["strict"], false,
            "Chat functions remain non-strict unless requested"
        );
        assert_eq!(converted["text"]["format"]["type"], "json_object");
        assert!(
            crate::translate::request(
                &serde_json::to_vec(&converted).unwrap(),
                crate::translate::Protocol::Responses,
                crate::translate::Protocol::Messages
            )
            .is_err(),
            "a provider without native JSON mode must refuse it rather than change the guarantee"
        );
    }

    fn data(frame: &Value) -> Vec<u8> {
        format!("data: {frame}\n\n").into_bytes()
    }

    #[test]
    fn fragmented_parallel_tools_keep_indices_and_terminal_items_do_not_duplicate_deltas() {
        let first = json!({"id":"fc_first","type":"function_call","call_id":"first","name":"read","arguments":"{\"a\":1}"});
        let second = json!({"id":"fc_second","type":"function_call","call_id":"second","name":"write","arguments":"{\"b\":2}"});
        let final_response = json!({"id":"resp_parallel","status":"completed","output":[first,second],"usage":{"input_tokens":4,"output_tokens":7}});
        let mut first_added = first.clone();
        first_added["arguments"] = json!("");
        let mut second_added = second.clone();
        second_added["arguments"] = json!("");
        let frames = [
            json!({"type":"response.created","response":{"id":"resp_parallel","created_at":123}}),
            json!({"type":"response.output_item.added","output_index":4,"item":first_added}),
            json!({"type":"response.output_item.added","output_index":8,"item":second_added}),
            json!({"type":"response.function_call_arguments.delta","output_index":8,"delta":"{\"b\":"}),
            json!({"type":"response.function_call_arguments.delta","output_index":4,"delta":"{\"a\":1}"}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_second","delta":"2}"}),
            json!({"type":"response.output_item.done","output_index":4,"item":first}),
            json!({"type":"response.output_item.done","output_index":8,"item":second}),
            json!({"type":"response.completed","response":final_response}),
        ];
        let raw: Vec<u8> = frames.iter().flat_map(data).collect();
        let mut stream = super::ChatStream::new("alias".into(), false);
        let mut output = Vec::new();
        for byte in raw.chunks(1) {
            output.extend(stream.push(byte).unwrap());
        }
        stream.finish().unwrap();
        let output = String::from_utf8(output).unwrap();
        let frames: Vec<Value> = output
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        for (index, args) in [(0, "{\"a\":1}"), (1, "{\"b\":2}")] {
            let actual: String = frames
                .iter()
                .filter_map(|f| f["choices"][0]["delta"]["tool_calls"].as_array())
                .flatten()
                .filter(|tool| tool["index"] == index)
                .filter_map(|tool| tool["function"]["arguments"].as_str())
                .collect();
            assert_eq!(actual, args);
        }
        assert!(frames
            .iter()
            .all(|f| f["id"] == "resp_parallel" && f["created"] == 123 && f["model"] == "alias"));
        assert!(frames.iter().all(|f| f.get("usage").is_none()));
        assert!(output.ends_with("data: [DONE]\n\n"));
    }

    #[test]
    fn terminal_only_text_is_emitted_and_transport_truncation_is_an_error() {
        let terminal = json!({"type":"response.completed","response":{"id":"resp_only","status":"incomplete",
            "incomplete_details":{"reason":"max_output_tokens"},"output":[{"id":"msg","type":"message","content":[{"type":"output_text","text":"✓"}]}],
            "usage":{"input_tokens":4,"output_tokens":7}}});
        let mut stream = super::ChatStream::new("model".into(), true);
        let output = String::from_utf8(stream.push(&data(&terminal)).unwrap()).unwrap();
        stream.finish().unwrap();
        assert!(output.contains("✓"));
        assert!(output.contains("\"finish_reason\":\"length\""));
        let mut truncated = super::ChatStream::new("model".into(), false);
        truncated
            .push(&data(
                &json!({"type":"response.created","response":{"id":"resp_truncated"}}),
            ))
            .unwrap();
        assert!(truncated
            .finish()
            .unwrap_err()
            .to_string()
            .contains("terminal event"));
        let failed = json!({"type":"response.failed","response":{"error":{"type":"server_error","message":"upstream failed"}}});
        let output = String::from_utf8(truncated.push(&data(&failed)).unwrap()).unwrap();
        truncated.finish().unwrap();
        assert!(output.contains("upstream failed"));
        assert!(!output.contains("\"finish_reason\":\"stop\""));
    }
}
