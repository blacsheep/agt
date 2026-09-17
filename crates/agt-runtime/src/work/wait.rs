//! Cooperative suspension with engine-specific committed checkpoints. An intent is not a released execution slot.
use super::control::check_scope;
use super::*;
use crate::{RunRecord, RunState, ScheduledTask};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Stored privately in runs.db, never included in task_get or task notifications.
#[derive(Serialize, Deserialize)]
#[serde(tag = "engine", rename_all = "snake_case")]
pub(crate) enum Checkpoint {
    Native {
        messages: Vec<Value>,
        seen_calls: BTreeSet<String>,
        published: bool,
        sent_messages: BTreeMap<String, String>,
        config_fingerprint: Option<String>,
    },
    Codex {
        session_record: String,
        thread_id: String,
        config_fingerprint: Option<String>,
    },
}

pub(super) fn register(
    home: &Path,
    task: &ScheduledTask,
    args: Value,
    kind: &str,
) -> Result<Value> {
    crate::scheduler::guarded_store(home,task,|store| {
        let operation=||->Result<Value>{
            let id=task.work_id.as_deref().ok_or_else(||anyhow::anyhow!("explicit work required"))?;
            let item=read(&store.connection,id)?;
            let WorkState::Running{run_id}=&item.state else { bail!("work is not running"); };
            if task.work_notification || item.request.assignee!=task.principal { bail!("only current assignee may suspend work"); }
            let deadline=args[if kind=="timer" {"not_before"} else {"deadline"}].as_i64().ok_or_else(||anyhow::anyhow!("future deadline required"))?;
            if deadline<=crate::schedule_store::now_ms(){bail!("deadline must be future Unix milliseconds");}
            let targets:Vec<String>=if kind=="dependency" {
                if args.get("task_ids").is_some(){
                    if args.get("task_id").is_some(){bail!("choose task_id or task_ids");}
                    serde_json::from_value(args["task_ids"].clone())?
                }else{vec![args["task_id"].as_str().ok_or_else(||anyhow::anyhow!("task_id or task_ids required"))?.into()]}
            }else{vec![String::new()]};
            if targets.is_empty() || targets.len()>20 || targets.iter().collect::<BTreeSet<_>>().len()!=targets.len(){bail!("provide 1..20 unique dependencies");}
            let payload=if kind=="dependency" {json!({})} else {
                let field=if kind=="input" {"question"} else {"reason"};
                let text=args[field].as_str().ok_or_else(||anyhow::anyhow!("question/reason required"))?;
                if text.trim().is_empty() || text.len()>4000 {bail!("question/reason requires 1..4000 bytes");}
                json!({field:text})
            };
            store.connection.execute("DELETE FROM work_dependencies WHERE work_id=?1",[id])?;
            for target in &targets {
                if kind=="dependency" {
                    let dependency=read(&store.connection,target)?;
                    check_scope(home,store,&dependency)?;
                    if dependency.request.requester!=task.principal || dependency.request.discussion!=item.request.discussion {bail!("await requires your own request in this discussion");}
                    let other:bool=store.connection.query_row("SELECT EXISTS(SELECT 1 FROM work_waits WHERE dependency=?1 AND work_id<>?2)",params![target,id],|r|r.get(0))?;
                    if other{bail!("dependency already belongs to another waiting task");}
                    queue::add_dependency(&store.connection,id,target)?;
                }
                store.connection.execute("INSERT INTO work_waits(run_id,work_id,dependency,deadline,kind,payload) VALUES(?1,?2,?3,?4,?5,?6)",params![run_id,id,target,deadline,kind,payload.to_string()])?;
            }
            Ok(json!({"status":"suspending","task_id":id,"dependencies":targets,"deadline":deadline,"instruction":"End this turn now. Runtime will resume this task when the waiting condition is met. Do not continue using tools."}))
        };
        operation().map_err(|e|crate::RuntimeError::Invalid(e.to_string()))
    }).map_err(Into::into)
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
    Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM work_waits WHERE dependency=?1 AND (json_extract(outcome,'$.consumed')=1 OR (json_extract(outcome,'$.consumed') IS NULL AND json_extract(outcome,'$.kind')='resolved')))",[dependency],|r|r.get(0))?)
}

