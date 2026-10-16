//! Running one agt framework tool call for an engine that reaches agt over MCP.
use crate::Event;
use anyhow::Result;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(crate) type Validators = BTreeMap<String, agt_core::tool_schema::ToolSchema>;

pub(crate) fn validators(tools: &[crate::Tool]) -> Result<Validators> {
    Ok(tools
        .iter()
        .map(|tool| {
            agt_core::tool_schema::ToolSchema::compile(&tool.input_schema)
                .map(|schema| (tool.name.clone(), schema))
        })
        .collect::<std::result::Result<_, _>>()?)
}

/// MCP tool descriptors for `tools/list`.
pub(crate) fn descriptors(tools: &[crate::Tool]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| json!({"name":t.name,"description":t.description,"inputSchema":t.input_schema}))
        .collect()
}

/// Validate and run the call, report it as framework activity, and return an MCP
/// `CallToolResult`. A failing tool is a result for the model, not an engine error.
pub(crate) fn call(
    id: &str,
    name: &str,
    arguments: Value,
    validators: &Validators,
    event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    tool: &mut dyn FnMut(&str, Value) -> Result<Value>,
) -> Result<Value> {
    event(Event::ToolDetail {
        id,
        item: &json!({"arguments": arguments}),
    })?;
    event(Event::Activity {
        tool_name: None,
        id,
        kind: "framework",
        status: "running",
    })?;
    let outcome = match validators.get(name) {
        Some(validator) => validator
            .validate(&arguments)
            .map_err(anyhow::Error::from)
            .and_then(|()| tool(name, arguments)),
        None => Err(anyhow::anyhow!("tool unavailable")),
    };
    let success = outcome.is_ok();
    let payload = match &outcome {
        Ok(value) => json!({"result": value}),
        Err(error) => json!({"error": error.to_string()}),
    };
    event(Event::ToolDetail { id, item: &payload })?;
    event(Event::Activity {
        tool_name: None,
        id,
        kind: "framework",
        status: if success { "completed" } else { "failed" },
    })?;
    let text = match outcome {
        Ok(value) => value.to_string(),
        Err(error) => error.to_string(),
    };
    Ok(json!({"content":[{"type":"text","text":text}],"isError":!success}))
}
