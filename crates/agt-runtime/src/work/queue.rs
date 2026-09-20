//! Queue decisions only. Running work is never preempted or relabeled as waiting.
use super::*;
use crate::ScheduledTask;

pub(super) fn ensure_queued(c: &Connection, item: &WorkItem, revision: i64) -> Result<()> {
    if item.revision != revision {
        bail!("stale work revision; read task_get before retrying");
    }
    if item.state != WorkState::Queued {
        bail!("only queued work can be changed");
    }
    let claimed = c.query_row("SELECT 1 FROM tasks WHERE id=?1 AND state IN ('evaluating','ready_to_reply','responding')",[format!("work:{}",item.id)], |_|Ok(())).optional()?.is_some();
    if claimed {
        bail!("work has already been admitted; running work cannot be deferred");
    }
    Ok(())
}

pub(super) fn set_dependency(c: &Connection, id: &str, target: &str) -> Result<()> {
    c.execute("DELETE FROM work_dependencies WHERE work_id=?1", [id])?;
    add_dependency(c, id, target)
}

pub(super) fn add_dependency(c: &Connection, id: &str, target: &str) -> Result<()> {
    let cycle: bool = c.query_row(
        "WITH RECURSIVE reachable(id) AS (SELECT ?1 UNION SELECT d.depends_on FROM work_dependencies d JOIN reachable r ON d.work_id=r.id) SELECT EXISTS(SELECT 1 FROM reachable WHERE id=?2)",params![target,id],|r|r.get(0))?;
    if cycle {
        bail!("task dependency cycle rejected");
    }
    c.execute(
        "INSERT OR IGNORE INTO work_dependencies VALUES(?1,?2)",
        params![id, target],
    )?;
    Ok(())
}

pub(super) fn save(c: &Connection, item: &mut WorkItem, kind: &str) -> Result<()> {
    item.revision += 1;
    item.updated_at = crate::schedule_store::now_ms();
    c.execute(
        "UPDATE work_items SET revision=?2,record=?3 WHERE id=?1",
        params![item.id, item.revision, serde_json::to_string(item)?],
    )?;
    // Queue updates are durable facts. They do not manufacture terminal publications
    // or trigger another model turn merely to announce a changed priority.
    append_event(c, item, kind, None, false)
}

pub(crate) fn blocker(c: &Connection, item: &WorkItem, now: i64) -> Result<Option<&'static str>> {
    if item.state != WorkState::Queued {
        return Ok(Some("not_queued"));
    }
    let Some(deferred) = &item.deferred else {
        return Ok(None);
    };
    if let Some(at) = deferred.not_before {
        return Ok((at > now).then_some("deferred_until"));
    }
    let Some(target) = &deferred.after_task else {
        return Ok(None);
    };
    let dependency = read(c, target)?;
    // Deadline is an escalation boundary, not permission to ignore an unfinished task.
    // A result arriving after the deadline still needs an explicit new queue decision.
    if deferred
        .deadline
        .is_some_and(|at| at <= now && dependency.updated_at > at)
    {
        return Ok(Some("dependency_deadline_exceeded"));
    }
    if matches!(
        dependency.state,
        WorkState::Succeeded { .. }
            | WorkState::Failed { .. }
            | WorkState::Canceled { .. }
            | WorkState::Interrupted { .. }
    ) || dependency.resolution.is_some()
    {
        return Ok(None);
    }
    if deferred.deadline.is_some_and(|at| at <= now) {
        return Ok(Some("dependency_deadline_exceeded"));
    }
    Ok(Some(
        if matches!(dependency.state, WorkState::OutcomeUnknown { .. }) {
            "dependency_outcome_unknown"
        } else {
            "waiting_for_task"
        },
    ))
}

/// Sort only the slots occupied by explicit work. Chat/notification scheduling keeps
/// its existing order. Aging raises low/normal to high, never to administrator urgent.
pub(crate) fn order(c: &Connection, tasks: &mut [ScheduledTask], now: i64) -> Result<()> {
    let mut work = Vec::new();
    for (index, task) in tasks.iter().enumerate() {
        if let Some(id) = &task.work_id
            && !task.work_notification
        {
            let item = read(c, id)?;
            let seq: i64 = c.query_row("SELECT rowid FROM work_items WHERE id=?1", [id], |r| {
                r.get(0)
            })?;
            let base = match item.priority {
                Priority::Low => 0,
                Priority::Normal => 1,
                Priority::High => 2,
                Priority::Urgent => 3,
            };
            let rank = if base == 3 {
                3
            } else {
                (base + (now.saturating_sub(item.created_at).max(0) / 300_000)).min(2)
            };
            work.push((index, rank, seq, task.clone()));
        }
    }
    let slots: Vec<_> = work.iter().map(|x| x.0).collect();
    work.sort_by_key(|(_, rank, seq, _)| (std::cmp::Reverse(*rank), *seq));
    for (index, (_, _, _, task)) in slots.into_iter().zip(work) {
        tasks[index] = task;
    }
    // After four notifications, admit available work before another notification for
    // the same identity. Other principals keep their relative opportunities.
    let principals = tasks
        .iter()
        .map(|t| t.principal.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for principal in principals {
        let streak: i64 = c
            .query_row(
                "SELECT notices FROM work_fairness WHERE principal=?1",
                [&principal],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if streak < 4 {
            continue;
        }
        let notification = tasks
            .iter()
            .position(|t| t.principal == principal && t.work_notification);
        let work = tasks.iter().position(|t| {
            t.principal == principal
                && !t.work_notification
                && t.work_id.as_ref().is_some_and(|id| {
                    read(c, id)
                        .and_then(|i| blocker(c, &i, now))
                        .is_ok_and(|b| b.is_none())
                })
        });
        if let (Some(n), Some(w)) = (notification, work)
            && n < w
        {
            tasks.swap(n, w);
        }
    }
    Ok(())
}
