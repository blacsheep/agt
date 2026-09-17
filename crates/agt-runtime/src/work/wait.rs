//! Native-only cooperative suspension. An intent is not a released execution slot.
use super::control::check_scope;
use super::*;
use crate::{RunRecord, RunState, ScheduledTask};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Stored privately in runs.db, never included in task_get or task notifications.
#[derive(Serialize, Deserialize)]
pub(crate) struct Checkpoint {
    pub messages: Vec<Value>,
    pub seen_calls: BTreeSet<String>,
    pub published: bool,
    pub sent_messages: BTreeMap<String, String>,
    pub config_fingerprint: Option<String>,
}

pub(super) fn register(home: &Path, task: &ScheduledTask, args: Value) -> Result<Value> {
    crate::scheduler::guarded_store(home, task, |store| {
        let operation = || -> Result<Value> {
            let id = task
                .work_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("explicit work required"))?;
            let item = read(&store.connection, id)?;
            let WorkState::Running { run_id } = &item.state else {
                bail!("work is not running");
            };
            let target = args["task_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("task_id required"))?;
            let deadline = args["deadline"]
                .as_i64()
                .ok_or_else(|| anyhow::anyhow!("deadline required"))?;
            if deadline <= crate::schedule_store::now_ms() {
                bail!("deadline must be future Unix milliseconds");
            }
            let dependency = read(&store.connection, target)?;
            check_scope(home, store, &dependency)?;
            if task.work_notification
                || item.request.assignee != task.principal
                || dependency.request.requester != task.principal
                || dependency.request.discussion != item.request.discussion
            {
                bail!("await requires your own request in this discussion");
            }
            let other: bool = store.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM work_waits WHERE dependency=?1 AND work_id<>?2)",
                params![target, id],
                |r| r.get(0),
            )?;
            if other {
                bail!("dependency already belongs to another waiting task");
            }
            queue::set_dependency(&store.connection, id, target)?;
            store.connection.execute(
                "INSERT INTO work_waits(run_id,work_id,dependency,deadline) VALUES(?1,?2,?3,?4)",
                params![run_id, id, target, deadline],
            )?;
            Ok(json!({"status":"suspending","task_id":id,"dependency":target,"deadline":deadline}))
        };
        operation().map_err(|e| crate::RuntimeError::Invalid(e.to_string()))
    })
    .map_err(Into::into)
}

pub(crate) fn outcome(home: &Path, run: &str) -> Result<Value> {
    let store = ScheduleStore::open(home)?;
    let value: String = store.connection.query_row(
        "SELECT outcome FROM work_waits WHERE run_id=?1",
        [run],
        |r| r.get(0),
    )?;
    Ok(serde_json::from_str(&value)?)
}

/// Prevent a separate notification Run from racing the continuation. An abandoned
/// intent does not consume a result. Timeout only consumes the timeout, not late results.
pub(crate) fn consumed_notice(c: &Connection, dependency: &str) -> Result<bool> {
    if read(c, dependency)?.resolution.is_some() {
        return Ok(false);
    }
    Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM work_waits WHERE dependency=?1 AND json_extract(outcome,'$.kind')='resolved')",[dependency],|r|r.get(0))?)
}

pub(crate) fn consumes_notice(c: &Connection, dependency: &str) -> Result<bool> {
    if read(c, dependency)?.resolution.is_some() {
        return Ok(false);
    }
    Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM work_waits w JOIN work_items i ON i.id=w.work_id WHERE w.dependency=?1 AND (json_extract(w.outcome,'$.kind')='resolved' OR (w.outcome IS NULL AND json_extract(i.record,'$.state') IN ('running','waiting_dependency'))))",[dependency],|r|r.get(0))?)
}

