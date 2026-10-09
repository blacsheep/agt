//! Explicit, bounded agent execution. IM remains a runtime-independent store.
pub mod approval;
mod audit;
mod context_store;
mod conversation_context;
mod conversation_instructions;
mod engine;
pub mod events;
mod harness;
mod harness_input;
mod harness_sessions;
mod permission;
mod run_inputs;
mod string_enum;
pub use harness_sessions::{
    ExecutionState as HarnessExecutionState, HarnessSession, HarnessSessions, SessionExecution,
};
mod local_execution;
pub use local_execution::LocalExecution;
mod participation;
mod progress;
mod schedule_state;
mod schedule_store;
mod scheduler;
mod store;
mod termination;
pub use termination::Termination;
pub mod work;
pub use audit::{CallRecord, CallState};
pub use schedule_store::{
    Discussion, DiscussionState, ScheduleStatus, ScheduleStore, ScheduledTask, TaskState,
};
pub use scheduler::{
    Scheduler, delete_scheduled_conversation, dissolve_scheduled_group, reset_conversation_context,
    start_discussion,
};

pub use engine::Runtime;
pub use progress::RunProgress;
// Model I/O lives in agt-inference; these paths keep the existing runtime API.
pub use agt_inference::{
    ChatCompletions, ContextCheckpoint, ModelClient, ModelEvent, Reason, ResolvedAgent, Result,
    RuntimeError, StopCode,
};
pub(crate) use agt_inference::{canonical_agent, context, fingerprint, model};
pub use store::{RunRecord, RunState, RunStore};

use serde::{Deserialize, Serialize};

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

mod context_history;
pub use context_history::context_reset_history;

mod harness_control;
pub use harness_control::SessionControl;

mod tool_details;
mod tool_progress;
pub use tool_details::{ToolDetail, ToolDetailField};
