//! Translation between the two client wire protocols. Native requests stay byte faithful.
//! Stateful streams retain incomplete SSE frames, never incomplete UTF-8 strings.

use std::collections::{BTreeMap, HashMap};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    Messages,
    Responses,
}

#[derive(Clone, Default, Debug)]
struct Tool {
    name: String,
    namespace: Option<String>,
    custom: bool,
}

type ToolMap = HashMap<String, Tool>;

fn object(value: &Value) -> Result<&Map<String, Value>> {
    value.as_object().context("expected a JSON object")
}

fn array<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>> {
    value[field]
        .as_array()
        .with_context(|| format!("{field} must be an array"))
}

fn string(value: &Value, field: &str) -> Result<String> {
    value[field]
        .as_str()
        .map(str::to_owned)
        .with_context(|| format!("{field} must be a string"))
}

fn unsupported(kind: &str) -> anyhow::Error {
    anyhow!("agent-router: cannot translate {kind} between Messages and Responses")
}

fn copy_fields(from: &Value, to: &mut Value, fields: &[&str]) {
    for field in fields {
        if let Some(value) = from.get(*field) {
            to[*field] = value.clone();
        }
    }
}

fn check_unmapped(value: &Value, fields: &[&str]) -> Result<()> {
    for field in fields {
        if let Some(v) = value.get(*field) {
            if !v.is_null() && v != &Value::Bool(false) && !v.as_array().is_some_and(Vec::is_empty)
            {
                return Err(unsupported(field));
            }
        }
    }
    Ok(())
}

/// Translate a request, rejecting features whose semantics cannot be represented.
pub fn request(body: &[u8], from: Protocol, to: Protocol) -> Result<Vec<u8>> {
    if from == to {
        return Ok(body.to_vec());
    }
    let value: Value = serde_json::from_slice(body).context("invalid request JSON")?;
    object(&value)?;
    let translated = match from {
        Protocol::Messages => messages_request(&value)?,
        Protocol::Responses => responses_request(&value)?,
    };
    Ok(serde_json::to_vec(&translated)?)
}

fn text_blocks(value: &Value) -> Result<String> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_owned());
    }
    let blocks = value
        .as_array()
        .context("system content must be text or text blocks")?;
    let mut texts = Vec::new();
    for block in blocks {
        if block["type"] != "text" && block["type"] != "input_text" {
            return Err(unsupported("non-text system content"));
        }
        texts.push(string(block, "text")?);
    }
    Ok(texts.join("\n\n"))
}

fn image_to_responses(block: &Value) -> Result<Value> {
    let source = &block["source"];
    let image_url = match source["type"].as_str() {
        Some("base64") => format!(
            "data:{};base64,{}",
            string(source, "media_type")?,
            string(source, "data")?
        ),
        Some("url") => string(source, "url")?,
        _ => return Err(unsupported("image source")),
    };
    Ok(json!({"type":"input_image", "image_url":image_url}))
}

fn image_to_messages(block: &Value) -> Result<Value> {
    let url = string(block, "image_url")?;
    let source = if let Some(data) = url.strip_prefix("data:") {
        let (media, data) = data
            .split_once(";base64,")
            .context("only base64 image data URLs can be translated")?;
        json!({"type":"base64", "media_type":media,"data":data})
    } else {
        json!({"type":"url","url":url})
    };
    Ok(json!({"type":"image","source":source}))
}

fn messages_request(value: &Value) -> Result<Value> {
    check_unmapped(value, &["stop_sequences", "container"])?;
    let messages = array(value, "messages")?;
    let mut thinking_cutoff = None;
    if let Some(edits) = value["context_management"]["edits"].as_array() {
        for edit in edits {
            match edit["type"].as_str() {
                Some("clear_thinking_20251015") => match edit.get("keep") {
                    None | Some(Value::String(_))
                        if edit.get("keep").is_none() || edit["keep"] == "all" => {}
                    Some(keep) if keep["type"] == "thinking_turns" => {
                        let count = keep["value"]
                            .as_u64()
                            .filter(|n| *n > 0)
                            .context("thinking_turns keep count must be positive")?
                            as usize;
                        let turns: Vec<_> = messages
                            .iter()
                            .enumerate()
                            .filter(|(_, m)| {
                                m["role"] == "assistant"
                                    && m["content"].as_array().is_some_and(|blocks| {
                                        blocks.iter().any(|b| {
                                            b["type"] == "thinking"
                                                || b["type"] == "redacted_thinking"
                                        })
                                    })
                            })
                            .map(|(i, _)| i)
                            .collect();
                        thinking_cutoff = turns.get(turns.len().saturating_sub(count)).copied();
                    }
                    _ => return Err(unsupported("thinking-retention setting")),
                },
                _ => return Err(unsupported("context-management compaction/edit")),
            }
        }
    }
    let mut out = json!({"model":string(value,"model")?,"input":[]});
    copy_fields(
        value,
        &mut out,
        &["stream", "temperature", "top_p", "metadata"],
    );
    if let Some(tokens) = value.get("max_tokens") {
        out["max_output_tokens"] = tokens.clone();
    }
    if let Some(system) = value.get("system") {
        out["instructions"] = Value::String(text_blocks(system)?);
    }
    if let Some(thinking) = value.get("thinking") {
        match thinking["type"].as_str() {
            Some("disabled") => {}
            Some("enabled" | "adaptive") => {
                let effort = match value["output_config"]["effort"].as_str().unwrap_or("high") {
                    "max" => "xhigh",
                    other => other,
                };
                out["reasoning"] = json!({"effort":effort});
            }
            _ => return Err(unsupported("thinking configuration")),
        }
    }
    if let Some(format) = value.get("output_config").and_then(|v| v.get("format")) {
        if format["type"] != "json_schema" {
            return Err(unsupported("output format"));
        }
        out["text"] = json!({"format":{"type":"json_schema","name":"response","schema":format["schema"],"strict":true}});
    }
    let mut input = Vec::new();
    for (message_index, message) in messages.iter().enumerate() {
        let clear_thinking = thinking_cutoff.is_some_and(|cutoff| message_index < cutoff);
        let role = string(message, "role")?;
        if role == "system" {
            let text = text_blocks(&message["content"])?;
            if input.is_empty() {
                let previous = out["instructions"].as_str().unwrap_or_default();
                out["instructions"] = json!(if previous.is_empty() {
                    text
                } else {
                    format!("{previous}\n\n{text}")
                });
                continue;
            }
            // Responses has no chronological system item: keep the update in its
            // original position as a clearly identified context update.
            input.push(json!({"type":"message","role":"user","content":[{"type":"input_text","text":format!("System context update:\n{text}")}]}));
            continue;
        }
        if role != "user" && role != "assistant" {
            return Err(unsupported("message role"));
        }
        let blocks = match &message["content"] {
            Value::String(text) => vec![json!({"type":"text","text":text})],
            Value::Array(blocks) => blocks.clone(),
            _ => bail!("message content must be a string or array"),
        };
        let mut content = Vec::new();
        for block in blocks {
            let kind = block["type"].as_str().unwrap_or_default();
            match kind {
                "text" => content.push(json!({"type":if role == "assistant" {"output_text"} else {"input_text"},"text":string(&block,"text")?})),
                "image" if role == "user" => content.push(image_to_responses(&block)?),
                "redacted_thinking" if clear_thinking => {}
                "tool_use" | "tool_result" | "thinking" => {
                    flush_response_message(&mut input, &role, &mut content);
                    let item = match kind {
                        "tool_use" => json!({"type":"function_call","call_id":string(&block,"id")?,"name":string(&block,"name")?,"arguments":serde_json::to_string(&block["input"])?}),
                        "tool_result" => {
                            if block["is_error"].as_bool() == Some(true) {
                                // OpenAI has no is_error flag. Preserve it in the output itself.
                                json!({"type":"function_call_output","call_id":string(&block,"tool_use_id")?,"output":format!("Tool execution failed: {}", tool_result_text(&block)? )})
                            } else {
                                json!({"type":"function_call_output","call_id":string(&block,"tool_use_id")?,"output":tool_result_output(&block)?})
                            }
                        }
                        _ if clear_thinking => continue,
                        _ => json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":string(&block,"thinking")?}]}),
                    };
                    input.push(item);
                }
                _ => return Err(unsupported(&format!("Messages content block {kind}"))),
            }
        }
        flush_response_message(&mut input, &role, &mut content);
    }
    out["input"] = Value::Array(input);
    if let Some(tools) = value["tools"].as_array() {
        let mut converted = Vec::new();
        for tool in tools {
            if tool.get("type").is_some() && tool["type"] != "custom" {
                return Err(unsupported("provider-specific tool"));
            }
            let mut converted_tool = json!({"type":"function","name":string(tool,"name")?,"parameters":tool["input_schema"]});
            copy_fields(tool, &mut converted_tool, &["description"]);
            converted.push(converted_tool);
        }
        out["tools"] = Value::Array(converted);
    }
    if let Some(choice) = value.get("tool_choice") {
        out["tool_choice"] = match choice["type"].as_str() {
            Some("auto") => json!("auto"),
            Some("any") => json!("required"),
            Some("none") => json!("none"),
            Some("tool") => json!({"type":"function","name":string(choice,"name")?}),
            _ => return Err(unsupported("tool choice")),
        };
        if let Some(disable) = choice.get("disable_parallel_tool_use") {
            out["parallel_tool_calls"] = json!(!disable
                .as_bool()
                .context("disable_parallel_tool_use must be boolean")?);
        }
    }
    Ok(out)
}

fn flush_response_message(input: &mut Vec<Value>, role: &str, content: &mut Vec<Value>) {
    if !content.is_empty() {
        input.push(json!({"type":"message","role":role,"content":std::mem::take(content)}));
    }
}

