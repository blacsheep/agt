//! Durable human decisions. Engine policy remains with the engine that requested approval.
use crate::{RunRecord, ScheduledTask};
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

mod automatic;
mod control;
#[cfg(test)]
mod tests;

#[derive(Clone, Serialize, Deserialize)]
pub struct Approval {
    pub id: String,
    pub revision: i64,
    pub source: String,
    pub run_id: String,
    pub work_id: Option<String>,
    pub conversation: Option<String>,
    pub principal: Option<String>,
    pub request: Value,
    pub state: String,
    pub decision: Option<Value>,
    pub reason: String,
    pub actor: Option<String>,
    pub decision_origin: Option<String>,
    pub delivery: String,
    pub created_at: i64,
    pub expires_at: i64,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Scope {
    pub run_id: String,
    pub execution_id: String,
    pub task: Option<ScheduledTask>,
    pub conversation: Option<String>,
    pub principal: Option<String>,
}
impl Scope {
    pub(crate) fn new(record: &RunRecord, task: Option<&ScheduledTask>) -> Self {
        Self {
            run_id: record.id.clone(),
            execution_id: uuid::Uuid::new_v4().to_string(),
            task: task.cloned(),
            conversation: record
                .request
                .trigger
                .as_ref()
                .map(|t| t.conversation_id.to_string()),
            principal: record
                .request
                .trigger
                .as_ref()
                .map(|t| t.principal.to_string()),
        }
    }
}
pub struct ApprovalStore {
    home: PathBuf,
    db: Connection,
}
fn now() -> i64 {
    crate::schedule_store::now_ms()
}
impl ApprovalStore {
    pub fn open(home: &Path) -> Result<Self> {
        let db = Connection::open(agt_storage::database_path(home, "runtime", "approvals.db")?)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 1 {
            bail!("unsupported approval schema");
        }
        db.execute_batch("BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS approvals(id TEXT PRIMARY KEY,run_id TEXT NOT NULL,request_key TEXT NOT NULL,scope TEXT NOT NULL,value TEXT NOT NULL,lease_until INTEGER NOT NULL,UNIQUE(run_id,request_key));
            CREATE TABLE IF NOT EXISTS change_outbox(seq INTEGER PRIMARY KEY AUTOINCREMENT,conversation TEXT,kind TEXT NOT NULL,at INTEGER NOT NULL);
            PRAGMA user_version=1; COMMIT;")?;
        Ok(Self {
            home: home.into(),
            db,
        })
    }
    pub(crate) fn create(
        &self,
        scope: &Scope,
        source: &str,
        key: &str,
        request: Value,
        timeout: u64,
    ) -> Result<Approval> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.db,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        if let Some((value, stored_scope)) = tx
            .query_row(
                "SELECT value,scope FROM approvals WHERE run_id=?1 AND request_key=?2",
                params![scope.run_id, key],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            let item: Approval = serde_json::from_str(&value)?;
            let stored_scope: Scope = serde_json::from_str(&stored_scope)?;
            if stored_scope.execution_id != scope.execution_id {
                bail!("approval belongs to a previous execution");
            }
            if item.request != request || item.source != source {
                bail!("approval key conflict");
            }
            return Ok(item);
        }
        let item = Approval {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            source: source.into(),
            run_id: scope.run_id.clone(),
            work_id: scope.task.as_ref().and_then(|t| t.work_id.clone()),
            conversation: scope.conversation.clone(),
            principal: scope.principal.clone(),
            request,
            state: "awaiting_human".into(),
            decision: None,
            reason: String::new(),
            actor: None,
            decision_origin: None,
            delivery: "not_sent".into(),
            created_at: now(),
            expires_at: now().saturating_add((timeout.min(86400) * 1000) as i64),
        };
        tx.execute(
            "INSERT INTO approvals VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                item.id,
                item.run_id,
                key,
                serde_json::to_string(scope)?,
                serde_json::to_string(&item)?,
                now() + 30_000
            ],
        )?;
        Self::notify(&tx, &item)?;
        tx.commit()?;
        agt_storage::wake::notify(&self.home);
        Ok(item)
    }
    fn notify(db: &Connection, item: &Approval) -> Result<()> {
        db.execute(
            "INSERT INTO change_outbox(conversation,kind,at) VALUES(?1,'approval',?2)",
            params![item.conversation, now()],
        )?;
        Ok(())
    }
    fn raw(&self, id: &str) -> Result<Approval> {
        let text: String =
            self.db
                .query_row("SELECT value FROM approvals WHERE id=?1", [id], |r| {
                    r.get(0)
                })?;
        Ok(serde_json::from_str(&text)?)
    }
    fn update(&self, previous: &Approval, next: &Approval) -> Result<()> {
        let tx = self.db.unchecked_transaction()?;
        if tx.execute(
            "UPDATE approvals SET value=?2 WHERE id=?1 AND value=?3",
            params![
                previous.id,
                serde_json::to_string(next)?,
                serde_json::to_string(previous)?
            ],
        )? != 1
        {
            bail!("approval changed; refresh before deciding");
        }
        Self::notify(&tx, next)?;
        tx.commit()?;
        agt_storage::wake::notify(&self.home);
        Ok(())
    }
    pub(crate) fn poll(&self, id: &str) -> Result<Approval> {
        self.reconcile()?;
        let item = self.raw(id)?;
        if let Some(actor) = item.actor.as_deref()
            && item.decision_origin.as_deref() == Some("human")
            && item.delivery == "not_sent"
            && !self.visible(&item, Some(actor), true)?
        {
            self.change(id, None, Some("invalidated"))?;
            return self.raw(id);
        }
        self.db.execute(
            "UPDATE approvals SET lease_until=?2 WHERE id=?1 AND lease_until>?3 AND lease_until<?4",
            params![id, now() + 30_000, now(), now() + 25_000],
        )?;
        Ok(item)
    }
    pub(crate) fn change(
        &self,
        id: &str,
        state: Option<&str>,
        delivery: Option<&str>,
    ) -> Result<()> {
        self.reconcile()?;
        let old = self.raw(id)?;
        let mut next = old.clone();
        if let Some(state) = state {
            if old.decision.is_some() || old.delivery != "not_sent" {
                return Ok(());
            }
            next.state = state.into();
        }
        if let Some(delivery) = delivery {
            if matches!(
                old.delivery.as_str(),
                "invalidated" | "unknown" | "resolved"
            ) {
                bail!("approval is no longer live");
            }
            next.delivery = delivery.into();
        }
        next.revision += 1;
        match self.update(&old, &next) {
            // A human can decide immediately after creation, before model review begins.
            Err(_) if state.is_some() && delivery.is_none() && self.raw(id)?.decision.is_some() => {
                Ok(())
            }
            result => result,
        }
    }
    pub(crate) fn invalidate_run(&self, run: &str) -> Result<()> {
        for item in self.all()? {
            if item.run_id == run
                && !matches!(
                    item.delivery.as_str(),
                    "resolved" | "invalidated" | "unknown"
                )
            {
                self.change(
                    &item.id,
                    if item.decision.is_none() {
                        Some("canceled")
                    } else {
                        None
                    },
                    Some(if item.delivery == "sending" {
                        "unknown"
                    } else {
                        "invalidated"
                    }),
                )?;
            }
        }
        Ok(())
    }
    pub(crate) fn fallback(&self, id: &str, revision: i64, reason: &str) -> Result<()> {
        let old = self.raw(id)?;
        if old.revision != revision || old.decision.is_some() {
            return Ok(());
        }
        let mut next = old.clone();
        next.revision += 1;
        next.state = "awaiting_human".into();
        next.reason = reason.into();
        match self.update(&old, &next) {
            Ok(()) => Ok(()),
            Err(_) if self.raw(id)?.revision != revision => Ok(()),
            Err(e) => Err(e),
        }
    }
    fn all(&self) -> Result<Vec<Approval>> {
        let mut stmt = self
            .db
            .prepare("SELECT value FROM approvals ORDER BY rowid DESC")?;
        stmt.query_map([], |r| r.get::<_, String>(0))?
            .map(|r| Ok(serde_json::from_str(&r?)?))
            .collect()
    }
    fn reconcile(&self) -> Result<()> {
        let mut stmt=self.db.prepare("SELECT value FROM approvals WHERE lease_until<=?1 OR json_extract(value,'$.expires_at')<=?1")?;
        let rows = stmt
            .query_map([now()], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for text in rows {
            let old: Approval = serde_json::from_str(&text)?;
            if matches!(
                old.delivery.as_str(),
                "resolved" | "invalidated" | "unknown"
            ) {
                continue;
            }
            let mut next = old.clone();
            next.revision += 1;
            if next.decision.is_none() {
                next.state = if now() >= next.expires_at {
                    "expired"
                } else {
                    "canceled"
                }
                .into();
            }
            next.delivery = if old.delivery == "sending" {
                "unknown"
            } else {
                "invalidated"
            }
            .into();
            // A live decision may win this race; the next read will reconcile it.
            let _ = self.update(&old, &next);
        }
        Ok(())
    }
}
