//! Explicit lifecycle controls; no automatic retries of uncertain external effects.
use super::control::{check_scope, member, open_im, view};
use super::*;
use crate::{DiscussionState, TaskState};
use serde_json::Value;

impl TaskControl {
    pub(super) fn lifecycle(&self, name: &str, args: Value) -> Result<Value> {
        let operation = |store: &ScheduleStore| -> Result<Value> {
            let id = args["task_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("task_id required"))?;
            let mut item = read(&store.connection, id)?;
            let im = open_im(&self.home)?;
            member(&im, &item.request.report_to, self.actor.principal.as_str())?;
            let human = self.human_manager(&im)?;
            if (!human && self.task.is_none())
                || (!human
                    && self.actor.principal.as_str() != item.request.requester
                    && self.actor.principal.as_str() != item.request.assignee)
            {
                bail!("task control requires hosted requester/assignee or human Manager");
            }
            if let Some(task) = &self.task
                && task.discussion != item.request.discussion
            {
                bail!("task belongs to another discussion");
            }
            if args["expected_revision"].as_i64() != Some(item.revision) {
                bail!("stale work revision; inspect task_get");
            }
            let text = args[if name == "task_resolve" {
                "evidence"
            } else {
                "reason"
            }]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("reason or evidence required"))?;
            if text.trim().is_empty() || text.len() > 4000 {
                bail!("reason/evidence requires 1..4000 bytes");
            }
            match name {
                "task_cancel" => {
                    cancel_tree(
                        &store.connection,
                        &item.id,
                        text,
                        self.actor.principal.as_str(),
                    )?;
                    item = read(&store.connection, id)?;
                }
                "task_resolve" => {
                    if !human
                        || !matches!(item.state, WorkState::OutcomeUnknown { .. })
                        || item.resolution.is_some()
                    {
                        bail!("only human Manager may verify an unresolved unknown outcome");
                    }
                    let outcome = args["outcome"].as_str().unwrap_or("");
                    if !matches!(outcome, "succeeded" | "failed") {
                        bail!("outcome must be succeeded or failed");
                    }
                    item.resolution = Some(Resolution {
                        outcome: outcome.into(),
                        evidence: text.into(),
                        actor: self.actor.principal.to_string(),
                        at: crate::schedule_store::now_ms(),
                    });
                    save_event(&store.connection, &mut item, "resolved", true)?;
                }
                "task_retry" => {
                    if self.actor.principal.as_str() != item.request.requester && !human {
                        bail!("only requester or human Manager can retry");
                    }
                    check_scope(&self.home, store, &item)?;
                    let round = store.discussion(&item.request.discussion)?.unwrap();
                    if round.state != DiscussionState::Active
                        || round.expires_at <= crate::schedule_store::now_ms()
                    {
                        bail!("renew discussion authorization before retrying");
                    }
                    if !matches!(
                        item.state,
                        WorkState::Failed { .. }
                            | WorkState::Canceled { .. }
                            | WorkState::Interrupted { .. }
                    ) && !item
                        .resolution
                        .as_ref()
                        .is_some_and(|r| r.outcome == "failed")
                    {
                        bail!(
                            "retry requires known failure or cancellation; verify unknown effects first"
                        );
                    }
                    let mut request = item.request.clone();
                    request.request_key = args["request_key"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("request_key required"))?
                        .into();
                    request.retry_of = Some(item.id.clone());
                    request.priority = item.priority;
                    let new = WorkLedger::enqueue_on(&store.connection, request)?;
                    return view(&self.home, store, &new);
                }
                _ => bail!("unknown lifecycle action"),
            }
            view(&self.home, store, &item)
        };
        if let Some(task) = &self.task {
            crate::scheduler::guarded_store(&self.home, task, |s| {
                operation(s).map_err(|e| crate::RuntimeError::Invalid(e.to_string()))
            })
            .map_err(Into::into)
        } else {
            let store = ScheduleStore::open(&self.home)?;
            let tx_db = store.connection.clone();
            let tx = agt_storage::sqlite::write(&tx_db)?;
            let result = operation(&store)?;
            tx.commit()?;
            agt_storage::wake::notify(&self.home);
            Ok(result)
        }
    }
}

