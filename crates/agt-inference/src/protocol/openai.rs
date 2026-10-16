//! OpenAI Chat Completions and Responses.
use super::{
    Protocol, Request, StreamDecoder, endpoint, incomplete, provider_error, required_tool,
};
use crate::model::attach_usage;
use crate::{ModelEvent, ResolvedAgent, Result, RuntimeError};
use serde_json::{Value, json};

pub(crate) struct Chat;
pub(crate) struct Responses;

fn bearer(agent: &ResolvedAgent) -> Vec<(&'static str, String)> {
    vec![(
        "authorization",
        format!("Bearer {}", agent.provider.api_key),
    )]
}

impl Protocol for Chat {
    fn request(
        &self,
        base: &reqwest::Url,
        agent: &ResolvedAgent,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<Request> {
        let params = &agent.model.params;
        let mut body = json!({"messages": messages});
        if !tools.is_empty() {
            body["tools"] = json!(wire_tools(tools));
        }
        if let Some(name) = required_tool(tools) {
            body["tool_choice"] = json!({"type":"function","function":{"name":name}});
            body["parallel_tool_calls"] = json!(false);
        }
        body["model"] = json!(agent.model.model);
        body["stream"] = json!(true);
        body["stream_options"] = json!({"include_usage":true});
        if let Some(effort) = &params.reasoning_effort {
            body["reasoning_effort"] = json!(effort.as_str());
        } else {
            body["temperature"] = json!(params.temperature);
        }
        if let Some(limit) = params.max_tokens {
            body["max_completion_tokens"] = json!(limit);
        }
        Ok(Request {
            url: endpoint(base, "chat/completions"),
            headers: bearer(agent),
            body,
        })
    }

    fn message(&self, response: &Value) -> Result<Value> {
        if response["choices"].as_array().is_some_and(Vec::is_empty) {
            return Err(RuntimeError::ModelTransient("empty model response".into()));
        }
        let choice = &response["choices"][0];
        let message = &choice["message"];
        match choice["finish_reason"].as_str() {
            Some("stop" | "tool_calls") if message["refusal"].is_null() => {
                Ok(attach_usage(message.clone(), &response["usage"], false))
            }
            _ => Err(incomplete()),
        }
    }

    fn decoder(&self) -> Box<dyn StreamDecoder> {
        Box::new(ChatStream {
            message: json!({"role":"assistant","content":"","tool_calls":[]}),
            finish: None,
            usage: Value::Null,
        })
    }
}

impl Protocol for Responses {
    fn request(
        &self,
        base: &reqwest::Url,
        agent: &ResolvedAgent,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<Request> {
        let params = &agent.model.params;
        let mut body = responses_request(messages, tools)?;
        if let Some(name) = required_tool(tools) {
            body["tool_choice"] = json!({"type":"function","name":name});
            body["parallel_tool_calls"] = json!(false);
        }
        body["model"] = json!(agent.model.model);
        body["stream"] = json!(true);
        if let Some(effort) = &params.reasoning_effort {
            body["reasoning"] = json!({"effort": effort.as_str()});
        } else {
            body["temperature"] = json!(params.temperature);
        }
        if let Some(limit) = params.max_tokens {
            body["max_output_tokens"] = json!(limit);
        }
        Ok(Request {
            url: endpoint(base, "responses"),
            headers: bearer(agent),
            body,
        })
    }

    fn message(&self, response: &Value) -> Result<Value> {
        responses_message(response).map(|message| attach_usage(message, &response["usage"], true))
    }

    fn decoder(&self) -> Box<dyn StreamDecoder> {
        Box::new(ResponsesStream)
    }
}

// Internal request constraint; never send AGT metadata to the provider.
pub(crate) fn wire_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            let mut tool = tool.clone();
            if let Some(object) = tool.as_object_mut() {
                object.remove("_agt_required");
            }
            tool
        })
        .collect()
}

pub(crate) fn responses_request(messages: &[Value], tools: &[Value]) -> Result<Value> {
    let mut input = Vec::new();
    for message in messages {
        // Keep complete output (including reasoning and phase) in this run's
        // in-memory transcript, never in shared client state or audit records.
        if let Some(output) = message["_responses_output"].as_array() {
            input.extend(output.iter().cloned());
        } else if message["role"] == "tool" {
            input.push(json!({"type":"function_call_output", "call_id":message["tool_call_id"], "output":message["content"]}));
        } else {
            input.push(message.clone());
        }
    }
    let mut body = json!({"input":input,"store":false,"include":["reasoning.encrypted_content"]});
    if !tools.is_empty() {
        let converted = tools
            .iter()
            .map(|tool| {
                let mut function = tool["function"]
                    .as_object()
                    .cloned()
                    .ok_or(RuntimeError::Model("invalid function tool schema".into()))?;
                function.insert("type".into(), json!("function"));
                // Responses defaults to strict; preserve optional fields in existing
                // MCP/framework schemas unless the schema explicitly opts in.
                function.entry("strict").or_insert(json!(false));
                Ok(Value::Object(function))
            })
            .collect::<Result<Vec<_>>>()?;
        body["tools"] = json!(converted);
    }
    Ok(body)
}

