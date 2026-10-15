//! Identity-owned, source-backed notes plus permission-checked historical retrieval.
//! IM remains the authority: memory never duplicates message bodies or grants membership.
use agt_im::{ImStore, MessageId, PrincipalId};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::path::Path;

mod schemas;
pub(crate) use schemas::schemas;

fn sources(im: &ImStore, owner: &PrincipalId, ids: &Value) -> Result<Vec<agt_im::Message>> {
    let ids = ids.as_array().context("source_messages must be an array")?;
    if ids.is_empty() || ids.len() > 32 {
        bail!("provide 1..32 source messages");
    }
    ids.iter()
        .map(|id| {
            let message = im.visible_message(
                &MessageId::new(id.as_str().context("message id required")?),
                owner,
            )?;
            if message.deleted_at.is_some() {
                bail!("memory source is no longer available");
            }
            Ok(message)
        })
        .collect()
}
fn bounded<'a>(args: &'a Value, key: &str, max: usize) -> Result<&'a str> {
    let value = args[key].as_str().context("text field required")?;
    if value.trim().is_empty() || value.chars().count() > max {
        bail!("{key} must contain 1..{max} characters");
    }
    Ok(value)
}
fn open(home: &Path) -> Result<Connection> {
    let db = Connection::open(agt_storage::database_path(home, "memory", "memory.db")?)?;
    db.busy_timeout(agt_storage::sqlite::BUSY_TIMEOUT)?;
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version > 1 {
        bail!("unsupported memory schema");
    }
    db.execute_batch("CREATE TABLE IF NOT EXISTS notes(id TEXT NOT NULL,owner TEXT NOT NULL,revision INTEGER NOT NULL,topic TEXT NOT NULL,summary TEXT NOT NULL,sources TEXT NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(owner,id)); PRAGMA user_version=1;")?;
    Ok(db)
}
fn note(db: &Connection, owner: &str, id: &str) -> Result<Value> {
    db.query_row("SELECT id,revision,topic,summary,sources,created_at,updated_at FROM notes WHERE owner=?1 AND id=?2",params![owner,id],|r| {
        let raw: String=r.get(4)?;
        Ok(json!({"id":r.get::<_,String>(0)?,"revision":r.get::<_,i64>(1)?,"topic":r.get::<_,String>(2)?,"summary":r.get::<_,String>(3)?,"source_messages":serde_json::from_str::<Value>(&raw).unwrap_or(Value::Null),"created_at":r.get::<_,i64>(5)?,"updated_at":r.get::<_,i64>(6)?,"kind":"note","authority":"agent_summary"}))
    }).optional()?.context("memory not found")
}
pub(crate) fn call(
    home: &Path,
    im: &ImStore,
    owner: &PrincipalId,
    name: &str,
    args: Value,
) -> Result<Value> {
    if !agt_config::RuntimeConfig::load(home)?.memory.enabled {
        bail!("memory is disabled in workspace configuration");
    }
    if im.get_principal(owner)?.deleted_at.is_some() {
        bail!("identity deleted");
    }
    let mut db = open(home)?;
    match name {
        "memory_save" => {
            let id = bounded(&args, "id", 128)?;
            let topic = bounded(&args, "topic", 120)?;
            let summary = bounded(&args, "summary", 4000)?;
            sources(im, owner, &args["source_messages"])?;
            let revision = args["expected_revision"]
                .as_i64()
                .context("expected_revision required")?;
            if revision < 0 {
                bail!("revision must be nonnegative");
            }
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis() as i64;
            let raw = serde_json::to_string(&args["source_messages"])?;
            let changed = if revision == 0 {
                tx.execute(
                    "INSERT OR IGNORE INTO notes VALUES(?1,?2,1,?3,?4,?5,?6,?6)",
                    params![id, owner.as_str(), topic, summary, raw, now],
                )?
            } else {
                tx.execute("UPDATE notes SET revision=revision+1,topic=?3,summary=?4,sources=?5,updated_at=?6 WHERE owner=?1 AND id=?2 AND revision=?7",params![owner.as_str(),id,topic,summary,raw,now,revision])?
            };
            if changed != 1 {
                bail!("stale memory revision; read current note before retrying");
            }
            sources(im, owner, &args["source_messages"])?;
            tx.commit()?;
            note(&db, owner.as_str(), id)
        }
        "memory_read" => {
            let value = note(&db, owner.as_str(), bounded(&args, "id", 128)?)?;
            let messages = sources(im, owner, &value["source_messages"])?;
            Ok(
                json!({"note":value,"sources":messages.into_iter().map(|m|json!({"message_id":m.id,"conversation":m.conversation_id,"sender":m.sender_id,"created_at":m.created_at,"body":m.body.chars().take(4000).collect::<String>(),"truncated":m.body.chars().count()>4000})).collect::<Vec<_>>()}),
            )
        }
        "memory_forget" => {
            let id = bounded(&args, "id", 128)?;
            let revision = args["expected_revision"]
                .as_i64()
                .context("expected_revision required")?;
            if db.execute(
                "DELETE FROM notes WHERE owner=?1 AND id=?2 AND revision=?3",
                params![owner.as_str(), id, revision],
            )? != 1
            {
                bail!("stale memory revision or memory not found");
            }
            Ok(json!({"forgotten":id,"messages_deleted":false}))
        }
        "memory_search" => {
            let query = args["query"].as_str().unwrap_or("");
            if query.chars().count() > 256 {
                bail!("query exceeds 256 characters");
            }
            let limit = args["limit"].as_u64().unwrap_or(20);
            if !(1..=50).contains(&limit) {
                bail!("limit must be 1..50");
            }
            let before = args["before"].as_i64().unwrap_or(i64::MAX);
            if before <= 0 {
                bail!("invalid cursor");
            }
            let since = args["since"].as_i64().unwrap_or(0);
            let until = args["until"].as_i64().unwrap_or(i64::MAX);
            if since < 0 || until <= since {
                bail!("invalid time range");
            }
            let conversation = args["conversation"].as_str();
            let day = args["day"].as_str();
            if let Some(day) = day {
                let valid: bool = db
                    .query_row("SELECT length(?1)=10 AND date(?1)=?1", [day], |r| {
                        r.get::<_, Option<bool>>(0)
                    })?
                    .unwrap_or(false);
                if !valid {
                    bail!("day must be a valid UTC date YYYY-MM-DD");
                }
            }
            let mut items = Vec::new();
            let mut cursor = None;
            // Bounded scan; a non-null cursor may accompany an empty page.
            if args["kind"].as_str() == Some("notes") {
                let mut stmt=db.prepare("SELECT rowid,id FROM notes WHERE owner=?1 AND rowid<?2 AND (instr(lower(topic),lower(?3))>0 OR instr(lower(summary),lower(?3))>0) ORDER BY rowid DESC LIMIT 500")?;
                let rows = stmt.query_map(params![owner.as_str(), before, query], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (position, id) = row?;
                    cursor = Some(position);
                    let value = note(&db, owner.as_str(), &id)?;
                    let Ok(messages) = sources(im, owner, &value["source_messages"]) else {
                        continue;
                    };
                    let matches = messages.iter().any(|m| {
                        m.created_at >= since
                            && m.created_at < until
                            && conversation.is_none_or(|c| c == m.conversation_id.as_str())
                            && day.is_none_or(|d| {
                                db.query_row(
                                    "SELECT date(?1/1000,'unixepoch')=?2",
                                    params![m.created_at, d],
                                    |r| r.get::<_, bool>(0),
                                )
                                .unwrap_or(false)
                            })
                    });
                    if matches {
                        items.push(value);
                    }
                    if items.len() >= limit as usize {
                        break;
                    }
                }
            } else {
                let page = im.search_messages(
                    owner,
                    agt_im::MessageSearch {
                        query,
                        conversation,
                        day,
                        since,
                        until,
                        before,
                        limit: limit as usize,
                    },
                )?;
                cursor = page.next_cursor;
                for m in page.messages {
                    items.push(json!({"kind":"message","message_id":m.id,"conversation":m.conversation_id,"sender":m.sender_id,"created_at":m.created_at,"excerpt":m.body.chars().take(600).collect::<String>(),"truncated":m.body.chars().count()>600}));
                }
            }
            Ok(
                json!({"items":items,"next_cursor":cursor,"time_basis":"UTC; since inclusive, until exclusive","note":"Use message_history for full message context. Notes are Agent-authored summaries, not authoritative business state."}),
            )
        }
        _ => bail!("unknown memory operation"),
    }
}
