use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

pub(crate) const LEASE_MS: i64 = 15_000;

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

crate::string_enum::string_enum! {
    /// Lifecycle of one scheduled delivery task.
    pub enum TaskState {
        Queued = "queued",
        Evaluating = "evaluating",
        ReadyToReply = "ready_to_reply",
        Responding = "responding",
        Waiting = "waiting",
        Limited = "limited",
        Blocked = "blocked",
        Input = "input",
        Suspended = "suspended",
        Completed = "completed",
        Silent = "silent",
        Skipped = "skipped",
        Failed = "failed",
        Canceled = "canceled",
        Interrupted = "interrupted",
        OutcomeUnknown = "outcome_unknown",
    }
}

crate::string_enum::string_enum! {
    pub enum DiscussionState {
        Active = "active",
        Paused = "paused",
        Limited = "limited",
        Ended = "ended",
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub discussion: String,
    pub principal: String,
    pub conversation: String,
    pub state: TaskState,
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
    #[serde(default)]
    pub work_event: Option<String>,
    #[serde(default)]
    pub work_input_id: Option<String>,
    /// Members this waiting task waits for; any of them settling wakes it.
    #[serde(default)]
    pub blocked_on: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Discussion {
    pub id: String,
    pub conversation: String,
    pub state: DiscussionState,
    pub calls: u32,
    pub max_replies: Option<u32>,
    pub expires_at: i64,
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

pub(crate) const SCHEMA: agt_storage::sqlite::Schema = agt_storage::sqlite::Schema {
    module: "scheduler",
    version: 1,
    ddl: "CREATE TABLE coordinator(id INTEGER PRIMARY KEY CHECK(id=1), owner TEXT NOT NULL, expires INTEGER NOT NULL);
        CREATE TABLE discussions(id TEXT PRIMARY KEY,conversation TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('active','paused','limited','ended')),calls INTEGER NOT NULL DEFAULT 0,expires INTEGER NOT NULL,last_human_seq INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE tasks(id TEXT PRIMARY KEY,discussion TEXT NOT NULL,principal TEXT NOT NULL,conversation TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('queued','evaluating','ready_to_reply','responding','waiting','limited','blocked','input','suspended','completed','silent','skipped','failed','canceled','interrupted','outcome_unknown')),due INTEGER NOT NULL,quiet INTEGER NOT NULL,version INTEGER NOT NULL,owner TEXT,record TEXT NOT NULL);
        CREATE UNIQUE INDEX autonomous_reply_slot ON tasks(conversation) WHERE state='responding' AND json_extract(record,'$.decision_required')=1;
        CREATE INDEX task_queue ON tasks(state,due);
        CREATE INDEX active_identity ON tasks(principal,state);
        CREATE TABLE assigned(delivery INTEGER PRIMARY KEY,task TEXT NOT NULL);
        CREATE TABLE usage(discussion TEXT NOT NULL,principal TEXT NOT NULL,replies INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(discussion,principal));
        CREATE TABLE reconciled(task TEXT PRIMARY KEY);
        CREATE TABLE human_messages(message TEXT PRIMARY KEY,discussion TEXT NOT NULL);
        CREATE TABLE context_resets(conversation TEXT PRIMARY KEY,through_seq INTEGER NOT NULL);
        CREATE TABLE context_reset_events(id INTEGER PRIMARY KEY AUTOINCREMENT,conversation TEXT NOT NULL,through_seq INTEGER NOT NULL,created_at INTEGER NOT NULL);
        CREATE INDEX context_reset_range ON context_reset_events(conversation,through_seq);
        CREATE TABLE run_inputs(input_task TEXT PRIMARY KEY,parent_task TEXT NOT NULL,run_id TEXT NOT NULL,mode TEXT NOT NULL,accepted_at INTEGER NOT NULL);
        CREATE INDEX run_input_parent ON run_inputs(parent_task);
        CREATE TRIGGER scheduled_created AFTER INSERT ON tasks BEGIN
            INSERT INTO change_outbox(conversation,kind,at) VALUES(json_extract(NEW.record,'$.conversation'),'schedule',CAST(unixepoch('subsec')*1000 AS INTEGER));
        END;
        CREATE TRIGGER scheduled_changed AFTER UPDATE ON tasks WHEN OLD.record<>NEW.record BEGIN
            INSERT INTO change_outbox(conversation,kind,at) VALUES(json_extract(NEW.record,'$.conversation'),'schedule',CAST(unixepoch('subsec')*1000 AS INTEGER));
        END;
        CREATE TRIGGER discussion_changed AFTER UPDATE ON discussions BEGIN
            INSERT INTO change_outbox(conversation,kind,at) VALUES(NEW.conversation,'schedule',CAST(unixepoch('subsec')*1000 AS INTEGER));
        END;
        CREATE TRIGGER context_reset_added AFTER INSERT ON context_reset_events BEGIN
            INSERT INTO change_outbox(conversation,kind,at) VALUES(NEW.conversation,'context_reset',NEW.created_at);
        END;",
};

pub struct ScheduleStore {
    pub(crate) home: std::path::PathBuf,
    config: agt_config::RuntimeConfig,
    pub(crate) connection: agt_storage::sqlite::Db,
}

impl ScheduleStore {
    pub fn open(home: &Path) -> Result<Self> {
        let connection = crate::store::runtime_db(home)?;
        Ok(Self {
            connection,
            home: home.into(),
            config: agt_config::RuntimeConfig::load(home)?,
        })
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
    pub(crate) fn start(&self, id: &str, conversation: &str, now: i64) -> Result<()> {
        self.connection.execute("INSERT OR IGNORE INTO discussions(id,conversation,state,expires) VALUES(?1,?2,'active',?3)", params![id,conversation,self.config.expires_at(now)])?;
        Ok(())
    }

    pub(crate) fn authorize_human(
        &self,
        message: &str,
        conversation: &str,
        seq: i64,
        now: i64,
    ) -> Result<()> {
        let tx = agt_storage::sqlite::write(&self.connection)?;
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
        tx.execute("INSERT OR IGNORE INTO discussions(id,conversation,state,expires) VALUES(?1,?2,'active',?3)", params![discussion,conversation,self.config.expires_at(now)])?;
        tx.execute(
            "INSERT INTO human_messages VALUES(?1,?2)",
            params![message, discussion],
        )?;
        if seq > previous_seq {
            tx.execute("DELETE FROM usage WHERE discussion=?1", [&discussion])?;
            tx.execute(
                "UPDATE discussions SET state='active',last_human_seq=?2,expires=?3 WHERE id=?1",
                params![discussion, seq, self.config.expires_at(now)],
            )?;
            // A human turn wakes pending work, including cross-conversation waits.
            // Running snapshots and possible external effects remain untouched.
            for mut task in query_tasks(
                &tx,
                "SELECT record FROM tasks WHERE state IN ('waiting','ready_to_reply','limited','blocked')",
            )? {
                if task.discussion == discussion {
                    task.wake_for_human(now);
                    save_task(&tx, &task)?;
                }
            }
        }
        tx.commit()?;
        agt_storage::wake::notify(&self.home);
        Ok(())
    }

    /// Give an active discussion a specific lifetime, as a timer that starts it asks.
    pub(crate) fn set_discussion_expiry(&self, discussion: &str, expires: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE discussions SET expires=?2 WHERE id=?1 AND state='active'",
            params![discussion, expires],
        )?;
        agt_storage::wake::notify(&self.home);
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

    pub fn control(&mut self, conversation: &str, action: &str, now: i64) -> Result<()> {
        let tx = agt_storage::sqlite::write(&self.connection)?;
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
                tx.execute("UPDATE discussions SET state='active',expires=?2 WHERE conversation=?1 AND state!='ended'",params![conversation,self.config.expires_at(now)])?;
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
                        task.resume_pending(now);
                        save_task(&tx, &task)?;
                    }
                }
            }
            _ => bail!("unknown scheduler control"),
        }
        tx.commit()?;
        agt_storage::wake::notify(&self.home);
        Ok(())
    }

    pub fn discussion(&self, id: &str) -> Result<Option<Discussion>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id,conversation,state,calls,expires FROM discussions WHERE id=?1",
                [id],
                |r| discussion_row(r, self.config.scheduler.replies_per_agent),
            )
            .optional()?)
    }

    pub(crate) fn acquire(&mut self, owner: &str, now: i64) -> Result<bool> {
        let tx = agt_storage::sqlite::write(&self.connection)?;
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
                // Only a reply that was executing can have changed anything.
                let changed = match executing_run_key(&tx, &task)? {
                    Some(key) => crate::RunStore::mark_owner_lost(&tx, &key)?,
                    None => false,
                };
                // A reply that changed nothing runs again; work reports its failed Run.
                task.recover_expired_lease(!changed && task.work_id.is_none());
                save_task(&tx, &task)?;
            }
        }
        tx.execute("INSERT INTO coordinator VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET owner=excluded.owner,expires=excluded.expires",params![owner,now+LEASE_MS])?;
        tx.commit()?;
        agt_storage::wake::notify(&self.home);
        Ok(true)
    }

    /// Why `claim` would refuse a task: lease, task and round state, for test failures.
    #[cfg(test)]
    pub(crate) fn claim_state(&self, id: &str, owner: &str, now: i64) -> String {
        let lease = self
            .connection
            .query_row("SELECT owner,expires FROM coordinator", [], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .optional();
        let task = self.get(id);
        let round = task
            .as_ref()
            .ok()
            .and_then(|t| self.discussion(&t.discussion).ok().flatten());
        format!(
            "claim refused at {now} by {owner}: lease {lease:?}; task {:?}; round {:?}",
            task.map(|t| (t.state, t.due_at, t.work_id, t.work_notification)),
            round.map(|r| (r.state, r.expires_at)),
        )
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

    pub(crate) fn ingest(&mut self, task: ScheduledTask) -> Result<()> {
        let tx = agt_storage::sqlite::write(&self.connection)?;
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
        let delivery = task.deliveries[0];
        if let Some(mut pending) = pending_decision(&tx, &task)? {
            pending.absorb(&task);
            save_task(&tx, &pending)?;
            tx.execute(
                "INSERT INTO assigned VALUES(?1,?2)",
                params![delivery, pending.id],
            )?;
            tx.commit()?;
            agt_storage::wake::notify(&self.home);
            return Ok(());
        }
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
        tx.execute(
            "INSERT INTO assigned VALUES(?1,?2)",
            params![delivery, task.id],
        )?;
        tx.commit()?;
        agt_storage::wake::notify(&self.home);
        Ok(())
    }

    /// Members other than `except` who are working in `conversation` now.
    pub(crate) fn active_members(&self, conversation: &str, except: &str) -> Result<Vec<String>> {
        let mut members: Vec<String> = active_tasks(&self.connection, conversation)?
            .into_iter()
            .map(|t| t.principal)
            .filter(|p| p != except)
            .collect();
        members.sort();
        members.dedup();
        Ok(members)
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
        let tx = agt_storage::sqlite::write(&self.connection)?;
        let record: String =
            tx.query_row("SELECT record FROM tasks WHERE id=?1", [id], |r| r.get(0))?;
        let mut task: ScheduledTask = serde_json::from_str(&record)?;
        if !matches!(
            task.state,
            TaskState::Queued | TaskState::Waiting | TaskState::ReadyToReply
        ) || task.due_at > now
        {
            return Ok(None);
        }
        if let Some(id) = &task.work_id
            && !task.work_notification
            && crate::work::queue::blocker(&tx, &crate::work::read(&tx, id)?, now)?.is_some()
        {
            return Ok(None);
        }
        if task.work_id.is_some()
            && task.work_notification
            && crate::work::notice_blocked(&tx, &task)?
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
        let replying = !task.decision_required || task.state == TaskState::ReadyToReply;
        if crate::work::execution_policy::blocker(&tx, &task, self.config.scheduler.parallel_work)?
            .is_some()
        {
            return Ok(None);
        }
        let busy = replying && task.decision_required && tx.query_row("SELECT 1 FROM tasks WHERE id!=?1 AND conversation=?2 AND state='responding' LIMIT 1",params![task.id,task.conversation], |_|Ok(())).optional()?.is_some();
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
        if self
            .config
            .scheduler
            .replies_per_agent
            .is_some_and(|limit| replies >= limit)
        {
            task.state = TaskState::Limited;
            task.reason = Some(format!(
                "{} replies reached; send a new human message to reset",
                self.config.scheduler.replies_per_agent.unwrap()
            ));
            save_task(&tx, &task)?;
            tx.commit()?;
            agt_storage::wake::notify(&self.home);
            return Ok(None);
        }
        if task.work_id.is_some() {
            tx.execute("INSERT INTO work_fairness VALUES(?1,?2) ON CONFLICT(principal) DO UPDATE SET notices=CASE WHEN ?2=0 THEN 0 ELSE MIN(4,notices+1) END",params![task.principal,if task.work_notification {1}else{0}])?;
        }
        task.claim_execution(replying, round.2, owner);
        if replying {
            tx.execute(
                "UPDATE usage SET replies=replies+1 WHERE discussion=?1 AND principal=?2",
                params![task.discussion, task.principal],
            )?;
        }
        save_task(&tx, &task)?;
        tx.commit()?;
        agt_storage::wake::notify(&self.home);
        Ok(Some(task))
    }

    pub(crate) fn update(&mut self, task: &ScheduledTask, now: i64) -> Result<bool> {
        let task = task.clone();
        let tx = agt_storage::sqlite::write(&self.connection)?;
        let valid=tx.query_row("SELECT 1 FROM tasks t JOIN coordinator c ON c.owner=t.owner WHERE t.id=?1 AND t.version=?2 AND t.owner=?3 AND t.state IN ('evaluating','responding') AND c.expires>?4",params![task.id,task.version,task.owner,now], |_|Ok(())).optional()?.is_some();
        if !valid {
            return Ok(false);
        }
        if matches!(task.state, TaskState::Blocked | TaskState::Queued) {
            let charged: bool = tx.query_row("SELECT t.state='responding' AND COALESCE(json_extract(t.record,'$.usage_epoch'),0)=d.last_human_seq FROM tasks t JOIN discussions d ON d.id=t.discussion WHERE t.id=?1", [&task.id], |r| r.get(0))?;
            if charged {
                tx.execute(
                "UPDATE usage SET replies=MAX(0,replies-1) WHERE discussion=?1 AND principal=?2",
                params![task.discussion, task.principal],
            )?;
            }
        }
        save_task(&tx, &task)?;
        tx.commit()?;
        agt_storage::wake::notify(&self.home);
        Ok(true)
    }

    pub(crate) fn invalidate_ready(&mut self, id: &str, seq: i64, now: i64) -> Result<()> {
        let tx = agt_storage::sqlite::write(&self.connection)?;
        let record: String =
            tx.query_row("SELECT record FROM tasks WHERE id=?1", [id], |r| r.get(0))?;
        let mut task: ScheduledTask = serde_json::from_str(&record)?;
        if task.state == TaskState::ReadyToReply && task.through_seq != seq {
            task.invalidate_ready_snapshot(now);
            save_task(&tx, &task)?;
        }
        tx.commit()?;
        agt_storage::wake::notify(&self.home);
        Ok(())
    }

    pub(crate) fn debit(&mut self, task: &ScheduledTask, now: i64) -> Result<bool> {
        let tx = agt_storage::sqlite::write(&self.connection)?;
        let valid=tx.query_row("SELECT 1 FROM tasks t JOIN coordinator c ON c.owner=t.owner WHERE t.id=?1 AND t.version=?2 AND t.owner=?3 AND t.state IN ('evaluating','responding') AND c.expires>?4",params![task.id,task.version,task.owner,now], |_|Ok(())).optional()?.is_some();
        if !valid {
            return Ok(false);
        }
        let changed=tx.execute("UPDATE discussions SET calls=calls+1 WHERE id=?1 AND state IN ('active','paused','limited')",params![task.discussion])?;
        tx.commit()?;
        agt_storage::wake::notify(&self.home);
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
        if task.cancel_pending() {
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
        let mut statement=self.connection.prepare("SELECT id,conversation,state,calls,expires FROM discussions WHERE id IN (SELECT discussion FROM tasks WHERE state IN ('queued','waiting','ready_to_reply','evaluating','responding','blocked','limited')) OR id IN (SELECT id FROM discussions ORDER BY rowid DESC LIMIT 100) ORDER BY rowid DESC")?;
        let discussions = statement
            .query_map([], |r| {
                discussion_row(r, self.config.scheduler.replies_per_agent)
            })?
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

    /// Ownership remains valid while a live executor is confirming cancellation.
    /// Discussion controls revoke permission, not the ability to report a terminal fact.
    pub(crate) fn owns_execution(&self, task: &ScheduledTask, now: i64) -> Result<bool> {
        Ok(self.connection.query_row("SELECT 1 FROM tasks t JOIN coordinator c ON c.owner=t.owner WHERE t.id=?1 AND t.version=?2 AND t.owner=?3 AND t.state IN ('evaluating','responding') AND c.expires>?4", params![task.id,task.version,task.owner,now], |_| Ok(())).optional()?.is_some())
    }

    // Execution fence after admission: natural expiry only gates new claims.
    // Explicit end, task version changes and lease loss still revoke execution.
    pub(crate) fn authorized(&self, task: &ScheduledTask, now: i64) -> Result<bool> {
        Ok(self.connection.query_row("SELECT 1 FROM tasks t JOIN coordinator c ON c.owner=t.owner JOIN discussions d ON d.id=t.discussion WHERE t.id=?1 AND t.version=?2 AND t.owner=?3 AND t.state IN ('evaluating','responding') AND c.expires>?4 AND d.state IN ('active','paused','limited')",params![task.id,task.version,task.owner,now], |_|Ok(())).optional()?.is_some())
    }
}

fn discussion_row(r: &rusqlite::Row<'_>, max_replies: Option<u32>) -> rusqlite::Result<Discussion> {
    Ok(Discussion {
        id: r.get(0)?,
        conversation: r.get(1)?,
        state: r.get(2)?,
        calls: r.get(3)?,
        max_replies,
        expires_at: r.get(4)?,
    })
}
fn query_tasks(c: &Connection, sql: &str) -> Result<Vec<ScheduledTask>> {
    let mut stmt = c.prepare(sql)?;
    stmt.query_map([], |r| r.get::<_, String>(0))?
        .map(|r| Ok(serde_json::from_str(&r?)?))
        .collect()
}
/// The request key of the Run a claimed task may have started, matching
/// `scheduler::execute`. A work Run not yet bound to its item never executed;
/// work reconciliation closes it as not started.
fn executing_run_key(c: &Connection, task: &ScheduledTask) -> Result<Option<String>> {
    if task.state != TaskState::Responding {
        return Ok(None);
    }
    match &task.work_id {
        Some(id) if !task.work_notification => {
            let item = crate::work::read(c, id)?;
            Ok(matches!(item.state, crate::work::WorkState::Running { .. })
                .then(|| crate::work::execution_key(&item)))
        }
        _ => Ok(Some(format!("schedule:{}:{}", task.id, task.version))),
    }
}

pub(crate) fn save_task(c: &Connection, t: &ScheduledTask) -> Result<()> {
    write_task(c, t)?;
    crate::run_inputs::settle(c, t)?;
    if t.state == TaskState::Waiting || !t.is_active() {
        release_waiters(c, &t.conversation)?;
    }
    Ok(())
}

/// Most pending peer messages a decision task absorbs before new ones get their own task.
const MERGED_TRIGGERS: usize = 20;

/// The pending decision task a new peer message should join instead of starting another
/// one for the same member, conversation and discussion. Direct work and replies to
/// people or mentions keep their own tasks.
fn pending_decision(c: &Connection, task: &ScheduledTask) -> Result<Option<ScheduledTask>> {
    if !task.decision_required || task.work_id.is_some() {
        return Ok(None);
    }
    let mut stmt = c.prepare(
        "SELECT record FROM tasks WHERE principal=?1 AND conversation=?2 AND discussion=?3 AND state IN ('queued','waiting') AND json_extract(record,'$.decision_required')=1 AND json_extract(record,'$.work_id') IS NULL ORDER BY rowid DESC LIMIT 1",
    )?;
    let found = stmt
        .query_row(
            params![task.principal, task.conversation, task.discussion],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    Ok(found
        .map(|r| serde_json::from_str::<ScheduledTask>(&r))
        .transpose()?
        .filter(|t| t.triggers.len() < MERGED_TRIGGERS))
}

/// Tasks whose member is working in `conversation`: replying, about to reply, steering a
/// running reply, or queued to answer without a participation decision.
fn active_tasks(c: &Connection, conversation: &str) -> Result<Vec<ScheduledTask>> {
    let mut stmt = c.prepare(
        "SELECT record FROM tasks WHERE conversation=?1 AND state IN ('queued','ready_to_reply','responding','input')",
    )?;
    let records = stmt
        .query_map([conversation], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut tasks = Vec::new();
    for record in records {
        let task: ScheduledTask = serde_json::from_str(&record)?;
        if task.is_active() {
            tasks.push(task);
        }
    }
    Ok(tasks)
}

/// Make a waiting task due as soon as any member it waits on has stopped working.
/// Runs on every save that can end someone's work, and when a wait is registered,
/// so a member that finished before the wait was saved still wakes it.
fn release_waiters(c: &Connection, conversation: &str) -> Result<()> {
    let mut stmt = c.prepare(
        "SELECT record FROM tasks WHERE conversation=?1 AND state='waiting' AND json_array_length(json_extract(record,'$.blocked_on'))>0",
    )?;
    let waiting = stmt
        .query_map([conversation], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if waiting.is_empty() {
        return Ok(());
    }
    let active: std::collections::HashSet<String> = active_tasks(c, conversation)?
        .into_iter()
        .map(|t| t.principal)
        .collect();
    let now = now_ms();
    for record in waiting {
        let mut task: ScheduledTask = serde_json::from_str(&record)?;
        if task.due_at > now && task.blocked_on.iter().any(|p| !active.contains(p)) {
            task.due_at = now;
            task.blocked_on.clear();
            write_task(c, &task)?;
        }
    }
    Ok(())
}

fn write_task(c: &Connection, t: &ScheduledTask) -> Result<()> {
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
            state: TaskState::Queued,
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
            work_event: None,
            work_input_id: None,
            blocked_on: Vec::new(),
        }
    }
    fn fixture() -> (tempfile::TempDir, ScheduleStore) {
        let home = tempfile::tempdir().unwrap();
        let mut store = ScheduleStore::open(home.path()).unwrap();
        store.start("round", "chat", 0).unwrap();
        assert!(store.acquire("scheduler:one", 0).unwrap());
        (home, store)
    }

    #[test]
    fn ending_target_conversation_stops_incoming_cross_conversation_work() {
        let (_home, mut store) = fixture();
        store.ingest(task(1, "agent", "target", false)).unwrap();
        store.control("target", "end", 1).unwrap();
        assert_eq!(
            store.discussion("round").unwrap().unwrap().state,
            DiscussionState::Ended
        );
    }

    #[test]
    fn human_wake_preserves_decision_audit_and_fences_the_old_claim_after_restart() {
        let (home, mut store) = fixture();
        store.ingest(task(1, "a", "chat", true)).unwrap();
        let mut waiting = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
        let decision = serde_json::from_value(serde_json::json!({
            "action":"wait", "reason_code":"observe_discussion",
            "public_reason":"waiting for additional information"
        }))
        .unwrap();
        waiting
            .finish_scheduled_decision(&decision, &["b".into()])
            .unwrap();
        let audit = serde_json::to_value(&waiting.last_decision).unwrap();
        assert!(store.update(&waiting, 1).unwrap());
        assert_eq!(store.status(1).unwrap().usage[0].replies, 0);
        store.authorize_human("new-human", "chat", 2, 2).unwrap();
        drop(store);
        let mut store = ScheduleStore::open(home.path()).unwrap();
        let queued = store.get("1").unwrap();
        assert_eq!(queued.state, TaskState::Queued);
        assert_eq!(queued.due_at, 2);
        assert!(!queued.quiet);
        assert!(queued.owner.is_none());
        assert!(queued.reason.is_none());
        assert_eq!(queued.version, waiting.version + 1);
        assert_eq!(serde_json::to_value(&queued.last_decision).unwrap(), audit);
        assert!(!store.update(&waiting, 3).unwrap());
        let next = store.claim("1", "scheduler:one", 3).unwrap().unwrap();
        assert_eq!(next.state, TaskState::Evaluating);
        assert_eq!(next.usage_epoch, 2);
        assert_eq!(store.status(3).unwrap().usage[0].replies, 0);
    }

    #[test]
    fn only_working_members_are_active_and_a_settled_member_wakes_its_waiters() {
        let (_home, mut store) = fixture();
        // b replies without deciding first; a decides, then waits for b.
        store.ingest(task(1, "b", "chat", false)).unwrap();
        store.ingest(task(2, "a", "chat", true)).unwrap();
        store.ingest(task(3, "c", "chat", true)).unwrap();
        assert_eq!(store.active_members("chat", "a").unwrap(), vec!["b"]);
        let mut a = store.claim("2", "scheduler:one", 0).unwrap().unwrap();
        assert!(store.active_members("chat", "b").unwrap().is_empty());
        let wait = serde_json::from_value(
            serde_json::json!({"action":"wait","reason_code":"observe_discussion"}),
        )
        .unwrap();
        a.finish_scheduled_decision(&wait, &["b".into()]).unwrap();
        assert!(store.update(&a, 0).unwrap());
        let a = store.get("2").unwrap();
        assert_eq!(a.blocked_on, vec!["b"]);
        assert!(a.due_at > now_ms());
        // A waiting member is not someone to wait for.
        assert_eq!(store.active_members("chat", "c").unwrap(), vec!["b"]);
        let mut b = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
        assert_eq!(store.get("2").unwrap().state, TaskState::Waiting);
        b.state = TaskState::Completed;
        assert!(store.update(&b, 0).unwrap());
        let a = store.get("2").unwrap();
        assert!(a.due_at <= now_ms());
        assert!(a.blocked_on.is_empty());
    }

    #[test]
    fn a_wait_saved_after_its_member_settled_is_due_at_once() {
        let (_home, mut store) = fixture();
        store.ingest(task(1, "a", "chat", true)).unwrap();
        let mut a = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
        let wait = serde_json::from_value(
            serde_json::json!({"action":"wait","reason_code":"observe_discussion"}),
        )
        .unwrap();
        // b was working when the decision started and has finished since.
        a.finish_scheduled_decision(&wait, &["b".into()]).unwrap();
        assert!(store.update(&a, 0).unwrap());
        assert!(store.get("1").unwrap().due_at <= now_ms());
    }

    #[test]
    fn old_task_records_have_no_invented_decision() {
        let mut old = serde_json::to_value(task(1, "agent", "chat", false)).unwrap();
        old.as_object_mut().unwrap().remove("last_decision");
        let restored: ScheduledTask = serde_json::from_value(old).unwrap();
        assert!(restored.last_decision.is_none());
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
        c.state = TaskState::ReadyToReply;
        d.state = TaskState::ReadyToReply;
        store.update(&c, 0).unwrap();
        store.update(&d, 0).unwrap();
        assert!(store.claim("4", "scheduler:one", 0).unwrap().is_none());
        a.state = TaskState::Completed;
        b.state = TaskState::Completed;
        store.update(&a, 0).unwrap();
        store.update(&b, 0).unwrap();
        assert!(store.claim("3", "scheduler:one", 0).unwrap().is_some());
        let mut reply = store.claim("4", "scheduler:one", 0).unwrap().unwrap();
        assert_eq!(reply.state, TaskState::Responding);
        assert!(store.claim("5", "scheduler:one", 0).unwrap().is_none());
        reply.state = TaskState::Completed;
        store.update(&reply, 0).unwrap();
        assert!(store.claim("5", "scheduler:one", 0).unwrap().is_some());
    }

    #[test]
    fn pending_peer_inputs_merge_idempotently_across_restart() {
        let (home, mut store) = fixture();
        store.ingest(task(1, "a", "chat", true)).unwrap();
        let mut waiting = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
        waiting.state = TaskState::Waiting;
        waiting.due_at = 30_000;
        store.update(&waiting, 0).unwrap();
        drop(store);
        let mut store = ScheduleStore::open(home.path()).unwrap();
        let mut newer = task(2, "a", "chat", true);
        newer.due_at = 50;
        store.ingest(newer.clone()).unwrap();
        store.ingest(newer).unwrap();
        let merged = store.get("1").unwrap();
        assert_eq!(merged.deliveries, vec![1, 2]);
        assert_eq!(merged.triggers, vec!["message-1", "message-2"]);
        assert_eq!(merged.message, "message-2");
        assert_eq!(merged.state, TaskState::Queued);
        assert_eq!(merged.due_at, 50);
        assert!(store.get("2").is_err());
        assert!(store.assigned(2).unwrap());
        // Another member's message and replies that skip the decision keep their own task.
        store.ingest(task(4, "b", "chat", true)).unwrap();
        assert_eq!(store.get("4").unwrap().deliveries, vec![4]);
        store.ingest(task(3, "a", "chat", false)).unwrap();
        let urgent = store.claim("3", "scheduler:one", 100).unwrap().unwrap();
        assert_eq!(urgent.state, TaskState::Responding);
        assert_eq!(urgent.deliveries, vec![3]);
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
        assert_eq!(store.get("1").unwrap().state, TaskState::Interrupted);
        assert_eq!(store.get("2").unwrap().state, TaskState::Queued);
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
        store.control("chat", "pause", 0).unwrap();
        assert!(store.claim("2", "scheduler:one", 0).unwrap().is_none());
        for _ in 0..150 {
            assert!(store.debit(&a, 0).unwrap());
        }
        store.control("chat", "resume", 0).unwrap();
        let b = store.claim("2", "scheduler:one", 0).unwrap().unwrap();
        store.expire(0).unwrap();
        assert_eq!(
            store.discussion("round").unwrap().unwrap().state,
            DiscussionState::Active
        );
        store.control("chat", "end", 0).unwrap();
        assert!(!store.authorized(&b, 0).unwrap());
        store.control("chat", "resume", 0).unwrap();
        assert_eq!(
            store.discussion("round").unwrap().unwrap().state,
            DiscussionState::Ended
        );
    }

    #[test]
    fn waiting_decisions_do_not_consume_replies_or_have_a_call_limit() {
        let (_home, mut store) = fixture();
        store.ingest(task(1, "a", "chat", true)).unwrap();
        for _ in 0..105 {
            let mut task = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
            assert!(store.debit(&task, 0).unwrap());
            task.state = TaskState::Waiting;
            store.update(&task, 0).unwrap();
        }
        assert_eq!(store.status(0).unwrap().usage[0].replies, 0);
        assert_eq!(store.discussion("round").unwrap().unwrap().calls, 105);
    }

    #[test]
    fn hundred_replies_reset_only_on_new_human_input_and_not_resume_or_replay() {
        let (_home, mut store) = fixture();
        store.authorize_human("human-1", "chat", 1, 0).unwrap();
        for id in 1..=100 {
            store.ingest(task(id, "a", "chat", false)).unwrap();
            let mut reply = store
                .claim(&id.to_string(), "scheduler:one", 0)
                .unwrap()
                .unwrap();
            reply.state = TaskState::Completed;
            store.update(&reply, 0).unwrap();
        }
        store.ingest(task(101, "a", "chat", false)).unwrap();
        assert!(store.claim("101", "scheduler:one", 0).unwrap().is_none());
        assert_eq!(store.get("101").unwrap().state, TaskState::Limited);
        store.control("chat", "resume", 0).unwrap();
        store.authorize_human("human-1", "chat", 1, 0).unwrap();
        assert_eq!(store.status(0).unwrap().usage[0].replies, 100);
        store.authorize_human("human-2", "chat", 102, 0).unwrap();
        assert_eq!(
            store.human_discussion("human-2").unwrap().as_deref(),
            Some("round")
        );
        assert!(store.status(0).unwrap().usage.is_empty());
        assert!(
            store
                .claim("101", "scheduler:one", 5_000)
                .unwrap()
                .is_some()
        );
        store.authorize_human("human-1", "chat", 1, 5_000).unwrap();
        assert_eq!(store.status(5_000).unwrap().usage[0].replies, 1);
    }

    #[test]
    fn ready_survives_restart_and_stale_snapshot_requires_a_new_decision() {
        let (home, mut store) = fixture();
        store.ingest(task(1, "a", "chat", true)).unwrap();
        let mut ready = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
        ready.state = TaskState::ReadyToReply;
        store.update(&ready, 0).unwrap();
        drop(store);
        let mut store = ScheduleStore::open(home.path()).unwrap();
        assert!(store.acquire("scheduler:two", LEASE_MS + 1).unwrap());
        assert_eq!(store.get("1").unwrap().state, TaskState::ReadyToReply);
        assert_eq!(store.status(LEASE_MS + 1).unwrap().usage[0].replies, 0);
        store.invalidate_ready("1", 2, LEASE_MS + 1).unwrap();
        let again = store
            .claim("1", "scheduler:two", LEASE_MS + 1)
            .unwrap()
            .unwrap();
        assert_eq!(again.state, TaskState::Evaluating);
    }

    #[test]
    fn arrivals_during_decision_keep_independent_delivery_snapshots() {
        let (_home, mut store) = fixture();
        store.ingest(task(1, "a", "chat", true)).unwrap();
        let mut deciding = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
        store.ingest(task(2, "a", "chat", false)).unwrap();
        deciding.state = TaskState::ReadyToReply;
        store.update(&deciding, 0).unwrap();
        let ready = store.get("1").unwrap();
        assert_eq!(ready.state, TaskState::ReadyToReply);
        assert_eq!(ready.deliveries, vec![1]);
        assert_eq!(ready.triggers, vec!["message-1"]);
        assert_eq!(store.get("2").unwrap().state, TaskState::Queued);
        assert_eq!(store.get("2").unwrap().deliveries, vec![2]);
    }

    #[test]
    fn status_keeps_old_live_work_and_reconciliation_is_persistent() {
        let (home, mut store) = fixture();
        store.ingest(task(1, "a", "chat", true)).unwrap();
        for id in 2..=510 {
            let mut old = task(id, "b", "history", false);
            old.state = TaskState::Completed;
            store.ingest(old).unwrap();
            store.mark_reconciled(&id.to_string()).unwrap();
        }
        for id in 0..110 {
            store.start(&format!("new-{id}"), "other", 0).unwrap();
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
        store.authorize_human("human-1", "chat", 1, 0).unwrap();
        store.ingest(task(1, "a", "chat", false)).unwrap();
        let mut old = store.claim("1", "scheduler:one", 0).unwrap().unwrap();
        store.authorize_human("human-2", "chat", 2, 0).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO usage(discussion,principal,replies) VALUES('round','a',10)",
                [],
            )
            .unwrap();
        old.state = TaskState::Queued;
        store.update(&old, 0).unwrap();
        assert_eq!(store.status(0).unwrap().usage[0].replies, 10);
        store
            .authorize_human("late-old-message", "chat", 1, 0)
            .unwrap();
        assert_eq!(store.status(0).unwrap().usage[0].replies, 10);
        store.control("chat", "end", 0).unwrap();
        store.authorize_human("human-2", "chat", 2, 0).unwrap();
        assert_eq!(
            store.discussion("round").unwrap().unwrap().state,
            DiscussionState::Ended
        );
        store.authorize_human("human-3", "chat", 3, 0).unwrap();
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
    mod policies;
}
