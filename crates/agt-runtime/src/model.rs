use std::io::Read;
use std::time::Duration;

use agt_core::CancellationToken;
use agt_model::ModelApiFormat;
use serde_json::{Value, json};

use crate::{ResolvedAgent, Result, RuntimeError};

/// Public text deltas and operational phases; never raw private reasoning.
pub enum ModelEvent<'a> {
    Attempt(u32),
    Retrying,
    Text(&'a str),
    Reasoning,
    Tool,
}

/// Returns one complete assistant message (content and/or function tool_calls).
/// Implementations must report truncated/refused responses as errors.
pub trait ModelClient {
    fn complete_stream(
        &mut self,
        agent: &ResolvedAgent,
        messages: &[Value],
        tools: &[Value],
        _progress: &mut dyn FnMut(ModelEvent<'_>) -> Result<()>,
    ) -> Result<Value> {
        self.complete(agent, messages, tools)
    }

    /// Retry waits must observe hosted cancellation as well as the caller token.
    fn check_canceled(&self) -> Result<()> {
        Ok(())
    }
    fn complete(
        &mut self,
        agent: &ResolvedAgent,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<Value>;
}

/// Retries one logical model step, never tools or the whole Run.
pub(crate) fn complete_with_retry(
    model: &mut dyn ModelClient,
    agent: &ResolvedAgent,
    messages: &[Value],
    tools: &[Value],
    cancellation: &CancellationToken,
    attempts: &mut u32,
) -> Result<Value> {
    complete_with_progress(
        model,
        agent,
        messages,
        tools,
        cancellation,
        attempts,
        &mut |_| Ok(()),
    )
}

pub(crate) fn complete_with_progress(
    model: &mut dyn ModelClient,
    agent: &ResolvedAgent,
    messages: &[Value],
    tools: &[Value],
    cancellation: &CancellationToken,
    attempts: &mut u32,
    progress: &mut dyn FnMut(ModelEvent<'_>) -> Result<()>,
) -> Result<Value> {
    const MAX_ATTEMPTS: u32 = 10;
    for attempt in 1..=MAX_ATTEMPTS {
        check_retry_canceled(model, cancellation)?;
        *attempts = attempt;
        progress(ModelEvent::Attempt(attempt))?;
        let result = model
            .complete_stream(agent, messages, tools, progress)
            .and_then(|message| {
                if message["content"]
                    .as_str()
                    .is_none_or(|s| s.trim().is_empty())
                    && message["tool_calls"].as_array().is_none_or(Vec::is_empty)
                {
                    Err(RuntimeError::ModelTransient("empty model response".into()))
                } else {
                    Ok(message)
                }
            });
        check_retry_canceled(model, cancellation)?;
        match result {
            Ok(message) => return Ok(message),
            Err(error) if retryable(&error) => {
                if attempt == MAX_ATTEMPTS {
                    return Err(RuntimeError::ModelRetriesExhausted {
                        attempts: attempt,
                        last: Box::new(error),
                    });
                }
                progress(ModelEvent::Retrying)?;
                // Bounded exponential backoff, interruptible between requests.
                let delay = Duration::from_millis((250u64 << (attempt - 1)).min(4000));
                let until = std::time::Instant::now() + delay;
                while let Some(remaining) = until.checked_duration_since(std::time::Instant::now())
                {
                    check_retry_canceled(model, cancellation)?;
                    std::thread::sleep(remaining.min(Duration::from_millis(50)));
                }
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}
fn check_retry_canceled(model: &dyn ModelClient, token: &CancellationToken) -> Result<()> {
    if token.is_canceled() {
        return Err(RuntimeError::Canceled);
    }
    model.check_canceled()
}
/// Only explicit provider codes trigger compaction; HTTP 413 alone is not token overflow.
pub(crate) fn context_rejected(value: &Value) -> bool {
    [&value["error"], &value["response"]["error"], value]
        .iter()
        .any(|error| {
            error["code"] == "context_length_exceeded" || error["type"] == "context_length_exceeded"
        })
}

fn retryable(error: &RuntimeError) -> bool {
    matches!(
        error,
        RuntimeError::HttpStatus(_) | RuntimeError::ModelTransient(_)
    )
}

/// Routes HTTP requests by the model's API format; retains its historical name.
#[derive(Default)]
pub struct ChatCompletions;

impl ModelClient for ChatCompletions {
    fn complete(
        &mut self,
        agent: &ResolvedAgent,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<Value> {
        self.complete_stream(agent, messages, tools, &mut |_| Ok(()))
    }
    fn complete_stream(
        &mut self,
        agent: &ResolvedAgent,
        messages: &[Value],
        tools: &[Value],
        progress: &mut dyn FnMut(ModelEvent<'_>) -> Result<()>,
    ) -> Result<Value> {
        let mut url = reqwest::Url::parse(&agent.provider.base_url).map_err(|e| {
            RuntimeError::Configuration(crate::Reason::because("invalid provider URL", e))
        })?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(RuntimeError::Configuration(
                "provider URL must be HTTP(S) without credentials, query or fragment".into(),
            ));
        }
        let responses = agent.model.api_format == ModelApiFormat::Responses;
        let endpoint = if responses {
            "responses"
        } else {
            "chat/completions"
        };
        url.set_path(&format!("{}/{endpoint}", url.path().trim_end_matches('/')));
        let params = &agent.model.params;
        let mut body = if responses {
            responses_request(messages, tools)?
        } else {
            let mut body = json!({"messages": messages});
            if !tools.is_empty() {
                body["tools"] = json!(wire_tools(tools));
            }
            body
        };
        apply_tool_choice(&mut body, tools, responses);
        body["model"] = json!(agent.model.model);
        body["stream"] = json!(true);
        if !responses {
            body["stream_options"] = json!({"include_usage":true});
        }
        if let Some(effort) = &params.reasoning_effort {
            if responses {
                body["reasoning"] = json!({"effort": effort.as_str()});
            } else {
                body["reasoning_effort"] = json!(effort.as_str());
            }
        } else {
            body["temperature"] = json!(params.temperature);
        }
        if let Some(limit) = params.max_tokens {
            body[if responses {
                "max_output_tokens"
            } else {
                "max_completion_tokens"
            }] = json!(limit);
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(u64::from(params.timeout)))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| {
                RuntimeError::Model(crate::Reason::because("cannot create HTTP client", e))
            })?;
        let response = client
            .post(url)
            .bearer_auth(&agent.provider.api_key)
            .json(&body)
            .send()
            .map_err(|e| {
                RuntimeError::ModelTransient(crate::Reason::because(
                    "HTTP request failed or timed out",
                    e,
                ))
            })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            // Inspect a bounded structured error, never persist provider bodies or credentials.
            let mut bytes = Vec::new();
            if response.take(65537).read_to_end(&mut bytes).is_ok()
                && bytes.len() <= 65536
                && let Ok(body) = serde_json::from_slice::<Value>(&bytes)
                && context_rejected(&body)
            {
                return Err(RuntimeError::ContextWindowRejected);
            }
            return Err(RuntimeError::HttpStatus(status));
        }
        if response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .eq_ignore_ascii_case("text/event-stream")
            })
        {
            return crate::model_stream::read(response, responses, progress);
        }
        // Compatible providers may ignore stream=true and return a single JSON response.
        const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;
        let mut bytes = Vec::new();
        response
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| {
                RuntimeError::ModelTransient(crate::Reason::because("cannot read HTTP response", e))
            })?;
        if bytes.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(RuntimeError::Model("response exceeds 4 MiB".into()));
        }
        let response: Value = serde_json::from_slice(&bytes).map_err(|e| {
            RuntimeError::ModelTransient(crate::Reason::because("invalid JSON response", e))
        })?;
        if context_rejected(&response) {
            return Err(RuntimeError::ContextWindowRejected);
        }
        if !response["error"].is_null() {
            return Err(RuntimeError::ModelTransient(
                "model returned an error".into(),
            ));
        }
        if response["choices"].as_array().is_some_and(Vec::is_empty)
            || response.is_null()
            || response.as_object().is_some_and(serde_json::Map::is_empty)
        {
            return Err(RuntimeError::ModelTransient("empty model response".into()));
        }
        if responses {
            return responses_message(&response)
                .map(|message| attach_usage(message, &response["usage"], true));
        }
        let choice = &response["choices"][0];
        let message = &choice["message"];
        match choice["finish_reason"].as_str() {
            Some("stop" | "tool_calls") if message["refusal"].is_null() => {
                Ok(attach_usage(message.clone(), &response["usage"], false))
            }
            _ => Err(RuntimeError::Model(
                "response was incomplete, refused or unsupported".into(),
            )),
        }
    }
}

// Host-only accounting. Runtime consumes/removes this before appending to history.
pub(crate) fn attach_usage(mut message: Value, usage: &Value, responses: bool) -> Value {
    let total = usage["total_tokens"].as_u64().or_else(|| {
        let (input, output) = if responses {
            ("input_tokens", "output_tokens")
        } else {
            ("prompt_tokens", "completion_tokens")
        };
        Some(
            usage[input]
                .as_u64()?
                .saturating_add(usage[output].as_u64()?),
        )
    });
    // Ignore nonsensical zero totals; cached input is already included in input/total.
    if let Some(total) = total.filter(|n| *n > 0) {
        message["_agt_total_tokens"] = json!(total);
    }
    message
}

// Internal request constraint; never send AGT metadata to the provider.
fn wire_tools(tools: &[Value]) -> Vec<Value> {
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
fn apply_tool_choice(body: &mut Value, tools: &[Value], responses: bool) {
    if let [tool] = tools
        && tool["_agt_required"] == true
    {
        let name = &tool["function"]["name"];
        body["tool_choice"] = if responses {
            json!({"type":"function","name":name})
        } else {
            json!({"type":"function","function":{"name":name}})
        };
        body["parallel_tool_calls"] = json!(false);
    }
}

fn responses_request(messages: &[Value], tools: &[Value]) -> Result<Value> {
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

pub(crate) fn responses_message(response: &Value) -> Result<Value> {
    let invalid = || RuntimeError::Model("response was incomplete, refused or unsupported".into());
    if response["status"] != "completed" || !response["error"].is_null() {
        return Err(invalid());
    }
    let output = response["output"].as_array().ok_or_else(invalid)?;
    let mut content = String::new();
    let mut calls = Vec::new();
    for item in output {
        if !item["status"].is_null() && item["status"] != "completed" {
            return Err(invalid());
        }
        match item["type"].as_str() {
            Some("reasoning") => {}
            Some("message") if item["role"] == "assistant" => {
                for part in item["content"].as_array().ok_or_else(invalid)? {
                    if part["type"] != "output_text" {
                        return Err(invalid());
                    }
                    content.push_str(part["text"].as_str().ok_or_else(invalid)?);
                }
            }
            Some("function_call") => calls.push(json!({"type":"function", "id":item["call_id"],
                "function":{"name":item["name"],"arguments":item["arguments"]}})),
            _ => return Err(invalid()),
        }
    }
    Ok(json!({"role":"assistant","content":content,"tool_calls":calls,"_responses_output":output}))
}

#[cfg(test)]
mod usage_tests {
    use super::*;
    #[test]
    fn protocol_usage_counts_cached_input_once_and_omissions_fall_back() {
        let message = json!({"role":"assistant","content":"ok"});
        assert_eq!(
            attach_usage(
                message.clone(),
                &json!({"total_tokens":101,"prompt_tokens":100,"completion_tokens":1,"prompt_tokens_details":{"cached_tokens":80}}),
                false
            )["_agt_total_tokens"],
            101
        );
        assert_eq!(
            attach_usage(
                message.clone(),
                &json!({"input_tokens":100,"output_tokens":2,"input_tokens_details":{"cached_tokens":80}}),
                true
            )["_agt_total_tokens"],
            102
        );
        for missing in [
            Value::Null,
            json!({}),
            json!({"total_tokens":0}),
            json!({"total_tokens":-1}),
        ] {
            assert!(
                attach_usage(message.clone(), &missing, false)
                    .get("_agt_total_tokens")
                    .is_none()
            );
        }
    }
}

#[cfg(test)]
#[path = "tests/context/provider_errors.rs"]
mod provider_errors;

#[cfg(test)]
mod required_tool_tests {
    use super::*;
    #[test]
    fn forced_tool_is_encoded_for_both_apis_without_private_metadata() {
        let tools = vec![
            json!({"type":"function","_agt_required":true,"function":{"name":"participation_decision","parameters":{"type":"object"},"strict":false}}),
        ];
        for responses in [false, true] {
            let mut body = if responses {
                responses_request(&[], &tools).unwrap()
            } else {
                json!({"tools":wire_tools(&tools)})
            };
            apply_tool_choice(&mut body, &tools, responses);
            assert_eq!(body["parallel_tool_calls"], false);
            assert_eq!(
                body["tool_choice"],
                if responses {
                    json!({"type":"function","name":"participation_decision"})
                } else {
                    json!({"type":"function","function":{"name":"participation_decision"}})
                }
            );
            assert!(!body.to_string().contains("_agt_required"));
        }
        let ordinary = wire_tools(&tools);
        let mut body = json!({});
        apply_tool_choice(&mut body, &ordinary, false);
        assert!(body.get("tool_choice").is_none());
    }
}
