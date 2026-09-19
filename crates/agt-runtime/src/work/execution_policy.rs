//! Model-declared scheduling intent; conflicts are enforced inside the admission transaction.
use crate::ScheduledTask;
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Policy {
    pub parallel: bool,
    pub exclusive_resources: Vec<String>,
}
impl Policy {
    pub fn validate(&mut self) -> Result<()> {
        if self.exclusive_resources.len() > 32 {
            bail!("at most 32 exclusive resources");
        }
        for key in &self.exclusive_resources {
            if key.is_empty()
                || key.len() > 128
                || !key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_:/.-".contains(c))
            {
                bail!("resource keys must be 1..128 ASCII letters/digits or _:/.-");
            }
        }
        self.exclusive_resources.sort();
        self.exclusive_resources.dedup();
        Ok(())
    }
}
pub(crate) fn get(db: &Connection, id: &str) -> Result<Policy> {
    let raw: Option<String> = db
        .query_row(
            "SELECT policy FROM work_execution_policy WHERE work_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw
        .map(|r| serde_json::from_str(&r))
        .transpose()?
        .unwrap_or_default())
}
pub(crate) fn create(db: &Connection, id: &str, policy: &Policy) -> Result<()> {
    let raw = serde_json::to_string(policy)?;
    db.execute(
        "INSERT OR IGNORE INTO work_execution_policy VALUES(?1,?2,?2)",
        params![id, raw],
    )?;
    let original: String = db.query_row(
        "SELECT requested FROM work_execution_policy WHERE work_id=?1",
        [id],
        |r| r.get(0),
    )?;
    if original != raw {
        bail!("request key reused with different execution policy");
    }
    Ok(())
}
pub(crate) fn set(db: &Connection, id: &str, policy: &Policy) -> Result<()> {
    // Trusted older tasks default to serial; preserve original creation policy on edits.
    db.execute(
        "INSERT OR IGNORE INTO work_execution_policy VALUES(?1,?2,?2)",
        params![id, serde_json::to_string(&Policy::default())?],
    )?;
    db.execute(
        "UPDATE work_execution_policy SET policy=?2 WHERE work_id=?1",
        params![id, serde_json::to_string(policy)?],
    )?;
    Ok(())
}
fn task_policy(db: &Connection, task: &ScheduledTask) -> Result<Policy> {
    match task.work_id.as_deref().filter(|_| !task.work_notification) {
        Some(id) => get(db, id),
        None => Ok(Policy::default()),
    }
}
pub(crate) fn blocker(
    db: &Connection,
    task: &ScheduledTask,
    enabled: bool,
) -> Result<Option<&'static str>> {
    let candidate = task_policy(db, task)?;
    let mut stmt=db.prepare("SELECT record FROM tasks WHERE id!=?1 AND state IN ('evaluating','ready_to_reply','responding')")?;
    let rows = stmt.query_map([&task.id], |r| r.get::<_, String>(0))?;
    for raw in rows {
        let active: ScheduledTask = serde_json::from_str(&raw?)?;
        let other = task_policy(db, &active)?;
        if active.principal == task.principal && !(enabled && candidate.parallel && other.parallel)
        {
            return Ok(Some("identity_busy"));
        }
        if candidate
            .exclusive_resources
            .iter()
            .any(|k| other.exclusive_resources.contains(k))
        {
            return Ok(Some("resource_busy"));
        }
    }
    // Unknown external effects retain their declared resources until human reconciliation.
    let mut uncertain=db.prepare("SELECT w.record FROM work_items w JOIN work_execution_policy p ON p.work_id=w.id WHERE json_extract(w.record,'$.state')='outcome_unknown' AND json_extract(w.record,'$.resolution') IS NULL")?;
    for raw in uncertain.query_map([], |r| r.get::<_, String>(0))? {
        let item: super::WorkItem = serde_json::from_str(&raw?)?;
        let policy = get(db, &item.id)?;
        if candidate
            .exclusive_resources
            .iter()
            .any(|k| policy.exclusive_resources.contains(k))
        {
            return Ok(Some("resource_outcome_unknown"));
        }
    }
    Ok(None)
}
