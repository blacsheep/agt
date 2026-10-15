//! Shared SQLite home databases. Each module owns its tables and records its own
//! schema version, so several modules can share one file and one transaction domain.
use rusqlite::{Connection, OptionalExtension};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// One connection per database file and thread, shared by every store on that thread.
/// A write nested inside another store's transaction therefore joins it as a savepoint
/// instead of waiting on the lock its own thread already holds.
pub type Db = Rc<Connection>;

thread_local! {
    static SHARED: RefCell<HashMap<PathBuf, Weak<Connection>>> = RefCell::default();
}

/// Runtime state shared by runs, scheduling, work, approvals, sessions, bindings and events.
pub const RUNTIME_DB: (&str, &str) = ("runtime", "runtime.db");

/// How long a connection waits for another process's write lock before failing
/// with "database is locked". The console, the scheduler and CLI commands are
/// separate processes on the same databases; no write holds the lock for long,
/// but slow disks (Windows in particular) can stretch a short wait past seconds.
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(30);

/// Generic invalidation outbox drained by the event projection. Modules write it
/// through triggers; it carries no message text or private Run data.
pub const OUTBOX: Schema = Schema {
    module: "outbox",
    version: 1,
    ddl: "CREATE TABLE change_outbox(seq INTEGER PRIMARY KEY AUTOINCREMENT, conversation TEXT, kind TEXT NOT NULL, at INTEGER NOT NULL);",
};

#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error(transparent)]
    Path(#[from] agt_core::AgtError),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("unsupported {module} schema version {found}; expected {expected}")]
    Unsupported {
        module: &'static str,
        found: u32,
        expected: u32,
    },
}

pub struct Schema {
    pub module: &'static str,
    pub version: u32,
    /// Applied once when the module is first installed in a database.
    pub ddl: &'static str,
}

/// Process-wide record of initialized (database, module) pairs. Initialization runs
/// once per process; later opens only connect.
fn initialized() -> &'static Mutex<HashSet<(PathBuf, &'static str)>> {
    static READY: OnceLock<Mutex<HashSet<(PathBuf, &'static str)>>> = OnceLock::new();
    READY.get_or_init(Default::default)
}

/// Open a home database and install the given module schemas, in order.
pub fn open(
    home: &Path,
    (directory, name): (&str, &str),
    schemas: &[&Schema],
) -> Result<Db, SchemaError> {
    let path = crate::database_path(home, directory, name)?;
    let shared = SHARED.with(|shared| shared.borrow().get(&path).and_then(Weak::upgrade));
    let connection = match shared {
        Some(connection) => connection,
        None => {
            let connection = Connection::open(&path)?;
            connection.busy_timeout(BUSY_TIMEOUT)?;
            connection.pragma_update(None, "foreign_keys", "ON")?;
            let connection = Rc::new(connection);
            SHARED.with(|shared| {
                let mut shared = shared.borrow_mut();
                shared.retain(|_, weak| weak.strong_count() > 0);
                shared.insert(path.clone(), Rc::downgrade(&connection));
            });
            connection
        }
    };
    let pending: Vec<_> = {
        let ready = initialized().lock().unwrap_or_else(|e| e.into_inner());
        schemas
            .iter()
            .filter(|s| !ready.contains(&(path.clone(), s.module)))
            .collect()
    };
    if !pending.is_empty() {
        enable_wal(&connection)?;
        install(&connection, &pending)?;
        // Releasing a savepoint does not commit its enclosing transaction. Only
        // cache installations that cannot still disappear with an outer rollback.
        if connection.is_autocommit() {
            let mut ready = initialized().lock().unwrap_or_else(|e| e.into_inner());
            ready.extend(pending.iter().map(|s| (path.clone(), s.module)));
        }
    }
    Ok(connection)
}

/// A write transaction that nests: outermost it is `BEGIN IMMEDIATE`, inside another
/// transaction on the same connection it is a savepoint. Dropping without `commit`
/// rolls back only this level.
pub struct Write<'a> {
    connection: &'a Connection,
    savepoint: Option<String>,
    open: bool,
}

