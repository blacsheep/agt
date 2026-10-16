//! Model API protocols. The runtime speaks one internal message shape (Chat
//! Completions roles with `tool_calls`); each protocol converts it to its own
//! request, and its response back, so adding a protocol touches only this module.
use crate::{ModelEvent, ResolvedAgent, Result, RuntimeError};
use agt_model::ModelApiFormat;
use serde_json::{Value, json};

mod anthropic;
mod gemini;
mod openai;

/// One HTTP request for a model step.
pub(crate) struct Request {
    pub url: reqwest::Url,
    pub headers: Vec<(&'static str, String)>,
    pub body: Value,
}

pub(crate) trait Protocol {
    /// Endpoint, headers (including credentials) and body for one model step.
    fn request(
        &self,
        base: &reqwest::Url,
        agent: &ResolvedAgent,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<Request>;
    /// One complete assistant message from a non-streamed JSON response.
    fn message(&self, response: &Value) -> Result<Value>;
    /// Assembles a streamed response, one server-sent event at a time.
    fn decoder(&self) -> Box<dyn StreamDecoder>;
    /// Whether a provider error body says the prompt exceeded the context window.
    /// Only explicit provider signals count; HTTP 413 alone is not token overflow.
    fn context_rejected(&self, body: &Value) -> bool {
        crate::model::context_rejected(body)
    }
}

pub(crate) trait StreamDecoder {
    /// Handle one event. Returns the finished message once the stream is complete.
    fn event(
        &mut self,
        event: &Value,
        progress: &mut dyn FnMut(ModelEvent<'_>) -> Result<()>,
    ) -> Result<Option<Value>>;
    /// The `[DONE]` marker (`done == true`) or the end of the byte stream.
    fn finish(&mut self, done: bool) -> Result<Value>;
    fn context_rejected(&self, event: &Value) -> bool {
        crate::model::context_rejected(event)
    }
}

pub(crate) fn select(format: &ModelApiFormat) -> Box<dyn Protocol> {
    match format {
        ModelApiFormat::ChatCompletions => Box::new(openai::Chat),
        ModelApiFormat::Responses => Box::new(openai::Responses),
        ModelApiFormat::AnthropicMessages => Box::new(anthropic::Anthropic),
        ModelApiFormat::Gemini => Box::new(gemini::Gemini),
    }
}

#[cfg(test)]
pub(crate) fn decoder(responses: bool) -> Box<dyn StreamDecoder> {
    if responses {
        openai::Responses.decoder()
    } else {
        openai::Chat.decoder()
    }
}

/// `{base}/{path}` without doubling slashes.
fn endpoint(base: &reqwest::Url, path: &str) -> reqwest::Url {
    let mut url = base.clone();
    url.set_path(&format!("{}/{path}", base.path().trim_end_matches('/')));
    url
}

/// The single tool the runtime requires the model to call, if any.
fn required_tool(tools: &[Value]) -> Option<&str> {
    match tools {
        [tool] if tool["_agt_required"] == true => tool["function"]["name"].as_str(),
        _ => None,
    }
}

fn incomplete() -> RuntimeError {
    RuntimeError::Model("response was incomplete, refused or unsupported".into())
}

fn provider_error() -> RuntimeError {
    RuntimeError::ModelTransient("model returned an error".into())
}

/// Tool call arguments are a JSON object encoded as text in the internal shape.
fn arguments(call: &Value) -> Result<Value> {
    let text = call["function"]["arguments"].as_str().unwrap_or("{}");
    let text = if text.trim().is_empty() { "{}" } else { text };
    serde_json::from_str(text)
        .map_err(|e| RuntimeError::Model(crate::Reason::because("tool arguments must be JSON", e)))
}

/// The text of an internal message's content, which is a string or text parts.
fn text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn assistant(content: String, calls: Vec<Value>) -> Value {
    json!({"role":"assistant","content":content,"tool_calls":calls})
}

#[cfg(test)]
mod tests;