fn tool_result_output(block: &Value) -> Result<Value> {
    match &block["content"] {
        Value::Null => Ok(json!("")),
        Value::String(_) => Ok(block["content"].clone()),
        Value::Array(content) => {
            let mut out = Vec::new();
            for item in content {
                match item["type"].as_str() {
                    Some("text") => {
                        out.push(json!({"type":"input_text","text":string(item,"text")?}))
                    }
                    Some("image") => out.push(image_to_responses(item)?),
                    _ => return Err(unsupported("tool-result content")),
                }
            }
            Ok(Value::Array(out))
        }
        _ => Err(unsupported("tool-result content")),
    }
}

fn tool_result_text(block: &Value) -> Result<String> {
    match tool_result_output(block)? {
        Value::String(s) => Ok(s),
        other => Ok(serde_json::to_string(&other)?),
    }
}

fn tools_from_responses(value: &Value) -> Result<(Vec<Value>, ToolMap)> {
    fn add(
        tool: &Value,
        namespace: Option<&str>,
        out: &mut Vec<Value>,
        map: &mut ToolMap,
    ) -> Result<()> {
        let kind = tool["type"].as_str().unwrap_or_default();
        if kind == "namespace" {
            if namespace.is_some() {
                return Err(unsupported("nested tool namespaces"));
            }
            let ns = string(tool, "name")?;
            for function in array(tool, "tools")? {
                add(function, Some(&ns), out, map)?;
            }
            return Ok(());
        }
        if kind != "function" && kind != "custom" {
            return Err(unsupported(&format!("Responses tool {kind}")));
        }
        let name = string(tool, "name")?;
        let flattened = namespace
            .map(|ns| format!("{ns}__{name}"))
            .unwrap_or_else(|| name.clone());
        if flattened.len() > 64 {
            bail!("translated tool name exceeds the Messages 64-character limit: {flattened}");
        }
        if map.contains_key(&flattened) {
            bail!("tool namespace produces duplicate tool name {flattened}");
        }
        let custom = kind == "custom";
        let mut parameters = if custom {
            json!({"type":"object","properties":{"input":{"type":"string"}},"required":["input"],"additionalProperties":false})
        } else {
            tool.get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type":"object","properties":{}}))
        };
        if custom {
            if let Some(format) = tool.get("format") {
                match format["type"].as_str() {
                    Some("text") => {}
                    Some("grammar") => {
                        // Preserve the grammar as an instruction to the target model.
                        // Messages enforces the JSON wrapper, not the embedded grammar.
                        parameters["properties"]["input"]["description"] = json!(format!(
                            "Return raw text matching this {} grammar exactly:\n{}",
                            string(format, "syntax")?,
                            string(format, "definition")?
                        ));
                    }
                    _ => return Err(unsupported("custom-tool format")),
                }
            }
        }
        let mut translated = json!({"name":flattened,"input_schema":parameters});
        copy_fields(tool, &mut translated, &["description"]);
        out.push(translated);
        map.insert(
            flattened,
            Tool {
                name,
                namespace: namespace.map(str::to_owned),
                custom,
            },
        );
        Ok(())
    }
    let mut out = Vec::new();
    let mut map = ToolMap::new();
    if let Some(tools) = value["tools"].as_array() {
        for tool in tools {
            add(tool, None, &mut out, &mut map)?;
        }
    }
    Ok((out, map))
}

fn flatten_call_name(item: &Value) -> Result<String> {
    let name = string(item, "name")?;
    Ok(item["namespace"]
        .as_str()
        .map(|ns| format!("{ns}__{name}"))
        .unwrap_or(name))
}

fn responses_request(value: &Value) -> Result<Value> {
    check_unmapped(
        value,
        &[
            "previous_response_id",
            "conversation",
            "background",
            "prompt",
        ],
    )?;
    if value
        .get("truncation")
        .is_some_and(|v| !v.is_null() && v != "disabled")
    {
        return Err(unsupported("automatic history truncation"));
    }
    let mut out = json!({"model":string(value,"model")?,"max_tokens":value.get("max_output_tokens").cloned().unwrap_or(json!(8192)),"messages":[]});
    copy_fields(
        value,
        &mut out,
        &["stream", "temperature", "top_p", "metadata"],
    );
    let (tools, _) = tools_from_responses(value)?;
    if !tools.is_empty() {
        out["tools"] = json!(tools);
    }
    let mut system = Vec::new();
    if let Some(instructions) = value["instructions"].as_str() {
        system.push(instructions.to_owned());
    }
    let mut messages = Vec::new();
    let input = match &value["input"] {
        Value::String(s) => vec![json!({"role":"user","content":s})],
        Value::Array(items) => items.clone(),
        Value::Null => Vec::new(),
        _ => bail!("Responses input must be a string or array"),
    };
    for item in input {
        let kind = item["type"].as_str().unwrap_or("message");
        match kind {
            "message" => {
                let role = string(&item, "role")?;
                if role == "system" || role == "developer" {
                    if !messages.is_empty() {
                        return Err(unsupported(
                            "system/developer messages after conversation history",
                        ));
                    }
                    system.push(text_blocks(&item["content"])?);
                    continue;
                }
                if role != "user" && role != "assistant" {
                    return Err(unsupported("message role"));
                }
                let blocks = match &item["content"] {
                    Value::String(text) => vec![json!({"type":"input_text","text":text})],
                    Value::Array(blocks) => blocks.clone(),
                    _ => bail!("message content must be text or an array"),
                };
                let mut content = Vec::new();
                for block in blocks {
                    content.push(match block["type"].as_str() {
                        Some("input_text" | "output_text" | "text") => {
                            json!({"type":"text","text":string(&block,"text")?})
                        }
                        Some("input_image") if role == "user" => image_to_messages(&block)?,
                        Some("refusal") => json!({"type":"text","text":string(&block,"refusal")?}),
                        _ => return Err(unsupported("Responses message content")),
                    });
                }
                append_message(&mut messages, &role, content);
            }
            "function_call" | "custom_tool_call" => {
                let input = if kind == "custom_tool_call" {
                    json!({"input":string(&item,"input")?})
                } else {
                    serde_json::from_str(
                        item["arguments"]
                            .as_str()
                            .context("function-call arguments must be JSON text")?,
                    )
                    .context("invalid function-call arguments")?
                };
                append_message(
                    &mut messages,
                    "assistant",
                    vec![
                        json!({"type":"tool_use","id":string(&item,"call_id")?,"name":flatten_call_name(&item)?,"input":input}),
                    ],
                );
            }
            "function_call_output" | "custom_tool_call_output" => {
                let content = response_tool_output(&item["output"])?;
                append_message(
                    &mut messages,
                    "user",
                    vec![
                        json!({"type":"tool_result","tool_use_id":string(&item,"call_id")?,"content":content}),
                    ],
                );
            }
            "reasoning" => {
                if item.get("encrypted_content").is_some_and(|v| !v.is_null()) {
                    return Err(unsupported("encrypted reasoning history"));
                }
                let text = reasoning_text(&item);
                if !text.is_empty() {
                    // A visible summary is portable; signed/hidden provider state is not.
                    append_message(
                        &mut messages,
                        "assistant",
                        vec![json!({"type":"text","text":text})],
                    );
                }
            }
            "item_reference" => return Err(unsupported("server-stored input item references")),
            _ => return Err(unsupported(&format!("Responses input item {kind}"))),
        }
    }
    if !system.is_empty() {
        out["system"] = json!(system.join("\n\n"));
    }
    out["messages"] = json!(messages);
    if let Some(choice) = value.get("tool_choice") {
        out["tool_choice"] = match choice {
            Value::String(choice) => match choice.as_str() {
                "auto" => json!({"type":"auto"}),
                "required" => json!({"type":"any"}),
                "none" => json!({"type":"none"}),
                _ => return Err(unsupported("tool choice")),
            },
            Value::Object(_) if choice["type"] == "function" || choice["type"] == "custom" => {
                json!({"type":"tool","name":flatten_call_name(choice)?})
            }
            _ => return Err(unsupported("tool choice")),
        };
    }
    if let Some(parallel) = value["parallel_tool_calls"].as_bool() {
        if out.get("tool_choice").is_none() {
            out["tool_choice"] = json!({"type":"auto"});
        }
        out["tool_choice"]["disable_parallel_tool_use"] = json!(!parallel);
    }
    if let Some(effort) = value["reasoning"]["effort"].as_str() {
        if effort != "none" {
            out["thinking"] = json!({"type":"adaptive"});
            out["output_config"]["effort"] = json!(match effort {
                "minimal" => "low",
                "xhigh" => "max",
                other => other,
            });
        }
    }
    if let Some(format) = value["text"].get("format") {
        match format["type"].as_str() {
            Some("text") => {}
            Some("json_schema") => {
                out["output_config"]["format"] =
                    json!({"type":"json_schema","schema":format["schema"]})
            }
            _ => return Err(unsupported("Responses output format")),
        }
    }
    Ok(out)
}

fn append_message(messages: &mut Vec<Value>, role: &str, content: Vec<Value>) {
    if content.is_empty() {
        return;
    }
    if let Some(last) = messages.last_mut().filter(|m| m["role"] == role) {
        last["content"].as_array_mut().unwrap().extend(content);
    } else {
        messages.push(json!({"role":role,"content":content}));
    }
}

fn response_tool_output(value: &Value) -> Result<Value> {
    if value.is_string() {
        return Ok(value.clone());
    }
    let mut out = Vec::new();
    for block in value
        .as_array()
        .context("tool output must be text or content blocks")?
    {
        out.push(match block["type"].as_str() {
            Some("input_text") => json!({"type":"text","text":string(block,"text")?}),
            Some("input_image") => image_to_messages(block)?,
            _ => return Err(unsupported("tool output block")),
        });
    }
    Ok(json!(out))
}