pub fn responses_message(response: &Value) -> Result<Value> {
    if response["status"] != "completed" || !response["error"].is_null() {
        return Err(incomplete());
    }
    let output = response["output"].as_array().ok_or_else(incomplete)?;
    let mut content = String::new();
    let mut calls = Vec::new();
    for item in output {
        if !item["status"].is_null() && item["status"] != "completed" {
            return Err(incomplete());
        }
        match item["type"].as_str() {
            Some("reasoning") => {}
            Some("message") if item["role"] == "assistant" => {
                for part in item["content"].as_array().ok_or_else(incomplete)? {
                    if part["type"] != "output_text" {
                        return Err(incomplete());
                    }
                    content.push_str(part["text"].as_str().ok_or_else(incomplete)?);
                }
            }
            Some("function_call") => calls.push(json!({"type":"function", "id":item["call_id"],
                "function":{"name":item["name"],"arguments":item["arguments"]}})),
            _ => return Err(incomplete()),
        }
    }
    Ok(json!({"role":"assistant","content":content,"tool_calls":calls,"_responses_output":output}))
}

struct ChatStream {
    message: Value,
    finish: Option<String>,
    usage: Value,
}

impl StreamDecoder for ChatStream {
    fn event(
        &mut self,
        value: &Value,
        progress: &mut dyn FnMut(ModelEvent<'_>) -> Result<()>,
    ) -> Result<Option<Value>> {
        if !value["usage"].is_null() {
            self.usage = value["usage"].clone();
        }
        for choice in value["choices"].as_array().into_iter().flatten() {
            if choice["index"].as_u64().unwrap_or(0) != 0 {
                continue;
            }
            let delta = &choice["delta"];
            if (!delta["role"].is_null() && delta["role"] != "assistant")
                || (!delta["content"].is_null() && !delta["content"].is_string())
                || (!delta["tool_calls"].is_null() && !delta["tool_calls"].is_array())
            {
                return Err(RuntimeError::Model(
                    "invalid streamed assistant message".into(),
                ));
            }
            if !delta["refusal"].is_null() {
                return Err(incomplete());
            }
            if let Some(text) = delta["content"].as_str() {
                append(&mut self.message["content"], text);
                progress(ModelEvent::Text(text))?;
            }
            // Never copy provider-private reasoning into progress or audit storage.
            if delta["reasoning_content"].is_string() || delta["reasoning"].is_string() {
                progress(ModelEvent::Reasoning)?;
            }
            for call in delta["tool_calls"].as_array().into_iter().flatten() {
                if !call["type"].is_null() && call["type"] != "function" {
                    return Err(RuntimeError::Model("unsupported streamed tool type".into()));
                }
                let index = call["index"]
                    .as_u64()
                    .filter(|i| *i < 32)
                    .ok_or(RuntimeError::Model("invalid streamed tool index".into()))?
                    as usize;
                let calls = self.message["tool_calls"].as_array_mut().expect("array");
                while calls.len() <= index {
                    calls.push(
                        json!({"type":"function","id":"","function":{"name":"","arguments":""}}),
                    );
                }
                if let Some(text) = call["id"].as_str() {
                    append(&mut calls[index]["id"], text);
                }
                for field in ["name", "arguments"] {
                    if let Some(text) = call["function"][field].as_str() {
                        append(&mut calls[index]["function"][field], text);
                    }
                }
                progress(ModelEvent::Tool)?;
            }
            if let Some(reason) = choice["finish_reason"].as_str() {
                if !matches!(reason, "stop" | "tool_calls") {
                    return Err(incomplete());
                }
                self.finish = Some(reason.to_owned());
            }
        }
        Ok(None)
    }

    fn finish(&mut self, done: bool) -> Result<Value> {
        if done && matches!(self.finish.as_deref(), Some("stop" | "tool_calls")) {
            return Ok(attach_usage(self.message.take(), &self.usage, false));
        }
        Err(RuntimeError::ModelTransient(
            "incomplete event stream".into(),
        ))
    }
}

struct ResponsesStream;

impl StreamDecoder for ResponsesStream {
    fn event(
        &mut self,
        value: &Value,
        progress: &mut dyn FnMut(ModelEvent<'_>) -> Result<()>,
    ) -> Result<Option<Value>> {
        match value["type"].as_str() {
            Some("response.completed") => {
                return responses_message(&value["response"])
                    .map(|m| Some(attach_usage(m, &value["response"]["usage"], true)));
            }
            Some("response.failed") => return Err(provider_error()),
            Some("response.incomplete") => return Err(incomplete()),
            Some("response.output_text.delta") => {
                if let Some(text) = value["delta"].as_str() {
                    progress(ModelEvent::Text(text))?;
                }
            }
            Some("response.reasoning_summary_text.delta" | "response.reasoning_text.delta") => {
                progress(ModelEvent::Reasoning)?
            }
            Some("response.output_item.added") if value["item"]["type"] == "reasoning" => {
                progress(ModelEvent::Reasoning)?
            }
            Some("response.function_call_arguments.delta") => progress(ModelEvent::Tool)?,
            _ => {}
        }
        Ok(None)
    }

    fn finish(&mut self, _done: bool) -> Result<Value> {
        Err(RuntimeError::ModelTransient(
            "incomplete event stream".into(),
        ))
    }
}

pub(super) fn append(value: &mut Value, text: &str) {
    if let Value::String(value) = value {
        value.push_str(text);
    }
}
