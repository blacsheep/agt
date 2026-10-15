//! Protocol assembly only. Partial tools never reach the execution loop.
use crate::{ModelEvent, Result, RuntimeError};
use serde_json::{Value, json};
use std::cell::Cell;
use std::io::{BufRead, BufReader, Read};
use std::time::{Duration, Instant};

/// `stall` bounds the silence between output deltas. Reasoning may be silent for
/// minutes, but once text or a tool call is streaming, heartbeats or empty events
/// alone must not keep a dead stream open.
pub fn read(
    input: impl Read,
    responses: bool,
    stall: Duration,
    progress: &mut dyn FnMut(ModelEvent<'_>) -> Result<()>,
) -> Result<Value> {
    let last_output = Cell::new(None::<Instant>);
    let progress = &mut |event: ModelEvent<'_>| {
        match event {
            ModelEvent::Text(_) | ModelEvent::Tool => last_output.set(Some(Instant::now())),
            ModelEvent::Reasoning => last_output.set(None),
            _ => {}
        }
        progress(event)
    };
    const LIMIT: u64 = 4 * 1024 * 1024;
    let mut reader = BufReader::new(input.take(LIMIT + 1));
    let mut line = String::new();
    let mut data = String::new();
    let mut total = 0;
    let mut message = json!({"role":"assistant","content":"","tool_calls":[]});
    let mut finish = None;
    let mut usage = Value::Null;
    loop {
        line.clear();
        let n = reader.read_line(&mut line).map_err(|e| {
            RuntimeError::ModelTransient(crate::Reason::because("cannot read HTTP response", e))
        })?;
        total += n as u64;
        if total > LIMIT {
            return Err(RuntimeError::Model("response exceeds 4 MiB".into()));
        }
        if n == 0 {
            return Err(RuntimeError::ModelTransient(
                "incomplete event stream".into(),
            ));
        }
        if last_output.get().is_some_and(|at| at.elapsed() > stall) {
            return Err(RuntimeError::ModelTransient(
                "event stream stalled after output began".into(),
            ));
        }
        let field = line.trim_end_matches(['\r', '\n']);
        if !field.is_empty() {
            if let Some(value) = field.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value.strip_prefix(' ').unwrap_or(value));
            }
            continue;
        }
        if data.is_empty() {
            continue;
        }
        let event = std::mem::take(&mut data);
        if event == "[DONE]" {
            if !responses && matches!(finish.as_deref(), Some("stop" | "tool_calls")) {
                return Ok(super::model::attach_usage(message, &usage, false));
            }
            return Err(RuntimeError::ModelTransient(
                "incomplete event stream".into(),
            ));
        }
        let value: Value = serde_json::from_str(&event).map_err(|e| {
            RuntimeError::ModelTransient(crate::Reason::because("invalid JSON response", e))
        })?;
        if super::model::context_rejected(&value) {
            return Err(RuntimeError::ContextWindowRejected);
        }
        if !value["error"].is_null() || value["type"] == "error" {
            return Err(RuntimeError::ModelTransient(
                "model returned an error".into(),
            ));
        }
        if responses {
            match value["type"].as_str() {
                Some("response.completed") => {
                    return super::model::responses_message(&value["response"])
                        .map(|m| super::model::attach_usage(m, &value["response"]["usage"], true));
                }
                Some("response.failed") => {
                    return Err(RuntimeError::ModelTransient(
                        "model returned an error".into(),
                    ));
                }
                Some("response.incomplete") => {
                    return Err(RuntimeError::Model(
                        "response was incomplete, refused or unsupported".into(),
                    ));
                }
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
        } else {
            if !value["usage"].is_null() {
                usage = value["usage"].clone();
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
                    return Err(RuntimeError::Model(
                        "response was incomplete, refused or unsupported".into(),
                    ));
                }
                if let Some(text) = delta["content"].as_str() {
                    append(&mut message["content"], text);
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
                    let calls = message["tool_calls"].as_array_mut().expect("array");
                    while calls.len() <= index {
                        calls.push(json!({"type":"function","id":"","function":{"name":"","arguments":""}}));
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
                        return Err(RuntimeError::Model(
                            "response was incomplete, refused or unsupported".into(),
                        ));
                    }
                    finish = Some(reason.to_owned());
                }
            }
        }
    }
}

