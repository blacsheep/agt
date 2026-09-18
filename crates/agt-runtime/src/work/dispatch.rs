use super::control::{blocked, check_scope, open_im};
use super::*;
use crate::{RunRecord, RunState, RunStore, ScheduledTask};
use agt_im::{ConversationId, Message, PrincipalId, SendMessageRequest};
use serde_json::json;

pub(crate) use super::control::check_task_scope;

pub(crate) fn run_key(id: &str) -> String {
    format!("work:{id}:attempt:1")
}

pub(crate) fn execution_key(item: &WorkItem) -> String {
    item.resume_from
        .as_ref()
        .map(|id| format!("work:{}:resume:{id}", item.id))
        .unwrap_or_else(|| run_key(&item.id))
}

/// Only framework-created messages matching a durable work record are routable.
pub(crate) fn route(
    store: &ScheduleStore,
    message: &Message,
) -> Result<Option<(String, String, bool)>> {
    let Some(id) = message.metadata["work_id"].as_str() else {
        return Ok(None);
    };
    let item = read(&store.connection, id)?;
    let notice = message.metadata["work_kind"] == "result";
    let (sender, target) = if notice {
        (&item.request.assignee, &item.request.requester)
    } else {
        (&item.request.requester, &item.request.assignee)
    };
    if message.sender_id.as_str() != sender
        || message.conversation_id.as_str() != item.request.report_to
    {
        bail!("invalid work message scope");
    }
    let valid = if notice {
        store.connection.query_row("SELECT EXISTS(SELECT 1 FROM work_publications p JOIN work_outbox e ON e.seq=p.event_seq WHERE e.work_id=?1 AND ('tool:' || p.request_key)=?2)",params![id,message.client_message_id],|r|r.get::<_,bool>(0))?
    } else {
        message.client_message_id.as_deref() == Some(format!("work:{id}:request").as_str())
    };
    if !valid {
        bail!("invalid work message key");
    }
    Ok(Some((id.into(), target.clone(), notice)))
}

pub(crate) fn notice_blocked(c: &Connection, task: &ScheduledTask) -> Result<bool> {
    let Some(id) = &task.work_id else {
        return Ok(false);
    };
    match task.work_event.as_deref() {
        Some("resolved" | "dependency_expired") => Ok(false),
        Some("input_requested") => Ok(
            !matches!(read(c,id)?.state,WorkState::WaitingInput{run_id,..} if task.work_input_id.as_deref()==Some(run_id.as_str())),
        ),
        _ => super::wait::consumes_notice(c, id),
    }
}

pub(crate) fn runnable(home: &Path, store: &ScheduleStore, task: &ScheduledTask) -> Result<bool> {
    let Some(id) = &task.work_id else {
        return Ok(true);
    };
    let item = read(&store.connection, id)?;
    if task.work_notification && notice_blocked(&store.connection, task)? {
        return Ok(false);
    }
    if !task.work_notification
        && (item.state != WorkState::Queued || item.cancellation_requested.is_some())
    {
        return Ok(false);
    }
    if !task.work_notification
        && queue::blocker(&store.connection, &item, crate::schedule_store::now_ms())?.is_some()
    {
        return Ok(false);
    }
    Ok(blocked(home, store, &item, &task.principal)?.is_none())
}

/// The Run claim is already durable. Link it before the first model/tool side effect.
pub(crate) fn bind_run(home: &Path, task: &ScheduledTask, run: &RunRecord) -> Result<()> {
    let Some(id) = &task.work_id else {
        return Ok(());
    };
    if task.work_notification {
        return Ok(());
    }
    let mut ledger = WorkLedger::open(home)?;
    let item = ledger.get(id)?;
    if run.request.request_key != execution_key(&item) {
        bail!("invalid work run key");
    }
    match &item.state {
        WorkState::Queued => {
            ledger.record_running(id, item.revision, &run.id)?;
        }
        WorkState::Running { run_id } if run_id == &run.id => {}
        _ => bail!("work run already resolved"),
    }

    Ok(())
}