pub(super) fn save_event(
    c: &Connection,
    item: &mut WorkItem,
    kind: &str,
    notify: bool,
) -> Result<()> {
    item.revision += 1;
    item.updated_at = crate::schedule_store::now_ms();
    c.execute(
        "UPDATE work_items SET revision=?2,record=?3 WHERE id=?1",
        params![item.id, item.revision, serde_json::to_string(item)?],
    )?;
    append_event(
        c,
        item,
        kind,
        notify.then_some(item.request.requester.as_str()),
        notify,
    )
}
fn cancel_tree(c: &Connection, id: &str, reason: &str, actor: &str) -> Result<()> {
    let ids = {
        let mut stmt=c.prepare("WITH RECURSIVE owned(id) AS (SELECT ?1 UNION SELECT w.id FROM work_items w JOIN owned p ON json_extract(w.record,'$.request.parent_id')=p.id) SELECT id FROM owned")?;
        stmt.query_map([id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for id in ids {
        let mut item = read(c, &id)?;
        if !matches!(
            item.state,
            WorkState::Queued
                | WorkState::Running { .. }
                | WorkState::WaitingDependency { .. }
                | WorkState::WaitingUntil { .. }
                | WorkState::WaitingInput { .. }
        ) {
            continue;
        }
        let adapter: Option<String> = c
            .query_row(
                "SELECT record FROM tasks WHERE id=?1",
                [format!("work:{id}")],
                |r| r.get(0),
            )
            .optional()?;
        let mut adapter = adapter
            .map(|s| serde_json::from_str::<crate::ScheduledTask>(&s))
            .transpose()?;
        let in_flight = matches!(item.state, WorkState::Running { .. })
            || adapter.as_ref().is_some_and(|t| {
                matches!(
                    t.state,
                    TaskState::Responding | TaskState::Evaluating | TaskState::ReadyToReply
                )
            });
        if item.cancellation_requested.is_some() {
            continue;
        }
        item.cancellation_requested = Some(reason.into());
        let mut termination = crate::Termination::new(crate::StopCode::TaskCanceled);
        termination.actor = Some(actor.into());
        if !in_flight {
            termination.confirmation = "not_running".into();
            termination.confirmed_at = Some(crate::schedule_store::now_ms());
            if item.resume_from.is_none() {
                termination.side_effects = "not_started".into();
            }
        }
        item.termination = Some(termination);
        if in_flight {
            save_event(c, &mut item, "cancellation_requested", false)?;
        } else {
            item.state = WorkState::Canceled {
                reason: reason.into(),
            };
            save_event(c, &mut item, "canceled", true)?;
            c.execute("DELETE FROM work_dependencies WHERE work_id=?1", [&id])?;
            if let Some(t) = &mut adapter {
                t.state = TaskState::Canceled;
                t.reason = Some(reason.into());
                t.owner = None;
                t.version += 1;
                crate::schedule_store::save_task(c, t)?;
            }
        }
    }
    Ok(())
}

impl TaskControl {
    pub(super) fn reply(&self, args: Value) -> Result<Value> {
        let operation = |store: &ScheduleStore| -> Result<Value> {
            let id = args["task_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("task_id required"))?;
            let mut item = read(&store.connection, id)?;
            let im = open_im(&self.home)?;
            member(&im, &item.request.report_to, self.actor.principal.as_str())?;
            let human = self.human_manager(&im)?;
            if (!human && self.task.is_none())
                || (!human && item.request.requester != self.actor.principal.as_str())
            {
                bail!("only hosted requester or human Manager may answer");
            }
            if let Some(task) = &self.task
                && task.discussion != item.request.discussion
            {
                bail!("input belongs to another discussion");
            }
            check_scope(&self.home, store, &item)?;
            let input = args["input_request_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("input_request_id required"))?;
            let answer = args["answer"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("answer required"))?;
            let key = args["request_key"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("request_key required"))?;
            if answer.trim().is_empty()
                || answer.len() > 16000
                || key.trim().is_empty()
                || key.len() > 256
            {
                bail!("invalid answer or request key");
            }
            let previous: Option<String> = store.connection.query_row(
                "SELECT outcome FROM work_waits WHERE run_id=?1 AND work_id=?2 AND kind='input'",
                params![input, id],
                |r| r.get(0),
            )?;
            if let Some(previous) = previous {
                let value: Value = serde_json::from_str(&previous)?;
                if value["request_key"] == key && value["answer"] == answer {
                    return view(&self.home, store, &item);
                }
                bail!("input was already answered or expired");
            }
            if args["expected_revision"].as_i64() != Some(item.revision)
                || !matches!(&item.state,WorkState::WaitingInput{run_id,deadline,..} if run_id==input && *deadline>crate::schedule_store::now_ms())
            {
                bail!("input is not current; inspect task_get");
            }
            let outcome = serde_json::json!({"kind":"input_answer","input_request_id":input,"answer":answer,"request_key":key,"actor":self.actor.principal});
            store.connection.execute(
                "UPDATE work_waits SET outcome=?2 WHERE run_id=?1",
                params![input, outcome.to_string()],
            )?;
            save_event(&store.connection, &mut item, "input_replied", false)?;
            store.connection.execute("UPDATE work_inbox SET state='handled' WHERE event_seq IN (SELECT seq FROM work_outbox WHERE work_id=?1 AND kind='input_requested')",[id])?;
            view(&self.home, store, &item)
        };
        if let Some(task) = &self.task {
            crate::scheduler::guarded_store(&self.home, task, |s| {
                operation(s).map_err(|e| crate::RuntimeError::Invalid(e.to_string()))
            })
            .map_err(Into::into)
        } else {
            let store = ScheduleStore::open(&self.home)?;
            let tx_db = store.connection.clone();
            let tx = agt_storage::sqlite::write(&tx_db)?;
            let result = operation(&store)?;
            tx.commit()?;
            agt_storage::wake::notify(&self.home);
            Ok(result)
        }
    }
}
