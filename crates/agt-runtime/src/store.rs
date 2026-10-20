use std::path::Path;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{Result, RunRequest, RuntimeError, fingerprint};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Running,
    Suspended,
    Completed,
    Failed,
    Canceled,
    Interrupted,
    OutcomeUnknown,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunRecord {
    pub id: String,
    pub parent_run_id: Option<String>,
    pub request: RunRequest,
    pub state: RunState,
    pub config_fingerprint: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    /// Local-only cause chain behind `error`; never forwarded to models or the Web.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
    pub termination: Option<crate::Termination>,
    /// Includes child model calls for parent runs.
    pub model_steps: u32,
    /// Local failed tool invocations, even when the model subsequently finishes.
    pub tool_failures: u32,
    #[serde(default)]
    pub context: crate::ContextCheckpoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<crate::RunProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<serde_json::Value>,
}

/// Every module in the shared runtime database. Opening any runtime store installs
/// all of them, so a later open never needs the write lock while another connection
/// in this process holds a transaction.
const RUNTIME_SCHEMAS: &[&agt_storage::sqlite::Schema] = &[
    &agt_storage::sqlite::OUTBOX,
    &SCHEMA,
    &crate::schedule_store::SCHEMA,
    &crate::timers::SCHEMA,
    &crate::work::SCHEMA,
    &crate::context_store::SCHEMA,
    &crate::harness_sessions::SCHEMA,
    &crate::approval::SCHEMA,
    &crate::approval::RULES_SCHEMA,
    &crate::events::SCHEMA,
    &agt_service::BINDINGS_SCHEMA,
];

pub(crate) fn runtime_db(
    home: &Path,
) -> std::result::Result<agt_storage::sqlite::Db, agt_storage::sqlite::SchemaError> {
    agt_storage::sqlite::open(home, agt_storage::sqlite::RUNTIME_DB, RUNTIME_SCHEMAS)
}

pub(crate) const SCHEMA: agt_storage::sqlite::Schema = agt_storage::sqlite::Schema {
    module: "runs",
    version: 1,
    ddl: "CREATE TABLE runs(
            id TEXT PRIMARY KEY,
            request_key TEXT NOT NULL UNIQUE,
            request_hash TEXT NOT NULL,
            parent_run_id TEXT REFERENCES runs(id),
            record_json TEXT NOT NULL);
        CREATE INDEX runs_parent ON runs(parent_run_id);
        CREATE TABLE run_calls(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id TEXT NOT NULL REFERENCES runs(id),
            record_json TEXT NOT NULL);
        CREATE INDEX run_calls_run ON run_calls(run_id, id);
        CREATE TABLE run_checkpoints(run_id TEXT PRIMARY KEY, checkpoint TEXT NOT NULL);
        CREATE TABLE tool_details(run_id TEXT NOT NULL REFERENCES runs(id), activity_id TEXT NOT NULL, detail_json TEXT NOT NULL, PRIMARY KEY(run_id, activity_id));
        CREATE TRIGGER run_created AFTER INSERT ON runs BEGIN
            INSERT INTO change_outbox(conversation,kind,at) VALUES(json_extract(NEW.record_json,'$.request.trigger.conversation_id'),'run',CAST(unixepoch('subsec')*1000 AS INTEGER));
        END;
        CREATE TRIGGER run_changed AFTER UPDATE ON runs WHEN OLD.record_json<>NEW.record_json BEGIN
            INSERT INTO change_outbox(conversation,kind,at) VALUES(json_extract(NEW.record_json,'$.request.trigger.conversation_id'),'run',CAST(unixepoch('subsec')*1000 AS INTEGER));
        END;",
};

pub struct RunStore {
    home: std::path::PathBuf,
    pub(crate) connection: agt_storage::sqlite::Db,
}

impl RunStore {
    pub fn open(home: &Path) -> Result<Self> {
        let connection = runtime_db(home).map_err(|e| {
            RuntimeError::Configuration(crate::Reason::because("runtime database unavailable", e))
        })?;
        Ok(Self {
            connection,
            home: home.into(),
        })
    }

