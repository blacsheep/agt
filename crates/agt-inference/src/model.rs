use std::io::Read;
use std::time::Duration;

use agt_core::CancellationToken;
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
pub fn complete_with_retry(
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

pub fn complete_with_progress(
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
pub fn context_rejected(value: &Value) -> bool {
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
        let base = reqwest::Url::parse(&agent.provider.base_url).map_err(|e| {
            RuntimeError::Configuration(crate::Reason::because("invalid provider URL", e))
        })?;
        if !matches!(base.scheme(), "http" | "https")
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(RuntimeError::Configuration(
                "provider URL must be HTTP(S) without credentials, query or fragment".into(),
            ));
        }
        let protocol = crate::protocol::select(&agent.model.api_format);
        let request = protocol.request(&base, agent, messages, tools)?;
        let params = &agent.model.params;
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(u64::from(params.timeout)))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| {
                RuntimeError::Model(crate::Reason::because("cannot create HTTP client", e))
            })?;
        let mut builder = client.post(request.url).json(&request.body);
        for (name, value) in &request.headers {
            builder = builder.header(*name, value);
        }
        let response = builder.send().map_err(|e| {
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
                && protocol.context_rejected(&body)
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
            return crate::model_stream::read_with(
                response,
                protocol.decoder(),
                Duration::from_secs(u64::from(params.timeout)),
                progress,
            );
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
        if protocol.context_rejected(&response) {
            return Err(RuntimeError::ContextWindowRejected);
        }
        if !response["error"].is_null() {
            return Err(RuntimeError::ModelTransient(
                "model returned an error".into(),
            ));
        }
        if response.is_null() || response.as_object().is_some_and(serde_json::Map::is_empty) {
            return Err(RuntimeError::ModelTransient("empty model response".into()));
        }
        protocol.message(&response)
    }
}

// Host-only accounting. Runtime consumes/removes this before appending to history.
pub fn attach_usage(message: Value, usage: &Value, responses: bool) -> Value {
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
    set_total_tokens(message, total.unwrap_or(0))
}

/// Record the provider's token count for this step; zero means unknown.
pub fn set_total_tokens(mut message: Value, total: u64) -> Value {
    if total > 0 {
        message["_agt_total_tokens"] = json!(total);
    }
    message
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
mod provider_errors;
