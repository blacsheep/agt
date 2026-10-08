use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
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

pub struct RunStore {
    home: std::path::PathBuf,
    pub(crate) connection: Connection,
}

impl RunStore {
    pub fn open(home: &Path) -> Result<Self> {
        let path = database_path(home, "runtime", "runs.db")?;
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u32 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => tx.execute_batch(
                "CREATE TABLE runs (
                    id TEXT PRIMARY KEY,
                    request_key TEXT NOT NULL UNIQUE,
                    request_hash TEXT NOT NULL,
                    parent_run_id TEXT REFERENCES runs(id),
                    record_json TEXT NOT NULL
                 );
                 CREATE INDEX runs_parent ON runs(parent_run_id);
                 CREATE TABLE run_calls (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    run_id TEXT NOT NULL REFERENCES runs(id),
                    record_json TEXT NOT NULL
                 );
                 CREATE INDEX run_calls_run ON run_calls(run_id, id);
                 PRAGMA user_version = 2;",
            )?,
            2 => {}
            3..=6 => {}
            _ => {
                return Err(RuntimeError::Configuration(
                    "unsupported runtime database version",
                ));
            }
        }
        if version < 3 {
            tx.execute_batch("CREATE TABLE native_checkpoints(run_id TEXT PRIMARY KEY, checkpoint TEXT NOT NULL); PRAGMA user_version=3;")?;
        }
        if version < 4 {
            tx.execute_batch("UPDATE native_checkpoints SET checkpoint=json_set(checkpoint,'$.engine','native'); ALTER TABLE native_checkpoints RENAME TO run_checkpoints; PRAGMA user_version=4;")?;
        }
        if version < 5 {
            tx.execute_batch("CREATE TABLE IF NOT EXISTS change_outbox(seq INTEGER PRIMARY KEY AUTOINCREMENT, conversation TEXT, kind TEXT NOT NULL, at INTEGER NOT NULL);
                CREATE TRIGGER IF NOT EXISTS run_created AFTER INSERT ON runs BEGIN
                    INSERT INTO change_outbox(conversation,kind,at) VALUES(json_extract(NEW.record_json,'$.request.trigger.conversation_id'),'run',CAST(unixepoch('subsec')*1000 AS INTEGER));
                END;
                CREATE TRIGGER IF NOT EXISTS run_changed AFTER UPDATE ON runs WHEN OLD.record_json<>NEW.record_json BEGIN
                    INSERT INTO change_outbox(conversation,kind,at) VALUES(json_extract(NEW.record_json,'$.request.trigger.conversation_id'),'run',CAST(unixepoch('subsec')*1000 AS INTEGER));
                END;
                PRAGMA user_version=5;")?;
        }
        if version < 6 {
            tx.execute_batch("CREATE TABLE IF NOT EXISTS tool_details(run_id TEXT NOT NULL REFERENCES runs(id), activity_id TEXT NOT NULL, detail_json TEXT NOT NULL, PRIMARY KEY(run_id, activity_id)); PRAGMA user_version=6;")?;
        }
        tx.commit()?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
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
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
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
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
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
    agt_storage::database_path(home, directory, name)
        .map_err(|_| RuntimeError::Configuration("unsafe or inaccessible database path"))
}

#[cfg(test)]
mod checkpoint_migration_tests {
    use super::*;
    #[test]
    fn version_two_upgrade_preserves_run_claims_and_rejects_future_schema() {
        let home = tempfile::tempdir().unwrap();
        let mut store = RunStore::open(home.path()).unwrap();
        let request = RunRequest::new("example", "existing work");
        let (record, _) = store.claim(&request, None).unwrap();
        store
            .connection
            .execute_batch("DROP TABLE run_checkpoints; PRAGMA user_version=2;")
            .unwrap();
        drop(store);
        let mut store = RunStore::open(home.path()).unwrap();
        let (replayed, claimed) = store.claim(&request, None).unwrap();
        assert!(!claimed);
        assert_eq!(replayed.id, record.id);
        assert_eq!(replayed.state, RunState::Running);
        let count: i64 = store
            .connection
            .query_row("SELECT count(*) FROM run_checkpoints", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        store
            .connection
            .pragma_update(None, "user_version", 99)
            .unwrap();
        drop(store);
        assert!(RunStore::open(home.path()).is_err());
    }
}
