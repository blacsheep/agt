//! AGT-owned session metadata; the engine alone creates thread IDs.
use agt_harness::{Codex, HarnessConfig, HarnessStore};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HarnessSession {
    /// AGT record key, deliberately distinct from the engine's session_id.
    pub id: String,
    pub principal: String,
    pub conversation: String,
    pub agent: String,
    pub harness: String,
    pub config: HarnessConfig,
    pub title: String,
    pub session_id: Option<String>,
    pub busy: bool,
}
pub struct HarnessSessions {
    pub(crate) db: Connection,
}
/// Before submission, errors cannot leave an active external turn behind.
/// After submission, only the normal durable completion path may unlock.
pub(crate) struct ExecutionLock<'a> {
    sessions: &'a HarnessSessions,
    id: &'a str,
    submitted: std::cell::Cell<bool>,
}
impl ExecutionLock<'_> {
    pub(crate) fn submitted(&self) {
        self.submitted.set(true);
    }
}
impl Drop for ExecutionLock<'_> {
    fn drop(&mut self) {
        if !self.submitted.get() {
            let _ = self.sessions.db.execute(
                "UPDATE harness_sessions SET dirty=0 WHERE scope=?1",
                [self.id],
            );
        }
    }
}
impl HarnessSessions {
    pub fn open(home: &Path) -> Result<Self> {
        let db = Connection::open(crate::store::database_path(
            home,
            "runtime",
            "harnesses.db",
        )?)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS harness_sessions(scope TEXT PRIMARY KEY,principal TEXT NOT NULL,conversation TEXT NOT NULL,agent TEXT NOT NULL,harness TEXT NOT NULL,config TEXT NOT NULL,title TEXT NOT NULL DEFAULT '',thread TEXT,through_seq INTEGER NOT NULL DEFAULT 0,source_hash TEXT NOT NULL DEFAULT '',dirty INTEGER NOT NULL DEFAULT 0,cancel INTEGER NOT NULL DEFAULT 0);")?;
        Ok(Self { db })
    }
    pub(crate) fn ensure(
        &self,
        identity: (&str, &str, &str, i64),
        config: &agt_agent::AgentConfig,
        default: &HarnessConfig,
        explicit: Option<&str>,
    ) -> Result<HarnessSession> {
        let (principal, conversation, agent, cutoff) = identity;
        let harness = config
            .harness
            .as_ref()
            .context("agent does not use a Harness")?
            .to_string();
        let id = explicit
            .map(str::to_owned)
            .unwrap_or(crate::fingerprint(&serde_json::to_vec(&json!([
                3,
                principal,
                conversation,
                cutoff,
                agent,
                format!("{config:?}")
            ]))?));
        self.db.execute("INSERT OR IGNORE INTO harness_sessions(scope,principal,conversation,agent,harness,config) VALUES(?1,?2,?3,?4,?5,?6)", params![id,principal,conversation,agent,harness,serde_json::to_string(default)?])?;
        self.get(&id)
    }
    pub fn current(
        &self,
        home: &Path,
        principal: &str,
        conversation: &str,
    ) -> Result<HarnessSession> {
        let im = agt_im::ImStore::open(crate::store::database_path(home, "im", "im.db")?)?;
        if !im
            .list_members(&agt_im::ConversationId::new(conversation))?
            .iter()
            .any(|m| m.principal_id.as_str() == principal && m.left_message_seq.is_none())
        {
            bail!("agent is not a current member");
        }
        let binding = agt_service::BindingStore::open(home)?
            .list_with_state()?
            .into_iter()
            .find(|b| b.binding.principal.as_str() == principal)
            .context("agent has no binding")?
            .binding;
        let config = crate::harness::agent_config(home, &binding.agent)?;
        let reference = config
            .harness
            .as_ref()
            .context("agent does not use a Harness")?;
        let default = HarnessStore::new(home)
            .read_in(reference.category(), reference.name().as_str())?
            .config;
        let cutoff = crate::ScheduleStore::open(home)?.context_cutoff(conversation)?;
        self.ensure(
            (principal, conversation, &binding.agent, cutoff),
            &config,
            &default,
            None,
        )
    }
    pub fn get(&self, id: &str) -> Result<HarnessSession> {
        let row = self.db.query_row("SELECT principal,conversation,agent,harness,config,title,thread,dirty FROM harness_sessions WHERE scope=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,bool>(7)?)))?;
        Ok(HarnessSession {
            id: id.into(),
            principal: row.0,
            conversation: row.1,
            agent: row.2,
            harness: row.3,
            config: serde_json::from_str(&row.4)?,
            title: row.5,
            session_id: row.6,
            busy: row.7,
        })
    }
    /// Pin inherited defaults once the engine reports them, without replacing explicit selections.
    pub(crate) fn resolve_default(
        &self,
        id: &str,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Result<()> {
        let mut session = self.get(id)?;
        if session.config.model.is_none() {
            session.config.model = model.map(str::to_owned);
        }
        if session.config.reasoning_effort.is_none() {
            session.config.reasoning_effort = effort.map(str::to_owned);
        }
        self.db.execute(
            "UPDATE harness_sessions SET config=?2 WHERE scope=?1",
            params![id, serde_json::to_string(&session.config)?],
        )?;
        Ok(())
    }
    pub fn list(&self) -> Result<Vec<HarnessSession>> {
        let mut stmt = self
            .db
            .prepare("SELECT scope FROM harness_sessions ORDER BY rowid DESC")?;
        stmt.query_map([], |r| r.get::<_, String>(0))?
            .map(|id| self.get(&id?))
            .collect()
    }
    pub(crate) fn lock(&self, id: &str) -> Result<()> {
        if self.db.execute(
            "UPDATE harness_sessions SET dirty=1,cancel=0 WHERE scope=?1 AND dirty=0",
            [id],
        )? != 1
        {
            bail!("session is running or interrupted; stop and reset it before retrying");
        }
        Ok(())
    }
    pub(crate) fn execution_lock<'a>(&'a self, id: &'a str) -> Result<ExecutionLock<'a>> {
        self.lock(id)?;
        Ok(ExecutionLock {
            sessions: self,
            id,
            submitted: std::cell::Cell::new(false),
        })
    }
    pub fn configure(&self, id: &str, model: &str, effort: Option<&str>) -> Result<HarnessSession> {
        self.lock(id)?;
        let result = (|| {
            let mut session = self.get(id)?;
            let catalog = Codex::models(&session.config)?;
            let selected = catalog["data"]
                .as_array()
                .context("missing models")?
                .iter()
                .find(|m| m["model"] == model)
                .context("model is not available in this Harness")?;
            let effort = effort.or_else(|| selected["defaultReasoningEffort"].as_str());
            if let Some(e) = effort
                && !selected["supportedReasoningEfforts"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|v| v["reasoningEffort"] == e))
            {
                bail!("reasoning effort is not supported by this model");
            }
            session.config.model = Some(model.into());
            session.config.reasoning_effort = effort.map(str::to_owned);
            self.db.execute(
                "UPDATE harness_sessions SET config=?2 WHERE scope=?1",
                params![id, serde_json::to_string(&session.config)?],
            )?;
            Ok(())
        })();
        self.db
            .execute("UPDATE harness_sessions SET dirty=0 WHERE scope=?1", [id])?;
        result?;
        self.get(id)
    }
    pub fn rename(&self, id: &str, title: &str) -> Result<HarnessSession> {
        if title.trim().is_empty() || title.len() > 256 {
            bail!("title must be 1–256 bytes");
        }
        self.lock(id)?;
        let result: Result<()> = (|| {
            let session = self.get(id)?;
            // No engine thread exists before first execution; apply the saved title on creation.
            if let Some(thread) = &session.session_id {
                Codex::rename(&session.config, thread, title)?;
            }
            self.db.execute(
                "UPDATE harness_sessions SET title=?2 WHERE scope=?1",
                params![id, title],
            )?;
            Ok(())
        })();
        self.db
            .execute("UPDATE harness_sessions SET dirty=0 WHERE scope=?1", [id])?;
        result?;
        self.get(id)
    }
    pub fn cancel_conversation(&self, conversation: &str) -> Result<()> {
        self.db.execute(
            "UPDATE harness_sessions SET cancel=1 WHERE conversation=?1 AND dirty=1",
            [conversation],
        )?;
        Ok(())
    }
    pub fn cancel(&self, id: &str) -> Result<()> {
        self.get(id)?;
        self.db.execute(
            "UPDATE harness_sessions SET cancel=1 WHERE scope=?1 AND dirty=1",
            [id],
        )?;
        Ok(())
    }
    pub(crate) fn canceled(&self, id: &str) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT cancel FROM harness_sessions WHERE scope=?1",
            [id],
            |r| r.get(0),
        )?)
    }
    pub fn compact(&self, id: &str, token: &agt_core::CancellationToken) -> Result<()> {
        if self.get(id)?.session_id.is_none() {
            bail!("session has not started yet");
        }
        self.lock(id)?;
        let session = self.get(id)?;
        let thread = session.session_id.context("session has not started yet")?;
        // Failure is deliberately left dirty: never race/replay a compaction with an unknown outcome.
        Codex::compact(&session.config, &thread, &mut |_| {
            if token.is_canceled() || self.canceled(id)? {
                bail!("compaction canceled");
            }
            Ok(())
        })?;
        self.db
            .execute("UPDATE harness_sessions SET dirty=0 WHERE scope=?1", [id])?;
        Ok(())
    }
}
