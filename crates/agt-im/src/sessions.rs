//! Personal conversation entries and membership-scoped history, separate from group lifecycle.
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};

use crate::store::append_event_tx;
use crate::{ConversationId, ImError, ImStore, Message, MessageId, PrincipalId, Result};

pub(crate) fn migrate_viewed(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch("ALTER TABLE conversation_members ADD COLUMN last_viewed_at INTEGER; PRAGMA user_version=4;")?;
    Ok(())
}

pub(crate) fn migrate(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch("ALTER TABLE conversation_members ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE conversations ADD COLUMN dissolved INTEGER NOT NULL DEFAULT 0;
        CREATE TABLE workspace_owner (singleton INTEGER PRIMARY KEY CHECK(singleton=1), principal TEXT NOT NULL REFERENCES principals(id));
        CREATE TABLE managed_groups (conversation TEXT PRIMARY KEY REFERENCES conversations(id), creator TEXT NOT NULL REFERENCES principals(id));
        CREATE TABLE membership_history (
            conversation TEXT NOT NULL, principal TEXT NOT NULL, message_start INTEGER NOT NULL,
            message_end INTEGER, event_start INTEGER NOT NULL, event_end INTEGER,
            FOREIGN KEY(conversation,principal) REFERENCES conversation_members(conversation_id,principal_id));
        CREATE INDEX membership_visibility ON membership_history(conversation,principal,message_start);")?;
    // Reconstruct intervals from durable membership events rather than granting the
    // overwritten current membership row access to periods spent outside a group.
    let members = tx.prepare("SELECT conversation_id,principal_id,joined_message_seq,left_message_seq FROM conversation_members")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?,r.get::<_, i64>(2)?,r.get::<_, Option<i64>>(3)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (conversation, principal, joined, left) in members {
        let events = tx.prepare("SELECT seq,event_type,payload_json FROM im_events WHERE conversation_id=?1 ORDER BY seq")?
            .query_map([&conversation], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut message_seq = 0;
        let mut start = None;
        let mut seen = false;
        for (seq, kind, payload) in events {
            let payload: Value = serde_json::from_str(&payload)
                .map_err(|e| ImError::InvalidStoredData(e.to_string()))?;
            if kind == "message.created" {
                message_seq = payload["seq"].as_i64().unwrap_or(message_seq);
            }
            if kind == "conversation.created"
                && payload["members"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|id| id.as_str() == Some(&principal)))
            {
                start = Some((0, 0));
                seen = true;
            }
            if payload["principal_id"].as_str() == Some(&principal) {
                if kind == "member.joined" {
                    start = Some(if seen { (message_seq + 1, seq) } else { (0, 0) });
                    seen = true;
                }
                if kind == "member.left"
                    && let Some((ms, es)) = start.take()
                {
                    tx.execute(
                        "INSERT INTO membership_history VALUES(?1,?2,?3,?4,?5,?6)",
                        params![conversation, principal, ms, message_seq, es, seq],
                    )?;
                }
            }
        }
        if let Some((ms, es)) = start {
            tx.execute(
                "INSERT INTO membership_history VALUES(?1,?2,?3,?4,?5,NULL)",
                params![conversation, principal, ms, left, es],
            )?;
        } else if !seen {
            // Incomplete legacy event data: expose only the retained membership range.
            tx.execute("INSERT INTO membership_history VALUES(?1,?2,?3,?4,9223372036854775807,9223372036854775807)",params![conversation,principal,joined,left])?;
        }
    }
    tx.execute_batch("CREATE TRIGGER membership_first AFTER INSERT ON conversation_members BEGIN
        INSERT INTO membership_history VALUES(NEW.conversation_id,NEW.principal_id,0,NEW.left_message_seq,0,NULL);
    END;
    CREATE TRIGGER membership_leave AFTER UPDATE OF left_message_seq ON conversation_members
    WHEN OLD.left_message_seq IS NULL AND NEW.left_message_seq IS NOT NULL BEGIN
        UPDATE membership_history SET message_end=NEW.left_message_seq,
            event_end=(SELECT last_event_seq+1 FROM conversations WHERE id=NEW.conversation_id)
        WHERE conversation=NEW.conversation_id AND principal=NEW.principal_id AND message_end IS NULL;
    END;
    CREATE TRIGGER membership_rejoin AFTER UPDATE OF left_message_seq ON conversation_members
    WHEN OLD.left_message_seq IS NOT NULL AND NEW.left_message_seq IS NULL BEGIN
        INSERT INTO membership_history VALUES(NEW.conversation_id,NEW.principal_id,NEW.joined_message_seq,NULL,
            (SELECT last_event_seq+1 FROM conversations WHERE id=NEW.conversation_id),NULL);
        UPDATE conversation_members SET hidden=0 WHERE conversation_id=NEW.conversation_id AND principal_id=NEW.principal_id;
    END;
    CREATE TRIGGER message_reveals_sessions AFTER INSERT ON messages BEGIN
        UPDATE conversation_members SET hidden=0 WHERE conversation_id=NEW.conversation_id AND left_message_seq IS NULL;
    END;
    PRAGMA user_version=3;")?;
    Ok(())
}

pub(crate) fn ensure_message_visible(
    conn: &Connection,
    message: &Message,
    viewer: &PrincipalId,
) -> Result<()> {
    let visible: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM membership_history WHERE conversation=?1 AND principal=?2 AND ?3>=message_start AND (message_end IS NULL OR ?3<=message_end))",params![message.conversation_id.as_str(),viewer.as_str(),message.seq], |r| r.get(0))?;
    if !visible {
        return Err(ImError::PermissionDenied(
            "message is outside your membership history".into(),
        ));
    }
    Ok(())
}

impl ImStore {
    pub fn workspace_owner(&self) -> Result<Option<PrincipalId>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT principal FROM workspace_owner WHERE singleton=1",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(PrincipalId::new))
    }

    /// Host configuration: first writer wins; subsequent Web launches cannot silently change ownership.
    pub fn ensure_workspace_owner(&self, principal: &PrincipalId) -> Result<PrincipalId> {
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = tx
            .query_row(
                "SELECT principal FROM workspace_owner WHERE singleton=1",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        {
            return Ok(PrincipalId::new(existing));
        }
        let kind: String = tx.query_row(
            "SELECT kind FROM principals WHERE id=?1 AND deleted_at IS NULL",
            [principal.as_str()],
            |r| r.get(0),
        )?;
        if kind != "user" {
            return Err(ImError::InvalidInput(
                "workspace owner must be a user identity".into(),
            ));
        }
        tx.execute(
            "INSERT INTO workspace_owner VALUES(1,?1)",
            [principal.as_str()],
        )?;
        let groups = tx.prepare("SELECT c.id,c.owner_id FROM conversations c JOIN principals p ON p.id=c.owner_id WHERE c.kind='group' AND p.kind='agent' AND c.dissolved=0")?
            .query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        for (group, creator) in groups {
            tx.execute(
                "INSERT INTO managed_groups VALUES(?1,?2)",
                params![group, creator],
            )?;
            tx.execute("UPDATE conversation_members SET role='admin' WHERE conversation_id=?1 AND principal_id=?2",params![group,creator])?;
            tx.execute("INSERT INTO conversation_members(conversation_id,principal_id,role,joined_message_seq) VALUES(?1,?2,'owner',0)
                ON CONFLICT(conversation_id,principal_id) DO UPDATE SET role='owner',left_message_seq=NULL,joined_message_seq=(SELECT last_message_seq+1 FROM conversations WHERE id=?1),hidden=0",params![group,principal.as_str()])?;
            tx.execute(
                "UPDATE conversations SET owner_id=?2 WHERE id=?1",
                params![group, principal.as_str()],
            )?;
            append_event_tx(
                &tx,
                &ConversationId::new(&group),
                Some(principal),
                "group.supervised",
                &json!({"owner":principal,"creator":creator}),
            )?;
        }
        tx.commit()?;
        Ok(principal.clone())
    }

    pub fn set_conversation_hidden(
        &self,
        conversation: &ConversationId,
        principal: &PrincipalId,
        hidden: bool,
    ) -> Result<()> {
        let changed = self.lock()?.execute("UPDATE conversation_members SET hidden=?3 WHERE conversation_id=?1 AND principal_id=?2",params![conversation.as_str(),principal.as_str(),hidden])?;
        if changed == 0 {
            return Err(ImError::NotMember {
                principal: principal.clone(),
                conversation: conversation.clone(),
            });
        }
        Ok(())
    }

    /// An explicit UI visit only; ordinary history reads and polling never touch this.
    pub fn mark_conversation_viewed(
        &self,
        conversation: &ConversationId,
        principal: &PrincipalId,
    ) -> Result<i64> {
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let viewed: Option<i64> = tx.query_row(
            "UPDATE conversation_members SET last_viewed_at=MAX(?3, COALESCE((SELECT MAX(last_viewed_at)+1 FROM conversation_members WHERE principal_id=?2),0))
             WHERE conversation_id=?1 AND principal_id=?2 RETURNING last_viewed_at",
            params![conversation.as_str(), principal.as_str(), now], |r| r.get(0)).optional()?;
        let viewed = viewed.ok_or_else(|| ImError::NotMember {
            principal: principal.clone(),
            conversation: conversation.clone(),
        })?;
        tx.commit()?;
        Ok(viewed)
    }

    pub fn visible_message(&self, id: &MessageId, viewer: &PrincipalId) -> Result<Message> {
        let message = self.get_message(id)?;
        let connection = self.lock()?;
        ensure_message_visible(&connection, &message, viewer)?;
        Ok(message)
    }

    /// Dissolution is terminal; personal hiding never calls this operation.
    pub fn dissolve_group(&self, conversation: &ConversationId, actor: &PrincipalId) -> Result<()> {
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let allowed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM conversations c JOIN conversation_members m ON m.conversation_id=c.id WHERE c.id=?1 AND c.kind='group' AND c.owner_id=?2 AND m.principal_id=?2 AND m.left_message_seq IS NULL)",params![conversation.as_str(),actor.as_str()], |r|r.get(0))?;
        if !allowed {
            return Err(ImError::PermissionDenied(
                "only the group owner can dissolve the group".into(),
            ));
        }
        let changed = tx.execute(
            "UPDATE conversations SET state='deleted',dissolved=1 WHERE id=?1 AND dissolved=0",
            [conversation.as_str()],
        )?;
        if changed > 0 {
            append_event_tx(
                &tx,
                conversation,
                Some(actor),
                "group.dissolved",
                &json!({}),
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