pub(crate) fn consumes_notice(c: &Connection, dependency: &str) -> Result<bool> {
    if read(c, dependency)?.resolution.is_some() {
        return Ok(false);
    }
    Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM work_waits w JOIN work_items i ON i.id=w.work_id WHERE w.dependency=?1 AND ((json_extract(w.outcome,'$.consumed')=1 OR (json_extract(w.outcome,'$.consumed') IS NULL AND json_extract(w.outcome,'$.kind')='resolved')) OR (w.outcome IS NULL AND json_extract(i.record,'$.state') IN ('running','waiting_dependency'))))",[dependency],|r|r.get(0))?)
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
        WorkState::WaitingDependency { run_id, .. }
        | WorkState::WaitingUntil { run_id, .. }
        | WorkState::WaitingInput { run_id, .. } => run_id.clone(),
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
    let dependencies = {
        let mut stmt = tx.prepare(
            "SELECT dependency,deadline FROM work_waits WHERE run_id=?1 ORDER BY dependency",
        )?;
        stmt.query_map([&run_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let (dependency_id, deadline) = dependencies
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("wait intent missing"))?;
    let (kind, payload, answer): (String, String, Option<String>) = tx.query_row(
        "SELECT kind,payload,outcome FROM work_waits WHERE run_id=?1 LIMIT 1",
        [&run_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let payload: Value = serde_json::from_str(&payload)?;
    let first_suspension = matches!(current.state, WorkState::Running { .. });
    if first_suspension {
        current.deferred = None;
        current.state = match kind.as_str() {
            "input" => WorkState::WaitingInput {
                run_id: run_id.clone(),
                question: payload["question"].as_str().unwrap_or("").into(),
                deadline,
            },
            "timer" => WorkState::WaitingUntil {
                run_id: run_id.clone(),
                not_before: deadline,
                reason: payload["reason"].as_str().unwrap_or("").into(),
            },
            _ => WorkState::WaitingDependency {
                run_id: run_id.clone(),
                dependency: dependency_id.clone(),
                dependencies: dependencies.iter().map(|(id, _)| id.clone()).collect(),
                deadline,
            },
        };
        queue::save(&tx, &mut current, "suspended")?;
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
    if kind == "input" && first_suspension {
        super::actions::save_event(&tx, &mut current, "input_requested", true)?;
    }
    let items = if kind == "dependency" {
        dependencies
            .iter()
            .map(|(id, _)| read(&tx, id))
            .collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let terminal = |i: &WorkItem| {
        !matches!(
            i.state,
            WorkState::Queued
                | WorkState::Running { .. }
                | WorkState::WaitingDependency { .. }
                | WorkState::WaitingInput { .. }
                | WorkState::WaitingUntil { .. }
        )
    };
    let all = items.iter().all(&terminal);
    let attention = items
        .iter()
        .any(|i| matches!(i.state, WorkState::OutcomeUnknown { .. }));
    let now = crate::schedule_store::now_ms();
    let ready = if kind == "dependency" {
        all || attention || now >= deadline
    } else {
        answer.is_some() || now >= deadline
    };
    if !ready {
        tx.commit()?;
        return Ok(());
    }
    let event = match kind.as_str() {
        "input" => answer
            .map(|s| serde_json::from_str::<Value>(&s))
            .transpose()?
            .unwrap_or_else(|| json!({"kind":"input_timeout","input_request_id":run_id})),
        "timer" => json!({"kind":"timer","reason":payload["reason"]}),
        _ => {
            json!({"kind":if all {"resolved"} else if attention {"attention"} else {"deadline"},"task_id":dependency_id,"state":serde_json::to_value(&items[0])?["state"],"deadline":deadline,"results":items.iter().map(|i|json!({"task_id":i.id,"state":serde_json::to_value(i).unwrap()["state"]})).collect::<Vec<_>>()})
        }
    };
    if items.is_empty() {
        tx.execute(
            "UPDATE work_waits SET outcome=?2 WHERE run_id=?1",
            params![run_id, event.to_string()],
        )?;
    }
    for dependency in &items {
        let mut value = event.clone();
        value["consumed"] = json!(terminal(dependency));
        tx.execute(
            "UPDATE work_waits SET outcome=?3 WHERE run_id=?1 AND dependency=?2",
            params![run_id, dependency.id, value.to_string()],
        )?;
    }
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
    for dependency in items.iter().filter(|i| terminal(i)) {
        let dependency_id = &dependency.id;
        let notices = {
            let mut stmt = tx.prepare("SELECT record FROM tasks WHERE json_extract(record,'$.work_id')=?1 AND json_extract(record,'$.work_notification')=1 AND COALESCE(json_extract(record,'$.work_event'),'') NOT IN ('input_requested','resolved') AND state IN ('queued','waiting','blocked','limited','ready_to_reply')")?;
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
