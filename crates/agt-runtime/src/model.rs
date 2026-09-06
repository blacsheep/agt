use std::io::Read;
use std::time::Duration;

use serde_json::{Value, json};

use crate::{ResolvedAgent, Result, RuntimeError};

/// Returns one assistant message (content and/or function tool_calls).
/// Implementations must report truncated/refused responses as errors.
pub trait ModelClient {
    fn complete(
        &mut self,
        agent: &ResolvedAgent,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<Value>;
}

#[derive(Default)]
pub struct ChatCompletions;

impl ModelClient for ChatCompletions {
    fn complete(
        &mut self,
        agent: &ResolvedAgent,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<Value> {
        let mut url = reqwest::Url::parse(&agent.provider.base_url)
            .map_err(|_| RuntimeError::Configuration("invalid provider URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(RuntimeError::Configuration(
                "provider URL must be HTTP(S) without credentials, query or fragment",
            ));
        }
        url.set_path(&format!(
            "{}/chat/completions",
            url.path().trim_end_matches('/')
        ));
        let params = &agent.model.params;
        let mut body = json!({"model": agent.model.model, "messages": messages, "stream": false});
        if let Some(effort) = params.reasoning_effort {
            body["reasoning_effort"] = json!(effort.as_str());
        } else {
            body["temperature"] = json!(params.temperature);
        }
        if let Some(limit) = params.max_tokens {
            body["max_completion_tokens"] = json!(limit);
        }
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(u64::from(params.timeout)))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| RuntimeError::Model("cannot create HTTP client"))?;
        let response = client
            .post(url)
            .bearer_auth(&agent.provider.api_key)
            .json(&body)
            .send()
            .map_err(|_| RuntimeError::Model("HTTP request failed or timed out"))?;
        if !response.status().is_success() {
            return Err(RuntimeError::HttpStatus(response.status().as_u16()));
        }
        const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;
        let mut bytes = Vec::new();
        response
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| RuntimeError::Model("cannot read HTTP response"))?;
        if bytes.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(RuntimeError::Model("response exceeds 4 MiB"));
        }
        let response: Value = serde_json::from_slice(&bytes)
            .map_err(|_| RuntimeError::Model("invalid JSON response"))?;
        let choice = &response["choices"][0];
        let message = &choice["message"];
        match choice["finish_reason"].as_str() {
            Some("stop" | "tool_calls") if message["refusal"].is_null() => Ok(message.clone()),
            _ => Err(RuntimeError::Model(
                "response was incomplete, refused or unsupported",
            )),
        }
    }
}