    pub(crate) fn by_request_key(&self, key: &str) -> Result<Option<RunRecord>> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT record_json FROM runs WHERE request_key=?1",
                [format!("root:{key}")],
                |r| r.get(0),
            )
            .optional()?;
        value
            .map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }

    pub fn get(&self, id: &str) -> Result<RunRecord> {
        let json: String = self
            .connection
            .query_row("SELECT record_json FROM runs WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?
            .ok_or(RuntimeError::NotFound)?;
        Ok(serde_json::from_str(&json)?)
    }

    pub fn children(&self, parent: &str) -> Result<Vec<RunRecord>> {
        let mut statement = self
            .connection
            .prepare("SELECT record_json FROM runs WHERE parent_run_id = ?1 ORDER BY rowid")?;
        let rows = statement.query_map([parent], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    pub fn list(&self, limit: u32) -> Result<Vec<RunRecord>> {
        if !(1..=200).contains(&limit) {
            return Err(RuntimeError::Invalid(
                "list limit must be in 1..=200".into(),
            ));
        }
        let mut statement = self
            .connection
            .prepare("SELECT record_json FROM runs ORDER BY rowid DESC LIMIT ?1")?;
        let rows = statement.query_map([limit], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    /// Newest root runs triggered from one conversation.
    pub fn list_for_conversation(&self, conversation: &str, limit: u32) -> Result<Vec<RunRecord>> {
        if !(1..=200).contains(&limit) {
            return Err(RuntimeError::Invalid(
                "list limit must be in 1..=200".into(),
            ));
        }
        let mut statement = self.connection.prepare(
            "SELECT record_json FROM runs WHERE parent_run_id IS NULL
             AND json_extract(record_json,'$.request.trigger.conversation_id')=?1
             ORDER BY rowid DESC LIMIT ?2",
        )?;
        let rows =
            statement.query_map(params![conversation, limit], |row| row.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    /// The unique claim commits before any model or tool invocation.
    pub(crate) fn claim(
        &mut self,
        request: &RunRequest,
        parent: Option<&str>,
    ) -> Result<(RunRecord, bool)> {
        let key = match parent {
            Some(parent) => format!("child:{parent}:{}", request.request_key),
            None => format!("root:{}", request.request_key),
        };
        let hash = fingerprint(&serde_json::to_vec(&(request, parent))?);
        let tx = agt_storage::sqlite::write(&self.connection)?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT request_hash, record_json FROM runs WHERE request_key = ?1",
                [&key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((previous, json)) = existing {
            if previous != hash {
                return Err(RuntimeError::Conflict);
            }
            return Ok((serde_json::from_str(&json)?, false));
        }
        let record = RunRecord {
            id: uuid::Uuid::new_v4().to_string(),
            parent_run_id: parent.map(str::to_owned),
            request: request.clone(),
            state: RunState::Running,
            config_fingerprint: None,
            output: None,
            error: None,
            termination: None,
            model_steps: 0,
            tool_failures: 0,
            diagnostic: None,
            context: Default::default(),
            progress: None,
            execution: None,
        };
        tx.execute(
            "INSERT INTO runs (id, request_key, request_hash, parent_run_id, record_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                record.id,
                key,
                hash,
                parent,
                serde_json::to_string(&record)?
            ],
        )?;
        tx.commit()?;
        agt_storage::wake::notify(&self.home);
        Ok((record, true))
    }

    pub(crate) fn save_checkpoint(
        &mut self,
        record: &RunRecord,
        checkpoint: &crate::work::wait::Checkpoint,
    ) -> Result<()> {
        let mut record = record.clone();
        self.merge_tool_progress(&mut record)?;
        let tx = agt_storage::sqlite::write(&self.connection)?;
        tx.execute(
            "INSERT INTO run_checkpoints VALUES(?1,?2)",
            params![record.id, serde_json::to_string(checkpoint)?],
        )?;
        tx.execute(
            "UPDATE runs SET record_json=?2 WHERE id=?1",
            params![record.id, serde_json::to_string(&record)?],
        )?;
        tx.commit()?;
        agt_storage::wake::notify(&self.home);
        Ok(())
    }
    pub(crate) fn checkpoint(&self, id: &str) -> Result<crate::work::wait::Checkpoint> {
        let value: String = self.connection.query_row(
            "SELECT checkpoint FROM run_checkpoints WHERE run_id=?1",
            [id],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&value)?)
    }
    /// Close a Run whose scheduler stopped while it executed. Its effects are unknown, so
    /// it becomes `outcome_unknown` instead of staying `running` forever.
    pub(crate) fn mark_owner_lost(
        connection: &rusqlite::Connection,
        request_key: &str,
    ) -> Result<()> {
        let value: Option<String> = connection
            .query_row(
                "SELECT record_json FROM runs WHERE request_key=?1",
                [format!("root:{request_key}")],
                |r| r.get(0),
            )
            .optional()?;
        let Some(value) = value else {
            return Ok(());
        };
        let mut record: RunRecord = serde_json::from_str(&value)?;
        if record.state != RunState::Running {
            return Ok(());
        }
        record.state = RunState::OutcomeUnknown;
        record.termination = Some(crate::Termination::new(crate::StopCode::SchedulerLeaseLost));
        record.error = Some("the scheduler stopped while this Run was executing".into());
        connection.execute(
            "UPDATE runs SET record_json = ?2 WHERE id = ?1",
            params![record.id, serde_json::to_string(&record)?],
        )?;
        Ok(())
    }

    pub(crate) fn save(&self, record: &RunRecord) -> Result<()> {
        let mut record = record.clone();
        self.merge_tool_progress(&mut record)?;
        self.connection.execute(
            "UPDATE runs SET record_json = ?2 WHERE id = ?1",
            params![record.id, serde_json::to_string(&record)?],
        )?;
        agt_storage::wake::notify(&self.home);
        Ok(())
    }

    /// Host-only metadata; deliberately separate from the shared IM transcript.
    pub fn calls(&self, run: &str, after: i64, limit: u32) -> Result<Vec<crate::CallRecord>> {
        if after < 0 || !(1..=200).contains(&limit) {
            return Err(RuntimeError::Invalid(
                "call cursor must be nonnegative and limit in 1..=200".into(),
            ));
        }
        self.get(run)?;
        let mut statement = self.connection.prepare(
            "SELECT id, record_json FROM run_calls WHERE run_id=?1 AND id>?2 ORDER BY id LIMIT ?3",
        )?;
        let rows = statement.query_map(params![run, after, limit], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (id, json) = row?;
            let mut record: crate::CallRecord = serde_json::from_str(&json)?;
            record.id = id;
            Ok(record)
        })
        .collect()
    }

    pub(crate) fn recent_tool_calls(&self, run: &str) -> Result<Vec<crate::CallRecord>> {
        let mut stmt=self.connection.prepare("SELECT id,record_json FROM run_calls WHERE run_id=?1 AND json_extract(record_json,'$.tool_name') IS NOT NULL ORDER BY id DESC LIMIT 40")?;
        let rows = stmt.query_map([run], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let mut result = Vec::new();
        for row in rows {
            let (id, json) = row?;
            let mut record: crate::CallRecord = serde_json::from_str(&json)?;
            record.id = id;
            result.push(record);
        }
        result.reverse();
        Ok(result)
    }
    pub(crate) fn start_call(&self, mut record: crate::CallRecord) -> Result<crate::CallRecord> {
        self.connection.execute(
            "INSERT INTO run_calls(run_id, record_json) VALUES(?1,?2)",
            params![record.run_id, serde_json::to_string(&record)?],
        )?;
        record.id = self.connection.last_insert_rowid();
        if record.tool_name.is_some() {
            self.save(&self.get(&record.run_id)?)?;
        }
        Ok(record)
    }

    pub(crate) fn finish_call(&self, record: &crate::CallRecord) -> Result<()> {
        self.connection.execute(
            "UPDATE run_calls SET record_json=?2 WHERE id=?1",
            params![record.id, serde_json::to_string(record)?],
        )?;
        if record.tool_name.is_some() {
            self.save(&self.get(&record.run_id)?)?;
        }
        agt_storage::wake::notify(&self.home);
        Ok(())
    }
}

pub(crate) fn database_path(
    home: &Path,
    directory: &str,
    name: &str,
) -> Result<std::path::PathBuf> {
    agt_storage::database_path(home, directory, name).map_err(|e| {
        RuntimeError::Configuration(crate::Reason::because(
            "unsafe or inaccessible database path",
            e,
        ))
    })
}