pub fn write(connection: &Connection) -> rusqlite::Result<Write<'_>> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let savepoint = if connection.is_autocommit() {
        connection.execute_batch("BEGIN IMMEDIATE")?;
        None
    } else {
        let name = format!("agt_write_{}", NEXT.fetch_add(1, Ordering::Relaxed));
        connection.execute_batch(&format!("SAVEPOINT {name}"))?;
        Some(name)
    };
    Ok(Write {
        connection,
        savepoint,
        open: true,
    })
}

impl Write<'_> {
    pub fn commit(mut self) -> rusqlite::Result<()> {
        match &self.savepoint {
            None => self.connection.execute_batch("COMMIT"),
            Some(name) => self.connection.execute_batch(&format!("RELEASE {name}")),
        }?;
        self.open = false;
        Ok(())
    }

    /// Commit on success, roll back on failure; the result passes through.
    pub fn finish<T, E: From<rusqlite::Error>>(self, result: Result<T, E>) -> Result<T, E> {
        match result {
            Ok(value) => {
                self.commit()?;
                Ok(value)
            }
            Err(error) => Err(error),
        }
    }
}

impl Deref for Write<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.connection
    }
}

impl Drop for Write<'_> {
    fn drop(&mut self) {
        if self.open {
            let _ = match &self.savepoint {
                None => self.connection.execute_batch("ROLLBACK"),
                Some(name) => self
                    .connection
                    .execute_batch(&format!("ROLLBACK TO {name}; RELEASE {name}")),
            };
        }
    }
}

// WAL is persistent in the file; switching can briefly report BUSY even with a busy timeout.
fn enable_wal(connection: &Connection) -> rusqlite::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match connection.pragma_update(None, "journal_mode", "WAL") {
            Err(error)
                if matches!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
                ) && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            result => return result,
        }
    }
}

/// Installed version of each module, read without taking the write lock.
fn installed(connection: &Connection, schemas: &[&&Schema]) -> rusqlite::Result<Vec<Option<u32>>> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_modules')",
        [],
        |r| r.get(0),
    )?;
    schemas
        .iter()
        .map(|schema| {
            if !exists {
                return Ok(None);
            }
            connection
                .query_row(
                    "SELECT version FROM schema_modules WHERE module=?1",
                    [schema.module],
                    |r| r.get(0),
                )
                .optional()
        })
        .collect()
}

