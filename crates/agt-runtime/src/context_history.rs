//! Permission-filtered presentation events; never inserted into model message history.
use agt_im::{ConversationId, PrincipalId};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::path::Path;
pub fn context_reset_history(
    home: &Path,
    conversation: &str,
    actor: &str,
    from: i64,
    through: i64,
) -> Result<Vec<Value>> {
    let im = agt_im::ImStore::open(agt_storage::database_path(home, "im", "im.db")?)?;
    let principal = PrincipalId::new(actor);
    if im.get_principal(&principal)?.deleted_at.is_some() {
        bail!("identity deleted");
    }
    let member = im.get_member(&ConversationId::new(conversation), &principal)?;
    let store = crate::ScheduleStore::open(home)?;
    let mut stmt=store.connection.prepare("SELECT id,through_seq,created_at FROM context_reset_events WHERE conversation=?1 AND through_seq>=?2 AND through_seq<=?3 ORDER BY through_seq,id")?;
    Ok(stmt.query_map(rusqlite::params![conversation,from.max(member.joined_message_seq),through.min(member.left_message_seq.unwrap_or(i64::MAX))],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"through_seq":r.get::<_,i64>(1)?,"created_at":r.get::<_,i64>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)
}
