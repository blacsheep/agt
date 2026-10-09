//! Execution facts, separate from task admission and from external side-effect review.
use crate::DiscussionState;
use crate::{RunRecord, RunState, RuntimeError};
use serde::{Deserialize, Serialize};

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
    fn canceled(self) -> bool {
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Termination {
    pub code: StopCode,
    pub source: String,
    pub observed_at: i64,
    pub interrupt_requested_at: Option<i64>,
    pub confirmed_at: Option<i64>,
    pub confirmation: String,
    pub side_effects: String,
    pub actor: Option<String>,
}
impl Termination {
    pub fn new(code: StopCode) -> Self {
        Self {
            code,
            source: code.source().into(),
            observed_at: crate::schedule_store::now_ms(),
            interrupt_requested_at: None,
            confirmed_at: None,
            confirmation: "unconfirmed".into(),
            side_effects: "may_have_changes".into(),
            actor: None,
        }
    }
    pub(crate) fn from_error(error: &anyhow::Error) -> Self {
        let code = if let Some(RuntimeError::Stopped(code)) = error.downcast_ref::<RuntimeError>() {
            *code
        } else if let Some(failure) = error.downcast_ref::<agt_harness::Failure>() {
            match failure.code {
                agt_harness::FailureCode::Timeout => StopCode::HarnessTimeout,
                agt_harness::FailureCode::Disconnected => StopCode::ConnectionLost,
                agt_harness::FailureCode::Interrupted => StopCode::EngineInterrupted,
                agt_harness::FailureCode::EngineFailed => StopCode::EngineFailed,
                agt_harness::FailureCode::ResultMissing => StopCode::ResultMissing,
                agt_harness::FailureCode::Protocol => StopCode::ProtocolError,
            }
        } else if matches!(
            error.downcast_ref::<RuntimeError>(),
            Some(RuntimeError::Canceled)
        ) {
            StopCode::CancellationRequested
        } else {
            StopCode::ProtocolError
        };
        Self::new(code)
    }
}

/// Finalize only from persisted protocol facts. A cancellation token is not an engine acknowledgement.
pub(crate) fn finish(record: &mut RunRecord, error: &RuntimeError) -> RunState {
    if record.termination.is_none()
        && record
            .execution
            .as_ref()
            .is_some_and(|e| e["turn_status"] == "completed")
    {
        record.termination = Some(Termination::new(StopCode::ResultPersistenceFailed));
    }
    if record.termination.is_none() {
        let code = match error {
            RuntimeError::Stopped(code) => Some(*code),
            RuntimeError::Canceled => Some(StopCode::CancellationRequested),
            _ => None,
        };
        record.termination = code.map(Termination::new);
    }
    let Some(termination) = record.termination.as_mut() else {
        return RunState::Failed;
    };
    if let Some(execution) = &record.execution {
        let status = execution["turn_status"].as_str();
        if execution["submitted"] != true {
            termination.confirmation = "not_submitted".into();
            termination.side_effects = "not_started".into();
        } else if let Some(status) = status {
            termination.confirmation = format!("engine_{status}");
        } else {
            return RunState::OutcomeUnknown;
        }
        if status == Some("completed") {
            return RunState::OutcomeUnknown;
        }
    } else {
        termination.confirmation = "runtime_stopped".into();
    }
    termination
        .confirmed_at
        .get_or_insert(crate::schedule_store::now_ms());
    if termination.code.canceled() {
        RunState::Canceled
    } else if termination.code == StopCode::EngineInterrupted {
        RunState::Interrupted
    } else {
        RunState::Failed
    }
}

pub(crate) fn guard_cause(
    store: &crate::ScheduleStore,
    task: &crate::ScheduledTask,
) -> crate::Result<StopCode> {
    // Explain the rejected fence using the same transaction, not a later snapshot.
    let cutoff = store
        .context_cutoff(&task.conversation)
        .map_err(|_| RuntimeError::Configuration("context reset state unavailable"))?;
    if let Some(id) = &task.work_id {
        let item = crate::work::read(&store.connection, id)
            .map_err(|_| RuntimeError::Configuration("work unavailable"))?;
        if store
            .context_cutoff(&item.request.origin_conversation)
            .map_err(|_| RuntimeError::Configuration("context unavailable"))?
            != item.request.context_epoch
            || store
                .context_cutoff(&item.request.report_to)
                .map_err(|_| RuntimeError::Configuration("context unavailable"))?
                != item.request.delivery_epoch
        {
            return Ok(StopCode::ContextReset);
        }
    }
    if task.through_seq <= cutoff {
        return Ok(StopCode::ContextReset);
    }
    if store
        .discussion(&task.discussion)
        .map_err(|_| RuntimeError::Configuration("discussion unavailable"))?
        .is_some_and(|d| d.state == DiscussionState::Ended)
    {
        return Ok(StopCode::DiscussionStopped);
    }
    let alive: bool = store.connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM coordinator WHERE owner=?1 AND expires>?2)",
        rusqlite::params![task.owner, crate::schedule_store::now_ms()],
        |r| r.get(0),
    )?;
    if !alive {
        return Ok(StopCode::SchedulerLeaseLost);
    }
    Ok(StopCode::AuthorizationRevoked)
}