fn reasoning_text(item: &Value) -> String {
    item["summary"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(item["content"].as_array().into_iter().flatten())
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn tool_map(original_request: &[u8]) -> Result<ToolMap> {
    let request: Value = serde_json::from_slice(original_request)?;
    Ok(tools_from_responses(&request)?.1)
}

#[cfg(test)]
pub fn response(body: &[u8], from: Protocol, to: Protocol) -> Result<Vec<u8>> {
    response_with_request(body, from, to, &[])
}

pub fn response_with_request(
    body: &[u8],
    from: Protocol,
    to: Protocol,
    original_request: &[u8],
) -> Result<Vec<u8>> {
    if from == to {
        return Ok(body.to_vec());
    }
    let value: Value = serde_json::from_slice(body).context("invalid response JSON")?;
    let tools = if to == Protocol::Responses && !original_request.is_empty() {
        tool_map(original_request)?
    } else {
        ToolMap::new()
    };
    let out = match from {
        Protocol::Messages => messages_response(&value, &tools)?,
        Protocol::Responses => responses_response(&value)?,
    };
    Ok(serde_json::to_vec(&out)?)
}

fn responses_usage(usage: &Value) -> Value {
    let input = usage["input_tokens"].as_u64().unwrap_or(0);
    let cached = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
    let created = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
    let output = usage["output_tokens"].as_u64().unwrap_or(0);
    json!({"input_tokens":input+cached+created,"input_tokens_details":{"cached_tokens":cached,"cache_creation_tokens":created},"output_tokens":output,"output_tokens_details":{"reasoning_tokens":usage["reasoning_tokens"].as_u64().unwrap_or(0)},"total_tokens":input+cached+created+output})
}

fn messages_usage(usage: &Value) -> Value {
    let input = usage["input_tokens"].as_u64().unwrap_or(0);
    let cached = usage["input_tokens_details"]["cached_tokens"]
        .as_u64()
        .unwrap_or(0);
    let created = usage["input_tokens_details"]["cache_creation_tokens"]
        .as_u64()
        .unwrap_or(0);
    json!({"input_tokens":input.saturating_sub(cached+created),"cache_read_input_tokens":cached,"cache_creation_input_tokens":created,"output_tokens":usage["output_tokens"].as_u64().unwrap_or(0)})
}

fn call_item(
    id: &str,
    call_id: &str,
    name: &str,
    args: &str,
    tools: &ToolMap,
    status: &str,
) -> Result<Value> {
    let tool = tools.get(name).cloned().unwrap_or_else(|| Tool {
        name: name.to_owned(),
        ..Tool::default()
    });
    let mut item = if tool.custom {
        let args: Value =
            serde_json::from_str(args).context("custom tool arguments must be valid JSON")?;
        let input = string(&args, "input")?;
        json!({"type":"custom_tool_call","id":id,"call_id":call_id,"name":tool.name,"input":input,"status":status})
    } else {
        json!({"type":"function_call","id":id,"call_id":call_id,"name":tool.name,"arguments":args,"status":status})
    };
    if let Some(namespace) = tool.namespace {
        item["namespace"] = json!(namespace);
    }
    Ok(item)
}

fn messages_response(value: &Value, tools: &ToolMap) -> Result<Value> {
    if value["type"] == "error" {
        return Ok(
            json!({"error":{"type":value["error"]["type"],"message":value["error"]["message"]}}),
        );
    }
    let id = value["id"].as_str().unwrap_or("resp_agent_router");
    let mut output = Vec::new();
    for (index, block) in array(value, "content")?.iter().enumerate() {
        let item_id = format!("{id}_{index}");
        output.push(match block["type"].as_str() {
            Some("text") => json!({"id":item_id,"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":string(block,"text")?,"annotations":[]}]}),
            Some("tool_use") => call_item(&item_id,&string(block,"id")?,&string(block,"name")?,&serde_json::to_string(&block["input"])?,tools,"completed")?,
            Some("thinking") => json!({"id":item_id,"type":"reasoning","summary":[{"type":"summary_text","text":string(block,"thinking")?}]}),
            _ => return Err(unsupported("Messages response content")),
        });
    }
    let incomplete = value["stop_reason"] == "max_tokens";
    Ok(
        json!({"id":id,"object":"response","created_at":0,"model":value["model"],"status":if incomplete {"incomplete"} else {"completed"},"output":output,"usage":responses_usage(&value["usage"]),"error":null,"incomplete_details":if incomplete {json!({"reason":"max_output_tokens"})} else {Value::Null}}),
    )
}

fn responses_response(value: &Value) -> Result<Value> {
    if let Some(detail) = value["detail"].as_str() {
        return Ok(
            json!({"type":"error","error":{"type":"invalid_request_error","message":detail}}),
        );
    }
    if value.get("error").is_some_and(|e| !e.is_null()) {
        return Ok(
            json!({"type":"error","error":{"type":value["error"]["type"],"message":value["error"]["message"]}}),
        );
    }
    let mut content = Vec::new();
    let mut tool = false;
    let mut refusal = false;
    for item in array(value, "output")? {
        match item["type"].as_str() {
            Some("message") => {
                for part in array(item, "content")? {
                    content.push(match part["type"].as_str() {
                        Some("output_text") => json!({"type":"text","text":string(part,"text")?}),
                        Some("refusal") => {
                            refusal = true;
                            json!({"type":"text","text":string(part,"refusal")?})
                        }
                        _ => return Err(unsupported("Responses output content")),
                    });
                }
            }
            Some("function_call" | "custom_tool_call") => {
                tool = true;
                let input = if item["type"] == "custom_tool_call" {
                    json!({"input":string(item,"input")?})
                } else {
                    serde_json::from_str(
                        item["arguments"]
                            .as_str()
                            .context("function-call arguments must be JSON text")?,
                    )
                    .context("invalid function-call arguments")?
                };
                content.push(json!({"type":"tool_use","id":string(item,"call_id")?,"name":flatten_call_name(item)?,"input":input}));
            }
            Some("reasoning") => {
                let text = reasoning_text(item);
                if !text.is_empty() {
                    content.push(json!({"type":"thinking","thinking":text,"signature":""}));
                }
            }
            _ => return Err(unsupported("Responses output item")),
        }
    }
    let stop_reason = if value["status"] == "incomplete" {
        "max_tokens"
    } else if tool {
        "tool_use"
    } else if refusal {
        "refusal"
    } else {
        "end_turn"
    };
    Ok(
        json!({"id":value["id"],"type":"message","role":"assistant","model":value["model"],"content":content,"stop_reason":stop_reason,"stop_sequence":null,"usage":messages_usage(&value["usage"])}),
    )
}

#[derive(Clone, Default)]
struct Block {
    kind: String,
    id: String,
    call_id: String,
    name: String,
    text: String,
    arguments: String,
    closed: bool,
    source_item: usize,
    source_part: usize,
}

/// Translate complete SSE events as they arrive. No HTTP/body boundary assumptions.
pub struct StreamTranslator {
    from: Protocol,
    to: Protocol,
    pending: Vec<u8>,
    scanned: usize,
    tools: ToolMap,
    id: String,
    model: String,
    usage: Value,
    stop_reason: String,
    blocks: BTreeMap<usize, Block>,
    response: Option<Value>,
    started: bool,
    completed: bool,
    sequence: u64,
    input_includes_cache: bool,
    stream_error: Option<Value>,
}

impl StreamTranslator {
    pub fn new(from: Protocol, to: Protocol) -> Self {
        Self {
            from,
            to,
            pending: Vec::new(),
            scanned: 0,
            tools: ToolMap::new(),
            id: "resp_agent_router".into(),
            model: String::new(),
            usage: json!({}),
            stop_reason: "end_turn".into(),
            blocks: BTreeMap::new(),
            response: None,
            started: false,
            completed: false,
            sequence: 0,
            input_includes_cache: false,
            stream_error: None,
        }
    }

    pub fn with_request(from: Protocol, to: Protocol, original_request: &[u8]) -> Result<Self> {
        let mut out = Self::new(from, to);
        if to == Protocol::Responses && !original_request.is_empty() {
            out.tools = tool_map(original_request)?;
        }
        Ok(out)
    }