// Another connection in this process may hold a write transaction (for example a
// scheduler fence), so an already installed schema must never need the write lock.
fn install(connection: &Connection, schemas: &[&&Schema]) -> Result<(), SchemaError> {
    let current = installed(connection, schemas)?;
    if current
        .iter()
        .zip(schemas)
        .all(|(found, schema)| *found == Some(schema.version))
    {
        return Ok(());
    }
    let tx = write(connection)?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_modules(module TEXT PRIMARY KEY, version INTEGER NOT NULL);",
    )?;
    for schema in schemas {
        let found: Option<u32> = tx
            .query_row(
                "SELECT version FROM schema_modules WHERE module=?1",
                [schema.module],
                |r| r.get(0),
            )
            .optional()?;
        match found {
            None => {
                tx.execute_batch(schema.ddl)?;
                tx.execute(
                    "INSERT INTO schema_modules VALUES(?1,?2)",
                    rusqlite::params![schema.module, schema.version],
                )?;
            }
            Some(found) if found == schema.version => {}
            Some(found) => {
                return Err(SchemaError::Unsupported {
                    module: schema.module,
                    found,
                    expected: schema.version,
                });
            }
        }
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Schema = Schema {
        module: "a",
        version: 1,
        ddl: "CREATE TABLE a(x);",
    };

    #[test]
    fn failed_commit_rolls_back_and_leaves_connection_reusable() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "PRAGMA foreign_keys=ON;
            CREATE TABLE parent(id INTEGER PRIMARY KEY);
            CREATE TABLE child(id INTEGER REFERENCES parent(id) DEFERRABLE INITIALLY DEFERRED);",
        )
        .unwrap();
        for use_finish in [false, true] {
            let tx = write(&db).unwrap();
            tx.execute("INSERT INTO child VALUES(1)", []).unwrap();
            let result = if use_finish {
                tx.finish(Ok::<_, rusqlite::Error>(()))
            } else {
                tx.commit()
            };
            assert!(result.is_err());
            assert!(db.is_autocommit());
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM child", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        let tx = write(&db).unwrap();
        tx.execute_batch("INSERT INTO parent VALUES(1); INSERT INTO child VALUES(1);")
            .unwrap();
        tx.commit().unwrap();
        assert!(db.is_autocommit());
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM child", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn nested_schema_install_is_cached_only_after_outer_commit() {
        const B: Schema = Schema {
            module: "b",
            version: 1,
            ddl: "CREATE TABLE b(x);",
        };
        for commit in [false, true] {
            let home = tempfile::tempdir().unwrap();
            let db = ("runtime", "schema.db");
            let connection = open(home.path(), db, &[&A]).unwrap();
            let key = (
                crate::database_path(home.path(), db.0, db.1).unwrap(),
                B.module,
            );
            {
                let outer = write(&connection).unwrap();
                open(home.path(), db, &[&B]).unwrap();
                // Repeated opens within the transaction must not promote tentative DDL.
                open(home.path(), db, &[&B]).unwrap();
                assert!(!initialized().lock().unwrap().contains(&key));
                if commit {
                    outer.commit().unwrap();
                }
            }
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='b')",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(exists, commit);
            let reopened = open(home.path(), db, &[&B]).unwrap();
            reopened.execute("INSERT INTO b VALUES(1)", []).unwrap();
            assert!(initialized().lock().unwrap().contains(&key));
        }
    }

    #[test]
    fn nested_writes_share_the_thread_connection() {
        let home = tempfile::tempdir().unwrap();
        let db = ("runtime", "nested.db");
        let outer = open(home.path(), db, &[&A]).unwrap();
        let inner = open(home.path(), db, &[&A]).unwrap();
        assert!(Rc::ptr_eq(&outer, &inner));
        let tx = write(&outer).unwrap();
        tx.execute("INSERT INTO a VALUES(1)", []).unwrap();
        {
            let nested = write(&inner).unwrap();
            nested.execute("INSERT INTO a VALUES(2)", []).unwrap();
            // Dropped without commit: only the nested level rolls back.
        }
        let nested = write(&inner).unwrap();
        nested.execute("INSERT INTO a VALUES(3)", []).unwrap();
        nested.commit().unwrap();
        tx.commit().unwrap();
        let rows: Vec<i64> = outer
            .prepare("SELECT x FROM a ORDER BY x")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(rows, vec![1, 3]);
        assert!(outer.is_autocommit());
    }

    #[test]
    fn modules_install_once_and_reject_other_versions() {
        let home = tempfile::tempdir().unwrap();
        let db = ("runtime", "test.db");
        open(home.path(), db, &[&OUTBOX, &A]).unwrap();
        // A repeated open in this process does not rerun DDL.
        let connection = open(home.path(), db, &[&OUTBOX, &A]).unwrap();
        let modules: u32 = connection
            .query_row("SELECT COUNT(*) FROM schema_modules", [], |r| r.get(0))
            .unwrap();
        assert_eq!(modules, 2);
        let newer = Schema {
            module: "a",
            version: 2,
            ddl: "",
        };
        let other = tempfile::tempdir().unwrap();
        open(other.path(), db, &[&A]).unwrap();
        initialized().lock().unwrap().remove(&(
            crate::database_path(other.path(), db.0, db.1).unwrap(),
            A.module,
        ));
        assert!(matches!(
            open(other.path(), db, &[&newer]),
            Err(SchemaError::Unsupported { found: 1, .. })
        ));
    }
}
