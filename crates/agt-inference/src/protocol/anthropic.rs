//! Anthropic Messages (`POST {base}/messages`, `anthropic-version: 2023-06-01`).
//!
//! The assistant's content blocks, including thinking and its signature, are kept
//! in `_anthropic_content` and sent back verbatim: tool use with thinking requires
//! the previous blocks unchanged. Like `_responses_output`, this lives only in the
//! run's in-memory transcript.
use super::openai::append;
use super::{
    Protocol, Request, StreamDecoder, arguments, assistant, endpoint, incomplete, provider_error,
    required_tool, text,
};
use crate::model::set_total_tokens;
use crate::{ModelEvent, ResolvedAgent, Result, RuntimeError};
use agt_model::ReasoningEffort;
use serde_json::{Map, Value, json};

pub(crate) struct Anthropic;

const VERSION: &str = "2023-06-01";
/// Anthropic requires an output limit; used when the model sets none.
const DEFAULT_MAX_TOKENS: u32 = 16_384;

impl Protocol for Anthropic {
    fn request(
        &self,
        base: &reqwest::Url,
        agent: &ResolvedAgent,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<Request> {
        let params = &agent.model.params;
        let (system, mut turns) = convert(messages)?;
        let required = required_tool(tools);
        // Forced tool use cannot be combined with extended thinking.
        let thinking = params
            .reasoning_effort
            .as_ref()
            .filter(|_| required.is_none())
            .and_then(budget);
        let mut max_tokens = params.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
        let mut body = json!({"model": agent.model.model, "stream": true});
        if let Some(budget) = thinking {
            // The thinking budget counts toward max_tokens and must stay below it.
            max_tokens = max_tokens.max(budget + 4096);
            body["thinking"] = json!({"type":"enabled","budget_tokens":budget});
        } else {
            body["temperature"] = json!(params.temperature.min(1.0));
        }
        body["max_tokens"] = json!(max_tokens);
        // Anthropic caches only up to explicit breakpoints: the system prompt, the
        // tools, and the newest turn, so each step reuses everything before it.
        if !system.is_empty() {
            body["system"] =
                json!([{"type":"text","text":system,"cache_control":{"type":"ephemeral"}}]);
        }
        if !tools.is_empty() {
            let mut converted: Vec<Value> = tools
                .iter()
                .map(|tool| {
                    let f = &tool["function"];
                    json!({"name":f["name"],"description":f["description"].as_str().unwrap_or(""),
                        "input_schema":f["parameters"]})
                })
                .collect();
            if let Some(last) = converted.last_mut() {
                last["cache_control"] = json!({"type":"ephemeral"});
            }
            body["tools"] = json!(converted);
        }
        if let Some(name) = required {
            body["tool_choice"] =
                json!({"type":"tool","name":name,"disable_parallel_tool_use":true});
        }
        if let Some(block) = turns
            .last_mut()
            .and_then(|turn| turn["content"].as_array_mut())
            .and_then(|blocks| blocks.last_mut())
            && block["type"] != "thinking"
            && block["type"] != "redacted_thinking"
        {
            block["cache_control"] = json!({"type":"ephemeral"});
        }
        body["messages"] = json!(turns);
        Ok(Request {
            url: endpoint(base, "messages"),
            headers: vec![
                ("x-api-key", agent.provider.api_key.clone()),
                ("anthropic-version", VERSION.into()),
            ],
            body,
        })
    }

    fn message(&self, response: &Value) -> Result<Value> {
        if response["type"] == "error" {
            return Err(provider_error());
        }
        let blocks = response["content"].as_array().ok_or_else(incomplete)?;
        finish(
            blocks.clone(),
            response["stop_reason"].as_str(),
            &response["usage"],
        )
    }

    fn decoder(&self) -> Box<dyn StreamDecoder> {
        Box::new(Stream::default())
    }

    fn context_rejected(&self, body: &Value) -> bool {
        rejected(body)
    }
}

/// Anthropic reports an oversized prompt as an invalid request whose message says so.
fn rejected(body: &Value) -> bool {
    body["error"]["type"] == "invalid_request_error"
        && body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("prompt is too long"))
        || crate::model::context_rejected(body)
}

