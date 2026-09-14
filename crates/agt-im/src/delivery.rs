use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{ConversationId, ImError, ImStore, MessageId, PrincipalId, Result};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Pending,
    Processing,
    Completed,
    Skipped,
    Failed,
}

impl DeliveryState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Processing => "processing",
            Self::Completed => "completed",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Delivery {
    pub id: i64,
    pub message_id: MessageId,
    pub principal_id: PrincipalId,
    pub state: DeliveryState,
    pub run_id: Option<String>,
    pub error: Option<String>,
}

impl ImStore {
    pub fn delivery_status(&self, message: &MessageId) -> Result<Vec<Delivery>> {
        let connection = self.lock()?;
        let mut statement=connection.prepare("SELECT id,message_id,principal_id,state,run_id,error FROM message_deliveries WHERE message_id=?1 ORDER BY principal_id")?;
        statement
            .query_map([message.as_str()], row)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
    pub fn ack_delivery(&self, id: i64, principal: &PrincipalId) -> Result<()> {
        let connection = self.lock()?;
        let changed=connection.execute("UPDATE message_deliveries SET state='completed' WHERE id=?1 AND principal_id=?2 AND state='pending'
            AND EXISTS(SELECT 1 FROM messages m JOIN conversation_members cm ON cm.conversation_id=m.conversation_id WHERE m.id=message_deliveries.message_id AND cm.principal_id=?2 AND cm.left_message_seq IS NULL)",params![id,principal.as_str()])?;
        if changed != 1 {
            return Err(ImError::PermissionDenied(
                "delivery is unavailable or already processing".into(),
            ));
        }
        Ok(())
    }
    /// Unprocessed deliveries are distinct from the human read cursor.
    pub fn deliveries(
        &self,
        principal: &PrincipalId,
        pending_only: bool,
        limit: u32,
    ) -> Result<Vec<Delivery>> {
        self.deliveries_in(principal, pending_only, limit, None)
    }

    /// Apply the conversation scope before the limit so unrelated queues cannot starve it.
    pub fn deliveries_in(
        &self,
        principal: &PrincipalId,
        pending_only: bool,
        limit: u32,
        conversation: Option<&ConversationId>,
    ) -> Result<Vec<Delivery>> {
        self.deliveries_after(principal, pending_only, limit, conversation, -1)
    }
    pub fn deliveries_after(
        &self,
        principal: &PrincipalId,
        pending_only: bool,
        limit: u32,
        conversation: Option<&ConversationId>,
        after: i64,
    ) -> Result<Vec<Delivery>> {
        if !(1..=200).contains(&limit) {
            return Err(ImError::InvalidInput("limit must be in 1..=200".into()));
        }
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT d.id,d.message_id,d.principal_id,d.state,d.run_id,d.error
             FROM message_deliveries d JOIN messages m ON m.id=d.message_id
             JOIN conversation_members cm ON cm.conversation_id=m.conversation_id AND cm.principal_id=d.principal_id
             WHERE d.principal_id=?1 AND cm.left_message_seq IS NULL AND m.seq>=cm.joined_message_seq
               AND (?2=0 OR d.state IN ('pending','processing'))
               AND (?4 IS NULL OR m.conversation_id=?4)
               AND d.id>?5
             ORDER BY CASE WHEN ?5=-1 AND d.state='pending' THEN 0 WHEN ?5=-1 THEN 1 ELSE 0 END,d.id LIMIT ?3"
        )?;
        let rows = statement.query_map(
            params![
                principal.as_str(),
                pending_only,
                limit,
                conversation.map(ConversationId::as_str),
                after
            ],
            row,
        )?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Terminal results cannot be overwritten by a stale task.
    pub fn update_delivery(
        &self,
        id: i64,
        state: DeliveryState,
        run_id: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let connection = self.lock()?;
        connection.execute(
            "UPDATE message_deliveries SET state=CASE WHEN state='processing' AND ?2='pending' THEN state ELSE ?2 END,run_id=COALESCE(?3,run_id),error=?4
             WHERE id=?1 AND state IN ('pending','processing')",
            params![id,state.as_str(),run_id,error],
        )?;
        Ok(())
    }
}

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Delivery> {
    let state: String = row.get(3)?;
    let state = match state.as_str() {
        "pending" => DeliveryState::Pending,
        "processing" => DeliveryState::Processing,
        "completed" => DeliveryState::Completed,
        "skipped" => DeliveryState::Skipped,
        "failed" => DeliveryState::Failed,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(Delivery {
        id: row.get(0)?,
        message_id: MessageId::new(row.get::<_, String>(1)?),
        principal_id: PrincipalId::new(row.get::<_, String>(2)?),
        state,
        run_id: row.get(4)?,
        error: row.get(5)?,
    })
}
