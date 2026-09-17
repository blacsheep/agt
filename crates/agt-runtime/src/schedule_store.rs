use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

pub(crate) const LEASE_MS: i64 = 15_000;
pub(crate) const QUIET_MS: i64 = 5_000;
pub(crate) const MAX_REPLIES: u32 = 100;
pub(crate) const ROUND_MS: i64 = 30 * 60 * 1000;

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordedDecision {
    #[serde(flatten)]
    pub decision: crate::participation::ParticipationDecision,
    pub decided_at: i64,
    pub through_seq: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub discussion: String,
    pub principal: String,
    pub conversation: String,
    pub state: String,
    pub deliveries: Vec<i64>,
    pub message: String,
    pub through_seq: i64,
    pub usage_epoch: i64,
    pub triggers: Vec<String>,
    pub decision_required: bool,
    pub quiet: bool,
    pub due_at: i64,
    pub deadline: i64,
    pub quiet_ms: i64,
    pub version: i64,
    pub owner: Option<String>,
    pub run_id: Option<String>,
    pub reason: Option<String>,
    #[serde(default)]
    pub last_decision: Option<RecordedDecision>,
    #[serde(default)]
    pub work_id: Option<String>,
    #[serde(default)]
    pub work_notification: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Discussion {
    pub id: String,
    pub conversation: String,
    pub state: String,
    pub calls: u32,
    pub max_replies: u32,
    pub expires_at: i64,
    pub allow_mcp: bool,
}

#[derive(Serialize)]
pub struct ScheduleStatus {
    pub online: bool,
    pub discussions: Vec<Discussion>,
    pub tasks: Vec<ScheduledTask>,
    pub usage: Vec<ReplyUsage>,
}

#[derive(Serialize)]
pub struct ReplyUsage {
    pub discussion: String,
    pub principal: String,
    pub replies: u32,
}

pub struct ScheduleStore {
    pub(crate) connection: Connection,
}

impl ScheduleStore {
    pub fn open(home: &Path) -> Result<Self> {
        let mut connection = Connection::open(crate::store::database_path(
            home,
            "runtime",
            "scheduler.db",
        )?)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        // SQLite's journal-mode transition can return BUSY immediately even
        // with busy_timeout when the coordinator and status reader open a new DB.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match connection.pragma_update(None, "journal_mode", "WAL") {
                Ok(()) => break,
                Err(error)
                    if matches!(
                        error.sqlite_error_code(),
                        Some(
                            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                        )
                    ) && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        }
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u32 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        match version {
            0 => tx.execute_batch("CREATE TABLE coordinator(id INTEGER PRIMARY KEY CHECK(id=1), owner TEXT NOT NULL, expires INTEGER NOT NULL);
                CREATE TABLE discussions(id TEXT PRIMARY KEY,conversation TEXT NOT NULL,state TEXT NOT NULL,calls INTEGER NOT NULL DEFAULT 0,expires INTEGER NOT NULL,allow_mcp INTEGER NOT NULL,last_human_seq INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE tasks(id TEXT PRIMARY KEY,discussion TEXT NOT NULL,principal TEXT NOT NULL,conversation TEXT NOT NULL,state TEXT NOT NULL,due INTEGER NOT NULL,quiet INTEGER NOT NULL,version INTEGER NOT NULL,owner TEXT,record TEXT NOT NULL);
                CREATE UNIQUE INDEX active_principal ON tasks(principal) WHERE state IN ('evaluating','ready_to_reply','responding');
                CREATE UNIQUE INDEX autonomous_reply_slot ON tasks(conversation) WHERE state='responding' AND json_extract(record,'$.decision_required')=1;
                CREATE INDEX task_queue ON tasks(state,due);
                CREATE TABLE assigned(delivery INTEGER PRIMARY KEY,task TEXT NOT NULL);
                CREATE TABLE usage(discussion TEXT NOT NULL,principal TEXT NOT NULL,replies INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(discussion,principal));
                CREATE TABLE reconciled(task TEXT PRIMARY KEY);
                CREATE TABLE human_messages(message TEXT PRIMARY KEY,discussion TEXT NOT NULL);
                PRAGMA user_version=3;")?,
            3..=7 => {},
            _ => bail!("unsupported scheduler database version"),
        }
        tx.execute_batch("CREATE TABLE IF NOT EXISTS context_resets(conversation TEXT PRIMARY KEY,through_seq INTEGER NOT NULL);")?;
        crate::work::migrate(&tx)?;
        tx.commit()?;
        Ok(Self { connection })
    }

    pub(crate) fn context_cutoff(&self, conversation: &str) -> Result<i64> {
        Ok(self
            .connection
            .query_row(
                "SELECT through_seq FROM context_resets WHERE conversation=?1",
                [conversation],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    #[cfg(test)]
    pub(crate) fn start(
        &self,
        id: &str,
        conversation: &str,
        allow_mcp: bool,
        now: i64,
    ) -> Result<()> {
        self.connection.execute("INSERT OR IGNORE INTO discussions(id,conversation,state,expires,allow_mcp) VALUES(?1,?2,'active',?3,?4)", params![id,conversation,now+ROUND_MS,allow_mcp])?;
        Ok(())
    }

    pub(crate) fn authorize_human(
        &mut self,
        message: &str,
        conversation: &str,
        seq: i64,
        allow_mcp: bool,
        now: i64,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx
            .query_row(
                "SELECT 1 FROM human_messages WHERE message=?1",
                [message],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            return Ok(());
        }
        let existing: Option<(String, i64)> = tx.query_row(
            "SELECT id,last_human_seq FROM discussions WHERE conversation=?1 AND state!='ended' ORDER BY rowid DESC LIMIT 1",
            [conversation], |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional()?;
        let (discussion, previous_seq) = existing.unwrap_or_else(|| (message.into(), 0));
        tx.execute("INSERT OR IGNORE INTO discussions(id,conversation,state,expires,allow_mcp) VALUES(?1,?2,'active',?3,?4)", params![discussion,conversation,now+ROUND_MS,allow_mcp])?;
        tx.execute(
            "INSERT INTO human_messages VALUES(?1,?2)",
            params![message, discussion],
        )?;
        if seq > previous_seq {
            tx.execute("DELETE FROM usage WHERE discussion=?1", [&discussion])?;
            tx.execute("UPDATE discussions SET state='active',last_human_seq=?2,expires=?3,allow_mcp=MAX(allow_mcp,?4) WHERE id=?1", params![discussion,seq,now+ROUND_MS,allow_mcp])?;
            // A human turn wakes pending work, including cross-conversation waits.
            // Running snapshots and possible external effects remain untouched.
            for mut task in query_tasks(
                &tx,
                "SELECT record FROM tasks WHERE state IN ('waiting','ready_to_reply','limited','blocked')",
            )? {
                if task.discussion == discussion {
                    task.state = "queued".into();
                    task.due_at = now + QUIET_MS;
                    task.quiet = false;
                    task.reason = None;
                    task.owner = None;
                    task.version += 1;
                    save_task(&tx, &task)?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn human_discussion(&self, message: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT discussion FROM human_messages WHERE message=?1",
                [message],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn control(
        &mut self,
        conversation: &str,
        action: &str,
        allow_mcp: bool,
        now: i64,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        match action {
            "pause" | "end" => {
                tx.execute(
                    "UPDATE discussions SET state=?2 WHERE (conversation=?1 OR (?2='ended' AND id IN (SELECT discussion FROM tasks WHERE conversation=?1))) AND state!='ended'",
                    params![
                        conversation,
                        if action == "pause" { "paused" } else { "ended" }
                    ],
                )?;
            }
            "resume" => {
                tx.execute("UPDATE discussions SET state='active',expires=?2,allow_mcp=MAX(allow_mcp,?3) WHERE conversation=?1 AND state!='ended'",params![conversation,now+ROUND_MS,allow_mcp])?;
                for mut task in query_tasks(
                    &tx,
                    "SELECT t.record FROM tasks t JOIN discussions d ON d.id=t.discussion WHERE t.state='blocked' AND d.state='active'",
                )? {
                    let root: String = tx.query_row(
                        "SELECT conversation FROM discussions WHERE id=?1",
                        [&task.discussion],
                        |r| r.get(0),
                    )?;
                    if root == conversation {
                        task.state = "queued".into();
                        task.due_at = now;
                        task.reason = None;
                        save_task(&tx, &task)?;
                    }
                }
            }
            _ => bail!("unknown scheduler control"),
        }
        tx.commit()?;
        Ok(())
    }

    pub fn discussion(&self, id: &str) -> Result<Option<Discussion>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id,conversation,state,calls,expires,allow_mcp FROM discussions WHERE id=?1",
                [id],
                discussion_row,
            )
            .optional()?)
    }

    pub(crate) fn acquire(&mut self, owner: &str, now: i64) -> Result<bool> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active: Option<String> = tx
            .query_row(
                "SELECT owner FROM coordinator WHERE id=1 AND expires>?1",
                [now],
                |r| r.get(0),
            )
            .optional()?;
        if active.as_deref().is_some_and(|v| v != owner) {
            return Ok(false);
        }
        if active.is_none() {
            // Never replay ambiguous tool execution. Pure decisions can retry.
            let records = query_tasks(
                &tx,
                "SELECT record FROM tasks WHERE state IN ('evaluating','responding')",
            )?;
            for mut task in records {
                task.state = if task.state == "evaluating" {
                    "queued"
                } else {
                    "interrupted"
                }
                .into();
                task.reason = Some("coordinator lease expired".into());
                task.owner = None;
                task.version += 1;
                save_task(&tx, &task)?;
            }
        }
        tx.execute("INSERT INTO coordinator VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET owner=excluded.owner,expires=excluded.expires",params![owner,now+LEASE_MS])?;
        tx.commit()?;
        Ok(true)
    }

    pub(crate) fn heartbeat(&self, owner: &str, now: i64) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE coordinator SET expires=?2 WHERE id=1 AND owner=?1 AND expires>?3",
            params![owner, now + LEASE_MS, now],
        )? == 1)
    }
    pub(crate) fn release(&self, owner: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM coordinator WHERE owner=?1", [owner])?;
        Ok(())
    }

    pub(crate) fn ingest(&mut self, mut task: ScheduledTask) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx
            .query_row(
                "SELECT 1 FROM assigned WHERE delivery=?1",
                [task.deliveries[0]],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            return Ok(());
        }
        let existing: Option<String> = if task.work_id.is_some() {
            None
        } else {
            tx.query_row("SELECT record FROM tasks WHERE discussion=?1 AND principal=?2 AND conversation=?3 AND json_extract(record,'$.work_id') IS NULL AND state IN ('queued','waiting','ready_to_reply') ORDER BY rowid LIMIT 1",params![task.discussion,task.principal,task.conversation],|r|r.get(0)).optional()?
        };
        let delivery = task.deliveries[0];
        if let Some(record) = existing {
            let mut merged: ScheduledTask = serde_json::from_str(&record)?;
            merged.deliveries.push(delivery);
            merged.triggers.extend(task.triggers);
            if task.through_seq > merged.through_seq {
                merged.through_seq = task.through_seq;
                merged.message = task.message;
            }
            merged.decision_required &= task.decision_required;
            if merged.state == "ready_to_reply" {
                merged.state = "queued".into();
                merged.reason = None;
                merged.owner = None;
                merged.version += 1;
            }
            merged.due_at = merged.due_at.max(task.due_at);
            if !task.decision_required {
                merged.state = "queued".into();
                merged.quiet = false;
                merged.due_at = task.due_at;
                merged.deadline = task.deadline;
            }
            task = merged;
            save_task(&tx, &task)?;
        } else {
            tx.execute(
                "INSERT INTO tasks VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    task.id,
                    task.discussion,
                    task.principal,
                    task.conversation,
                    task.state,
                    task.due_at,
                    task.quiet,
                    task.version,
                    task.owner,
                    serde_json::to_string(&task)?
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO assigned VALUES(?1,?2)",
            params![delivery, task.id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn candidates(&self) -> Result<Vec<ScheduledTask>> {
        let mut tasks = query_tasks(
            &self.connection,
            "SELECT t.record FROM tasks t JOIN discussions d ON d.id=t.discussion WHERE t.state IN ('queued','waiting','ready_to_reply') AND d.state='active' ORDER BY CASE WHEN t.state='ready_to_reply' THEN 0 ELSE 1 END,t.due,t.rowid",
        )?;
        crate::work::queue::order(&self.connection, &mut tasks, now_ms())?;
        Ok(tasks)
    }
    pub(crate) fn tasks(&self) -> Result<Vec<ScheduledTask>> {
        query_tasks(
            &self.connection,
            "SELECT record FROM tasks ORDER BY rowid DESC",
        )
    }
    pub(crate) fn unreconciled(&self) -> Result<Vec<ScheduledTask>> {
        query_tasks(
            &self.connection,
            "SELECT record FROM tasks WHERE id NOT IN (SELECT task FROM reconciled) ORDER BY rowid DESC",
        )
    }
    pub(crate) fn mark_reconciled(&self, id: &str) -> Result<()> {
        self.connection
            .execute("INSERT OR IGNORE INTO reconciled VALUES(?1)", [id])?;
        Ok(())
    }
    pub(crate) fn get(&self, id: &str) -> Result<ScheduledTask> {
        let record: String =
            self.connection
                .query_row("SELECT record FROM tasks WHERE id=?1", [id], |r| r.get(0))?;
        Ok(serde_json::from_str(&record)?)
    }

    pub(crate) fn claim(
        &mut self,
        id: &str,
        owner: &str,
        now: i64,
    ) -> Result<Option<ScheduledTask>> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record: String =
            tx.query_row("SELECT record FROM tasks WHERE id=?1", [id], |r| r.get(0))?;
        let mut task: ScheduledTask = serde_json::from_str(&record)?;
        if !matches!(task.state.as_str(), "queued" | "waiting" | "ready_to_reply")
            || task.due_at > now
        {
            return Ok(None);
        }
        if let Some(id) = &task.work_id
            && !task.work_notification
            && crate::work::queue::blocker(&tx, &crate::work::read(&tx, id)?, now)?.is_some()
        {
            return Ok(None);
        }
        let live = tx
            .query_row(
                "SELECT 1 FROM coordinator WHERE owner=?1 AND expires>?2",
                params![owner, now],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        let round = tx.query_row(
            "SELECT state,expires,last_human_seq FROM discussions WHERE id=?1",
            [&task.discussion],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )?;
        if !live || round.0 != "active" || round.1 <= now {
            return Ok(None);
        }
        let replying = !task.decision_required || task.state == "ready_to_reply";
        let busy=tx.query_row("SELECT 1 FROM tasks WHERE id!=?1 AND ((principal=?2 AND state IN ('evaluating','ready_to_reply','responding')) OR (?3 AND conversation=?4 AND state='responding')) LIMIT 1",params![task.id,task.principal,replying && task.decision_required,task.conversation], |_|Ok(())).optional()?.is_some();
        if busy {
            return Ok(None);
        }
        tx.execute(
            "INSERT OR IGNORE INTO usage(discussion,principal) VALUES(?1,?2)",
            params![task.discussion, task.principal],
        )?;
        let replies: u32 = tx.query_row(
            "SELECT replies FROM usage WHERE discussion=?1 AND principal=?2",
            params![task.discussion, task.principal],
            |r| r.get(0),
        )?;
        if replies >= MAX_REPLIES {
            task.state = "limited".into();
            task.reason = Some("100 replies reached; send a new human message to reset".into());
            save_task(&tx, &task)?;
            tx.commit()?;
            return Ok(None);
        }
        task.state = if replying { "responding" } else { "evaluating" }.into();
        task.usage_epoch = round.2;
        task.owner = Some(owner.into());
        task.version += 1;
        if replying {
            tx.execute(
                "UPDATE usage SET replies=replies+1 WHERE discussion=?1 AND principal=?2",
                params![task.discussion, task.principal],
            )?;
        }
        save_task(&tx, &task)?;
        tx.commit()?;
        Ok(Some(task))
    }

    pub(crate) fn update(&mut self, task: &ScheduledTask, now: i64) -> Result<bool> {
        let mut task = task.clone();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let valid=tx.query_row("SELECT 1 FROM tasks t JOIN coordinator c ON c.owner=t.owner WHERE t.id=?1 AND t.version=?2 AND t.owner=?3 AND t.state IN ('evaluating','responding') AND c.expires>?4",params![task.id,task.version,task.owner,now], |_|Ok(())).optional()?.is_some();
        if !valid {
            return Ok(false);
        }
        if matches!(task.state.as_str(), "blocked" | "queued") {
            let charged: bool = tx.query_row("SELECT t.state='responding' AND COALESCE(json_extract(t.record,'$.usage_epoch'),0)=d.last_human_seq FROM tasks t JOIN discussions d ON d.id=t.discussion WHERE t.id=?1", [&task.id], |r| r.get(0))?;
            if charged {
                tx.execute(
                "UPDATE usage SET replies=MAX(0,replies-1) WHERE discussion=?1 AND principal=?2",
                params![task.discussion, task.principal],
            )?;
            }
        }
        if task.work_id.is_none()
            && matches!(task.state.as_str(), "queued" | "waiting" | "ready_to_reply")
        {
            let pending = query_tasks(
                &tx,
                "SELECT record FROM tasks WHERE state IN ('queued','waiting')",
            )?;
            for mut other in pending {
                if other.work_id.is_some()
                    || other.id == task.id
                    || other.discussion != task.discussion
                    || other.principal != task.principal
                    || other.conversation != task.conversation
                {
                    continue;
                }
                task.deliveries.append(&mut other.deliveries);
                task.triggers.append(&mut other.triggers);
                if other.through_seq > task.through_seq {
                    task.through_seq = other.through_seq;
                    task.message = other.message.clone();
                }
                task.decision_required &= other.decision_required;
                task.state = "queued".into();
                task.quiet = false;
                task.due_at = task.due_at.max(other.due_at);
                task.reason = None;
                tx.execute(
                    "UPDATE assigned SET task=?1 WHERE task=?2",
                    params![task.id, other.id],
                )?;
                other.state = "canceled".into();
                other.reason = Some("merged into pending message batch".into());
                save_task(&tx, &other)?;
                tx.execute("INSERT OR IGNORE INTO reconciled VALUES(?1)", [&other.id])?;
            }
        }
        save_task(&tx, &task)?;
        tx.commit()?;
        Ok(true)
    }

    pub(crate) fn invalidate_ready(&mut self, id: &str, seq: i64, now: i64) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record: String =
            tx.query_row("SELECT record FROM tasks WHERE id=?1", [id], |r| r.get(0))?;
        let mut task: ScheduledTask = serde_json::from_str(&record)?;
        if task.state == "ready_to_reply" && task.through_seq != seq {
            task.state = "queued".into();
            task.due_at = now;
            task.owner = None;
            task.reason = None;
            task.version += 1;
            save_task(&tx, &task)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn debit(&mut self, task: &ScheduledTask, now: i64) -> Result<bool> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let valid=tx.query_row("SELECT 1 FROM tasks t JOIN coordinator c ON c.owner=t.owner WHERE t.id=?1 AND t.version=?2 AND t.owner=?3 AND t.state IN ('evaluating','responding') AND c.expires>?4",params![task.id,task.version,task.owner,now], |_|Ok(())).optional()?.is_some();
        if !valid {
            return Ok(false);
        }
        let changed=tx.execute("UPDATE discussions SET calls=calls+1 WHERE id=?1 AND state IN ('active','paused') AND expires>?2",params![task.discussion,now])?;
        tx.commit()?;
        Ok(changed == 1)
    }

    pub(crate) fn expire(&self, now: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE discussions SET state='limited' WHERE state='active' AND expires<=?1",
            [now],
        )?;
        Ok(())
    }
    pub(crate) fn cancel_pending(&self, id: &str) -> Result<()> {
        let mut task = self.get(id)?;
        if matches!(
            task.state.as_str(),
            "queued" | "waiting" | "ready_to_reply" | "limited" | "blocked"
        ) {
            task.state = "canceled".into();
            task.reason =
                Some("discussion ended, member removed or conversation unavailable".into());
            save_task(&self.connection, &task)?;
        }
        Ok(())
    }

    pub fn status(&self, now: i64) -> Result<ScheduleStatus> {
        let online = self
            .connection
            .query_row(
                "SELECT 1 FROM coordinator WHERE expires>?1 AND owner LIKE 'scheduler:%'",
                [now],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        let mut statement=self.connection.prepare("SELECT id,conversation,state,calls,expires,allow_mcp FROM discussions WHERE id IN (SELECT discussion FROM tasks WHERE state IN ('queued','waiting','ready_to_reply','evaluating','responding','blocked','limited')) OR id IN (SELECT id FROM discussions ORDER BY rowid DESC LIMIT 100) ORDER BY rowid DESC")?;
        let discussions = statement
            .query_map([], discussion_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ScheduleStatus {
            online,
            discussions,
            usage: {
                let mut stmt = self.connection.prepare("SELECT discussion,principal,replies FROM usage WHERE discussion IN (SELECT id FROM discussions WHERE state!='ended')")?;
                stmt.query_map([], |r| {
                    Ok(ReplyUsage {
                        discussion: r.get(0)?,
                        principal: r.get(1)?,
                        replies: r.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
            },
            tasks: query_tasks(
                &self.connection,
                "SELECT record FROM tasks WHERE state IN ('queued','waiting','ready_to_reply','evaluating','responding','blocked','limited') OR id IN (SELECT id FROM tasks ORDER BY rowid DESC LIMIT 500) ORDER BY rowid DESC",
            )?,
        })
    }

    pub fn snapshot(&self) -> Result<ScheduleStatus> {
        self.status(now_ms())
    }

    pub(crate) fn assigned(&self, delivery: i64) -> Result<bool> {
        Ok(self
            .connection
            .query_row(
                "SELECT 1 FROM assigned WHERE delivery=?1",
                [delivery],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub(crate) fn authorized(&self, task: &ScheduledTask, now: i64) -> Result<bool> {
        Ok(self.connection.query_row("SELECT 1 FROM tasks t JOIN coordinator c ON c.owner=t.owner JOIN discussions d ON d.id=t.discussion WHERE t.id=?1 AND t.version=?2 AND t.owner=?3 AND t.state IN ('evaluating','responding') AND c.expires>?4 AND d.state IN ('active','paused','limited') AND d.expires>?4",params![task.id,task.version,task.owner,now], |_|Ok(())).optional()?.is_some())
    }
}

fn discussion_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Discussion> {
    Ok(Discussion {
        id: r.get(0)?,
        conversation: r.get(1)?,
        state: r.get(2)?,
        calls: r.get(3)?,
        max_replies: MAX_REPLIES,
        expires_at: r.get(4)?,
        allow_mcp: r.get(5)?,
    })
}
fn query_tasks(c: &Connection, sql: &str) -> Result<Vec<ScheduledTask>> {
    let mut stmt = c.prepare(sql)?;
    stmt.query_map([], |r| r.get::<_, String>(0))?
        .map(|r| Ok(serde_json::from_str(&r?)?))
        .collect()
}
fn save_task(c: &Connection, t: &ScheduledTask) -> Result<()> {
    c.execute(
        "UPDATE tasks SET state=?2,due=?3,quiet=?4,version=?5,owner=?6,record=?7 WHERE id=?1",
        params![
            t.id,
            t.state,
            t.due_at,
            t.quiet,
            t.version,
            t.owner,
            serde_json::to_string(t)?
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: i64, principal: &str, conversation: &str, decision: bool) -> ScheduledTask {
        ScheduledTask {
            id: id.to_string(),
            discussion: "round".into(),
            principal: principal.into(),
            conversation: conversation.into(),
            state: "queued".into(),
            deliveries: vec![id],
            message: format!("message-{id}"),
            through_seq: id,
            usage_epoch: 0,
            triggers: vec![format!("message-{id}")],
            decision_required: decision,
            quiet: decision,
            due_at: 0,
            deadline: 60_000,
            quiet_ms: 3000,
            version: 0,
            owner: None,
            run_id: None,
            reason: None,
            last_decision: None,
            work_id: None,
            work_notification: false,
        }
    }
    fn fixture() -> (tempfile::TempDir, ScheduleStore) {
        let home = tempfile::tempdir().unwrap();
        let mut store = ScheduleStore::open(home.path()).unwrap();
        store.start("round", "chat", false, 0).unwrap();
        assert!(store.acquire("scheduler:one", 0).unwrap());
        (home, store)
    }

    #[test]
    fn ending_target_conversation_stops_incoming_cross_conversation_work() {
        let (_home, mut store) = fixture();
        store.ingest(task(1, "agent", "target", false)).unwrap();
        store.control("target", "end", false, 1).unwrap();
        assert_eq!(store.discussion("round").unwrap().unwrap().state, "ended");
    }

    #[test]
    fn old_task_records_have_no_invented_decision() {
        let mut old = serde_json::to_value(task(1, "agent", "chat", false)).unwrap();
        old.as_object_mut().unwrap().remove("last_decision");
        let restored: ScheduledTask = serde_json::from_value(old).unwrap();
        assert!(restored.last_decision.is_none());
    }

    #[test]
    fn rejects_unsupported_schema_without_changing_it() {
        for version in [1, 2, 99] {
            let home = tempfile::tempdir().unwrap();
            let store = ScheduleStore::open(home.path()).unwrap();
            store
                .connection
                .pragma_update(None, "user_version", version)
                .unwrap();
            assert!(ScheduleStore::open(home.path()).is_err());
            assert_eq!(
                store
                    .connection
                    .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                    .unwrap(),
                version
            );
        }
    }

    #[test]
    fn parallel_decisions_and_direct_replies_but_one_autonomous_reply() {
        let (_home, mut store) = fixture();
        for t in [
            task(1, "a", "chat", false),
            task(2, "b", "chat", false),
            task(3, "a", "other", false),
            task(4, "c", "chat", true),
            task(5, "d", "chat", true),
        ] {
            store.ingest(t).unwrap();
        }
        let mut a = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
        let mut b = store.claim("2", "scheduler:one", 0).unwrap().unwrap();
        assert!(store.claim("3", "scheduler:one", 0).unwrap().is_none());
        // Decisions can overlap each other and already-running direct replies.
        let mut c = store.claim("4", "scheduler:one", 0).unwrap().unwrap();
        let mut d = store.claim("5", "scheduler:one", 0).unwrap().unwrap();
        c.state = "ready_to_reply".into();
        d.state = "ready_to_reply".into();
        store.update(&c, 0).unwrap();
        store.update(&d, 0).unwrap();
        assert!(store.claim("4", "scheduler:one", 0).unwrap().is_none());
        a.state = "completed".into();
        b.state = "completed".into();
        store.update(&a, 0).unwrap();
        store.update(&b, 0).unwrap();
        assert!(store.claim("3", "scheduler:one", 0).unwrap().is_some());
        let mut reply = store.claim("4", "scheduler:one", 0).unwrap().unwrap();
        assert_eq!(reply.state, "responding");
        assert!(store.claim("5", "scheduler:one", 0).unwrap().is_none());
        reply.state = "completed".into();
        store.update(&reply, 0).unwrap();
        assert!(store.claim("5", "scheduler:one", 0).unwrap().is_some());
    }

    #[test]
    fn wait_persists_merge_is_idempotent_and_mentions_release_model_wait() {
        let (home, mut store) = fixture();
        store.ingest(task(1, "a", "chat", true)).unwrap();
        let mut waiting = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
        waiting.state = "waiting".into();
        waiting.due_at = 30_000;
        store.update(&waiting, 0).unwrap();
        drop(store);
        let mut store = ScheduleStore::open(home.path()).unwrap();
        store.ingest(task(2, "a", "chat", true)).unwrap();
        store.ingest(task(2, "a", "chat", true)).unwrap();
        let merged = store.get("1").unwrap();
        assert_eq!(merged.deliveries, vec![1, 2]);
        assert_eq!(merged.due_at, 30_000);
        store.ingest(task(3, "a", "chat", false)).unwrap();
        let urgent = store.claim("1", "scheduler:one", 100).unwrap().unwrap();
        assert_eq!(urgent.state, "responding");
        assert_eq!(urgent.deliveries, vec![1, 2, 3]);
        assert!(!urgent.quiet);
    }

    #[test]
    fn expired_lease_fences_old_work_and_never_replays_response_tools() {
        let (_home, mut store) = fixture();
        store.ingest(task(1, "a", "chat", false)).unwrap();
        store.ingest(task(2, "b", "other", true)).unwrap();
        let old = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
        let decision = store.claim("2", "scheduler:one", 0).unwrap().unwrap();
        assert!(!store.acquire("scheduler:two", 100).unwrap());
        assert!(store.acquire("scheduler:two", LEASE_MS + 1).unwrap());
        assert_eq!(store.get("1").unwrap().state, "interrupted");
        assert_eq!(store.get("2").unwrap().state, "queued");
        assert!(!store.update(&old, LEASE_MS + 1).unwrap());
        assert!(!store.debit(&decision, LEASE_MS + 1).unwrap());
        assert!(!store.authorized(&old, LEASE_MS + 1).unwrap());
        assert!(
            store
                .claim("2", "scheduler:two", LEASE_MS + 1)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn pause_end_and_expiry_still_fence_calls_without_a_call_limit() {
        let (_home, mut store) = fixture();
        store.ingest(task(1, "a", "chat", false)).unwrap();
        store.ingest(task(2, "b", "other", false)).unwrap();
        let a = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
        store.control("chat", "pause", false, 0).unwrap();
        assert!(store.claim("2", "scheduler:one", 0).unwrap().is_none());
        for _ in 0..150 {
            assert!(store.debit(&a, 0).unwrap());
        }
        store.control("chat", "resume", false, 0).unwrap();
        let b = store.claim("2", "scheduler:one", 0).unwrap().unwrap();
        store.expire(0).unwrap();
        assert_eq!(store.discussion("round").unwrap().unwrap().state, "active");
        store.control("chat", "end", false, 0).unwrap();
        assert!(!store.authorized(&b, 0).unwrap());
        store.control("chat", "resume", true, 0).unwrap();
        assert_eq!(store.discussion("round").unwrap().unwrap().state, "ended");
    }

    #[test]
    fn waiting_decisions_do_not_consume_replies_or_have_a_call_limit() {
        let (_home, mut store) = fixture();
        store.ingest(task(1, "a", "chat", true)).unwrap();
        for _ in 0..105 {
            let mut task = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
            assert!(store.debit(&task, 0).unwrap());
            task.state = "waiting".into();
            store.update(&task, 0).unwrap();
        }
        assert_eq!(store.status(0).unwrap().usage[0].replies, 0);
        assert_eq!(store.discussion("round").unwrap().unwrap().calls, 105);
    }

    #[test]
    fn hundred_replies_reset_only_on_new_human_input_and_not_resume_or_replay() {
        let (_home, mut store) = fixture();
        store
            .authorize_human("human-1", "chat", 1, false, 0)
            .unwrap();
        for id in 1..=100 {
            store.ingest(task(id, "a", "chat", false)).unwrap();
            let mut reply = store
                .claim(&id.to_string(), "scheduler:one", 0)
                .unwrap()
                .unwrap();
            reply.state = "completed".into();
            store.update(&reply, 0).unwrap();
        }
        store.ingest(task(101, "a", "chat", false)).unwrap();
        assert!(store.claim("101", "scheduler:one", 0).unwrap().is_none());
        assert_eq!(store.get("101").unwrap().state, "limited");
        store.control("chat", "resume", false, 0).unwrap();
        store
            .authorize_human("human-1", "chat", 1, false, 0)
            .unwrap();
        assert_eq!(store.status(0).unwrap().usage[0].replies, 100);
        store
            .authorize_human("human-2", "chat", 102, false, 0)
            .unwrap();
        assert_eq!(
            store.human_discussion("human-2").unwrap().as_deref(),
            Some("round")
        );
        assert!(store.status(0).unwrap().usage.is_empty());
        assert!(
            store
                .claim("101", "scheduler:one", QUIET_MS)
                .unwrap()
                .is_some()
        );
        store
            .authorize_human("human-1", "chat", 1, false, QUIET_MS)
            .unwrap();
        assert_eq!(store.status(QUIET_MS).unwrap().usage[0].replies, 1);
    }

    #[test]
    fn ready_survives_restart_and_stale_snapshot_requires_a_new_decision() {
        let (home, mut store) = fixture();
        store.ingest(task(1, "a", "chat", true)).unwrap();
        let mut ready = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
        ready.state = "ready_to_reply".into();
        store.update(&ready, 0).unwrap();
        drop(store);
        let mut store = ScheduleStore::open(home.path()).unwrap();
        assert!(store.acquire("scheduler:two", LEASE_MS + 1).unwrap());
        assert_eq!(store.get("1").unwrap().state, "ready_to_reply");
        assert_eq!(store.status(LEASE_MS + 1).unwrap().usage[0].replies, 0);
        store.invalidate_ready("1", 2, LEASE_MS + 1).unwrap();
        let again = store
            .claim("1", "scheduler:two", LEASE_MS + 1)
            .unwrap()
            .unwrap();
        assert_eq!(again.state, "evaluating");
    }

    #[test]
    fn arrivals_during_decision_merge_before_reply_and_keep_all_deliveries() {
        let (_home, mut store) = fixture();
        store.ingest(task(1, "a", "chat", true)).unwrap();
        let mut deciding = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
        store.ingest(task(2, "a", "chat", false)).unwrap();
        deciding.state = "ready_to_reply".into();
        store.update(&deciding, 0).unwrap();
        let merged = store.get("1").unwrap();
        assert_eq!(merged.state, "queued");
        assert_eq!(merged.deliveries, vec![1, 2]);
        assert_eq!(merged.triggers, vec!["message-1", "message-2"]);
        assert_eq!(store.get("2").unwrap().state, "canceled");
        assert!(store.get("2").unwrap().deliveries.is_empty());
        assert!(!merged.decision_required);
    }

    #[test]
    fn status_keeps_old_live_work_and_reconciliation_is_persistent() {
        let (home, mut store) = fixture();
        store.ingest(task(1, "a", "chat", true)).unwrap();
        for id in 2..=510 {
            let mut old = task(id, "b", "history", false);
            old.state = "completed".into();
            store.ingest(old).unwrap();
            store.mark_reconciled(&id.to_string()).unwrap();
        }
        for id in 0..110 {
            store
                .start(&format!("new-{id}"), "other", false, 0)
                .unwrap();
        }
        let status = store.status(0).unwrap();
        assert!(status.tasks.iter().any(|t| t.id == "1"));
        assert!(status.discussions.iter().any(|d| d.id == "round"));
        drop(store);
        let store = ScheduleStore::open(home.path()).unwrap();
        assert_eq!(store.unreconciled().unwrap().len(), 1);
    }
    #[test]
    fn old_claim_cannot_refund_the_new_human_turn_and_end_stays_idempotent() {
        let (_home, mut store) = fixture();
        store
            .authorize_human("human-1", "chat", 1, false, 0)
            .unwrap();
        store.ingest(task(1, "a", "chat", false)).unwrap();
        let mut old = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
        store
            .authorize_human("human-2", "chat", 2, false, 0)
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO usage(discussion,principal,replies) VALUES('round','a',10)",
                [],
            )
            .unwrap();
        old.state = "queued".into();
        store.update(&old, 0).unwrap();
        assert_eq!(store.status(0).unwrap().usage[0].replies, 10);
        store
            .authorize_human("late-old-message", "chat", 1, false, 0)
            .unwrap();
        assert_eq!(store.status(0).unwrap().usage[0].replies, 10);
        store.control("chat", "end", false, 0).unwrap();
        store
            .authorize_human("human-2", "chat", 2, false, 0)
            .unwrap();
        assert_eq!(store.discussion("round").unwrap().unwrap().state, "ended");
        store
            .authorize_human("human-3", "chat", 3, false, 0)
            .unwrap();
        assert_eq!(
            store.human_discussion("human-3").unwrap().as_deref(),
            Some("human-3")
        );
    }
    #[test]
    fn first_open_is_safe_for_concurrent_coordinator_and_status_readers() {
        let home = tempfile::tempdir().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(6));
        let threads: Vec<_> = (0..6)
            .map(|_| {
                let path = home.path().to_owned();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let store = ScheduleStore::open(&path).unwrap();
                    assert!(store.snapshot().unwrap().tasks.is_empty());
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }
}