pub(super) fn reconcile(
    home: &Path,
    ledger: &mut WorkLedger,
    item: &WorkItem,
    run: Option<&RunRecord>,
    adapter: Option<&ScheduledTask>,
) -> Result<()> {
    // Even a committed checkpoint cannot release a worker still closing tool sessions.
    // Scheduler updates to suspended only after Runtime::run returns. Lease recovery
    // fences interrupted workers before they can be considered here.
    if adapter.is_some_and(|t| {
        matches!(
            t.state.as_str(),
            "evaluating" | "responding" | "ready_to_reply"
        )
    }) {
        return Ok(());
    }
    let run_id = match &item.state {
        WorkState::WaitingDependency { run_id, .. } => run_id.clone(),
        WorkState::Running { run_id }
            if run.is_some_and(|r| r.state == RunState::Suspended && r.id == *run_id) =>
        {
            run_id.clone()
        }
        _ => bail!("invalid suspended work association"),
    };
    // Missing checkpoints fail closed; never restart from the original request.
    if matches!(item.state, WorkState::Running { .. }) {
        crate::RunStore::open(home)?.checkpoint(&run_id)?;
    }
    let scope_store = ScheduleStore::open(home)?;
    let tx = ledger
        .store
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut current = read(&tx, &item.id)?;
    if current.revision != item.revision {
        return Ok(());
    }
    let (dependency_id, deadline): (String, i64) = tx.query_row(
        "SELECT dependency,deadline FROM work_waits WHERE run_id=?1 AND outcome IS NULL",
        [&run_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if matches!(current.state, WorkState::Running { .. }) {
        current.deferred = None;
        current.state = WorkState::WaitingDependency {
            run_id: run_id.clone(),
            dependency: dependency_id.clone(),
            deadline,
        };
        queue::save(&tx, &mut current, "waiting_dependency")?;
    }
    // Scope validation uses a separate read connection; the transaction above already
    // owns the scheduler write lock, so reset/end cannot interleave with this decision.
    let invalid = current.cancellation_requested.is_some()
        || check_scope(home, &scope_store, &current).is_err()
        || adapter.is_none_or(|t| !matches!(t.state.as_str(), "suspended" | "interrupted"));
    if invalid {
        current.state = WorkState::Canceled {
            reason: "Task authorization changed while suspended.".into(),
        };
        current.revision += 1;
        current.updated_at = crate::schedule_store::now_ms();
        tx.execute(
            "UPDATE work_items SET revision=?2,record=?3 WHERE id=?1",
            params![
                current.id,
                current.revision,
                serde_json::to_string(&current)?
            ],
        )?;
        append_event(
            &tx,
            &current,
            "canceled",
            Some(&current.request.requester),
            true,
        )?;
        tx.execute(
            "DELETE FROM work_dependencies WHERE work_id=?1",
            [&current.id],
        )?;
        if let Some(adapter) = adapter {
            let mut task = adapter.clone();
            task.state = "canceled".into();
            task.reason = Some("task authorization changed while suspended".into());
            task.owner = None;
            task.version += 1;
            crate::schedule_store::save_task(&tx, &task)?;
        }
        tx.commit()?;
        return Ok(());
    }
    let dependency = read(&tx, &dependency_id)?;
    let resolved = !matches!(
        dependency.state,
        WorkState::Queued | WorkState::Running { .. } | WorkState::WaitingDependency { .. }
    );
    let now = crate::schedule_store::now_ms();
    if !resolved && now < deadline {
        tx.commit()?;
        return Ok(());
    }
    let event = json!({"kind":if resolved {"resolved"} else {"deadline"},"task_id":dependency_id,"state":serde_json::to_value(&dependency)?["state"],"deadline":deadline});
    tx.execute(
        "UPDATE work_waits SET outcome=?2 WHERE run_id=?1",
        params![run_id, event.to_string()],
    )?;
    current.resume_from = Some(run_id);
    current.state = WorkState::Queued;
    current.deferred = None;
    queue::save(&tx, &mut current, "continuation_queued")?;
    tx.execute(
        "DELETE FROM work_dependencies WHERE work_id=?1",
        [&current.id],
    )?;
    let mut task = adapter.unwrap().clone();
    task.state = "queued".into();
    task.owner = None;
    task.version += 1;
    task.run_id = None;
    task.reason = None;
    task.due_at = now;
    task.quiet = false;
    crate::schedule_store::save_task(&tx, &task)?;
    tx.execute("DELETE FROM reconciled WHERE task=?1", [&task.id])?;
    if resolved {
        let notices = {
            let mut stmt = tx.prepare("SELECT record FROM tasks WHERE json_extract(record,'$.work_id')=?1 AND json_extract(record,'$.work_notification')=1 AND state IN ('queued','waiting','blocked','limited','ready_to_reply')")?;
            stmt.query_map([&dependency_id], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for value in notices {
            let mut notice: ScheduledTask = serde_json::from_str(&value)?;
            notice.state = "canceled".into();
            notice.reason = Some("dependency result consumed by work continuation".into());
            notice.owner = None;
            notice.version += 1;
            crate::schedule_store::save_task(&tx, &notice)?;
        }
        tx.execute("UPDATE work_inbox SET state='handled' WHERE recipient=?1 AND event_seq IN (SELECT seq FROM work_outbox WHERE work_id=?2 AND kind IN ('succeeded','failed','canceled','outcome_unknown'))",params![current.request.assignee,dependency_id])?;
    }
    tx.commit()?;
    Ok(())
}