fn budget(effort: &ReasoningEffort) -> Option<u32> {
    Some(match effort {
        ReasoningEffort::None => return None,
        ReasoningEffort::Minimal => 1024,
        ReasoningEffort::Low => 2048,
        ReasoningEffort::Medium => 8192,
        ReasoningEffort::High => 16_384,
        ReasoningEffort::XHigh => 32_768,
        ReasoningEffort::Max => 64_000,
        // A number is taken as the budget itself; other values use a medium budget.
        ReasoningEffort::Custom(value) => value.parse().ok().filter(|n| *n >= 1024).unwrap_or(8192),
    })
}

/// Internal messages to Anthropic's system text and alternating user/assistant turns.
fn convert(messages: &[Value]) -> Result<(String, Vec<Value>)> {
    let mut system = Vec::new();
    let mut turns: Vec<Value> = Vec::new();
    for message in messages {
        let (role, blocks) = match message["role"].as_str() {
            Some("system") => {
                system.push(text(&message["content"]));
                continue;
            }
            Some("user") => (
                "user",
                vec![json!({"type":"text","text":text(&message["content"])})],
            ),
            Some("tool") => (
                "user",
                vec![
                    json!({"type":"tool_result","tool_use_id":message["tool_call_id"],
                    "content":text(&message["content"])}),
                ],
            ),
            Some("assistant") => ("assistant", assistant_blocks(message)?),
            _ => return Err(RuntimeError::Model("unsupported message role".into())),
        };
        let blocks: Vec<Value> = blocks
            .into_iter()
            .filter(|b| b["type"] != "text" || b["text"].as_str().is_some_and(|t| !t.is_empty()))
            .collect();
        if blocks.is_empty() {
            continue;
        }
        // Consecutive messages of one role merge: tool results and a steering message
        // become one user turn, as the API requires strict alternation.
        match turns.last_mut() {
            Some(last) if last["role"] == role => last["content"]
                .as_array_mut()
                .expect("content blocks")
                .extend(blocks),
            _ => turns.push(json!({"role":role,"content":blocks})),
        }
    }
    Ok((system.join("\n\n"), turns))
}

fn assistant_blocks(message: &Value) -> Result<Vec<Value>> {
    if let Some(blocks) = message["_anthropic_content"].as_array() {
        return Ok(blocks.clone());
    }
    let mut blocks = vec![json!({"type":"text","text":text(&message["content"])})];
    for call in message["tool_calls"].as_array().into_iter().flatten() {
        blocks.push(
            json!({"type":"tool_use","id":call["id"],"name":call["function"]["name"],
            "input":arguments(call)?}),
        );
    }
    Ok(blocks)
}

/// A complete response's content blocks to the internal assistant message.
fn finish(blocks: Vec<Value>, stop: Option<&str>, usage: &Value) -> Result<Value> {
    if !matches!(stop, Some("end_turn" | "tool_use" | "stop_sequence")) {
        return Err(incomplete());
    }
    let mut content = String::new();
    let mut calls = Vec::new();
    for block in &blocks {
        match block["type"].as_str() {
            Some("text") => content.push_str(block["text"].as_str().unwrap_or("")),
            Some("tool_use") => calls.push(json!({"type":"function","id":block["id"],
                "function":{"name":block["name"],"arguments":block["input"].to_string()}})),
            Some("thinking" | "redacted_thinking") => {}
            _ => return Err(incomplete()),
        }
    }
    let mut message = assistant(content, calls);
    message["_anthropic_content"] = json!(blocks);
    let total = [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "output_tokens",
    ]
    .iter()
    .filter_map(|k| usage[*k].as_u64())
    .sum::<u64>();
    Ok(set_total_tokens(message, total))
}

