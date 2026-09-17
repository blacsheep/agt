//! Explicit lifecycle controls; no automatic retries of uncertain external effects.
use super::control::{check_scope, member, open_im, view};
use super::*;
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
                    cancel_tree(&store.connection, &item.id, text)?;
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
                    if round.state != "active"
                        || round.expires_at <= crate::schedule_store::now_ms()
                    {
                        bail!("renew discussion authorization before retrying");
                    }
                    if !matches!(
                        item.state,
                        WorkState::Failed { .. } | WorkState::Canceled { .. }
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
            store.connection.execute_batch("BEGIN IMMEDIATE")?;
            let result = operation(&store)?;
            store.connection.execute_batch("COMMIT")?;
            Ok(result)
        }
    }
}

fn save_event(c: &Connection, item: &mut WorkItem, kind: &str, notify: bool) -> Result<()> {
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
fn cancel_tree(c: &Connection, id: &str, reason: &str) -> Result<()> {
    let ids = {
        let mut stmt=c.prepare("WITH RECURSIVE owned(id) AS (SELECT ?1 UNION SELECT w.id FROM work_items w JOIN owned p ON json_extract(w.record,'$.request.parent_id')=p.id) SELECT id FROM owned")?;
        stmt.query_map([id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for id in ids {
        let mut item = read(c, &id)?;
        if !matches!(
            item.state,
            WorkState::Queued | WorkState::Running { .. } | WorkState::WaitingDependency { .. }
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
                    t.state.as_str(),
                    "responding" | "evaluating" | "ready_to_reply"
                )
            });
        item.cancellation_requested = Some(reason.into());
        if in_flight {
            save_event(c, &mut item, "cancellation_requested", false)?;
        } else {
            item.state = WorkState::Canceled {
                reason: reason.into(),
            };
            save_event(c, &mut item, "canceled", true)?;
            c.execute("DELETE FROM work_dependencies WHERE work_id=?1", [&id])?;
            if let Some(t) = &mut adapter {
                t.state = "canceled".into();
                t.reason = Some(reason.into());
                t.owner = None;
                t.version += 1;
                crate::schedule_store::save_task(c, t)?;
            }
        }
    }
    Ok(())
}