    /// Kimi counts cached prompt tokens inside input_tokens, unlike Anthropic.
    pub fn set_input_includes_cache(&mut self, includes_cache: bool) {
        self.input_includes_cache = includes_cache;
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<u8>> {
        if self.from == self.to {
            return Ok(bytes.to_vec());
        }
        self.pending.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some((offset, delimiter)) = frame_end(&self.pending[self.scanned..]) {
            let end = self.scanned + offset;
            let frame = self.pending[..end].to_vec();
            self.pending.drain(..end + delimiter);
            self.scanned = 0;
            if let Some(value) = parse_frame(&frame)? {
                self.event(value, &mut out)?;
            }
        }
        self.scanned = self.pending.len().saturating_sub(3);
        if self.pending.len() > 16 * 1024 * 1024 {
            bail!("upstream SSE frame exceeds 16 MiB");
        }
        Ok(out)
    }

    pub fn finish(&mut self) -> Result<Vec<u8>> {
        if self.from == self.to {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        if !self.pending.is_empty() {
            let frame = std::mem::take(&mut self.pending);
            if let Some(value) = parse_frame(&frame)? {
                self.event(value, &mut out)?;
            }
        }
        if !self.completed {
            bail!("upstream stream ended before its terminal event");
        }
        Ok(out)
    }

    fn emit(&mut self, mut value: Value, out: &mut Vec<u8>) -> Result<()> {
        let event = string(&value, "type")?;
        if self.to == Protocol::Responses {
            value["sequence_number"] = json!(self.sequence);
            self.sequence += 1;
        }
        out.extend_from_slice(
            format!(
                "event: {event}\ndata: {}\n\n",
                serde_json::to_string(&value)?
            )
            .as_bytes(),
        );
        Ok(())
    }

    fn event(&mut self, value: Value, out: &mut Vec<u8>) -> Result<()> {
        match self.from {
            Protocol::Messages => self.messages_event(value, out),
            Protocol::Responses => self.responses_event(value, out),
        }
    }

    fn response_envelope(&self, status: &str, output: Vec<Value>) -> Value {
        json!({"id":self.id,"object":"response","created_at":0,"model":self.model,"status":status,"output":output,"usage":responses_usage(&self.usage),"error":null,"incomplete_details":if status == "incomplete" {json!({"reason":"max_output_tokens"})} else {Value::Null}})
    }

    fn messages_event(&mut self, value: Value, out: &mut Vec<u8>) -> Result<()> {
        match value["type"].as_str().unwrap_or_default() {
            "ping" => {}
            "message_start" => {
                if self.started {
                    bail!("duplicate message_start");
                }
                let message = &value["message"];
                self.id = message["id"].as_str().unwrap_or("resp_agent_router").into();
                self.model = message["model"].as_str().unwrap_or_default().into();
                self.usage = message["usage"].clone();
                if self.input_includes_cache {
                    normalize_inclusive_usage(&mut self.usage);
                }
                self.started = true;
                self.emit(json!({"type":"response.created","response":self.response_envelope("in_progress",Vec::new())}),out)?;
                self.emit(json!({"type":"response.in_progress","response":self.response_envelope("in_progress",Vec::new())}),out)?;
            }
            "content_block_start" => {
                let index = index(&value, "index")?;
                let content = &value["content_block"];
                let kind = string(content, "type")?;
                if self.blocks.contains_key(&index) {
                    bail!("duplicate content-block index");
                }
                let block = Block {
                    kind: kind.clone(),
                    id: format!("{}_{}", self.id, index),
                    call_id: content["id"].as_str().unwrap_or_default().into(),
                    name: content["name"].as_str().unwrap_or_default().into(),
                    ..Block::default()
                };
                self.blocks.insert(index, block);
                match kind.as_str() {
                    "text" => {
                        let block = &self.blocks[&index];
                        self.emit(json!({"type":"response.output_item.added","output_index":index,"item":{"id":block.id,"type":"message","role":"assistant","status":"in_progress","content":[]}}),out)?;
                        self.emit(json!({"type":"response.content_part.added","item_id":self.blocks[&index].id,"output_index":index,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),out)?;
                        if let Some(text) = content["text"].as_str().filter(|t| !t.is_empty()) {
                            self.text_delta(index, text, "text", out)?;
                        }
                    }
                    "tool_use" => {
                        let block = &self.blocks[&index];
                        let tool = self.tools.get(&block.name);
                        let mut item = json!({"id":block.id,"call_id":block.call_id,"name":tool.map(|t| t.name.as_str()).unwrap_or(&block.name),"type":if tool.is_some_and(|t| t.custom) {"custom_tool_call"} else {"function_call"},"status":"in_progress"});
                        if let Some(namespace) = tool.and_then(|t| t.namespace.as_ref()) {
                            item["namespace"] = json!(namespace);
                        }
                        if tool.is_some_and(|t| t.custom) {
                            item["input"] = json!("");
                        } else {
                            item["arguments"] = json!("");
                        }
                        self.emit(json!({"type":"response.output_item.added","output_index":index,"item":item}),out)?;
                        if content["input"].as_object().is_some_and(|o| !o.is_empty()) {
                            let args = serde_json::to_string(&content["input"])?;
                            self.arguments_delta(index, &args, out)?;
                        }
                    }
                    "thinking" => {
                        self.emit(json!({"type":"response.output_item.added","output_index":index,"item":{"id":self.blocks[&index].id,"type":"reasoning","summary":[]}}),out)?;
                        self.emit(json!({"type":"response.reasoning_summary_part.added","item_id":self.blocks[&index].id,"output_index":index,"summary_index":0,"part":{"type":"summary_text","text":""}}),out)?;
                        if let Some(text) = content["thinking"].as_str().filter(|t| !t.is_empty()) {
                            self.text_delta(index, text, "thinking", out)?;
                        }
                    }
                    _ => return Err(unsupported(&format!("streamed Messages block {kind}"))),
                }
            }
            "content_block_delta" => {
                let index = index(&value, "index")?;
                let delta = &value["delta"];
                match delta["type"].as_str() {
                    Some("text_delta") => {
                        self.text_delta(index, &string(delta, "text")?, "text", out)?
                    }
                    Some("thinking_delta") => {
                        self.text_delta(index, &string(delta, "thinking")?, "thinking", out)?
                    }
                    Some("input_json_delta") => {
                        self.arguments_delta(index, &string(delta, "partial_json")?, out)?
                    }
                    Some("signature_delta") => {} // Provider signatures cannot be used by the other provider.
                    _ => return Err(unsupported("Messages stream delta")),
                }
            }
            "content_block_stop" => self.finish_response_block(index(&value, "index")?, out)?,
            "message_delta" => {
                if let Some(reason) = value["delta"]["stop_reason"].as_str() {
                    self.stop_reason = reason.into();
                }
                let mut usage = value["usage"].clone();
                if self.input_includes_cache {
                    normalize_inclusive_usage(&mut usage);
                }
                merge_usage(&mut self.usage, &usage);
            }
            "message_stop" => {
                if !self.started {
                    bail!("message_stop without message_start");
                }
                let indices: Vec<_> = self
                    .blocks
                    .iter()
                    .filter(|(_, b)| !b.closed)
                    .map(|(i, _)| *i)
                    .collect();
                for i in indices {
                    self.finish_response_block(i, out)?;
                }
                let output = self
                    .blocks
                    .iter()
                    .map(|(i, _)| self.response_item(*i))
                    .collect::<Result<Vec<_>>>()?;
                let status = if self.stop_reason == "max_tokens" {
                    "incomplete"
                } else {
                    "completed"
                };
                let response = self.response_envelope(status, output);
                self.response = Some(response.clone());
                self.emit(json!({"type":if status == "incomplete" {"response.incomplete"} else {"response.completed"},"response":response}),out)?;
                self.completed = true;
            }
            "error" => {
                self.stream_error = Some(value.clone());
                self.emit(json!({"type":"error","code":value["error"]["type"],"message":value["error"]["message"],"param":null}),out)?;
                self.completed = true;
            }
            unknown => return Err(unsupported(&format!("Messages event {unknown}"))),
        }
        Ok(())
    }

    fn text_delta(
        &mut self,
        index: usize,
        text: &str,
        kind: &str,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        let block = self
            .blocks
            .get_mut(&index)
            .context("delta references unknown block")?;
        if block.kind != kind || block.closed {
            bail!("invalid delta for content block");
        }
        block.text.push_str(text);
        let id = block.id.clone();
        self.emit(if kind == "thinking" {
            json!({"type":"response.reasoning_summary_text.delta","item_id":id,"output_index":index,"summary_index":0,"delta":text})
        } else {
            json!({"type":"response.output_text.delta","item_id":id,"output_index":index,"content_index":0,"delta":text})
        },out)
    }

    fn arguments_delta(&mut self, index: usize, args: &str, out: &mut Vec<u8>) -> Result<()> {
        let block = self
            .blocks
            .get_mut(&index)
            .context("delta references unknown tool block")?;
        if block.kind != "tool_use" || block.closed {
            bail!("invalid tool argument delta");
        }
        block.arguments.push_str(args);
        let id = block.id.clone();
        let custom = self.tools.get(&block.name).is_some_and(|t| t.custom);
        if !custom {
            self.emit(json!({"type":"response.function_call_arguments.delta","item_id":id,"output_index":index,"delta":args}),out)?;
        }
        // Custom tools unwrap a JSON string. Emit its raw text once the wrapper is complete.
        Ok(())
    }

    fn response_item(&self, index: usize) -> Result<Value> {
        let b = self.blocks.get(&index).context("unknown content block")?;
        let status = if b.closed { "completed" } else { "in_progress" };
        match b.kind.as_str() {
            "text" => Ok(
                json!({"id":b.id,"type":"message","role":"assistant","status":status,"content":[{"type":"output_text","text":b.text,"annotations":[]}]}),
            ),
            "thinking" => Ok(
                json!({"id":b.id,"type":"reasoning","summary":[{"type":"summary_text","text":b.text}]}),
            ),
            "tool_use" => call_item(
                &b.id,
                &b.call_id,
                &b.name,
                if b.arguments.is_empty() {
                    "{}"
                } else {
                    &b.arguments
                },
                &self.tools,
                status,
            ),
            _ => Err(unsupported("streamed output block")),
        }
    }

    fn finish_response_block(&mut self, index: usize, out: &mut Vec<u8>) -> Result<()> {
        let b = self
            .blocks
            .get_mut(&index)
            .context("stop references unknown content block")?;
        if b.closed {
            return Ok(());
        }
        b.closed = true;
        let b = b.clone();
        match b.kind.as_str() {
            "text" => {
                self.emit(json!({"type":"response.output_text.done","item_id":b.id,"output_index":index,"content_index":0,"text":b.text}),out)?;
                self.emit(json!({"type":"response.content_part.done","item_id":b.id,"output_index":index,"content_index":0,"part":{"type":"output_text","text":b.text,"annotations":[]}}),out)?;
            }
            "thinking" => {
                self.emit(json!({"type":"response.reasoning_summary_text.done","item_id":b.id,"output_index":index,"summary_index":0,"text":b.text}),out)?;
                self.emit(json!({"type":"response.reasoning_summary_part.done","item_id":b.id,"output_index":index,"summary_index":0,"part":{"type":"summary_text","text":b.text}}),out)?;
            }
            "tool_use" => {
                let item = self.response_item(index)?;
                if item["type"] == "custom_tool_call" {
                    self.emit(json!({"type":"response.custom_tool_call_input.delta","item_id":b.id,"output_index":index,"delta":item["input"]}),out)?;
                    self.emit(json!({"type":"response.custom_tool_call_input.done","item_id":b.id,"output_index":index,"input":item["input"]}),out)?;
                } else {
                    self.emit(json!({"type":"response.function_call_arguments.done","item_id":b.id,"output_index":index,"arguments":item["arguments"]}),out)?;
                }
            }
            _ => return Err(unsupported("streamed output block")),
        }
        self.emit(json!({"type":"response.output_item.done","output_index":index,"item":self.response_item(index)?}),out)
    }

    fn start_messages(&mut self, response: &Value, out: &mut Vec<u8>) -> Result<()> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        self.id = response["id"].as_str().unwrap_or("msg_agent_router").into();
        self.model = response["model"].as_str().unwrap_or_default().into();
        self.usage = response["usage"].clone();
        let mut usage = messages_usage(&self.usage);
        usage["output_tokens"] = json!(0);
        self.emit(json!({"type":"message_start","message":{"id":self.id,"type":"message","role":"assistant","model":self.model,"content":[],"stop_reason":null,"stop_sequence":null,"usage":usage}}),out)
    }

    fn start_messages_block(
        &mut self,
        source_item: usize,
        source_part: usize,
        kind: &str,
        item: &Value,
        out: &mut Vec<u8>,
    ) -> Result<usize> {
        if let Some((index, _)) = self.blocks.iter().find(|(_, b)| {
            b.source_item == source_item && b.source_part == source_part && b.kind == kind
        }) {
            return Ok(*index);
        }
        if !self.started {
            self.start_messages(&json!({}), out)?;
        }
        let index = self.blocks.len();
        let block = Block {
            kind: kind.into(),
            id: item["id"].as_str().unwrap_or_default().into(),
            call_id: item["call_id"].as_str().unwrap_or_default().into(),
            name: if kind == "tool_use" {
                flatten_call_name(item)?
            } else {
                String::new()
            },
            source_item,
            source_part,
            ..Block::default()
        };
        let content = match kind {
            "text" => json!({"type":"text","text":""}),
            "thinking" => json!({"type":"thinking","thinking":"","signature":""}),
            "tool_use" => {
                json!({"type":"tool_use","id":block.call_id,"name":block.name,"input":{}})
            }
            _ => return Err(unsupported("Responses stream item")),
        };
        self.blocks.insert(index, block);
        self.emit(
            json!({"type":"content_block_start","index":index,"content_block":content}),
            out,
        )?;
        Ok(index)
    }

    fn messages_delta(
        &mut self,
        index: usize,
        text: &str,
        kind: &str,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        let block = self
            .blocks
            .get_mut(&index)
            .context("unknown output block")?;
        if block.closed {
            bail!("delta after content block stop");
        }
        let delta = match kind {
            "text" => {
                block.text.push_str(text);
                json!({"type":"text_delta","text":text})
            }
            "thinking" => {
                block.text.push_str(text);
                json!({"type":"thinking_delta","thinking":text})
            }
            "tool_use" => {
                block.arguments.push_str(text);
                json!({"type":"input_json_delta","partial_json":text})
            }
            _ => return Err(unsupported("Responses delta")),
        };
        self.emit(
            json!({"type":"content_block_delta","index":index,"delta":delta}),
            out,
        )
    }

    fn finish_messages_block(&mut self, index: usize, out: &mut Vec<u8>) -> Result<()> {
        let b = self
            .blocks
            .get_mut(&index)
            .context("unknown output block")?;
        if b.closed {
            return Ok(());
        }
        b.closed = true;
        self.emit(json!({"type":"content_block_stop","index":index}), out)
    }

    fn responses_event(&mut self, value: Value, out: &mut Vec<u8>) -> Result<()> {
        match value["type"].as_str().unwrap_or_default() {
            "response.created" | "response.in_progress" => {
                self.start_messages(&value["response"], out)?
            }
            "response.output_item.added" => {
                let item = &value["item"];
                let output_index = index(&value, "output_index")?;
                match item["type"].as_str() {
                    Some("message" | "reasoning") => {}
                    Some("function_call" | "custom_tool_call") => {
                        let i =
                            self.start_messages_block(output_index, 0, "tool_use", item, out)?;
                        if item["type"] == "custom_tool_call" {
                            self.blocks.get_mut(&i).unwrap().kind = "custom_tool_use".into();
                        }
                    }
                    _ => return Err(unsupported("Responses stream output item")),
                }
            }
            "response.content_part.added" | "response.reasoning_summary_part.added" => {
                let part = &value["part"];
                let kind = if value["type"] == "response.reasoning_summary_part.added" {
                    "thinking"
                } else {
                    "text"
                };
                if !matches!(
                    part["type"].as_str(),
                    Some("output_text" | "summary_text" | "refusal")
                ) {
                    return Err(unsupported("Responses stream content part"));
                }
                let part_index = if kind == "thinking" {
                    value["summary_index"].as_u64().unwrap_or(0)
                } else {
                    value["content_index"].as_u64().unwrap_or(0)
                } as usize;
                let i = self.start_messages_block(
                    index(&value, "output_index")?,
                    part_index,
                    kind,
                    &json!({}),
                    out,
                )?;
                if let Some(text) = part["text"].as_str().filter(|t| !t.is_empty()) {
                    self.messages_delta(i, text, kind, out)?;
                }
            }
            "response.output_text.delta"
            | "response.refusal.delta"
            | "response.reasoning_summary_text.delta"
            | "response.reasoning_text.delta" => {
                let kind = if value["type"] == "response.reasoning_summary_text.delta"
                    || value["type"] == "response.reasoning_text.delta"
                {
                    "thinking"
                } else {
                    "text"
                };
                let part = if kind == "thinking" {
                    value["summary_index"].as_u64().unwrap_or(0)
                } else {
                    value["content_index"].as_u64().unwrap_or(0)
                } as usize;
                let i = self.start_messages_block(
                    index(&value, "output_index")?,
                    part,
                    kind,
                    &json!({}),
                    out,
                )?;
                self.messages_delta(i, &string(&value, "delta")?, kind, out)?;
                if value["type"] == "response.refusal.delta" {
                    self.stop_reason = "refusal".into();
                }
            }
            "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta" => {
                let source = index(&value, "output_index")?;
                let i = self
                    .blocks
                    .iter()
                    .find(|(_, b)| {
                        b.source_item == source
                            && (b.kind == "tool_use" || b.kind == "custom_tool_use")
                    })
                    .map(|(i, _)| *i)
                    .context("tool delta without output_item.added")?;
                let delta = string(&value, "delta")?;
                if self.blocks[&i].kind == "custom_tool_use" {
                    self.blocks.get_mut(&i).unwrap().text.push_str(&delta);
                } else {
                    self.messages_delta(i, &delta, "tool_use", out)?;
                }
            }
            "response.output_item.done" => {
                let source = index(&value, "output_index")?;
                let item = &value["item"];
                let indices: Vec<_> = self
                    .blocks
                    .iter()
                    .filter(|(_, b)| b.source_item == source && !b.closed)
                    .map(|(i, _)| *i)
                    .collect();
                for i in indices {
                    if self.blocks[&i].kind == "custom_tool_use" {
                        let text = item["input"]
                            .as_str()
                            .unwrap_or(&self.blocks[&i].text)
                            .to_owned();
                        self.messages_delta(
                            i,
                            &serde_json::to_string(&json!({"input":text}))?,
                            "tool_use",
                            out,
                        )?;
                    } else if self.blocks[&i].kind == "tool_use"
                        && self.blocks[&i].arguments.is_empty()
                    {
                        let arguments = item["arguments"].as_str().unwrap_or("{}");
                        self.messages_delta(i, arguments, "tool_use", out)?;
                    }
                    self.finish_messages_block(i, out)?;
                }
            }
            "response.content_part.done" | "response.reasoning_summary_part.done" => {
                let source = index(&value, "output_index")?;
                let kind = if value["type"] == "response.reasoning_summary_part.done" {
                    "thinking"
                } else {
                    "text"
                };
                let part = if kind == "thinking" {
                    value["summary_index"].as_u64().unwrap_or(0)
                } else {
                    value["content_index"].as_u64().unwrap_or(0)
                } as usize;
                if let Some(i) = self
                    .blocks
                    .iter()
                    .find(|(_, b)| {
                        b.source_item == source && b.source_part == part && b.kind == kind
                    })
                    .map(|(i, _)| *i)
                {
                    self.finish_messages_block(i, out)?;
                }
            }
            "response.completed" | "response.incomplete" => {
                let response = &value["response"];
                self.start_messages(response, out)?;
                self.usage = response["usage"].clone();
                self.response = Some(response.clone());
                let indices: Vec<_> = self
                    .blocks
                    .iter()
                    .filter(|(_, b)| !b.closed)
                    .map(|(i, _)| *i)
                    .collect();
                for i in indices {
                    self.finish_messages_block(i, out)?;
                }
                let reason = if value["type"] == "response.incomplete" {
                    "max_tokens"
                } else if self
                    .blocks
                    .values()
                    .any(|b| b.kind == "tool_use" || b.kind == "custom_tool_use")
                {
                    "tool_use"
                } else {
                    &self.stop_reason
                };
                self.emit(json!({"type":"message_delta","delta":{"stop_reason":reason,"stop_sequence":null},"usage":messages_usage(&self.usage)}),out)?;
                self.emit(json!({"type":"message_stop"}), out)?;
                self.completed = true;
            }
            "error" | "response.failed" => {
                self.stream_error = Some(value.clone());
                let error = if value["type"] == "response.failed" {
                    &value["response"]["error"]
                } else {
                    &value
                };
                self.emit(json!({"type":"error","error":{"type":error.get("code").or_else(|| error.get("type")).cloned().unwrap_or(json!("api_error")),"message":error["message"]}}),out)?;
                self.completed = true;
            }
            // These confirm values already delivered as deltas. Citations have no
            // Messages representation; text remains available to the client.
            "response.output_text.done"
            | "response.refusal.done"
            | "response.function_call_arguments.done"
            | "response.custom_tool_call_input.done"
            | "response.reasoning_summary_text.done"
            | "response.reasoning_text.done"
            | "response.output_text.annotation.added"
            | "response.queued" => {}
            unknown => return Err(unsupported(&format!("Responses event {unknown}"))),
        }
        Ok(())
    }
}

fn merge_usage(usage: &mut Value, update: &Value) {
    if let Some(values) = update.as_object() {
        if !usage.is_object() {
            *usage = json!({});
        }
        for (name, value) in values {
            usage[name] = value.clone();
        }
    }
}

fn normalize_inclusive_usage(usage: &mut Value) {
    if let Some(input) = usage["input_tokens"].as_u64() {
        let cached = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
        usage["input_tokens"] = json!(input.saturating_sub(cached));
    }
}

fn index(value: &Value, name: &str) -> Result<usize> {
    value[name]
        .as_u64()
        .map(|i| i as usize)
        .with_context(|| format!("{name} must be a nonnegative integer"))
}

fn frame_end(bytes: &[u8]) -> Option<(usize, usize)> {
    for i in 0..bytes.len() {
        if bytes[i..].starts_with(b"\n\n") {
            return Some((i, 2));
        }
        if bytes[i..].starts_with(b"\r\n\r\n") {
            return Some((i, 4));
        }
    }
    None
}

fn parse_frame(bytes: &[u8]) -> Result<Option<Value>> {
    let text = std::str::from_utf8(bytes).context("invalid UTF-8 in upstream SSE frame")?;
    let data = text
        .lines()
        .filter_map(|line| {
            line.strip_prefix("data:")
                .map(|d| d.strip_prefix(' ').unwrap_or(d))
        })
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() || data == "[DONE]" {
        return Ok(None);
    }
    Ok(Some(
        serde_json::from_str(&data).context("invalid upstream SSE JSON")?,
    ))
}

/// Assemble an upstream SSE response for a client that requested JSON.
pub fn collect_sse(body: &[u8], protocol: Protocol) -> Result<Vec<u8>> {
    if protocol == Protocol::Responses {
        // Native collection must accept provider tools that require no translation.
        let mut remaining = body;
        loop {
            let (frame, rest) = match frame_end(remaining) {
                Some((end, delimiter)) => (&remaining[..end], &remaining[end + delimiter..]),
                None => (remaining, &[][..]),
            };
            if let Some(event) = parse_frame(frame)? {
                match event["type"].as_str() {
                    Some("response.completed" | "response.incomplete") => {
                        return Ok(serde_json::to_vec(&event["response"])?)
                    }
                    Some("response.failed" | "error") => {
                        bail!("upstream response stream failed: {}", event)
                    }
                    _ => {}
                }
            }
            if rest.is_empty() {
                bail!("upstream stream ended before its terminal event");
            }
            remaining = rest;
        }
    }
    let opposite = match protocol {
        Protocol::Messages => Protocol::Responses,
        Protocol::Responses => Protocol::Messages,
    };
    let mut stream = StreamTranslator::new(protocol, opposite);
    stream.push(body)?;
    stream.finish()?;
    if let Some(error) = stream.stream_error {
        bail!("upstream response stream failed: {error}");
    }
    let response = match protocol {
        Protocol::Responses => stream
            .response
            .context("stream has no completed response")?,
        Protocol::Messages => {
            let mut content = Vec::new();
            for block in stream.blocks.values() {
                content.push(match block.kind.as_str() {
                    "text" => json!({"type":"text","text":block.text}),
                    "thinking" => json!({"type":"thinking","thinking":block.text,"signature":""}),
                    "tool_use" => json!({"type":"tool_use","id":block.call_id,"name":block.name,"input":serde_json::from_str::<Value>(if block.arguments.is_empty() {"{}"} else {&block.arguments})?}),
                    _ => return Err(unsupported("stream collection block")),
                });
            }
            json!({"id":stream.id,"type":"message","role":"assistant","model":stream.model,"content":content,"stop_reason":stream.stop_reason,"stop_sequence":null,"usage":stream.usage})
        }
    };
    Ok(serde_json::to_vec(&response)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translate(value: Value, from: Protocol, to: Protocol) -> Value {
        serde_json::from_slice(&request(&serde_json::to_vec(&value).unwrap(), from, to).unwrap())
            .unwrap()
    }

    fn frames(values: &[Value]) -> Vec<u8> {
        values
            .iter()
            .map(|v| {
                format!(
                    "event: {}\r\ndata: {}\r\n\r\n",
                    v["type"].as_str().unwrap(),
                    v
                )
            })
            .collect::<String>()
            .into_bytes()
    }

    fn events(bytes: &[u8]) -> Vec<Value> {
        std::str::from_utf8(bytes)
            .unwrap()
            .split("\n\n")
            .filter(|s| !s.is_empty())
            .filter_map(|s| parse_frame(s.as_bytes()).unwrap())
            .collect()
    }

    #[test]
    fn text_images_and_tool_history_round_trip() {
        let source = json!({"model":"model","max_tokens":123,"system":"Be concise","tools":[{"name":"lookup","description":"Lookup","input_schema":{"type":"object","properties":{"q":{"type":"string"}}}}],"messages":[{"role":"user","content":[{"type":"text","text":"Find it"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"abcd"}}]},{"role":"assistant","content":[{"type":"text","text":"Searching"},{"type":"tool_use","id":"call1","name":"lookup","input":{"q":"it"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"call1","content":"Found"}]}]});
        let responses = translate(source.clone(), Protocol::Messages, Protocol::Responses);
        assert_eq!(responses["input"][2]["type"], "function_call");
        let restored = translate(responses, Protocol::Responses, Protocol::Messages);
        assert_eq!(restored["messages"], source["messages"]);
        assert_eq!(restored["system"], source["system"]);
        assert_eq!(restored["max_tokens"], 123);
        assert_eq!(restored["tools"], source["tools"]);
    }

    #[test]
    fn namespace_and_custom_tool_are_restored() {
        let request = json!({"model":"model","input":"do it","tools":[{"type":"namespace","name":"functions","tools":[{"type":"function","name":"run","parameters":{"type":"object"}},{"type":"custom","name":"patch","format":{"type":"text"}}]}]});
        let translated = translate(request.clone(), Protocol::Responses, Protocol::Messages);
        assert_eq!(translated["tools"][0]["name"], "functions__run");
        assert_eq!(
            translated["tools"][1]["input_schema"]["required"],
            json!(["input"])
        );
        let message = json!({"id":"msg1","model":"model","type":"message","content":[{"type":"tool_use","id":"call1","name":"functions__patch","input":{"input":"*** patch ***"}}],"stop_reason":"tool_use","usage":{}});
        let response: Value = serde_json::from_slice(
            &response_with_request(
                &serde_json::to_vec(&message).unwrap(),
                Protocol::Messages,
                Protocol::Responses,
                &serde_json::to_vec(&request).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(response["output"][0]["type"], "custom_tool_call");
        assert_eq!(response["output"][0]["namespace"], "functions");
        assert_eq!(response["output"][0]["name"], "patch");
        assert_eq!(response["output"][0]["input"], "*** patch ***");
    }

    #[test]
    fn usage_normalizes_cached_and_created_tokens() {
        let anthropic = json!({"input_tokens":10,"cache_read_input_tokens":20,"cache_creation_input_tokens":30,"output_tokens":4});
        let openai = responses_usage(&anthropic);
        assert_eq!(openai["input_tokens"], 60);
        assert_eq!(openai["total_tokens"], 64);
        assert_eq!(messages_usage(&openai), anthropic);
    }

    #[test]
    fn rejects_history_references_and_builtin_tools() {
        for source in [
            json!({"model":"m","input":"x","previous_response_id":"resp_1"}),
            json!({"model":"m","input":"x","tools":[{"type":"web_search"}]}),
            json!({"model":"m","input":[{"type":"reasoning","encrypted_content":"ciphertext","summary":[]}]} ),
        ] {
            assert!(request(
                &serde_json::to_vec(&source).unwrap(),
                Protocol::Responses,
                Protocol::Messages
            )
            .is_err());
        }
    }

    fn message_stream() -> Vec<u8> {
        frames(&[
            json!({"type":"message_start","message":{"id":"msg1","model":"m","usage":{"input_tokens":10,"cache_read_input_tokens":5,"output_tokens":0}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"héllo 🌎"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call1","name":"run","input":{}}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"command\":"}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"pwd\"}"}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":8}}),
            json!({"type":"message_stop"}),
        ])
    }

    #[test]
    fn fragmented_sse_and_bidirectional_tool_text_usage() {
        let source = message_stream();
        let mut direct = StreamTranslator::new(Protocol::Messages, Protocol::Responses);
        let expected = direct.push(&source).unwrap();
        direct.finish().unwrap();
        for chunk_size in [1, 2, 3, 7, 31, 127] {
            let mut stream = StreamTranslator::new(Protocol::Messages, Protocol::Responses);
            let mut translated = Vec::new();
            for chunk in source.chunks(chunk_size) {
                translated.extend(stream.push(chunk).unwrap());
            }
            translated.extend(stream.finish().unwrap());
            assert_eq!(translated, expected, "chunk size {chunk_size}");
        }
        let complete = events(&expected)
            .into_iter()
            .find(|e| e["type"] == "response.completed")
            .unwrap();
        assert_eq!(
            complete["response"]["output"][0]["content"][0]["text"],
            "héllo 🌎"
        );
        assert_eq!(
            complete["response"]["output"][1]["arguments"],
            "{\"command\":\"pwd\"}"
        );
        assert_eq!(complete["response"]["usage"]["input_tokens"], 15);
        let mut reverse = StreamTranslator::new(Protocol::Responses, Protocol::Messages);
        let restored = reverse.push(&expected).unwrap();
        reverse.finish().unwrap();
        let collected: Value =
            serde_json::from_slice(&collect_sse(&restored, Protocol::Messages).unwrap()).unwrap();
        assert_eq!(collected["content"][0]["text"], "héllo 🌎");
        assert_eq!(collected["content"][1]["input"], json!({"command":"pwd"}));
        assert_eq!(collected["usage"]["cache_read_input_tokens"], 5);
        assert_eq!(collected["usage"]["output_tokens"], 8);
        assert_eq!(collected["stop_reason"], "tool_use");
    }

    #[test]
    fn incomplete_stream_is_an_error_and_native_is_byte_faithful() {
        let mut stream = StreamTranslator::new(Protocol::Messages, Protocol::Responses);
        stream
            .push(&frames(&[
                json!({"type":"message_start","message":{"id":"m"}}),
            ]))
            .unwrap();
        assert!(stream.finish().is_err());
        let body = b"not json but native";
        assert_eq!(
            request(body, Protocol::Messages, Protocol::Messages).unwrap(),
            body
        );
        let mut native = StreamTranslator::new(Protocol::Responses, Protocol::Responses);
        assert_eq!(native.push(body).unwrap(), body);
        native.finish().unwrap();
    }

    #[test]
    fn codex_namespace_grammar_and_custom_history_keep_raw_input() {
        let source = json!({"model":"claude-opus-4-6","instructions":"You are a coding agent","store":false,"stream":true,"include":["reasoning.encrypted_content"],"parallel_tool_calls":true,"reasoning":{"effort":"high","summary":"auto"},"text":{"verbosity":"medium"},"tools":[{"type":"namespace","name":"functions","description":"Workspace tools","tools":[{"type":"function","name":"exec_command","description":"Execute command","strict":false,"parameters":{"type":"object","properties":{"cmd":{"type":"string"}},"required":["cmd"],"additionalProperties":false}},{"type":"custom","name":"apply_patch","description":"Apply a patch","format":{"type":"grammar","syntax":"lark","definition":"start: \"*** Begin Patch\" NEWLINE patch \"*** End Patch\""}}]}],"input":[{"role":"developer","content":[{"type":"input_text","text":"Follow repository instructions"}]},{"role":"user","content":[{"type":"input_text","text":"Fix a bug"}]},{"type":"custom_tool_call","id":"ctc1","call_id":"call1","namespace":"functions","name":"apply_patch","input":"*** Begin Patch\n*** End Patch"},{"type":"custom_tool_call_output","call_id":"call1","output":"Success"}]});
        let translated = translate(source.clone(), Protocol::Responses, Protocol::Messages);
        assert_eq!(
            translated["system"],
            "You are a coding agent\n\nFollow repository instructions"
        );
        assert_eq!(
            translated["messages"][1]["content"][0]["name"],
            "functions__apply_patch"
        );
        assert_eq!(
            translated["messages"][1]["content"][0]["input"]["input"],
            "*** Begin Patch\n*** End Patch"
        );
        assert_eq!(
            translated["messages"][2]["content"][0]["content"],
            "Success"
        );
        assert!(
            translated["tools"][1]["input_schema"]["properties"]["input"]["description"]
                .as_str()
                .unwrap()
                .contains("lark grammar")
        );
        assert_eq!(
            translated["tool_choice"]["disable_parallel_tool_use"],
            false
        );
        assert_eq!(translated["max_tokens"], 8192);
    }

    #[test]
    fn claude_thinking_context_edits_and_system_updates_translate() {
        let source = json!({"model":"gpt-5.4","max_tokens":8192,"system":[{"type":"text","text":"Instructions","cache_control":{"type":"ephemeral"}}],"stream":true,"stop_sequences":[],"thinking":{"type":"adaptive"},"output_config":{"effort":"max"},"context_management":{"edits":[{"type":"clear_thinking_20251015","keep":"all"}]},"messages":[{"role":"system","content":"Initial environment"},{"role":"user","content":"Fix it"},{"role":"assistant","content":[{"type":"thinking","thinking":"Investigate","signature":"signed_provider_state"},{"type":"text","text":"Working"}]},{"role":"system","content":"Environment changed"}]});
        let translated = translate(source.clone(), Protocol::Messages, Protocol::Responses);
        assert_eq!(
            translated["instructions"],
            "Instructions\n\nInitial environment"
        );
        assert_eq!(translated["reasoning"]["effort"], "xhigh");
        assert_eq!(translated["input"].as_array().unwrap().len(), 4);
        assert_eq!(translated["input"][1]["content"][0]["text"], "Investigate");
        assert_eq!(
            translated["input"][3]["content"][0]["text"],
            "System context update:\nEnvironment changed"
        );
        let mut without_clear = source;
        without_clear
            .as_object_mut()
            .unwrap()
            .remove("context_management");
        let translated = translate(without_clear, Protocol::Messages, Protocol::Responses);
        assert_eq!(translated["input"][1]["content"][0]["text"], "Investigate");
    }

    #[test]
    fn clear_thinking_retains_requested_recent_turns() {
        let source = json!({"model":"m","max_tokens":8192,"context_management":{"edits":[{"type":"clear_thinking_20251015","keep":{"type":"thinking_turns","value":1}}]},"messages":[{"role":"user","content":"First"},{"role":"assistant","content":[{"type":"thinking","thinking":"Old thought","signature":"s1"},{"type":"text","text":"First answer"}]},{"role":"user","content":"Second"},{"role":"assistant","content":[{"type":"thinking","thinking":"Recent thought","signature":"s2"},{"type":"text","text":"Second answer"}]}]});
        let translated = translate(source, Protocol::Messages, Protocol::Responses);
        let history = serde_json::to_string(&translated["input"]).unwrap();
        assert!(!history.contains("Old thought"));
        assert!(history.contains("Recent thought"));
        assert!(history.contains("First answer"));
        assert!(history.contains("Second answer"));
    }

    #[test]
    fn custom_tool_stream_unwraps_and_restores_argument_wrapper() {
        let original = json!({"model":"m","input":"patch","tools":[{"type":"namespace","name":"functions","tools":[{"type":"custom","name":"apply_patch","format":{"type":"grammar","syntax":"lark","definition":"start: WORD"}}]}]});
        let source = frames(&[
            json!({"type":"message_start","message":{"id":"msg1","model":"m","usage":{"input_tokens":3}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call1","name":"functions__apply_patch","input":{}}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"input\":\"*** Begin"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":" Patch\\n*** End Patch\"}"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":8}}),
            json!({"type":"message_stop"}),
        ]);
        let mut stream = StreamTranslator::with_request(
            Protocol::Messages,
            Protocol::Responses,
            &serde_json::to_vec(&original).unwrap(),
        )
        .unwrap();
        let mut translated = Vec::new();
        for chunk in source.chunks(3) {
            translated.extend(stream.push(chunk).unwrap());
        }
        stream.finish().unwrap();
        let output = events(&translated);
        let delta = output
            .iter()
            .find(|v| v["type"] == "response.custom_tool_call_input.delta")
            .unwrap();
        assert_eq!(delta["delta"], "*** Begin Patch\n*** End Patch");
        let native: Value =
            serde_json::from_slice(&collect_sse(&translated, Protocol::Responses).unwrap())
                .unwrap();
        assert_eq!(native["output"][0]["name"], "apply_patch");
        assert_eq!(native["output"][0]["namespace"], "functions");
        let mut reverse = StreamTranslator::new(Protocol::Responses, Protocol::Messages);
        let mut messages = Vec::new();
        for chunk in translated.chunks(2) {
            messages.extend(reverse.push(chunk).unwrap());
        }
        reverse.finish().unwrap();
        let native: Value =
            serde_json::from_slice(&collect_sse(&messages, Protocol::Messages).unwrap()).unwrap();
        assert_eq!(
            native["content"][0]["input"]["input"],
            "*** Begin Patch\n*** End Patch"
        );
        assert_eq!(native["content"][0]["name"], "functions__apply_patch");
    }

    #[test]
    fn kimi_cached_usage_is_not_double_counted() {
        let mut stream = StreamTranslator::new(Protocol::Messages, Protocol::Responses);
        stream.set_input_includes_cache(true);
        let output = stream.push(&frames(&[
            json!({"type":"message_start","message":{"id":"m","model":"kimi","usage":{"input_tokens":100,"cache_read_input_tokens":80}}}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":10}}),
            json!({"type":"message_stop"}),
        ])).unwrap();
        stream.finish().unwrap();
        let terminal = events(&output).pop().unwrap();
        assert_eq!(terminal["response"]["usage"]["input_tokens"], 100);
        assert_eq!(
            terminal["response"]["usage"]["input_tokens_details"]["cached_tokens"],
            80
        );
    }

    #[test]
    fn errors_and_length_limits_reach_client() {
        let detail = "Unsupported parameter: metadata";
        let translated: Value = serde_json::from_slice(
            &response(
                &serde_json::to_vec(&json!({"detail":detail})).unwrap(),
                Protocol::Responses,
                Protocol::Messages,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            translated,
            json!({"type":"error","error":{"type":"invalid_request_error","message":detail}})
        );
        let source = frames(&[
            json!({"type":"error","error":{"type":"overloaded_error","message":"Try again"}}),
        ]);
        let mut stream = StreamTranslator::new(Protocol::Messages, Protocol::Responses);
        let output = stream.push(&source).unwrap();
        stream.finish().unwrap();
        assert_eq!(events(&output)[0]["code"], "overloaded_error");
        assert!(collect_sse(&source, Protocol::Messages).is_err());
        let source = frames(&[
            json!({"type":"response.created","response":{"id":"r","model":"m"}}),
            json!({"type":"response.incomplete","response":{"id":"r","model":"m","status":"incomplete","output":[],"incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":5,"output_tokens":10}}}),
        ]);
        let mut stream = StreamTranslator::new(Protocol::Responses, Protocol::Messages);
        let output = stream.push(&source).unwrap();
        stream.finish().unwrap();
        assert_eq!(
            events(&output)
                .iter()
                .find(|e| e["type"] == "message_delta")
                .unwrap()["delta"]["stop_reason"],
            "max_tokens"
        );
    }

    #[test]
    fn native_responses_collection_accepts_builtin_items() {
        let final_response = json!({"id":"r","object":"response","model":"m","status":"completed","output":[{"type":"web_search_call","id":"ws1","status":"completed"}],"usage":{"input_tokens":10}});
        let source = frames(&[
            json!({"type":"response.output_item.added","output_index":0,"item":{"type":"web_search_call","id":"ws1"}}),
            json!({"type":"response.completed","response":final_response}),
        ]);
        let actual: Value =
            serde_json::from_slice(&collect_sse(&source, Protocol::Responses).unwrap()).unwrap();
        assert_eq!(actual, final_response);
    }

    #[test]
    fn json_response_roundtrip_preserves_tools_reasoning_and_usage() {
        let source = json!({"id":"msg1","type":"message","model":"m","role":"assistant","content":[{"type":"thinking","thinking":"Use the lookup tool","signature":"provider_signature"},{"type":"text","text":"Checking"},{"type":"tool_use","id":"call1","name":"lookup","input":{"query":"rust"}}],"stop_reason":"tool_use","stop_sequence":null,"usage":{"input_tokens":10,"cache_read_input_tokens":20,"cache_creation_input_tokens":30,"output_tokens":5}});
        let translated: Value = serde_json::from_slice(
            &response(
                &serde_json::to_vec(&source).unwrap(),
                Protocol::Messages,
                Protocol::Responses,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            translated["output"][0]["summary"][0]["text"],
            "Use the lookup tool"
        );
        let restored: Value = serde_json::from_slice(
            &response(
                &serde_json::to_vec(&translated).unwrap(),
                Protocol::Responses,
                Protocol::Messages,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            restored["content"][0]["thinking"],
            source["content"][0]["thinking"]
        );
        assert_eq!(restored["content"][1], source["content"][1]);
        assert_eq!(restored["content"][2], source["content"][2]);
        assert_eq!(restored["usage"], source["usage"]);
        assert_eq!(restored["stop_reason"], "tool_use");
    }

    #[test]
    fn streamed_visible_reasoning_and_multiple_text_parts_are_preserved() {
        let final_response = json!({"id":"r","model":"m","status":"completed","output":[{"id":"rs1","type":"reasoning","summary":[{"type":"summary_text","text":"Think through it"}],"encrypted_content":"provider_cipher"},{"id":"m1","type":"message","role":"assistant","content":[{"type":"output_text","text":"First"},{"type":"output_text","text":"Second"}]}],"usage":{"input_tokens":6,"output_tokens":9,"output_tokens_details":{"reasoning_tokens":4}}});
        let source = frames(&[
            json!({"type":"response.created","response":{"id":"r","model":"m"}}),
            json!({"type":"response.output_item.added","output_index":0,"item":{"id":"rs1","type":"reasoning","summary":[]}}),
            json!({"type":"response.reasoning_summary_part.added","output_index":0,"summary_index":0,"part":{"type":"summary_text","text":""}}),
            json!({"type":"response.reasoning_summary_text.delta","output_index":0,"summary_index":0,"delta":"Think through it"}),
            json!({"type":"response.reasoning_summary_part.done","output_index":0,"summary_index":0,"part":{"type":"summary_text","text":"Think through it"}}),
            json!({"type":"response.output_item.done","output_index":0,"item":final_response["output"][0]}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"id":"m1","type":"message","role":"assistant","content":[]}}),
            json!({"type":"response.content_part.added","output_index":1,"content_index":0,"part":{"type":"output_text","text":""}}),
            json!({"type":"response.output_text.delta","output_index":1,"content_index":0,"delta":"First"}),
            json!({"type":"response.content_part.done","output_index":1,"content_index":0,"part":{"type":"output_text","text":"First"}}),
            json!({"type":"response.content_part.added","output_index":1,"content_index":1,"part":{"type":"output_text","text":""}}),
            json!({"type":"response.output_text.delta","output_index":1,"content_index":1,"delta":"Second"}),
            json!({"type":"response.output_item.done","output_index":1,"item":final_response["output"][1]}),
            json!({"type":"response.completed","response":final_response}),
        ]);
        let mut stream = StreamTranslator::new(Protocol::Responses, Protocol::Messages);
        let mut translated = Vec::new();
        for chunk in source.chunks(1) {
            translated.extend(stream.push(chunk).unwrap());
        }
        stream.finish().unwrap();
        let collected: Value =
            serde_json::from_slice(&collect_sse(&translated, Protocol::Messages).unwrap()).unwrap();
        assert_eq!(collected["content"][0]["thinking"], "Think through it");
        assert_eq!(collected["content"][1]["text"], "First");
        assert_eq!(collected["content"][2]["text"], "Second");
        assert_eq!(collected["usage"]["output_tokens"], 9);
    }

    #[test]
    fn native_codex_cli_request_shape_requires_disabled_provider_web_search() {
        // Captured from codex-cli 0.162.0-alpha.17.2 against a local mock, using
        // temporary CODEX_HOME/workspace and synthetic credentials. Prompt/tool
        // prose and session identifiers are shortened; all wire fields remain.
        let mut source = json!({
            "model":"claude-sonnet-4-6",
            "stream":true,
            "input":[
                {"type":"message","role":"developer","content":[{"type":"input_text","text":"You are a coding agent running in the Codex CLI."}]},
                {"type":"message","role":"developer","content":[{"type":"input_text","text":"Available skills."},{"type":"input_text","text":"Use a read-only sandbox."}]},
                {"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>Temporary workspace</environment_context>"}]},
                {"type":"message","role":"user","content":[{"type":"input_text","text":"Reply with an acknowledgement."}]}
            ],
            "tools":[
                {"type":"function","name":"exec_command","description":"Execute a command","strict":false,"parameters":{"type":"object","properties":{"cmd":{"type":"string"}},"required":["cmd"]}},
                {"type":"function","name":"write_stdin","description":"Write to a session","strict":false,"parameters":{"type":"object","properties":{"session_id":{"type":"integer"}},"required":["session_id"]}},
                {"type":"namespace","name":"multi_agent_v1","description":"Multi-agent tools","tools":[{"type":"function","name":"wait_agent","description":"Wait for an agent","strict":false,"parameters":{"type":"object","properties":{"timeout_ms":{"type":"integer"}}}}]},
                {"type":"web_search","external_web_access":false}
            ],
            "tool_choice":"auto",
            "parallel_tool_calls":true,
            "reasoning":{"summary":"auto"},
            "store":false,
            "include":["reasoning.encrypted_content"],
            "prompt_cache_key":"synthetic-session",
            "client_metadata":{"session_id":"synthetic-session","root_turn_id":"synthetic-turn","x-codex-window-id":"synthetic-session:0","turn_id":"synthetic-turn","x-codex-installation-id":"synthetic-installation","x-codex-turn-metadata":"{\"session_id\":\"synthetic-session\"}","thread_id":"synthetic-session"}
        });
        let error = request(
            &serde_json::to_vec(&source).unwrap(),
            Protocol::Responses,
            Protocol::Messages,
        )
        .unwrap_err();
        assert!(error.to_string().contains("Responses tool web_search"));
        // Codex's web_search="disabled" config removes the provider tool entirely.
        source["tools"].as_array_mut().unwrap().pop();
        let translated = translate(source.clone(), Protocol::Responses, Protocol::Messages);
        assert_eq!(translated["messages"].as_array().unwrap().len(), 1);
        assert_eq!(
            translated["messages"][0]["content"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(translated["tools"][2]["name"], "multi_agent_v1__wait_agent");
        assert_eq!(translated["system"],"You are a coding agent running in the Codex CLI.\n\nAvailable skills.\n\nUse a read-only sandbox.");
        assert_eq!(
            translated["tool_choice"]["disable_parallel_tool_use"],
            false
        );
        assert!(translated.get("include").is_none());
        assert!(translated.get("client_metadata").is_none());
        let output = "Chunk ID: synthetic\nWall time: 0.0000 seconds\nProcess exited with code 0\nOriginal token count: 3\nOutput:\n/tmp/workspace\n";
        source["input"].as_array_mut().unwrap().extend([
            json!({"type":"function_call","id":"msg_mock_tool_0","name":"exec_command","arguments":"{\"cmd\":\"pwd\",\"max_output_tokens\":100}","call_id":"call_mock_pwd"}),
            json!({"type":"function_call_output","id":"fco_synthetic","call_id":"call_mock_pwd","output":output}),
        ]);
        let translated = translate(source, Protocol::Responses, Protocol::Messages);
        assert_eq!(translated["messages"].as_array().unwrap().len(), 3);
        assert_eq!(translated["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(
            translated["messages"][1]["content"][0]["input"],
            json!({"cmd":"pwd","max_output_tokens":100})
        );
        assert_eq!(
            translated["messages"][2]["content"][0]["type"],
            "tool_result"
        );
        assert_eq!(
            translated["messages"][2]["content"][0]["tool_use_id"],
            "call_mock_pwd"
        );
        assert_eq!(translated["messages"][2]["content"][0]["content"], output);
    }
}
