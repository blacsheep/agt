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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_summary: Option<String>,
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
            public_label: None,
            public_summary: None,
            argument_summary,
            state: CallState::Started,
            error: None,
            started_at: now(),
            finished_at: None,
        }
    }
    pub(crate) fn with_public_context(mut self, args: &Value) -> Self {
        let name = self
            .tool_name
            .as_deref()
            .unwrap_or("")
            .trim_start_matches("framework_");
        self.public_label = match name {
            "harness_models" | "harness_session_models" => Some("查询可用模型"),
            "harness_session_get" => Some("读取执行会话"),
            "local_exec" => Some("执行本机命令"),
            "harness_session_configure" => Some("修改会话模型"),
            "resource_write" => Some(match args["kind"].as_str() {
                Some("harness") => "配置 Harness 连接",
                Some("agent") => "配置 Agent",
                Some("model") => "配置模型",
                _ => "配置资源",
            }),
            "agent_bind" => Some("绑定并启用 Agent"),
            "memory_search" => Some("检索历史记忆"),
            "memory_save" => Some("记录主题记忆"),
            "memory_read" => Some("读取记忆来源"),
            "memory_forget" => Some("移除主题记忆"),
            "task_execution_policy" => Some("安排任务并行与互斥"),
            "task_request" => Some("安排协作任务"),
            _ => None,
        }
        .map(str::to_owned);
        self
    }
    pub(crate) fn summarize_result(&mut self, value: &Value) {
        let name = self
            .tool_name
            .as_deref()
            .unwrap_or("")
            .trim_start_matches("framework_");
        self.public_summary = match name {
            // Fixed text, so the console can translate it.
            "harness_models" | "harness_session_models" => {
                value["data"].is_array().then(|| "已读取模型列表".into())
            }
            "harness_session_configure" => Some("会话模型已更新".into()),
            _ => None,
        };
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
    if let ModelRetriesExhausted { attempts, last } = error {
        let mut detail = runtime_error(last);
        detail["attempts"] = json!(attempts);
        detail["message"] = json!(format!(
            "{} Failed after {} attempts.",
            detail["message"]
                .as_str()
                .unwrap_or("Model request failed."),
            attempts
        ));
        return detail;
    }
    if let ContextOverflow {
        stage,
        tokens,
        budget,
    } = error
    {
        return json!({"code":"context_overflow","stage":stage,"estimated_tokens":tokens,"available_tokens":budget,"message":error.to_string()});
    }
    if matches!(error, ContextWindowRejected) {
        return json!({"code":"provider_context_exceeded","message":error.to_string()});
    }
    if let HttpStatus(status) = error {
        return json!({"code":"model_http_error","http_status":status,"message":format!("Model endpoint returned HTTP {status}.")});
    }
    if let Stopped(code) = error {
        return json!({"code":code,"source":code.source(),"message":error.to_string()});
    }
    let (code, message) = match error {
        Canceled => (
            "canceled",
            "Execution was canceled; inspect side effects before retrying.",
        ),
        Budget => ("budget_exhausted", "Model step budget exhausted."),
        Model(reason) | ModelTransient(reason) => ("model_error", reason.message()),
        Tool(reason) => ("tool_error", reason.message()),
        _ => (
            "execution_error",
            "Execution failed; inspect the host Run and current state before retrying.",
        ),
    };
    json!({"code":code,"message":message})
}
