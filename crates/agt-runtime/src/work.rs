//! Durable work ledger, separate from chat scheduling.
//!
//! This is a trusted persistence API, not an authorization or execution API.
//! Callers must enforce actor visibility and scheduler admission before mutations.
//! Model/CLI access is mediated by TaskControl and the scheduler.
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::ScheduleStore;
mod actions;
mod control;
mod dispatch;
pub(crate) mod queue;
mod reporting;
pub(crate) mod triage;
pub(crate) mod wait;
pub use control::TaskControl;
pub(crate) use dispatch::*;

pub(crate) fn migrate(c: &Connection) -> Result<()> {
    let version: u32 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version >= 14 {
        return Ok(());
    }
    if version < 5 {
        c.execute_batch(
            "CREATE TABLE work_items(
            id TEXT PRIMARY KEY, requester TEXT NOT NULL, request_key TEXT NOT NULL,
            request_json TEXT NOT NULL, revision INTEGER NOT NULL, record TEXT NOT NULL,
            UNIQUE(requester,request_key));
        CREATE TABLE work_outbox(
            seq INTEGER PRIMARY KEY AUTOINCREMENT, work_id TEXT NOT NULL,
            revision INTEGER NOT NULL, kind TEXT NOT NULL, created_at INTEGER NOT NULL,
            UNIQUE(work_id,revision));
        CREATE TABLE work_inbox(
            recipient TEXT NOT NULL, event_seq INTEGER NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('pending','handled')),
            PRIMARY KEY(recipient,event_seq));
        CREATE TABLE work_publications(
            event_seq INTEGER PRIMARY KEY, request_key TEXT NOT NULL UNIQUE,
            destination TEXT NOT NULL, message_id TEXT);
        PRAGMA user_version=5;",
        )?;
    }
    if version < 6 {
        c.execute_batch("ALTER TABLE work_publications ADD COLUMN error TEXT; CREATE INDEX work_execution_state ON work_items(json_extract(record,'$.state')); PRAGMA user_version=6;")?;
    }
    if version < 7 {
        c.execute_batch("CREATE TABLE work_dependencies(work_id TEXT PRIMARY KEY, depends_on TEXT NOT NULL); CREATE INDEX work_dependency_target ON work_dependencies(depends_on); PRAGMA user_version=7;")?;
    }
    if version < 8 {
        c.execute_batch("CREATE TABLE work_waits(run_id TEXT PRIMARY KEY, work_id TEXT NOT NULL, dependency TEXT NOT NULL, deadline INTEGER NOT NULL, outcome TEXT); CREATE INDEX work_wait_dependency ON work_waits(dependency); PRAGMA user_version=8;")?;
    }
    if version < 10 {
        c.execute_batch("ALTER TABLE work_waits RENAME TO work_waits_old;
      CREATE TABLE work_waits(run_id TEXT NOT NULL,work_id TEXT NOT NULL,dependency TEXT NOT NULL,deadline INTEGER NOT NULL,outcome TEXT, PRIMARY KEY(run_id,dependency));
      INSERT INTO work_waits SELECT * FROM work_waits_old; DROP TABLE work_waits_old;
      CREATE INDEX work_wait_dependency ON work_waits(dependency);
      ALTER TABLE work_dependencies RENAME TO work_dependencies_old;
      CREATE TABLE work_dependencies(work_id TEXT NOT NULL,depends_on TEXT NOT NULL,PRIMARY KEY(work_id,depends_on));
      INSERT INTO work_dependencies SELECT * FROM work_dependencies_old; DROP TABLE work_dependencies_old;
      CREATE INDEX work_dependency_target ON work_dependencies(depends_on); PRAGMA user_version=10;")?;
    }
    if version < 11 {
        c.execute_batch("CREATE TABLE work_fairness(principal TEXT PRIMARY KEY,notices INTEGER NOT NULL); PRAGMA user_version=11;")?;
    }
    c.execute_batch("ALTER TABLE work_waits ADD COLUMN kind TEXT NOT NULL DEFAULT 'dependency'; ALTER TABLE work_waits ADD COLUMN payload TEXT NOT NULL DEFAULT '{}'; ALTER TABLE work_outbox ADD COLUMN payload TEXT NOT NULL DEFAULT '{}'; PRAGMA user_version=12;")?;
    if version < 13 {
        c.execute_batch("CREATE TABLE IF NOT EXISTS change_outbox(seq INTEGER PRIMARY KEY AUTOINCREMENT, conversation TEXT, kind TEXT NOT NULL, at INTEGER NOT NULL);
            CREATE TRIGGER IF NOT EXISTS work_change AFTER INSERT ON work_outbox BEGIN
                INSERT INTO change_outbox(conversation,kind,at) SELECT json_extract(record,'$.request.report_to'),'work',NEW.created_at FROM work_items WHERE id=NEW.work_id;
            END;
            CREATE TRIGGER IF NOT EXISTS scheduled_created AFTER INSERT ON tasks BEGIN
                INSERT INTO change_outbox(conversation,kind,at) VALUES(json_extract(NEW.record,'$.conversation'),'schedule',CAST(unixepoch('subsec')*1000 AS INTEGER));
            END;
            CREATE TRIGGER IF NOT EXISTS scheduled_changed AFTER UPDATE ON tasks WHEN OLD.record<>NEW.record BEGIN
                INSERT INTO change_outbox(conversation,kind,at) VALUES(json_extract(NEW.record,'$.conversation'),'schedule',CAST(unixepoch('subsec')*1000 AS INTEGER));
            END;
            CREATE TRIGGER IF NOT EXISTS discussion_changed AFTER UPDATE ON discussions BEGIN
                INSERT INTO change_outbox(conversation,kind,at) VALUES(NEW.conversation,'schedule',CAST(unixepoch('subsec')*1000 AS INTEGER));
            END;
            PRAGMA user_version=13;")?;
    }

    if version < 14 {
        c.execute_batch("CREATE TABLE IF NOT EXISTS work_reports(event_seq INTEGER PRIMARY KEY,work_id TEXT NOT NULL,body TEXT,actor TEXT,message_id TEXT,error TEXT,created_at INTEGER NOT NULL,attention_at INTEGER);
            INSERT OR IGNORE INTO work_reports(event_seq,work_id,created_at) SELECT seq,work_id,created_at FROM work_outbox WHERE kind IN ('succeeded','failed','canceled','outcome_unknown','resolved');
            CREATE TRIGGER IF NOT EXISTS report_changed AFTER UPDATE ON work_reports WHEN OLD.message_id IS NOT NEW.message_id OR OLD.error IS NOT NEW.error OR OLD.body IS NOT NEW.body BEGIN
                INSERT INTO change_outbox(conversation,kind,at) SELECT json_extract(record,'$.request.report_to'),'work',CAST(unixepoch('subsec')*1000 AS INTEGER) FROM work_items WHERE id=NEW.work_id;
            END;
            CREATE TRIGGER IF NOT EXISTS report_overdue AFTER UPDATE ON work_reports WHEN OLD.attention_at IS NULL AND NEW.attention_at IS NOT NULL BEGIN
                INSERT INTO change_outbox(kind,at) VALUES('attention',NEW.attention_at);
            END;
            PRAGMA user_version=14;")?;
    }

    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkRequest {
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub retry_of: Option<String>,
    pub requester: String,
    pub assignee: String,
    pub origin_conversation: String,
    pub report_to: String,
    /// Existing authorization inherited from the initiating discussion.
    pub discussion: String,
    /// Context boundary captured by the caller; never grants authorization itself.
    pub context_epoch: i64,
    #[serde(default)]
    pub delivery_epoch: i64,
    pub input: String,
    pub request_key: String,
    #[serde(default)]
    pub priority: Priority,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
    Urgent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deferred {
    pub reason: String,
    pub not_before: Option<i64>,
    pub after_task: Option<String>,
    pub deadline: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WorkState {
    Queued,
    Canceled {
        reason: String,
    },
    Running {
        run_id: String,
    },
    WaitingDependency {
        run_id: String,
        dependency: String,
        #[serde(default)]
        dependencies: Vec<String>,
        deadline: i64,
    },
    WaitingUntil {
        run_id: String,
        not_before: i64,
        reason: String,
    },
    WaitingInput {
        run_id: String,
        question: String,
        deadline: i64,
    },
    Succeeded {
        run_id: String,
        output: String,
    },
    Failed {
        run_id: String,
        error: String,
    },
    OutcomeUnknown {
        run_id: String,
        reason: String,
    },
}

/// Only committed Run results, or a confirmed loss of execution ownership, belong here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkOutcome {
    Succeeded(String),
    Failed(String),
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItem {
    pub id: String,
    pub request: WorkRequest,
    pub revision: i64,
    #[serde(flatten)]
    pub state: WorkState,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    pub priority_reason: Option<String>,
    #[serde(default)]
    pub deferred: Option<Deferred>,
    #[serde(default)]
    pub resume_from: Option<String>,
    #[serde(default)]
    pub cancellation_requested: Option<String>,
    #[serde(default)]
    pub resolution: Option<Resolution>,
    #[serde(default)]
    pub triage: Option<Triage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Triage {
    pub at: i64,
    pub source: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub outcome: String,
    pub evidence: String,
    pub actor: String,
    pub at: i64,
}

#[derive(Debug, Serialize)]
pub struct WorkEvent {
    pub seq: i64,
    pub work_id: String,
    pub revision: i64,
    pub kind: String,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
pub struct WorkPublication {
    pub event_seq: i64,
    pub work_id: String,
    pub revision: i64,
    pub request_key: String,
    pub destination: String,
}

pub struct WorkLedger {
    store: ScheduleStore,
}

impl WorkLedger {
    pub fn open(home: &Path) -> Result<Self> {
        Ok(Self {
            store: ScheduleStore::open(home)?,
        })
    }

    /// Persist a validated request and its assignee inbox item atomically.
    /// Replaying the same key returns the original task; different input conflicts.
    pub fn enqueue(&mut self, request: WorkRequest) -> Result<WorkItem> {
        let tx = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let item = Self::enqueue_on(&tx, request)?;
        tx.commit()?;
        agt_storage::wake::notify(&self.store.home);
        Ok(item)
    }

    pub(crate) fn enqueue_on(tx: &Connection, request: WorkRequest) -> Result<WorkItem> {
        for value in [
            &request.requester,
            &request.assignee,
            &request.origin_conversation,
            &request.report_to,
            &request.discussion,
            &request.request_key,
        ] {
            if value.trim().is_empty() || value.len() > 256 {
                bail!("work identifiers must contain 1..256 bytes");
            }
        }
        if request.input.trim().is_empty()
            || serde_json::to_string(&request.input)?.len() > 120 * 1024
            || request.context_epoch < 0
            || request.delivery_epoch < 0
        {
            bail!("invalid work input or context epoch");
        }
        let encoded = serde_json::to_string(&request)?;
        let previous: Option<(String, String)> = tx
            .query_row(
                "SELECT request_json,record FROM work_items WHERE requester=?1 AND request_key=?2",
                params![request.requester, request.request_key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((original, record)) = previous {
            if serde_json::from_str::<WorkRequest>(&original)? != request {
                bail!("work request key conflicts with existing input");
            }
            return Ok(serde_json::from_str(&record)?);
        }
        let now = crate::schedule_store::now_ms();
        let item = WorkItem {
            id: uuid::Uuid::new_v4().to_string(),
            priority: request.priority,
            priority_reason: None,
            deferred: None,
            resume_from: None,
            cancellation_requested: None,
            resolution: None,
            triage: None,
            request,
            revision: 1,
            state: WorkState::Queued,
            created_at: now,
            updated_at: now,
        };
        tx.execute(
            "INSERT INTO work_items VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                item.id,
                item.request.requester,
                item.request.request_key,
                encoded,
                item.revision,
                serde_json::to_string(&item)?
            ],
        )?;
        append_event(tx, &item, "requested", Some(&item.request.assignee), false)?;
        Ok(item)
    }

    pub(crate) fn pending_execution(&self) -> Result<Vec<WorkItem>> {
        let mut stmt = self
            .store
            .connection
            .prepare("SELECT record FROM work_items WHERE json_extract(record,'$.state') IN ('queued','running','waiting_dependency','waiting_until','waiting_input') ORDER BY rowid")?;
        stmt.query_map([], |r| r.get::<_, String>(0))?
            .map(|r| Ok(serde_json::from_str(&r?)?))
            .collect()
    }
    pub(crate) fn cancel_queued(&mut self, item: &WorkItem, reason: &str) -> Result<WorkItem> {
        self.change(&item.id, item.revision, |item| {
            if !matches!(
                item.state,
                WorkState::Queued
                    | WorkState::WaitingDependency { .. }
                    | WorkState::WaitingUntil { .. }
                    | WorkState::WaitingInput { .. }
            ) {
                bail!("work is not queued or waiting");
            }
            item.state = WorkState::Canceled {
                reason: reason.into(),
            };
            Ok(("canceled", true))
        })
    }
    pub fn get(&self, id: &str) -> Result<WorkItem> {
        read(&self.store.connection, id)
    }

    /// Associate a Run after unified scheduler admission, before executing side effects.
    /// This does not reserve an execution slot or authorize starting the Run.
    pub fn record_running(&mut self, id: &str, revision: i64, run_id: &str) -> Result<WorkItem> {
        if run_id.trim().is_empty() || run_id.len() > 256 {
            bail!("invalid run id");
        }
        self.change(id, revision, |item| {
            if item.state != WorkState::Queued {
                bail!("work is not queued");
            }
            item.state = WorkState::Running {
                run_id: run_id.into(),
            };
            Ok(("running", false))
        })
    }

    /// Commit result, requester inbox and publication obligation in one transaction.
    /// An unknown outcome cannot be queued again by this API.
    pub fn record_outcome(
        &mut self,
        id: &str,
        revision: i64,
        run_id: &str,
        outcome: WorkOutcome,
    ) -> Result<WorkItem> {
        self.change(id, revision, |item| {
            if !matches!(&item.state, WorkState::Running {run_id: active} if active == run_id) {
                bail!("work attempt is not current");
            }
            let run_id = run_id.to_owned();
            let kind = match outcome {
                WorkOutcome::Succeeded(output) => {
                    if output.trim().is_empty() {
                        bail!("empty output is not a successful work result");
                    }
                    item.state = WorkState::Succeeded { run_id, output };
                    "succeeded"
                }
                WorkOutcome::Failed(error) => {
                    if error.trim().is_empty() {
                        bail!("failure requires an error");
                    }
                    item.state = WorkState::Failed { run_id, error };
                    "failed"
                }
                WorkOutcome::Unknown(reason) => {
                    if reason.trim().is_empty() {
                        bail!("unknown outcome requires a reason");
                    }
                    item.state = WorkState::OutcomeUnknown { run_id, reason };
                    "outcome_unknown"
                }
            };
            Ok((kind, true))
        })
    }

    fn change(
        &mut self,
        id: &str,
        revision: i64,
        apply: impl FnOnce(&mut WorkItem) -> Result<(&'static str, bool)>,
    ) -> Result<WorkItem> {
        let tx = self
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut item = read(&tx, id)?;
        if item.revision != revision {
            bail!("stale work revision");
        }
        let (kind, notify) = apply(&mut item)?;
        if notify {
            tx.execute("DELETE FROM work_dependencies WHERE work_id=?1", [&item.id])?;
        }
        if kind == "running" {
            tx.execute("UPDATE work_inbox SET state='handled' WHERE recipient=?1 AND event_seq IN (SELECT seq FROM work_outbox WHERE work_id=?2 AND kind='requested')",params![item.request.assignee,item.id])?;
        }
        item.revision += 1;
        item.updated_at = crate::schedule_store::now_ms();
        tx.execute(
            "UPDATE work_items SET revision=?2,record=?3 WHERE id=?1",
            params![id, item.revision, serde_json::to_string(&item)?],
        )?;
        append_event(
            &tx,
            &item,
            kind,
            notify.then_some(item.request.requester.as_str()),
            notify,
        )?;
        tx.commit()?;
        agt_storage::wake::notify(&self.store.home);
        Ok(item)
    }

    /// Source cursor; consumers must persist their own acknowledgement after processing.
    pub fn events_after(&self, seq: i64, limit: u32) -> Result<Vec<WorkEvent>> {
        let mut statement = self.store.connection.prepare(
            "SELECT seq,work_id,revision,kind,created_at FROM work_outbox WHERE seq>?1 ORDER BY seq LIMIT ?2")?;
        Ok(statement
            .query_map(params![seq, limit.clamp(1, 500)], event_row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Reading an inbox does not acknowledge its items.
    pub fn inbox(&self, recipient: &str, after: i64, limit: u32) -> Result<Vec<WorkEvent>> {
        let mut statement = self.store.connection.prepare(
            "SELECT e.seq,e.work_id,e.revision,e.kind,e.created_at FROM work_outbox e JOIN work_inbox i ON i.event_seq=e.seq WHERE i.recipient=?1 AND i.state='pending' AND e.seq>?2 ORDER BY e.seq LIMIT ?3")?;
        Ok(statement
            .query_map(params![recipient, after, limit.clamp(1, 500)], event_row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn pending_publications(&self, limit: u32) -> Result<Vec<WorkPublication>> {
        self.publications_after(0, limit)
    }
    pub(crate) fn publications_after(
        &self,
        after: i64,
        limit: u32,
    ) -> Result<Vec<WorkPublication>> {
        let mut statement = self.store.connection.prepare(
            "SELECT p.event_seq,e.work_id,e.revision,p.request_key,p.destination FROM work_publications p JOIN work_outbox e ON e.seq=p.event_seq WHERE p.message_id IS NULL AND p.event_seq>?2 ORDER BY p.event_seq LIMIT ?1")?;
        Ok(statement
            .query_map(params![limit.clamp(1, 500), after], |r| {
                Ok(WorkPublication {
                    event_seq: r.get(0)?,
                    work_id: r.get(1)?,
                    revision: r.get(2)?,
                    request_key: r.get(3)?,
                    destination: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Call only after IM confirms the stable publication key, including on replay.
    /// Publication does not mean the requester has processed its inbox notification.
    pub fn confirm_publication(&self, seq: i64, message_id: &str) -> Result<()> {
        if message_id.trim().is_empty() {
            bail!("publication requires a message id");
        }
        let updated = self.store.connection.execute(
            "UPDATE work_publications SET message_id=?2,error=NULL WHERE event_seq=?1 AND (message_id IS NULL OR message_id=?2)",params![seq,message_id])?;
        if updated != 1 {
            bail!("publication missing or already confirmed with another message");
        }
        Ok(())
    }
}

pub(crate) fn read(c: &Connection, id: &str) -> Result<WorkItem> {
    let value: String = c.query_row("SELECT record FROM work_items WHERE id=?1", [id], |r| {
        r.get(0)
    })?;
    Ok(serde_json::from_str(&value)?)
}

fn event_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<WorkEvent> {
    Ok(WorkEvent {
        seq: r.get(0)?,
        work_id: r.get(1)?,
        revision: r.get(2)?,
        kind: r.get(3)?,
        created_at: r.get(4)?,
    })
}

fn append_event(
    c: &Connection,
    item: &WorkItem,
    kind: &str,
    recipient: Option<&str>,
    publish: bool,
) -> Result<()> {
    c.execute(
        "INSERT INTO work_outbox(work_id,revision,kind,created_at,payload) VALUES(?1,?2,?3,?4,?5)",
        params![
            item.id,
            item.revision,
            kind,
            item.updated_at,
            serde_json::to_string(item)?
        ],
    )?;
    let seq = c.last_insert_rowid();
    if matches!(
        kind,
        "succeeded" | "failed" | "canceled" | "outcome_unknown" | "resolved"
    ) {
        c.execute(
            "INSERT INTO work_reports(event_seq,work_id,created_at) VALUES(?1,?2,?3)",
            params![seq, item.id, item.updated_at],
        )?;
    }

    if let Some(recipient) = recipient {
        c.execute(
            "INSERT INTO work_inbox VALUES(?1,?2,'pending')",
            params![recipient, seq],
        )?;
    }
    if publish {
        // Terminal reports and later human verification have distinct stable keys.
        let mut key = format!(
            "work:{}:attempt:1:report:{}",
            item.id,
            crate::fingerprint(item.request.report_to.as_bytes())
        );
        if !matches!(
            kind,
            "succeeded" | "failed" | "canceled" | "outcome_unknown"
        ) {
            key.push_str(&format!(":resolution:{}", item.revision));
        }
        c.execute(
            "INSERT INTO work_publications(event_seq,request_key,destination,message_id) VALUES(?1,?2,?3,NULL)",
            params![seq, key, item.request.report_to],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
