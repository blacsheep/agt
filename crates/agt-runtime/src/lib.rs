//! Explicit, bounded agent execution. IM remains a runtime-independent store.
mod engine;
mod model;
mod resolve;
mod store;

pub use engine::Runtime;
pub use model::{ChatCompletions, ModelClient};
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
    #[error("model HTTP status {0}")]
    HttpStatus(u16),
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
    pub max_steps: u32,
    pub working_directory: std::path::PathBuf,
}

impl RunRequest {
    pub fn new(agent: impl Into<String>, input: impl Into<String>) -> Self {
        Self {
            agent: agent.into(),
            input: input.into(),
            request_key: uuid::Uuid::new_v4().to_string(),
            delegates: Vec::new(),
            max_steps: 12,
            working_directory: std::path::PathBuf::from("."),
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
        if self.input.trim().is_empty() || self.input.len() > 256 * 1024 {
            return Err(RuntimeError::Invalid(
                "input must contain 1..262144 bytes".into(),
            ));
        }
        if self.request_key.trim().is_empty() || self.request_key.len() > 256 {
            return Err(RuntimeError::Invalid(
                "request key must contain 1..256 bytes".into(),
            ));
        }
        if self.max_steps == 0 || self.max_steps > 100 {
            return Err(RuntimeError::Invalid("max_steps must be in 1..=100".into()));
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