fn finish(ledger: &mut WorkLedger, item: &WorkItem, run: &RunRecord) -> Result<()> {
    let outcome = match run.state {
        RunState::Completed => {
            let output = run.output.as_deref().unwrap_or("");
            if output.trim().is_empty() || output.trim() == "[[silent]]" {
                WorkOutcome::Failed(
                    "Task finished without a substantive result; inspect the Run.".into(),
                )
            } else {
                WorkOutcome::Succeeded(output.into())
            }
        }
        RunState::Failed if run.execution.as_ref().is_some_and(|e|e["outcome_unknown"]==true || e["turn_status"]=="completed") => WorkOutcome::Unknown(
            "External execution did not produce a confirmed durable result; inspect side effects before retrying.".into(),
        ),
        RunState::Failed => WorkOutcome::Failed(
            "Agent execution failed; inspect the associated Run for diagnostics.".into(),
        ),
        RunState::Suspended => bail!("suspended run requires continuation reconciliation"),
        RunState::Canceled | RunState::Running => WorkOutcome::Unknown(
            "Execution stopped or ownership was lost; inspect side effects before retrying.".into(),
        ),
    };
    ledger.record_outcome(&item.id, item.revision, &run.id, outcome)?;
    Ok(())
}

/// Reconcile database facts only; this function never starts a model or harness.
pub(crate) fn reconcile(home: &Path) -> Result<()> {
    let mut ledger = WorkLedger::open(home)?;
    let runs = RunStore::open(home)?;
    for mut item in ledger.pending_execution()? {
        let adapter = ledger.store.get(&format!("work:{}", item.id)).ok();
        let live = adapter.as_ref().is_some_and(|t| {
            ledger
                .store
                .authorized(t, crate::schedule_store::now_ms())
                .unwrap_or(false)
        });
        let run = runs.by_request_key(&execution_key(&item))?;
        if matches!(
            item.state,
            WorkState::WaitingDependency { .. }
                | WorkState::WaitingUntil { .. }
                | WorkState::WaitingInput { .. }
        ) || run.as_ref().is_some_and(|r| r.state == RunState::Suspended)
        {
            super::wait::reconcile(home, &mut ledger, &item, run.as_ref(), adapter.as_ref())?;
            continue;
        }
        if item.state == WorkState::Queued {
            item = super::reporting::expired_barrier(home, &ledger.store, item)?;
            if let Some(run) = &run {
                // Crash between runs.db claim and the work association. Never execute again.
                item = ledger.record_running(&item.id, item.revision, &run.id)?;
            } else if check_scope(home, &ledger.store, &item).is_err() {
                ledger.cancel_queued(&item, "Task authorization changed before execution.")?;
                if let Some(adapter) = &adapter {
                    ledger.store.cancel_pending(&adapter.id)?;
                }
                continue;
            } else if adapter.as_ref().is_some_and(|t| {
                matches!(
                    t.state.as_str(),
                    "failed" | "interrupted" | "canceled" | "skipped" | "silent"
                )
            }) {
                ledger
                    .cancel_queued(&item, "Task could not start; inspect its scheduler record.")?;
                continue;
            } else if adapter.is_none() {
                // Permission and context control are fenced against /new/end during publication.
                ledger.store.connection.execute_batch("BEGIN IMMEDIATE")?;
                let result = (|| -> Result<()> {
                    check_scope(home, &ledger.store, &item)?;
                    open_im(home)?.send_message(SendMessageRequest {
                        conversation_id: ConversationId::new(&item.request.report_to),
                        sender_id: PrincipalId::new(&item.request.requester),
                        body: format!("[AGT task {}]\n{}\n\nThis is an explicit work request. Return a substantive final result or explain the failure. The framework records and reports your final result to the requester; do not use [[silent]].",item.id,item.request.input),
                        client_message_id: Some(format!("work:{}:request",item.id)), reply_to_id: None,
                        metadata: json!({"work_id":item.id,"work_kind":"request","mentions":[item.request.assignee],"discussion_id":item.request.discussion}), idempotency_key: None,
                    })?;
                    Ok(())
                })();
                ledger.store.connection.execute_batch(if result.is_ok() {
                    "COMMIT"
                } else {
                    "ROLLBACK"
                })?;
                result?;
            }
        }
        if matches!(item.state, WorkState::Running { .. })
            && let Some(run) = run
            && (run.state != RunState::Running || !live)
        {
            finish(&mut ledger, &item, &run)?;
        }
    }
    let mut after = 0;
    loop {
        let page = ledger.publications_after(after, 200)?;
        if page.is_empty() {
            break;
        }
        for publication in page {
            after = publication.event_seq;
            let item = ledger.get(&publication.work_id)?;
            ledger.store.connection.execute_batch("BEGIN IMMEDIATE")?;
            let result = (|| -> Result<()> {
                check_scope(home, &ledger.store, &item)?;
                let event_kind: String = ledger.store.connection.query_row(
                    "SELECT kind FROM work_outbox WHERE seq=?1",
                    [publication.event_seq],
                    |r| r.get(0),
                )?;
                let snapshot: String = ledger.store.connection.query_row(
                    "SELECT payload FROM work_outbox WHERE seq=?1",
                    [publication.event_seq],
                    |r| r.get(0),
                )?;
                let item =
                    serde_json::from_str::<WorkItem>(&snapshot).unwrap_or_else(|_| item.clone());
                let verification = item
                    .resolution
                    .as_ref()
                    .map(|r| format!("Verified {} by {}: {}", r.outcome, r.actor, r.evidence));
                let body = if event_kind == "resolved" {
                    verification
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("verification missing"))?
                } else if event_kind == "dependency_expired" {
                    "The queue dependency deadline expired. The task remains blocked; inspect task_get and explicitly reconsider its priority/defer condition, or escalate to the user. Do not treat this as task completion."
                } else {
                    match &item.state {
                        WorkState::WaitingInput { question, .. } => question.as_str(),
                        WorkState::Succeeded { output, .. } => output.as_str(),
                        WorkState::Failed { error, .. } => error.as_str(),
                        WorkState::OutcomeUnknown { reason, .. }
                        | WorkState::Canceled { reason } => reason.as_str(),
                        _ => bail!("result not ready"),
                    }
                };
                // Bound the encoded model input too: quotes/control characters can
                // expand sixfold in JSON. Full output stays in the WorkItem.
                let mut end = body.len().min(64 * 1024);
                loop {
                    while !body.is_char_boundary(end) {
                        end -= 1;
                    }
                    if serde_json::to_string(&body[..end])?.len() <= 96 * 1024 {
                        break;
                    }
                    end /= 2;
                }
                let instruction = if matches!(
                    event_kind.as_str(),
                    "input_requested" | "dependency_expired"
                ) {
                    "This is a decision request, not a completed result. Inspect task_get, handle the question or blocking condition, and escalate to the user when needed.".to_owned()
                } else {
                    format!(
                        "Review this result, then use task_report(task_id, expected_revision, body) to reliably report to the original conversation {}. Reading or finishing this notification does not discharge the reporting obligation.",
                        item.request.origin_conversation
                    )
                };
                let mut text = format!(
                    "[AGT task {} · {}]\n{}{}\n\n{} Full task details: task_get(task_id).",
                    item.id,
                    if matches!(
                        event_kind.as_str(),
                        "resolved" | "dependency_expired" | "input_requested"
                    ) {
                        event_kind.as_str()
                    } else {
                        match item.state {
                            WorkState::WaitingInput { .. } => "input_required",
                            WorkState::Succeeded { .. } => "succeeded",
                            WorkState::Failed { .. } => "failed",
                            WorkState::Canceled { .. } => "canceled",
                            _ => "outcome_unknown",
                        }
                    },
                    &body[..end],
                    if end < body.len() {
                        "\n[truncated; use task_get for full result]"
                    } else {
                        ""
                    },
                    instruction
                );
                if let WorkState::WaitingInput { run_id, .. } = &item.state {
                    text.push_str(&format!("\nInput request {run_id}: inspect task_get for the current revision, then reply with task_reply(task_id, input_request_id, expected_revision, answer, request_key). Ask the user if needed. This is business input, never a sandbox or execution permission grant."));
                }
                let sent = open_im(home)?.send_message(SendMessageRequest {
                conversation_id: ConversationId::new(&publication.destination),sender_id: PrincipalId::new(&item.request.assignee),body:text,
                client_message_id:Some(format!("tool:{}",publication.request_key)),reply_to_id:None,
                metadata:json!({"work_id":item.id,"work_kind":"result","work_event":event_kind,"input_request_id":if let WorkState::WaitingInput{run_id,..}=&item.state {Some(run_id)}else{None},"mentions":[item.request.requester],"discussion_id":item.request.discussion}),idempotency_key:None,
            })?;
                ledger.confirm_publication(publication.event_seq, sent.message.id.as_str())?;
                Ok(())
            })();
            ledger.store.connection.execute_batch(if result.is_ok() {
                "COMMIT"
            } else {
                "ROLLBACK"
            })?;
            if result.is_err() {
                ledger.store.connection.execute("UPDATE work_publications SET error='publication_unavailable' WHERE event_seq=?1 AND message_id IS NULL",[publication.event_seq])?;
            }
            // A revoked destination leaves the result pending and queryable. Other tasks proceed.
        }
    }
    super::reporting::publish(home, &ledger.store)?;
    Ok(())
}
