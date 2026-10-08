//! Whole-task external execution: deliberately outside ModelClient and model retries.
use crate::{Result, RunRecord, RunStore, RuntimeError, ScheduledTask};
use agt_core::CancellationToken;
use agt_harness::{Codex, Event, Execution};
use rusqlite::params;
use serde_json::{Value, json};
use std::{cell::Cell, path::Path};

mod approval;
mod events;
mod finish;
mod prepare;
mod tools;
use prepare::PreparedHarness;

struct HarnessRun<'a> {
    home: &'a Path,
    token: &'a CancellationToken,
    task: Option<&'a ScheduledTask>,
    prepared: &'a PreparedHarness,
    session: crate::harness_sessions::HarnessSession,
    execution_lock: crate::harness_sessions::ExecutionLock<'a>,
    thread: Option<String>,
    input: String,
    completed: Cell<bool>,
}

fn error(e: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Invalid(format!("harness: {e}"))
}
pub(crate) fn agent_config(home: &Path, name: &str) -> Result<agt_agent::AgentConfig> {
    let name = crate::canonical_agent(name)?;
    let (c, n) = name.split_once('/').unwrap();
    agt_agent::AgentStore::new(home)
        .read_in(&agt_core::CategoryName::new(c).map_err(error)?, n)
        .map(|s| s.config)
        .map_err(error)
}
pub(crate) fn execute(
    home: &Path,
    record: &mut RunRecord,
    store: &mut RunStore,
    token: &CancellationToken,
    task: Option<&ScheduledTask>,
    discussion: Option<&str>,
) -> Result<String> {
    let prepared = PreparedHarness::load(home, record, task, discussion)?;
    let execution_lock = prepared
        .sessions
        .execution_lock(&prepared.scope)
        .map_err(error)?;
    // Read configuration after acquiring the same lock used by session controls.
    let session = prepared.sessions.get(&prepared.scope).map_err(error)?;
    let (thread, input) = prepared.input(home, record, store, task, &session)?;
    let run = HarnessRun {
        home,
        token,
        task,
        prepared: &prepared,
        session,
        execution_lock,
        thread,
        input,
        completed: Cell::new(false),
    };
    run.check()?;
    crate::progress::stage(record, store, task, "preparing")?;
    let mut invocation =
        crate::CallRecord::new(record, None, json!({"engine":"codex","request_attempts":1}));
    invocation.kind = "harness".into();
    let mut invocation = store.start_call(invocation)?;
    let audit_record = record.clone();
    record.execution = Some(
        json!({"engine":"codex","harness":prepared.reference,"session_record":prepared.scope,"timeout_seconds":run.session.config.timeout_seconds}),
    );
    let check = || run.check();
    let mut bridge = tools::ToolBridge {
        home,
        task,
        service: prepared.service.as_ref(),
        check: &check,
        audit_record: &audit_record,
        store,
        current_chat: record
            .request
            .trigger
            .as_ref()
            .map(|t| t.conversation_id.to_string()),
        state: tools::ToolState::default(),
    };
    let mut approval_bridge = approval::Bridge {
        store: crate::approval::ApprovalStore::open(home).map_err(error)?,
        scope: crate::approval::Scope::new(&audit_record, task),
        check: &check,
    };
    let mut event = |event: Event<'_>| run.event(record, store, event);
    let mut client = match Codex::connect(&run.session.config, &mut event) {
        Ok(client) => client,
        Err(e) => {
            record
                .termination
                .get_or_insert_with(|| crate::Termination::from_error(&e));
            let e = run.check().err().unwrap_or_else(|| error(e));
            invocation.finish(
                if matches!(e, RuntimeError::Canceled | RuntimeError::Stopped(_)) {
                    crate::CallState::Canceled
                } else {
                    crate::CallState::Failed
                },
                Some(crate::audit::runtime_error(&e)),
            );
            store.finish_call(&invocation)?;
            return Err(e);
        }
    };
    let result = client.execute_with_approvals(
        &run.session.config,
        Execution {
            input: &run.input,
            instructions: &prepared.instructions,
            thread: run.thread.as_deref(),
            tools: &prepared.tools,
        },
        &mut event,
        &mut |name, args| bridge.call(name, args),
        Some(&mut approval_bridge),
    );
    if let Err(error) = &result {
        record
            .termination
            .get_or_insert_with(|| crate::Termination::from_error(error));
        record.execution.as_mut().unwrap()["diagnostics"] = json!(client.diagnostics());
    }
    drop(client);
    let state = bridge.state;
    run.finish(record, store, invocation, &state, result)
}
