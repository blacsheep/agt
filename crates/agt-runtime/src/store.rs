use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::{Result, RunRequest, RuntimeError, fingerprint};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Running,
    Completed,
    Failed,
    Canceled,
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
    /// Includes child model calls for parent runs.
    pub model_steps: u32,
}

pub struct RunStore {
    connection: Connection,
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
                 PRAGMA user_version = 1;",
            )?,
            1 => {}
            _ => {
                return Err(RuntimeError::Configuration(
                    "unsupported runtime database version",
                ));
            }
        }
        tx.commit()?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Self { connection })
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
            model_steps: 0,
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
        Ok((record, true))
    }

    pub(crate) fn save(&self, record: &RunRecord) -> Result<()> {
        self.connection.execute(
            "UPDATE runs SET record_json = ?2 WHERE id = ?1",
            params![record.id, serde_json::to_string(record)?],
        )?;
        Ok(())
    }
}

pub(crate) fn database_path(
    home: &Path,
    directory: &str,
    name: &str,
) -> Result<std::path::PathBuf> {
    let dir = home.join(directory);
    agt_storage::ensure_dir_under(home, &dir)
        .map_err(|_| RuntimeError::Configuration("unsafe or inaccessible data directory"))?;
    for suffix in ["", "-wal", "-shm", "-journal"] {
        agt_storage::check_no_symlinks_under(home, dir.join(format!("{name}{suffix}")))
            .map_err(|_| RuntimeError::Configuration("unsafe database path"))?;
    }
    Ok(dir.join(name))
}
