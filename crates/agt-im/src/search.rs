//! Bounded keyword history retrieval with the same visibility as message_history.
use crate::{ImError, ImStore, Message, MessageId, PrincipalId, Result};
use rusqlite::params;
pub struct MessageSearch<'a> {
    pub query: &'a str,
    pub conversation: Option<&'a str>,
    pub day: Option<&'a str>,
    pub since: i64,
    pub until: i64,
    pub before: i64,
    pub limit: usize,
}
pub struct MessageSearchPage {
    pub messages: Vec<Message>,
    pub next_cursor: Option<i64>,
}
impl ImStore {
    pub fn search_messages(
        &self,
        viewer: &PrincipalId,
        options: MessageSearch<'_>,
    ) -> Result<MessageSearchPage> {
        if !(1..=50).contains(&options.limit)
            || options.before <= 0
            || options.since < 0
            || options.until <= options.since
            || options.query.chars().count() > 256
        {
            return Err(ImError::InvalidInput(
                "invalid history search bounds".into(),
            ));
        }
        if self.get_principal(viewer)?.deleted_at.is_some() {
            return Err(ImError::InvalidInput("identity deleted".into()));
        }
        let candidates = {
            let db = self.lock()?;
            if let Some(day) = options.day {
                let valid = db
                    .query_row("SELECT length(?1)=10 AND date(?1)=?1", [day], |r| {
                        r.get::<_, Option<bool>>(0)
                    })?
                    .unwrap_or(false);
                if !valid {
                    return Err(ImError::InvalidInput("day must be UTC YYYY-MM-DD".into()));
                }
            }
            let mut stmt=db.prepare("SELECT rowid,id FROM messages WHERE rowid<?1 AND deleted_at IS NULL AND created_at>=?2 AND created_at<?3 AND (?4 IS NULL OR conversation_id=?4) AND instr(lower(body),lower(?5))>0 AND (?6 IS NULL OR date(created_at/1000,'unixepoch')=?6) ORDER BY rowid DESC LIMIT 500")?;
            stmt.query_map(
                params![
                    options.before,
                    options.since,
                    options.until,
                    options.conversation,
                    options.query,
                    options.day
                ],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut page = MessageSearchPage {
            messages: Vec::new(),
            next_cursor: None,
        };
        for (position, id) in candidates {
            page.next_cursor = Some(position);
            if let Ok(message) = self.visible_message(&MessageId::new(id), viewer)
                && message.deleted_at.is_none()
            {
                page.messages.push(message);
            }
            if page.messages.len() >= options.limit {
                break;
            }
        }
        Ok(page)
    }
}
