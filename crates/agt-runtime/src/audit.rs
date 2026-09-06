use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CallRecord {
    pub id: i64,
    pub run_id: String,
    pub model_step: u32,
    pub kind: String,
    pub tool_name: Option<String>,
    pub argument_summary: Value,
    pub state: CallState,
    pub error: Option<Value>,
    pub started_at: u64,
    pub finished_at: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CallState {
    Started,
    Succeeded,
    Failed,
    Canceled,
}

impl CallRecord {
    pub(crate) fn new(
        run: &crate::RunRecord,
        tool_name: Option<&str>,
        argument_summary: Value,
    ) -> Self {
        Self {
            id: 0,
            run_id: run.id.clone(),
            model_step: run.model_steps,
            kind: if tool_name.is_some() { "tool" } else { "model" }.into(),
            tool_name: tool_name.map(str::to_owned),
            argument_summary,
            state: CallState::Started,
            error: None,
            started_at: now(),
            finished_at: None,
        }
    }
    pub(crate) fn finish(&mut self, state: CallState, error: Option<Value>) {
        self.state = state;
        self.error = error;
        self.finished_at = Some(now());
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Preserve the distinction that matters for debugging: absent/null/blank/type.
/// Values, unknown property names, array elements, and external tool keys are not logged.
pub(crate) fn argument_summary(value: &Value, known_fields: &[&str]) -> Value {
    fn shape(value: &Value) -> Value {
        match value {
            Value::Null => json!({"type":"null"}),
            Value::String(s) => {
                json!({"type":"string","bytes":s.len(),"blank":s.trim().is_empty()})
            }
            Value::Array(a) => json!({"type":"array","items":a.len()}),
            Value::Object(o) => json!({"type":"object","fields":o.len()}),
            Value::Bool(_) => json!({"type":"boolean"}),
            Value::Number(_) => json!({"type":"number"}),
        }
    }
    let mut result = shape(value);
    if let Some(object) = value.as_object() {
        let fields = known_fields
            .iter()
            .filter_map(|name| object.get(*name).map(|v| ((*name).to_owned(), shape(v))))
            .collect::<serde_json::Map<_, _>>();
        result["unknown_field_count"] = json!(object.len() - fields.len());
        result["fields"] = Value::Object(fields);
    }
    result
}

pub(crate) fn runtime_error(error: &crate::RuntimeError) -> Value {
    use crate::RuntimeError::*;
    let (code, message) = match error {
        Canceled => (
            "canceled",
            "Execution was canceled; inspect side effects before retrying.",
        ),
        Budget => ("budget_exhausted", "Model step budget exhausted."),
        Model(message) => ("model_error", *message),
        Tool(message) => ("tool_error", *message),
        HttpStatus(_) => (
            "model_http_error",
            "Model endpoint returned an unsuccessful HTTP status.",
        ),
        _ => (
            "execution_error",
            "Execution failed; inspect the host Run and current state before retrying.",
        ),
    };
    json!({"code":code,"message":message})
}
