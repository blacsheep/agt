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

crate::string_enum::string_enum! {
    pub enum ApprovalSource {
        Native = "native",
        Harness = "harness",
    }
}

crate::string_enum::string_enum! {
    /// Decision state. Only Native model review uses `Reviewing`.
    pub enum ApprovalState {
        Reviewing = "reviewing",
        AwaitingHuman = "awaiting_human",
        Approved = "approved",
        Denied = "denied",
        Canceled = "canceled",
        Expired = "expired",
    }
}

impl ApprovalState {
    pub fn is_pending(self) -> bool {
        matches!(self, Self::Reviewing | Self::AwaitingHuman)
    }
}

crate::string_enum::string_enum! {
    /// Whether a decision reached the requesting engine. Native requests stay `NotSent`
    /// until applied to the call, then become `Resolved`.
    pub enum Delivery {
        NotSent = "not_sent",
        Sending = "sending",
        Sent = "sent",
        Resolved = "resolved",
        Invalidated = "invalidated",
        Unknown = "unknown",
    }
}

impl Delivery {
    /// No further delivery transition is possible.
    pub fn is_final(self) -> bool {
        matches!(self, Self::Resolved | Self::Invalidated | Self::Unknown)
    }

    /// Terminal delivery for a request abandoned in this state.
    fn abandoned(self) -> Self {
        if self == Self::Sending {
            Self::Unknown
        } else {
            Self::Invalidated
        }
    }
}

impl From<agt_harness::DeliveryProgress> for Delivery {
    fn from(progress: agt_harness::DeliveryProgress) -> Self {
        match progress {
            agt_harness::DeliveryProgress::Sending => Self::Sending,
            agt_harness::DeliveryProgress::Sent => Self::Sent,
            agt_harness::DeliveryProgress::Resolved => Self::Resolved,
        }
    }
}

crate::string_enum::string_enum! {
    pub enum DecisionOrigin {
        Human = "human",
        LocalAdmin = "local_admin",
        AutoAllow = "auto_allow",
        ModelReview = "model_review",
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Approval {
    pub id: String,
    pub revision: i64,
    pub source: ApprovalSource,
    pub run_id: String,
    pub work_id: Option<String>,
    pub conversation: Option<String>,
    pub principal: Option<String>,
    pub request: Value,
    pub state: ApprovalState,
    pub decision: Option<Value>,
    pub reason: String,
    pub actor: Option<String>,
    pub decision_origin: Option<DecisionOrigin>,
    pub delivery: Delivery,
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
pub(crate) const SCHEMA: agt_storage::sqlite::Schema = agt_storage::sqlite::Schema {
    module: "approvals",
    version: 1,
    ddl: "CREATE TABLE approvals(id TEXT PRIMARY KEY,run_id TEXT NOT NULL,request_key TEXT NOT NULL,scope TEXT NOT NULL,value TEXT NOT NULL,lease_until INTEGER NOT NULL,UNIQUE(run_id,request_key));",
};
pub struct ApprovalStore {
    home: PathBuf,
    db: agt_storage::sqlite::Db,
}
fn now() -> i64 {
    crate::schedule_store::now_ms()
}
impl ApprovalStore {
    pub fn open(home: &Path) -> Result<Self> {
        let db = crate::store::runtime_db(home)?;
        Ok(Self {
            home: home.into(),
            db,
        })
    }
    pub(crate) fn create(
        &self,
        scope: &Scope,
        source: ApprovalSource,
        key: &str,
        request: Value,
        timeout: u64,
    ) -> Result<Approval> {
        let tx = agt_storage::sqlite::write(&self.db)?;
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
            source,
            run_id: scope.run_id.clone(),
            work_id: scope.task.as_ref().and_then(|t| t.work_id.clone()),
            conversation: scope.conversation.clone(),
            principal: scope.principal.clone(),
            request,
            state: ApprovalState::AwaitingHuman,
            decision: None,
            reason: String::new(),
            actor: None,
            decision_origin: None,
            delivery: Delivery::NotSent,
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
        let tx = agt_storage::sqlite::write(&self.db)?;
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
            && item.decision_origin == Some(DecisionOrigin::Human)
            && item.delivery == Delivery::NotSent
            && !self.visible(&item, Some(actor), true)?
        {
            self.change(id, None, Some(Delivery::Invalidated))?;
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
        state: Option<ApprovalState>,
        delivery: Option<Delivery>,
    ) -> Result<()> {
        self.reconcile()?;
        let old = self.raw(id)?;
        let mut next = old.clone();
        if let Some(state) = state {
            if old.decision.is_some() || old.delivery != Delivery::NotSent {
                return Ok(());
            }
            next.state = state;
        }
        if let Some(delivery) = delivery {
            if old.delivery.is_final() {
                bail!("approval is no longer live");
            }
            next.delivery = delivery;
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
            if item.run_id == run && !item.delivery.is_final() {
                self.change(
                    &item.id,
                    item.decision.is_none().then_some(ApprovalState::Canceled),
                    Some(item.delivery.abandoned()),
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
        next.state = ApprovalState::AwaitingHuman;
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
            if old.delivery.is_final() {
                continue;
            }
            let mut next = old.clone();
            next.revision += 1;
            if next.decision.is_none() {
                next.state = if now() >= next.expires_at {
                    ApprovalState::Expired
                } else {
                    ApprovalState::Canceled
                };
            }
            next.delivery = old.delivery.abandoned();
            // A live decision may win this race; the next read will reconcile it.
            let _ = self.update(&old, &next);
        }
        Ok(())
    }
}
