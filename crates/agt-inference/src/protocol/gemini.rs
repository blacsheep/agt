//! Google Gemini (`POST {base}/models/{model}:streamGenerateContent?alt=sse`).
//!
//! Gemini function calls carry no IDs of their own and may carry a
//! `thoughtSignature` that must be sent back unchanged. The model's parts are kept
//! in `_gemini_parts` for that, in the run's in-memory transcript only, and calls
//! get IDs here so tool results can find their function name again.
use super::{
    Protocol, Request, StreamDecoder, arguments, assistant, endpoint, incomplete, provider_error,
    required_tool, text,
};
use crate::model::set_total_tokens;
use crate::{ModelEvent, ResolvedAgent, Result, RuntimeError};
use agt_model::ReasoningEffort;
use serde_json::{Value, json};
use std::collections::HashMap;

pub(crate) struct Gemini;

impl Protocol for Gemini {
    fn request(
        &self,
        base: &reqwest::Url,
        agent: &ResolvedAgent,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<Request> {
        let params = &agent.model.params;
        let (system, contents) = convert(messages)?;
        let mut body = json!({"contents": contents});
        if !system.is_empty() {
            body["systemInstruction"] = json!({"parts":[{"text":system}]});
        }
        if !tools.is_empty() {
            // `parametersJsonSchema` takes JSON Schema as is; `parameters` accepts only
            // an OpenAPI subset that rejects common MCP schemas.
            let declarations: Vec<Value> = tools
                .iter()
                .map(|tool| {
                    let f = &tool["function"];
                    json!({"name":f["name"],"description":f["description"].as_str().unwrap_or(""),
                        "parametersJsonSchema":f["parameters"]})
                })
                .collect();
            body["tools"] = json!([{"functionDeclarations": declarations}]);
        }
        if let Some(name) = required_tool(tools) {
            body["toolConfig"] =
                json!({"functionCallingConfig":{"mode":"ANY","allowedFunctionNames":[name]}});
        }
        let mut config = json!({"temperature": params.temperature});
        if let Some(limit) = params.max_tokens {
            config["maxOutputTokens"] = json!(limit);
        }
        if let Some(budget) = params.reasoning_effort.as_ref().map(budget) {
            config["thinkingConfig"] = json!({"thinkingBudget": budget});
        }
        body["generationConfig"] = config;
        let model = agent.model.model.trim_start_matches("models/");
        let mut url = endpoint(base, &format!("models/{model}:streamGenerateContent"));
        url.set_query(Some("alt=sse"));
        Ok(Request {
            url,
            headers: vec![("x-goog-api-key", agent.provider.api_key.clone())],
            body,
        })
    }

    fn message(&self, response: &Value) -> Result<Value> {
        // A non-streamed reply is a JSON array of chunks, or a single chunk.
        let chunks = match response {
            Value::Array(chunks) => chunks.clone(),
            chunk => vec![chunk.clone()],
        };
        let mut stream = Stream::default();
        for chunk in &chunks {
            stream.event(chunk, &mut |_| Ok(()))?;
        }
        stream.finish(true)
    }

    fn decoder(&self) -> Box<dyn StreamDecoder> {
        Box::new(Stream::default())
    }

    fn context_rejected(&self, body: &Value) -> bool {
        rejected(body)
    }
}

/// Gemini reports an oversized prompt as INVALID_ARGUMENT with this message.
fn rejected(body: &Value) -> bool {
    let error = match body {
        Value::Array(items) => &items.first().unwrap_or(&Value::Null)["error"],
        body => &body["error"],
    };
    error["status"] == "INVALID_ARGUMENT"
        && error["message"]
            .as_str()
            .is_some_and(|m| m.contains("exceeds the maximum number of tokens"))
        || crate::model::context_rejected(body)
}

fn budget(effort: &ReasoningEffort) -> i64 {
    match effort {
        ReasoningEffort::None => 0,
        ReasoningEffort::Minimal => 512,
        ReasoningEffort::Low => 2048,
        ReasoningEffort::Medium => 8192,
        ReasoningEffort::High => 16_384,
        ReasoningEffort::XHigh => 24_576,
        // -1 asks the model to choose its own budget.
        ReasoningEffort::Max => -1,
        ReasoningEffort::Custom(value) => value.parse().unwrap_or(-1),
    }
}

/// Internal messages to Gemini's system text and alternating user/model contents.
fn convert(messages: &[Value]) -> Result<(String, Vec<Value>)> {
    let mut system = Vec::new();
    let mut contents: Vec<Value> = Vec::new();
    // Tool results name their function; internal tool messages only carry the call ID.
    let mut names: HashMap<String, Value> = HashMap::new();
    for message in messages {
        let (role, parts) = match message["role"].as_str() {
            Some("system") => {
                system.push(text(&message["content"]));
                continue;
            }
            Some("user") => ("user", vec![json!({"text":text(&message["content"])})]),
            Some("tool") => {
                let id = message["tool_call_id"].as_str().unwrap_or("");
                let name = names.get(id).cloned().ok_or_else(|| {
                    RuntimeError::Model("tool result without a matching call".into())
                })?;
                (
                    "user",
                    vec![json!({"functionResponse":{"name":name,
                        "response":{"content":text(&message["content"])}}})],
                )
            }
            Some("assistant") => {
                for call in message["tool_calls"].as_array().into_iter().flatten() {
                    if let Some(id) = call["id"].as_str() {
                        names.insert(id.to_owned(), call["function"]["name"].clone());
                    }
                }
                ("model", model_parts(message)?)
            }
            _ => return Err(RuntimeError::Model("unsupported message role".into())),
        };
        let parts: Vec<Value> = parts
            .into_iter()
            .filter(|p| {
                p.get("text")
                    .is_none_or(|t| t.as_str().is_some_and(|t| !t.is_empty()))
            })
            .collect();
        if parts.is_empty() {
            continue;
        }
        match contents.last_mut() {
            Some(last) if last["role"] == role => {
                last["parts"].as_array_mut().expect("parts").extend(parts)
            }
            _ => contents.push(json!({"role":role,"parts":parts})),
        }
    }
    Ok((system.join("\n\n"), contents))
}

fn model_parts(message: &Value) -> Result<Vec<Value>> {
    if let Some(parts) = message["_gemini_parts"].as_array() {
        return Ok(parts.clone());
    }
    let mut parts = vec![json!({"text":text(&message["content"])})];
    for call in message["tool_calls"].as_array().into_iter().flatten() {
        parts.push(
            json!({"functionCall":{"name":call["function"]["name"],"args":arguments(call)?}}),
        );
    }
    Ok(parts)
}

#[derive(Default)]
struct Stream {
    parts: Vec<Value>,
    finish: Option<String>,
    total: u64,
}

impl StreamDecoder for Stream {
    fn event(
        &mut self,
        chunk: &Value,
        progress: &mut dyn FnMut(ModelEvent<'_>) -> Result<()>,
    ) -> Result<Option<Value>> {
        if !chunk["error"].is_null() {
            return Err(provider_error());
        }
        if chunk["promptFeedback"]["blockReason"].is_string() {
            return Err(incomplete());
        }
        if let Some(total) = chunk["usageMetadata"]["totalTokenCount"].as_u64() {
            self.total = total;
        }
        let candidate = &chunk["candidates"][0];
        for part in candidate["content"]["parts"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if part["thought"] == true {
                // A thought summary: kept for replay, never shown.
                progress(ModelEvent::Reasoning)?;
            } else if let Some(text) = part["text"].as_str() {
                progress(ModelEvent::Text(text))?;
            } else if part["functionCall"].is_object() {
                progress(ModelEvent::Tool)?;
            }
            self.parts.push(part.clone());
        }
        if let Some(reason) = candidate["finishReason"].as_str() {
            self.finish = Some(reason.to_owned());
        }
        Ok(None)
    }

    /// Gemini streams have no end marker: the stream simply ends after the last chunk.
    fn finish(&mut self, _done: bool) -> Result<Value> {
        match self.finish.as_deref() {
            Some("STOP") => {}
            None => {
                return Err(RuntimeError::ModelTransient(
                    "incomplete event stream".into(),
                ));
            }
            Some(_) => return Err(incomplete()),
        }
        let mut content = String::new();
        let mut calls = Vec::new();
        for part in &self.parts {
            if part["thought"] == true {
                continue;
            }
            if let Some(text) = part["text"].as_str() {
                content.push_str(text);
            } else if let Some(call) = part.get("functionCall") {
                let id = call["id"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("gemini_call_{}", uuid::Uuid::new_v4().simple()));
                calls.push(json!({"type":"function","id":id,
                    "function":{"name":call["name"],"arguments":call.get("args").filter(|a| a.is_object()).cloned().unwrap_or_else(|| json!({})).to_string()}}));
            }
        }
        let mut message = assistant(content, calls);
        message["_gemini_parts"] = json!(std::mem::take(&mut self.parts));
        Ok(set_total_tokens(message, self.total))
    }

    fn context_rejected(&self, event: &Value) -> bool {
        rejected(event)
    }
}

#[cfg(test)]
pub(super) fn convert_for_test(messages: &[Value]) -> Result<(String, Vec<Value>)> {
    convert(messages)
}
