//! Execution state is separate from the expiring, generation-fenced ownership lease.
use super::*;
use rusqlite::OptionalExtension;
use std::{sync::mpsc, thread, time::Duration};

const LEASE_MS: i64 = 30_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Preparing,
    Running,
    Canceling,
    Reconciling,
    Succeeded,
    Failed,
    Canceled,
    Interrupted,
    NeedsReview,
}
impl ExecutionState {
    pub fn blocks(self) -> bool {
        matches!(
            self,
            Self::Preparing
                | Self::Running
                | Self::Canceling
                | Self::Reconciling
                | Self::NeedsReview
        )
    }
    fn text(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::Canceling => "canceling",
            Self::Reconciling => "reconciling",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Interrupted => "interrupted",
            Self::NeedsReview => "needs_review",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionExecution {
    pub id: String,
    pub state: ExecutionState,
    pub generation: i64,
    pub reason: Option<String>,
}
impl HarnessSessions {
    /// Recovery never treats an expired lease as permission to replay external effects.
    pub fn reconcile(&self) -> Result<()> {
        let before = self.db.total_changes();
        let now = crate::schedule_store::now_ms();
        self.db.execute("UPDATE harness_operations SET state=CASE WHEN state IN ('failed','canceled','interrupted','succeeded') THEN state WHEN submitted=0 THEN 'failed' ELSE 'reconciling' END,owner=NULL,expires=0,reason=CASE WHEN submitted=0 THEN 'Owner exited before submission' ELSE 'Execution owner lease expired; external result must be checked' END WHERE owner IS NOT NULL AND expires<=?1", [now])?;
        // This adapter cannot reattach a live external turn. Keep the original thread intact.
        self.db.execute("UPDATE harness_operations SET state='needs_review',reason='Connection lost; inspect the original engine session and its effects before starting a new context' WHERE state='reconciling' AND owner IS NULL", [])?;
        if self.db.total_changes() != before {
            agt_storage::wake::notify(&self.home);
        }
        Ok(())
    }
    pub(super) fn execution(&self, id: &str) -> Result<Option<SessionExecution>> {
        self.reconcile()?;
        let row: Option<(String, String, i64, Option<String>)> = self
            .db
            .query_row(
                "SELECT id,state,generation,reason FROM harness_operations WHERE scope=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        row.map(|(id, state, generation, reason)| {
            Ok(SessionExecution {
                id,
                state: serde_json::from_value(json!(state))?,
                generation,
                reason,
            })
        })
        .transpose()
    }
    pub(crate) fn execution_lock<'a>(&'a self, id: &'a str) -> Result<ExecutionLock<'a>> {
        self.reconcile()?;
        let owner = uuid::Uuid::new_v4().to_string();
        let now = crate::schedule_store::now_ms();
        let tx_db = self.db.clone();
        let tx = agt_storage::sqlite::write(&tx_db)?;
        let acquired = (|| -> Result<i64> {
            let exists: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM harness_sessions WHERE scope=?1)",
                [id],
                |r| r.get(0),
            )?;
            if !exists {
                bail!("session does not exist");
            }
            let changed = self.db.execute("INSERT INTO harness_operations(scope,id,state,owner,generation,expires) VALUES(?1,?2,'preparing',?2,1,?3) ON CONFLICT(scope) DO UPDATE SET id=excluded.id,state='preparing',owner=excluded.owner,generation=harness_operations.generation+1,expires=excluded.expires,submitted=0,cancel=0,reason=NULL,output=NULL WHERE harness_operations.owner IS NULL AND harness_operations.state IN ('succeeded','failed','canceled','interrupted')",params![id,owner,now+LEASE_MS])?;
            if changed != 1 {
                bail!(
                    "session is active, recovering, or needs review; inspect its execution state"
                );
            }
            Ok(self.db.query_row(
                "SELECT generation FROM harness_operations WHERE scope=?1",
                [id],
                |r| r.get(0),
            )?)
        })();
        if acquired.is_ok() {
            tx.commit()?;
        } else {
            drop(tx);
        }
        let generation = acquired?;
        agt_storage::wake::notify(&self.home);
        let path = self
            .db
            .path()
            .context("session database has no path")?
            .to_owned();
        let (stop, receiver) = mpsc::channel();
        let scope = id.to_owned();
        let heartbeat_owner = owner.clone();
        let heartbeat = thread::spawn(move || {
            let Ok(db) = Connection::open(path) else {
                return;
            };
            let _ = db.busy_timeout(agt_storage::sqlite::BUSY_TIMEOUT);
            while receiver.recv_timeout(Duration::from_secs(5))
                == Err(mpsc::RecvTimeoutError::Timeout)
            {
                let now = crate::schedule_store::now_ms();
                if !matches!(db.execute("UPDATE harness_operations SET expires=?4 WHERE scope=?1 AND owner=?2 AND generation=?3 AND expires>?5",params![scope,heartbeat_owner,generation,now+LEASE_MS,now]),Ok(1)) { break; }
            }
        });
        Ok(ExecutionLock {
            sessions: self,
            id,
            owner,
            generation,
            stop,
            heartbeat: Some(heartbeat),
        })
    }
}
pub(crate) struct ExecutionLock<'a> {
    sessions: &'a HarnessSessions,
    id: &'a str,
    owner: String,
    generation: i64,
    stop: mpsc::Sender<()>,
    heartbeat: Option<thread::JoinHandle<()>>,
}
impl ExecutionLock<'_> {
    pub(crate) fn fenced<T>(&self, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        let before = self.sessions.db.total_changes();
        let tx_db = self.sessions.db.clone();
        let tx = agt_storage::sqlite::write(&tx_db)?;
        let result = (|| {
            let owns:bool=self.sessions.db.query_row("SELECT EXISTS(SELECT 1 FROM harness_operations WHERE scope=?1 AND owner=?2 AND generation=?3 AND expires>?4)",params![self.id,self.owner,self.generation,crate::schedule_store::now_ms()],|r|r.get(0))?;
            if !owns {
                bail!("execution lease lost; stale operation rejected");
            }
            operation()
        })();
        if result.is_ok() {
            tx.commit()?;
        } else {
            drop(tx);
        }
        if result.is_ok() && self.sessions.db.total_changes() != before {
            agt_storage::wake::notify(&self.sessions.home);
        }
        result
    }
    pub(crate) fn check(&self) -> Result<()> {
        let owns: bool = self.sessions.db.query_row("SELECT EXISTS(SELECT 1 FROM harness_operations WHERE scope=?1 AND owner=?2 AND generation=?3 AND expires>?4)",params![self.id,self.owner,self.generation,crate::schedule_store::now_ms()],|r|r.get(0))?;
        if !owns {
            bail!("execution lease lost; stale operation rejected");
        }
        Ok(())
    }
    pub(crate) fn canceling(&self) -> Result<()> {
        self.fenced(|| {
            self.sessions.db.execute("UPDATE harness_operations SET cancel=1,state='canceling' WHERE scope=?1 AND state IN ('preparing','running','canceling')",[self.id])?;
            Ok(())
        })
    }
    pub(crate) fn submitted(&self) -> Result<()> {
        self.fenced(|| {
            let changed = self.sessions.db.execute(
                "UPDATE harness_operations SET submitted=1 WHERE scope=?1 AND state='preparing' AND cancel=0",
                [self.id],
            )?;
            if changed != 1 { bail!("submission requires an uncanceled preparing execution"); }
            Ok(())
        })
    }
    pub(crate) fn running(&self) -> Result<()> {
        self.fenced(|| {
            let changed=self.sessions.db.execute("UPDATE harness_operations SET state=CASE WHEN cancel=1 THEN 'canceling' ELSE 'running' END WHERE scope=?1 AND state IN ('preparing','running','canceling')",[self.id])?;
            if changed != 1 { bail!("terminal execution cannot become running"); }
            Ok(())
        })
    }
    pub(crate) fn settled(&self, status: &str) -> Result<()> {
        let state = match status {
            "rejected" | "failed" => ExecutionState::Failed,
            "interrupted" => {
                let requested: bool = self.sessions.db.query_row(
                    "SELECT cancel FROM harness_operations WHERE scope=?1",
                    [self.id],
                    |r| r.get(0),
                )?;
                if requested {
                    ExecutionState::Canceled
                } else {
                    ExecutionState::Interrupted
                }
            }
            _ => return Ok(()),
        };
        // Keep ownership until client cleanup; Drop releases after the operation unwinds.
        self.fenced(|| {
            self.sessions.db.execute(
                "UPDATE harness_operations SET state=?2,reason=?3 WHERE scope=?1",
                params![self.id, state.text(), format!("Engine confirmed {status}")],
            )?;
            Ok(())
        })
    }
    pub(crate) fn finish(&self, state: ExecutionState, output: Option<&str>) -> Result<()> {
        self.fenced(|| { self.sessions.db.execute("UPDATE harness_operations SET state=?2,output=?3,owner=NULL,expires=0 WHERE scope=?1",params![self.id,state.text(),output])?; Ok(()) })
    }
}
impl Drop for ExecutionLock<'_> {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.heartbeat.take() {
            let _ = thread.join();
        }
        let _ = self.fenced(|| {
            self.sessions.db.execute("UPDATE harness_operations SET state=CASE WHEN state IN ('failed','canceled','interrupted','succeeded') THEN state WHEN submitted=0 THEN CASE WHEN cancel=1 THEN 'canceled' ELSE 'failed' END ELSE 'needs_review' END,reason=COALESCE(reason,'Execution ended without a confirmed durable result'),owner=NULL,expires=0 WHERE scope=?1",[self.id])?;
            Ok(())
        });
    }
}