fn append(value: &mut Value, text: &str) {
    if let Value::String(value) = value {
        value.push_str(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SECS_60: Duration = Duration::from_secs(60);
    fn frame(v: Value) -> String {
        format!("data: {v}\r\n\r\n")
    }
    struct Bytes<'a>(&'a [u8]);
    impl Read for Bytes<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.0.is_empty() || out.is_empty() {
                return Ok(0);
            }
            out[0] = self.0[0];
            self.0 = &self.0[1..];
            Ok(1)
        }
    }
    #[test]
    fn split_utf8_tools_usage_and_private_reasoning() {
        let mut input = ": heartbeat\n\ndata: {\ndata: \"choices\": []}\n\n".to_owned();
        for delta in [
            json!({"reasoning_content":"PRIVATE"}),
            json!({"content":"你好"}),
            json!({"tool_calls":[{"index":0,"id":"call","function":{"name":"search","arguments":"{\"q\":"}}]}),
            json!({"tool_calls":[{"index":0,"function":{"arguments":"\"中文\"}"}}]}),
        ] {
            input += &frame(json!({"choices":[{"index":0,"delta":delta}]}));
        }
        input += &frame(json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}));
        input += &frame(json!({"choices":[],"usage":{"total_tokens":52}}));
        input += "data: [DONE]\n\n";
        let mut preview = String::new();
        let value = read(Bytes(input.as_bytes()), false, SECS_60, &mut |event| {
            if let ModelEvent::Text(text) = event {
                preview.push_str(text);
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(preview, "你好");
        assert_eq!(
            value["tool_calls"][0]["function"]["arguments"],
            "{\"q\":\"中文\"}"
        );
        assert_eq!(value["_agt_total_tokens"], 52);
        assert!(!value.to_string().contains("PRIVATE"));
    }
    #[test]
    fn responses_complete_items_are_authoritative_and_private_items_only_in_transcript() {
        let output = json!([{"type":"reasoning","encrypted_content":"opaque"},{"type":"message","role":"assistant","content":[{"type":"output_text","text":"完成"}]},{"type":"function_call","call_id":"c","name":"tool","arguments":"{}"}]);
        let input = frame(json!({"type":"response.output_text.delta","delta":"完成"}))
            + &frame(
                json!({"type":"response.completed","response":{"status":"completed","output":output,"usage":{"input_tokens":5,"output_tokens":2}}}),
            );
        let mut preview = String::new();
        let value = read(input.as_bytes(), true, SECS_60, &mut |event| {
            if let ModelEvent::Text(text) = event {
                preview.push_str(text);
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(preview, "完成");
        assert_eq!(value["_agt_total_tokens"], 7);
        assert_eq!(value["_responses_output"], output);
        assert_eq!(value["tool_calls"][0]["id"], "c");
    }
    #[test]
    fn truncated_refused_and_canceled_streams_do_not_produce_a_message() {
        let partial = frame(json!({"choices":[{"delta":{"content":"partial"}}]}));
        assert!(matches!(
            read(partial.as_bytes(), false, SECS_60, &mut |_| Ok(())),
            Err(RuntimeError::ModelTransient(reason))
                if reason.message() == "incomplete event stream"
        ));
        assert!(matches!(
            read(partial.as_bytes(), false, SECS_60, &mut |_| Err(
                RuntimeError::Canceled
            )),
            Err(RuntimeError::Canceled)
        ));
        for event in [
            json!({"choices":[{"delta":{"refusal":"no"}}]}),
            json!({"choices":[{"finish_reason":"length"}]}),
            json!({"error":{"message":"private error"}}),
        ] {
            assert!(read(frame(event).as_bytes(), false, SECS_60, &mut |_| Ok(())).is_err());
        }
        for kind in ["response.failed", "response.incomplete"] {
            assert!(
                read(
                    frame(json!({"type":kind})).as_bytes(),
                    true,
                    SECS_60,
                    &mut |_| Ok(())
                )
                .is_err()
            );
        }
    }
    #[test]
    fn heartbeats_do_not_keep_a_stalled_output_stream_open() {
        let text = frame(json!({"choices":[{"index":0,"delta":{"content":"[["}}]}));
        let stalled = text.clone() + ": heartbeat\n\n" + &frame(json!({"choices":[]}));
        assert!(matches!(
            read(stalled.as_bytes(), false, Duration::ZERO, &mut |_| Ok(())),
            Err(RuntimeError::ModelTransient(reason))
                if reason.message() == "event stream stalled after output began"
        ));
        // Silence while reasoning is not a stall, even with a zero bound.
        let thinking = frame(json!({"choices":[{"index":0,"delta":{"reasoning_content":"…"}}]}))
            + ": heartbeat\n\n"
            + &frame(json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}))
            + "data: [DONE]\n\n";
        assert!(read(thinking.as_bytes(), false, Duration::ZERO, &mut |_| Ok(())).is_ok());
    }
}

#[cfg(test)]
mod context_errors;
