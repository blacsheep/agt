//! Runtime error and stop vocabulary shared by inference and the execution host.
use agt_core::CategoryName;
use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, RuntimeError>;

/// A fixed public message plus an optional private cause. Display shows only the
/// message; the cause is kept for local diagnostics and never sent to models or the Web.
#[derive(Debug)]
pub struct Reason {
    message: &'static str,
    cause: Option<anyhow::Error>,
}

impl Reason {
    pub fn because(message: &'static str, cause: impl Into<anyhow::Error>) -> Self {
        Self {
            message,
            cause: Some(cause.into()),
        }
    }

    pub fn message(&self) -> &'static str {
        self.message
    }
}

impl From<&'static str> for Reason {
    fn from(message: &'static str) -> Self {
        Self {
            message,
            cause: None,
        }
    }
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("configuration: {0}")]
    Configuration(Reason),
    #[error("model: {0}")]
    Model(Reason),
    /// Transport or response failures that may succeed on retry.
    #[error("model: {0}")]
    ModelTransient(Reason),
    #[error(
        "context overflow at {stage}: {tokens} tokens exceed available budget {budget}; compaction not installed"
    )]
    ContextOverflow {
        stage: &'static str,
        tokens: usize,
        budget: usize,
    },
    #[error(
        "model rejected context as too large; shorten fixed input or verify model context_window"
    )]
    ContextWindowRejected,
    #[error("model HTTP status {0}")]
    HttpStatus(u16),
    #[error("model request failed after {attempts} attempts: {last}")]
    ModelRetriesExhausted {
        attempts: u32,
        last: Box<RuntimeError>,
    },
    #[error("tool: {0}")]
    Tool(Reason),
    #[error("IM: {0}")]
    Im(Reason),
    #[error("request key already belongs to a different request")]
    Conflict,
    #[error("run not found")]
    NotFound,
    #[error("run is still running or was interrupted; inspect it before a new attempt")]
    Busy,
    #[error("execution stopped: {0}")]
    Stopped(StopCode),
    #[error("execution canceled")]
    Canceled,
    #[error("shared model-step budget exhausted")]
    Budget,
    #[error("runtime storage: {0}")]
    Storage(#[from] rusqlite::Error),
    #[error("runtime serialization: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl RuntimeError {
    /// Private cause chain for local diagnostics, if the public message hides one.
    pub fn diagnostic(&self) -> Option<String> {
        match self {
            Self::Configuration(reason)
            | Self::Model(reason)
            | Self::ModelTransient(reason)
            | Self::Tool(reason)
            | Self::Im(reason) => reason.cause.as_ref().map(|cause| format!("{cause:#}")),
            Self::ModelRetriesExhausted { last, .. } => last.diagnostic(),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopCode {
    CancellationRequested,
    TaskCanceled,
    DiscussionStopped,
    ContextReset,
    AuthorizationRevoked,
    ServiceShutdown,
    SchedulerLeaseLost,
    ExecutionOwnerLost,
    HarnessLeaseLost,
    HarnessTimeout,
    ConnectionLost,
    EngineFailed,
    EngineInterrupted,
    ProtocolError,
    ResultMissing,
    ResultPersistenceFailed,
}
impl std::fmt::Display for StopCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            serde_json::to_value(self).unwrap().as_str().unwrap()
        )
    }
}
impl StopCode {
    pub fn source(self) -> &'static str {
        match self {
            Self::TaskCanceled => "task_control",
            Self::CancellationRequested | Self::ServiceShutdown => "runtime",
            Self::DiscussionStopped
            | Self::ContextReset
            | Self::AuthorizationRevoked
            | Self::SchedulerLeaseLost
            | Self::ExecutionOwnerLost => "scheduler",
            _ => "harness",
        }
    }
    pub fn token(token: &agt_core::CancellationToken) -> Self {
        match token.reason() {
            Some("service_shutdown") => Self::ServiceShutdown,
            Some("scheduler_lease_lost") => Self::SchedulerLeaseLost,
            Some("task_canceled") => Self::TaskCanceled,
            Some("authorization_revoked") => Self::AuthorizationRevoked,
            _ => Self::CancellationRequested,
        }
    }
    /// Stops requested by a person or control action rather than a failure.
    pub fn canceled(self) -> bool {
        matches!(
            self,
            Self::CancellationRequested
                | Self::TaskCanceled
                | Self::DiscussionStopped
                | Self::ContextReset
                | Self::AuthorizationRevoked
                | Self::ServiceShutdown
        )
    }
}

/// Normalize `name` or `category/name` to the canonical agent reference.
pub fn canonical_agent(raw: &str) -> Result<String> {
    let (category, name) = raw.split_once('/').unwrap_or(("default", raw));
    let category = CategoryName::new(category)
        .map_err(|_| RuntimeError::Invalid("invalid agent category".into()))?;
    agt_agent::validate_agent_name(name)
        .map_err(|_| RuntimeError::Invalid("invalid agent name".into()))?;
    Ok(format!("{category}/{name}"))
}

pub fn fingerprint(value: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(value))
}

#[cfg(test)]
mod reason_tests {
    use super::*;

    #[test]
    fn cause_stays_out_of_public_message() {
        let cause = anyhow::anyhow!("secret path /home/x/.agt").context("open failed");
        let error = RuntimeError::Im(Reason::because("cannot open message store", cause));
        assert_eq!(error.to_string(), "IM: cannot open message store");
        let diagnostic = error.diagnostic().unwrap();
        assert!(diagnostic.contains("open failed") && diagnostic.contains("secret path"));
        let retried = RuntimeError::ModelRetriesExhausted {
            attempts: 3,
            last: Box::new(error),
        };
        assert!(!retried.to_string().contains("secret"));
        assert_eq!(retried.diagnostic(), Some(diagnostic));
        assert_eq!(RuntimeError::Tool("plain".into()).diagnostic(), None);
    }
}