#[derive(Default)]
struct Stream {
    blocks: Vec<Value>,
    /// Streamed tool input arrives as JSON text, parsed when the block ends.
    partial: Map<String, Value>,
    stop: Option<String>,
    usage: Map<String, Value>,
}

impl StreamDecoder for Stream {
    fn event(
        &mut self,
        event: &Value,
        progress: &mut dyn FnMut(ModelEvent<'_>) -> Result<()>,
    ) -> Result<Option<Value>> {
        let index = event["index"].as_u64().unwrap_or(0) as usize;
        match event["type"].as_str() {
            Some("message_start") => {
                if let Some(usage) = event["message"]["usage"].as_object() {
                    self.usage.extend(usage.clone());
                }
            }
            Some("content_block_start") => {
                if index != self.blocks.len() || index >= 64 {
                    return Err(RuntimeError::Model("invalid streamed content block".into()));
                }
                let mut block = event["content_block"].clone();
                match block["type"].as_str() {
                    Some("tool_use") => {
                        block["input"] = json!({});
                        progress(ModelEvent::Tool)?;
                    }
                    Some("thinking" | "redacted_thinking") => progress(ModelEvent::Reasoning)?,
                    Some("text") => {}
                    _ => return Err(incomplete()),
                }
                self.blocks.push(block);
            }
            Some("content_block_delta") => {
                let block = self
                    .blocks
                    .get_mut(index)
                    .ok_or_else(|| RuntimeError::Model("invalid streamed content block".into()))?;
                let delta = &event["delta"];
                match delta["type"].as_str() {
                    Some("text_delta") => {
                        let text = delta["text"].as_str().unwrap_or("");
                        append(&mut block["text"], text);
                        progress(ModelEvent::Text(text))?;
                    }
                    Some("input_json_delta") => {
                        let entry = self
                            .partial
                            .entry(index.to_string())
                            .or_insert_with(|| json!(""));
                        append(entry, delta["partial_json"].as_str().unwrap_or(""));
                        progress(ModelEvent::Tool)?;
                    }
                    // Kept for replay only; never shown or stored.
                    Some("thinking_delta") => {
                        append(
                            &mut block["thinking"],
                            delta["thinking"].as_str().unwrap_or(""),
                        );
                        progress(ModelEvent::Reasoning)?;
                    }
                    Some("signature_delta") => {
                        if block["signature"].is_null() {
                            block["signature"] = json!("");
                        }
                        append(
                            &mut block["signature"],
                            delta["signature"].as_str().unwrap_or(""),
                        );
                    }
                    _ => {}
                }
            }
            Some("content_block_stop") => {
                if let Some(text) = self.partial.remove(&index.to_string())
                    && let Some(block) = self.blocks.get_mut(index)
                {
                    let text = text.as_str().unwrap_or("");
                    block["input"] = if text.trim().is_empty() {
                        json!({})
                    } else {
                        serde_json::from_str(text).map_err(|e| {
                            RuntimeError::Model(crate::Reason::because(
                                "streamed tool input is not JSON",
                                e,
                            ))
                        })?
                    };
                }
            }
            Some("message_delta") => {
                if let Some(stop) = event["delta"]["stop_reason"].as_str() {
                    self.stop = Some(stop.to_owned());
                }
                if let Some(usage) = event["usage"].as_object() {
                    self.usage.extend(usage.clone());
                }
            }
            Some("message_stop") => {
                return finish(
                    std::mem::take(&mut self.blocks),
                    self.stop.as_deref(),
                    &Value::Object(self.usage.clone()),
                )
                .map(Some);
            }
            Some("error") => return Err(provider_error()),
            _ => {}
        }
        Ok(None)
    }

    fn finish(&mut self, _done: bool) -> Result<Value> {
        Err(RuntimeError::ModelTransient(
            "incomplete event stream".into(),
        ))
    }

    fn context_rejected(&self, event: &Value) -> bool {
        rejected(event)
    }
}

#[cfg(test)]
pub(super) fn convert_for_test(messages: &[Value]) -> Result<(String, Vec<Value>)> {
    convert(messages)
}
