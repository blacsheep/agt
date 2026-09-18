//! Durable, permission-filtered invalidations. Never carries message text or private Run data.
use agt_im::{ConversationId, ImStore, PrincipalId};
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub struct Events {
    home: PathBuf,
    connection: Connection,
    generation: String,
}
#[derive(Debug, Serialize)]
pub struct Change {
    pub conversation: Option<String>,
    pub kind: String,
}
#[derive(Debug, Serialize)]
pub struct Page {
    pub event: &'static str,
    pub cursor: String,
    pub changes: Vec<Change>,
    pub more: bool,
    pub scheduler_online: bool,
}
impl Events {
    pub fn open(home: &Path) -> Result<Self> {
        ImStore::open(agt_storage::database_path(home, "im", "im.db")?)?;
        crate::RunStore::open(home)?;
        crate::ScheduleStore::open(home)?;
        agt_service::BindingStore::open(home)?;
        let mut connection =
            Connection::open(agt_storage::database_path(home, "runtime", "events.db")?)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u32 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > 1 {
            bail!("unsupported event database version");
        }
        tx.execute_batch("CREATE TABLE IF NOT EXISTS feed(seq INTEGER PRIMARY KEY AUTOINCREMENT, source TEXT NOT NULL, source_seq INTEGER NOT NULL, conversation TEXT, kind TEXT NOT NULL, at INTEGER NOT NULL, UNIQUE(source,source_seq));
            CREATE INDEX IF NOT EXISTS feed_age ON feed(at);
            CREATE TABLE IF NOT EXISTS cursors(source TEXT PRIMARY KEY, seq INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS feed_meta(id INTEGER PRIMARY KEY CHECK(id=1), generation TEXT NOT NULL, floor INTEGER NOT NULL);
            PRAGMA user_version=1;")?;
        tx.execute(
            "INSERT OR IGNORE INTO feed_meta VALUES(1,?1,0)",
            [uuid::Uuid::new_v4().to_string()],
        )?;
        let generation = tx.query_row("SELECT generation FROM feed_meta WHERE id=1", [], |r| {
            r.get(0)
        })?;
        tx.commit()?;
        Ok(Self {
            home: home.into(),
            connection,
            generation,
        })
    }
    /// Source cursors and projection rows commit together. Notifications are only hints.
    pub fn project(&mut self) -> Result<bool> {
        let mut more = false;
        for (source, directory, name) in [
            ("im", "im", "im.db"),
            ("run", "runtime", "runs.db"),
            ("schedule", "runtime", "scheduler.db"),
            ("binding", "runtime", "bindings.db"),
        ] {
            let db = Connection::open(agt_storage::database_path(&self.home, directory, name)?)?;
            db.busy_timeout(Duration::from_secs(5))?;
            let tx = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let cursor = tx
                .query_row("SELECT seq FROM cursors WHERE source=?1", [source], |r| {
                    r.get::<_, i64>(0)
                })
                .optional()?
                .unwrap_or(0);
            let rows = {
                let mut stmt = db.prepare("SELECT seq,conversation,kind,at FROM change_outbox WHERE seq>?1 ORDER BY seq LIMIT 1000")?;
                stmt.query_map([cursor], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
            };
            more |= rows.len() == 1000;
            let mut last = cursor;
            for (seq, conversation, kind, at) in rows {
                tx.execute("INSERT OR IGNORE INTO feed(source,source_seq,conversation,kind,at) VALUES(?1,?2,?3,?4,?5)",params![source,seq,conversation,kind,at])?;
                last = seq;
            }
            tx.execute("INSERT INTO cursors VALUES(?1,?2) ON CONFLICT(source) DO UPDATE SET seq=excluded.seq",params![source,last])?;
            tx.commit()?;
            // This projection is the required consumer of the generic invalidation outbox.
            // Work inbox/publication records are independent and never age-pruned here.
            db.execute("DELETE FROM change_outbox WHERE seq<=?1", [last])?;
        }
        let cutoff = crate::schedule_store::now_ms() - 7 * 24 * 60 * 60 * 1000;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Trim a prefix so all cursors at/above floor remain replayable even with clock skew.
        let prefix: Option<i64> =
            tx.query_row("SELECT MIN(seq)-1 FROM feed WHERE at>=?1", [cutoff], |r| {
                r.get(0)
            })?;
        let floor: i64 = match prefix {
            Some(value) => value,
            None => tx.query_row("SELECT COALESCE(MAX(seq),0) FROM feed", [], |r| r.get(0))?,
        };
        tx.execute(
            "UPDATE feed_meta SET floor=MAX(floor,?1) WHERE id=1",
            [floor],
        )?;
        tx.execute("DELETE FROM feed WHERE seq<=?1", [floor])?;
        tx.commit()?;
        Ok(more)
    }
    pub fn page(
        &self,
        principal: &str,
        conversation: Option<&str>,
        after: Option<&str>,
    ) -> Result<Page> {
        let im = ImStore::open(agt_storage::database_path(&self.home, "im", "im.db")?)?;
        let actor = PrincipalId::new(principal);
        if im.get_principal(&actor)?.deleted_at.is_some() {
            bail!("subscription identity was deleted");
        }
        let visible = |id: &str| {
            im.get_member(&ConversationId::new(id), &actor)
                .is_ok_and(|m| m.left_message_seq.is_none())
        };
        if conversation.is_some_and(|id| !visible(id)) {
            bail!("subscription membership revoked");
        }
        // Heartbeats detect a crashed coordinator even when no data changed.
        let scheduler = Connection::open(agt_storage::database_path(
            &self.home,
            "runtime",
            "scheduler.db",
        )?)?;
        scheduler.busy_timeout(Duration::from_secs(5))?;
        let scheduler_online = scheduler.query_row(
            "SELECT EXISTS(SELECT 1 FROM coordinator WHERE expires>?1 AND owner LIKE 'scheduler:%')",
            [crate::schedule_store::now_ms()],
            |r| r.get::<_, bool>(0),
        )?;
        let scope = crate::fingerprint(&serde_json::to_vec(&(principal, conversation))?);
        let prefix = format!("{}:{}:", self.generation, scope);
        let floor: i64 =
            self.connection
                .query_row("SELECT floor FROM feed_meta WHERE id=1", [], |r| r.get(0))?;
        let high: i64 = self.connection.query_row(
            "SELECT MAX(?1,COALESCE(MAX(seq),0)) FROM feed",
            [floor],
            |r| r.get(0),
        )?;
        let parsed = after
            .and_then(|s| s.strip_prefix(&prefix))
            .and_then(|s| s.parse::<i64>().ok());
        let Some(cursor) = parsed.filter(|n| *n >= floor && *n <= high) else {
            return Ok(Page {
                event: if after.is_some() {
                    "resync_required"
                } else {
                    "ready"
                },
                cursor: format!("{prefix}{high}"),
                changes: vec![],
                more: false,
                scheduler_online,
            });
        };
        let mut stmt = self.connection.prepare(
            "SELECT seq,conversation,kind FROM feed WHERE seq>?1 ORDER BY seq LIMIT 200",
        )?;
        let rows = stmt
            .query_map([cursor], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let owner = im.workspace_owner()?.as_ref() == Some(&actor);
        let mut last = cursor;
        let mut changes = Vec::new();
        for (seq, mut id, mut kind) in rows {
            last = seq;
            if let Some(chat) = &id {
                if conversation.is_some_and(|filter| filter != chat) {
                    continue;
                }
                if !visible(chat) {
                    if owner && conversation.is_none() && kind == "work" {
                        id = None;
                        kind = "attention".into();
                    } else {
                        continue;
                    }
                }
            } else if kind == "attention" {
                if !owner || conversation.is_some() {
                    continue;
                }
            } else if kind != "im" && kind != "access" {
                continue;
            }
            if !changes
                .iter()
                .any(|c: &Change| c.conversation == id && c.kind == kind)
            {
                changes.push(Change {
                    conversation: id,
                    kind,
                });
            }
        }
        Ok(Page {
            event: "change",
            cursor: format!("{prefix}{last}"),
            changes,
            more: last < high,
            scheduler_online,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agt_im::{PrincipalInput, PrincipalKind};
    #[test]
    fn heartbeat_detects_coordinator_expiry_without_new_events() {
        let (home, feed, _im, _) = setup();
        let initial = feed.page("a", None, None).unwrap();
        assert!(!initial.scheduler_online);
        let db = Connection::open(home.path().join("runtime/scheduler.db")).unwrap();
        db.execute(
            "INSERT INTO coordinator VALUES(1,'scheduler:test',?1)",
            [crate::schedule_store::now_ms() + 60_000],
        )
        .unwrap();
        let online = feed.page("a", None, Some(&initial.cursor)).unwrap();
        assert!(online.scheduler_online);
        assert!(online.changes.is_empty());
        db.execute("UPDATE coordinator SET expires=0", []).unwrap();
        let offline = feed.page("a", None, Some(&online.cursor)).unwrap();
        assert!(!offline.scheduler_online);
        assert!(offline.changes.is_empty());
    }
    fn setup() -> (tempfile::TempDir, Events, ImStore, String) {
        let home = tempfile::tempdir().unwrap();
        let feed = Events::open(home.path()).unwrap();
        let im = ImStore::open(home.path().join("im/im.db")).unwrap();
        for id in ["a", "b", "outsider"] {
            im.create_principal(PrincipalInput {
                id: PrincipalId::new(id),
                kind: PrincipalKind::User,
                display_name: id.into(),
                metadata: serde_json::json!({}),
            })
            .unwrap();
        }
        let chat = im
            .create_direct(&PrincipalId::new("a"), &PrincipalId::new("b"))
            .unwrap();
        (home, feed, im, chat.id.to_string())
    }
    #[test]
    fn replay_is_idempotent_scoped_and_never_contains_content() {
        let (home, mut feed, _, chat) = setup();
        let cursor = feed.page("a", None, None).unwrap().cursor;
        let other = feed.page("outsider", None, None).unwrap().cursor;
        feed.project().unwrap();
        let page = feed.page("a", None, Some(&cursor)).unwrap();
        assert!(
            page.changes
                .iter()
                .any(|c| c.conversation.as_deref() == Some(&chat))
        );
        assert!(
            !feed
                .page("outsider", None, Some(&other))
                .unwrap()
                .changes
                .iter()
                .any(|c| c.conversation.is_some())
        );
        assert_eq!(
            feed.page("outsider", None, Some(&cursor)).unwrap().event,
            "resync_required"
        );
        assert!(feed.page("outsider", Some(&chat), None).is_err());
        feed.project().unwrap();
        assert!(
            feed.page("a", None, Some(&page.cursor))
                .unwrap()
                .changes
                .is_empty()
        );
        drop(feed);
        let reopened = Events::open(home.path()).unwrap();
        assert_eq!(
            reopened.page("a", None, Some(&cursor)).unwrap().cursor,
            page.cursor
        );
    }
    #[test]
    fn source_rollback_creates_no_event_and_old_cursors_require_resync() {
        let (home, mut feed, _, _) = setup();
        feed.project().unwrap();
        let cursor = feed.page("a", None, None).unwrap().cursor;
        let db = Connection::open(home.path().join("im/im.db")).unwrap();
        db.execute_batch(
            "BEGIN; UPDATE principals SET display_name='rolled back' WHERE id='a'; ROLLBACK;",
        )
        .unwrap();
        feed.project().unwrap();
        assert!(
            feed.page("a", None, Some(&cursor))
                .unwrap()
                .changes
                .is_empty()
        );
        feed.connection
            .execute("UPDATE feed_meta SET floor=9999", [])
            .unwrap();
        assert_eq!(
            feed.page("a", None, Some(&cursor)).unwrap().event,
            "resync_required"
        );
        db.execute("UPDATE principals SET deleted_at=1 WHERE id='a'", [])
            .unwrap();
        assert!(feed.page("a", None, None).is_err());
    }
    #[test]
    fn owner_receives_only_generic_attention_for_private_work() {
        let (home, mut feed, im, chat) = setup();
        im.ensure_workspace_owner(&PrincipalId::new("outsider"))
            .unwrap();
        feed.project().unwrap();
        let cursor = feed.page("outsider", None, None).unwrap().cursor;
        let db = Connection::open(home.path().join("runtime/scheduler.db")).unwrap();
        db.execute(
            "INSERT INTO change_outbox(conversation,kind,at) VALUES(?1,'work',?2)",
            params![chat, crate::schedule_store::now_ms()],
        )
        .unwrap();
        feed.project().unwrap();
        let page = feed.page("outsider", None, Some(&cursor)).unwrap();
        assert_eq!(page.changes.len(), 1);
        assert_eq!(page.changes[0].kind, "attention");
        assert!(page.changes[0].conversation.is_none());
    }
}
