//! Explicit, bounded agent execution. IM remains a runtime-independent store.
mod audit;
mod context;
mod context_store;
mod conversation_context;
pub use context::ContextCheckpoint;
mod engine;
mod model;
mod model_stream;
mod participation;
mod progress;
mod resolve;
mod schedule_store;
mod scheduler;
mod store;
pub use audit::{CallRecord, CallState};
pub use schedule_store::{Discussion, ScheduleStatus, ScheduleStore, ScheduledTask};
pub use scheduler::{
    Scheduler, delete_scheduled_conversation, dissolve_scheduled_group, reset_conversation_context,
    start_discussion,
};

pub use engine::Runtime;
pub use model::{ChatCompletions, ModelClient, ModelEvent};
pub use progress::RunProgress;
pub use resolve::ResolvedAgent;
pub use store::{RunRecord, RunState, RunStore};

use agt_core::CategoryName;
use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, RuntimeError>;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("configuration: {0}")]
    Configuration(&'static str),
    #[error("model: {0}")]
    Model(&'static str),
    #[error(
        "context overflow at {stage}: {tokens} tokens exceed available budget {budget}; compaction not installed"
    )]
    ContextOverflow {
        stage: &'static str,
        tokens: usize,
        budget: usize,
    },
    #[error("model HTTP status {0}")]
    HttpStatus(u16),
    #[error("model request failed after {attempts} attempts: {last}")]
    ModelRetriesExhausted {
        attempts: u32,
        last: Box<RuntimeError>,
    },
    #[error("tool: {0}")]
    Tool(&'static str),
    #[error("IM: {0}")]
    Im(&'static str),
    #[error("request key already belongs to a different request")]
    Conflict,
    #[error("run not found")]
    NotFound,
    #[error("run is still running or was interrupted; inspect it before a new attempt")]
    Busy,
    #[error("execution canceled")]
    Canceled,
    #[error("shared model-step budget exhausted")]
    Budget,
    #[error("runtime storage: {0}")]
    Storage(#[from] rusqlite::Error),
    #[error("runtime serialization: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunRequest {
    pub agent: String,
    pub input: String,
    pub request_key: String,
    pub delegates: Vec<String>,
    /// Legacy compatibility field; ignored by execution.
    #[serde(default)]
    pub max_steps: u32,
    pub working_directory: std::path::PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<MessageTrigger>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct MessageTrigger {
    pub message_id: agt_im::MessageId,
    pub conversation_id: agt_im::ConversationId,
    pub principal: agt_im::PrincipalId,
    pub depth: u32,
    pub allow_mcp: bool,
}

impl RunRequest {
    pub fn new(agent: impl Into<String>, input: impl Into<String>) -> Self {
        Self {
            agent: agent.into(),
            input: input.into(),
            request_key: uuid::Uuid::new_v4().to_string(),
            delegates: Vec::new(),
            max_steps: 0,
            working_directory: std::path::PathBuf::from("."),
            trigger: None,
        }
    }

    fn normalize(mut self) -> Result<Self> {
        self.agent = canonical_agent(&self.agent)?;
        self.working_directory = std::fs::canonicalize(&self.working_directory)
            .map_err(|_| RuntimeError::Invalid("working directory does not exist".into()))?;
        if !self.working_directory.is_dir() {
            return Err(RuntimeError::Invalid(
                "working directory must be a directory".into(),
            ));
        }
        self.delegates = self
            .delegates
            .iter()
            .map(|v| canonical_agent(v))
            .collect::<Result<_>>()?;
        self.delegates.sort();
        self.delegates.dedup();
        if self.delegates.contains(&self.agent) {
            return Err(RuntimeError::Invalid(
                "self delegation is not allowed".into(),
            ));
        }
        let input_limit = if self.trigger.is_some() {
            1024 * 1024
        } else {
            256 * 1024
        };
        if self.input.trim().is_empty() || self.input.len() > input_limit {
            return Err(RuntimeError::Invalid(format!(
                "input must contain 1..{input_limit} bytes"
            )));
        }
        if self.request_key.trim().is_empty() || self.request_key.len() > 256 {
            return Err(RuntimeError::Invalid(
                "request key must contain 1..256 bytes".into(),
            ));
        }
        Ok(self)
    }
}

pub(crate) fn canonical_agent(raw: &str) -> Result<String> {
    let (category, name) = raw.split_once('/').unwrap_or(("default", raw));
    let category = CategoryName::new(category)
        .map_err(|_| RuntimeError::Invalid("invalid agent category".into()))?;
    agt_agent::validate_agent_name(name)
        .map_err(|_| RuntimeError::Invalid("invalid agent name".into()))?;
    Ok(format!("{category}/{name}"))
}

pub(crate) fn fingerprint(value: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(value))
}
