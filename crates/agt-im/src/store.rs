use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::{ImError, Result};
use crate::model::{
    Conversation, ConversationId, ConversationKind, ConversationMember, ConversationState,
    ConversationSummary, EventId, EventPage, ImEvent, MemberRole, Message, MessageId,
    MessageListOptions, MessagePage, Principal, PrincipalId, PrincipalInput, PrincipalKind,
    SendMessageOutcome, SendMessageRequest,
};

const MAX_PAGE_SIZE: usize = 200;

/// Transactional generic IM storage. Agent execution is intentionally outside
/// this crate; agents are just one possible PrincipalKind.
#[derive(Clone)]
pub struct ImStore {
    connection: Arc<Mutex<Connection>>,
    path: Option<PathBuf>,
}

impl ImStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|source| ImError::io(parent.display().to_string(), source))?;
        }
        let connection = Connection::open(&path)?;
        configure_connection(&connection, true)?;
        initialize_schema(&connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            path: Some(path),
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        configure_connection(&connection, false)?;
        initialize_schema(&connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            path: None,
        })
    }

    pub fn database_path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn create_principal(&self, input: PrincipalInput) -> Result<Principal> {
        validate_id(&input.id, "principal id")?;
        validate_display_name(&input.display_name)?;
        let metadata = serde_json::to_string(&input.metadata)?;
        let now = now_millis();
        let connection = self.lock()?;
        if connection
            .query_row(
                "SELECT 1 FROM principals WHERE id = ?1",
                params![input.id.as_str()],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            return Err(ImError::AlreadyExists(format!("principal {}", input.id)));
        }
        connection.execute(
            "INSERT INTO principals
                (id, kind, display_name, metadata_json, created_at, updated_at, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, NULL)",
            params![
                input.id.as_str(),
                principal_kind_to_str(&input.kind),
                input.display_name,
                metadata,
                now,
            ],
        )?;
        Ok(Principal {
            id: input.id,
            kind: input.kind,
            display_name: input.display_name,
            metadata: input.metadata,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        })
    }

    pub fn get_principal(&self, id: &PrincipalId) -> Result<Principal> {
        let connection = self.lock()?;
        get_principal_from_connection(&connection, id)
    }

    pub fn list_principals(&self) -> Result<Vec<Principal>> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, kind, display_name, metadata_json, created_at, updated_at, deleted_at
             FROM principals WHERE deleted_at IS NULL ORDER BY id ASC",
        )?;
        let rows = statement.query_map([], principal_from_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(ImError::from)
    }

    /// Historical directory entries retain names after identity deletion.
    pub fn list_principals_including_deleted(&self) -> Result<Vec<Principal>> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, kind, display_name, metadata_json, created_at, updated_at, deleted_at
             FROM principals ORDER BY id ASC",
        )?;
        statement
            .query_map([], principal_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(ImError::from)
    }

    /// Retire Agent identities without removing memberships or historical messages.
    pub fn delete_agent_principals(&self, principals: &[PrincipalId]) -> Result<()> {
        let mut connection = self.lock()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for id in principals {
            tx.execute(
                "UPDATE principals SET deleted_at=?2, updated_at=?2
                WHERE id=?1 AND kind='agent' AND deleted_at IS NULL",
                params![id.as_str(), now_millis()],
            )?;
            tx.execute(
                "UPDATE message_deliveries SET state='skipped', error='Agent deleted'
                WHERE principal_id=?1 AND state IN ('pending','processing')
                AND EXISTS(SELECT 1 FROM principals WHERE id=?1 AND deleted_at IS NOT NULL)",
                [id.as_str()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn create_direct(&self, first: &PrincipalId, second: &PrincipalId) -> Result<Conversation> {
        if first == second {
            return Err(ImError::InvalidInput(
                "a direct conversation needs two different principals".to_string(),
            ));
        }
        let (principal_a, principal_b) = canonical_pair(first, second);
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_principal_tx(&transaction, &principal_a)?;
        ensure_principal_tx(&transaction, &principal_b)?;
        if let Some(conversation_id) = transaction
            .query_row(
                "SELECT conversation_id FROM direct_pairs
                 WHERE principal_a = ?1 AND principal_b = ?2",
                params![principal_a.as_str(), principal_b.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            transaction.commit()?;
            drop(connection);
            return self.get_conversation(&ConversationId::new(conversation_id));
        }

        let conversation_id = ConversationId::generate();
        let now = now_millis();
        transaction.execute(
            "INSERT INTO conversations
                (id, kind, state, title, owner_id, last_message_seq,
                 last_event_seq, created_at, updated_at)
             VALUES (?1, 'direct', 'active', NULL, NULL, 0, 0, ?2, ?2)",
            params![conversation_id.as_str(), now],
        )?;
        transaction.execute(
            "INSERT INTO direct_pairs (conversation_id, principal_a, principal_b)
             VALUES (?1, ?2, ?3)",
            params![
                conversation_id.as_str(),
                principal_a.as_str(),
                principal_b.as_str()
            ],
        )?;
        transaction.execute(
            "INSERT INTO conversation_members
                (conversation_id, principal_id, role, joined_message_seq,
                 left_message_seq, last_read_seq)
             VALUES (?1, ?2, 'member', 0, NULL, 0), (?1, ?3, 'member', 0, NULL, 0)",
            params![
                conversation_id.as_str(),
                principal_a.as_str(),
                principal_b.as_str()
            ],
        )?;
        append_event_tx(
            &transaction,
            &conversation_id,
            None,
            "conversation.created",
            &json!({
                "kind": "direct",
                "members": [principal_a.as_str(), principal_b.as_str()],
            }),
        )?;
        transaction.commit()?;
        drop(connection);
        self.get_conversation(&conversation_id)
    }

    pub fn create_group(
        &self,
        owner_id: &PrincipalId,
        title: Option<String>,
        members: Vec<PrincipalId>,
    ) -> Result<Conversation> {
        ensure_non_empty_title(title.as_deref())?;
        let mut unique_members = BTreeSet::new();
        unique_members.insert(owner_id.clone());
        unique_members.extend(members);
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for principal_id in &unique_members {
            ensure_principal_tx(&transaction, principal_id)?;
        }
        let creator = owner_id;
        let kind: String = transaction.query_row(
            "SELECT kind FROM principals WHERE id=?1",
            [creator.as_str()],
            |r| r.get(0),
        )?;
        let responsible = if kind == "agent" {
            Some(PrincipalId::new(
                transaction
                    .query_row(
                        "SELECT principal FROM workspace_owner WHERE singleton=1",
                        [],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?
                    .ok_or_else(|| {
                        ImError::InvalidInput(
                            "configure the workspace owner first: agt im owner ensure <user>"
                                .into(),
                        )
                    })?,
            ))
        } else {
            None
        };
        let owner_id = responsible.as_ref().unwrap_or(creator);
        unique_members.insert(owner_id.clone());
        let conversation_id = ConversationId::generate();
        let now = now_millis();
        transaction.execute(
            "INSERT INTO conversations
                (id, kind, state, title, owner_id, last_message_seq,
                 last_event_seq, created_at, updated_at)
             VALUES (?1, 'group', 'active', ?2, ?3, 0, 0, ?4, ?4)",
            params![conversation_id.as_str(), title, owner_id.as_str(), now],
        )?;
        let member_values = unique_members
            .iter()
            .map(|id| id.as_str().to_string())
            .collect::<Vec<_>>();
        append_event_tx(
            &transaction,
            &conversation_id,
            Some(owner_id),
            "conversation.created",
            &json!({
                "kind": "group",
                "title": title,
                "members": member_values,
            }),
        )?;
        for principal_id in unique_members {
            let role = if &principal_id == owner_id {
                MemberRole::Owner
            } else if &principal_id == creator {
                MemberRole::Admin
            } else {
                MemberRole::Member
            };
            transaction.execute(
                "INSERT INTO conversation_members
                    (conversation_id, principal_id, role, joined_message_seq,
                     left_message_seq, last_read_seq)
                 VALUES (?1, ?2, ?3, 0, NULL, 0)",
                params![
                    conversation_id.as_str(),
                    principal_id.as_str(),
                    member_role_to_str(&role),
                ],
            )?;
        }
        if responsible.is_some() {
            transaction.execute(
                "INSERT INTO managed_groups VALUES(?1,?2)",
                params![conversation_id.as_str(), creator.as_str()],
            )?;
        }
        transaction.commit()?;
        drop(connection);
        self.get_conversation(&conversation_id)
    }

    pub fn get_conversation(&self, id: &ConversationId) -> Result<Conversation> {
        let connection = self.lock()?;
        get_conversation_from_connection(&connection, id)
    }

    pub fn list_conversations(
        &self,
        principal_id: &PrincipalId,
    ) -> Result<Vec<ConversationSummary>> {
        self.list_conversation_entries(principal_id, false, false)
    }

    pub fn list_conversation_entries(
        &self,
        principal_id: &PrincipalId,
        include_hidden: bool,
        groups_only: bool,
    ) -> Result<Vec<ConversationSummary>> {
        ensure_id(principal_id, "principal id")?;
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT c.id, c.kind, c.state, c.title, c.owner_id,
                    c.last_message_seq, c.created_at, c.updated_at,
                    (SELECT COUNT(*) FROM conversation_members cm2
                     WHERE cm2.conversation_id = c.id
                       AND cm2.left_message_seq IS NULL),
                    (SELECT COUNT(*) FROM messages msg
                     WHERE msg.conversation_id = c.id
                       AND msg.seq > cm.last_read_seq
                       AND msg.seq >= cm.joined_message_seq
                       AND msg.deleted_at IS NULL
                       AND msg.sender_id != cm.principal_id
                       AND cm.left_message_seq IS NULL), cm.left_message_seq,cm.hidden,c.dissolved,cm.last_viewed_at,
                    (SELECT json_object('sender_id', m.sender_id, 'body', substr(m.body, 1, 120), 'created_at', m.created_at)
                     FROM messages m
                     WHERE m.conversation_id = c.id AND m.deleted_at IS NULL
                       AND EXISTS(SELECT 1 FROM membership_history h
                                  WHERE h.conversation = c.id AND h.principal = cm.principal_id
                                    AND m.seq >= h.message_start
                                    AND (h.message_end IS NULL OR m.seq <= h.message_end))
                     ORDER BY m.seq DESC LIMIT 1)
             FROM conversations c
             JOIN conversation_members cm ON cm.conversation_id = c.id
             WHERE cm.principal_id = ?1 AND (?2 OR cm.hidden=0)
               AND (c.state != 'deleted' OR c.dissolved=1)
               AND (NOT ?3 OR (c.kind='group' AND cm.left_message_seq IS NULL AND c.dissolved=0))
             ORDER BY c.updated_at DESC, c.id ASC",
        )?;
        let rows = statement.query_map(
            params![principal_id.as_str(), include_hidden, groups_only],
            |row| {
                Ok(ConversationSummary {
                    conversation: conversation_from_row(row)?,
                    member_count: row.get(8)?,
                    unread_count: row.get(9)?,
                    left_message_seq: row.get(10)?,
                    hidden: row.get(11)?,
                    dissolved: row.get(12)?,
                    last_viewed_at: row.get(13)?,
                    last_message: row
                        .get::<_, Option<String>>(14)?
                        .and_then(|json| serde_json::from_str(&json).ok()),
                })
            },
        )?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(ImError::from)
    }

    pub fn list_members(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<Vec<ConversationMember>> {
        let connection = self.lock()?;
        ensure_conversation_from_connection(&connection, conversation_id)?;
        let mut statement = connection.prepare(
            "SELECT conversation_id, principal_id, role, joined_message_seq,
                    left_message_seq, last_read_seq
             FROM conversation_members
             WHERE conversation_id = ?1
             ORDER BY joined_message_seq ASC, principal_id ASC",
        )?;
        let rows = statement.query_map(params![conversation_id.as_str()], member_from_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(ImError::from)
    }

    pub fn add_member(
        &self,
        conversation_id: &ConversationId,
        actor_id: &PrincipalId,
        principal_id: &PrincipalId,
    ) -> Result<ConversationMember> {
        if actor_id == principal_id {
            return Err(ImError::Conflict(
                "principal is already a member candidate".to_string(),
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (kind, state, last_message_seq) = conversation_state_tx(&transaction, conversation_id)?;
        if kind != ConversationKind::Group {
            return Err(ImError::InvalidInput(
                "members can only be changed on group conversations".to_string(),
            ));
        }
        ensure_active_state(&state)?;
        ensure_principal_tx(&transaction, principal_id)?;
        ensure_role_can_manage_members(&transaction, conversation_id, actor_id)?;

        let current: Option<Option<i64>> = transaction
            .query_row(
                "SELECT left_message_seq FROM conversation_members
                 WHERE conversation_id = ?1 AND principal_id = ?2",
                params![conversation_id.as_str(), principal_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        match current {
            Some(None) => {
                return Err(ImError::Conflict(format!(
                    "principal {} is already a member",
                    principal_id
                )));
            }
            Some(Some(_)) => {
                transaction.execute(
                    "UPDATE conversation_members
                     SET role = 'member', joined_message_seq = ?3,
                         left_message_seq = NULL, last_read_seq = ?4
                     WHERE conversation_id = ?1 AND principal_id = ?2",
                    params![
                        conversation_id.as_str(),
                        principal_id.as_str(),
                        last_message_seq + 1,
                        last_message_seq
                    ],
                )?;
            }
            None => {
                transaction.execute(
                    "INSERT INTO conversation_members
                        (conversation_id, principal_id, role, joined_message_seq,
                         left_message_seq, last_read_seq)
                     VALUES (?1, ?2, 'member', ?3, NULL, ?4)",
                    params![
                        conversation_id.as_str(),
                        principal_id.as_str(),
                        last_message_seq + 1,
                        last_message_seq
                    ],
                )?;
            }
        }
        append_event_tx(
            &transaction,
            conversation_id,
            Some(actor_id),
            "member.joined",
            &json!({ "principal_id": principal_id.as_str() }),
        )?;
        transaction.commit()?;
        drop(connection);
        self.get_member(conversation_id, principal_id)
    }

    /// Only the group owner may change roles, and only between admin and
    /// member; ownership itself cannot be transferred.
    pub fn set_member_role(
        &self,
        conversation_id: &ConversationId,
        actor_id: &PrincipalId,
        principal_id: &PrincipalId,
        role: MemberRole,
    ) -> Result<ConversationMember> {
        if role == MemberRole::Owner {
            return Err(ImError::InvalidInput(
                "role must be admin or member; ownership cannot be transferred".to_string(),
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (kind, state, _) = conversation_state_tx(&transaction, conversation_id)?;
        if kind != ConversationKind::Group {
            return Err(ImError::InvalidInput(
                "roles only exist in group conversations".to_string(),
            ));
        }
        ensure_active_state(&state)?;
        if member_role_tx(&transaction, conversation_id, actor_id)? != MemberRole::Owner {
            return Err(ImError::PermissionDenied(
                "only the group owner can change member roles".to_string(),
            ));
        }
        let current = member_role_tx(&transaction, conversation_id, principal_id)?;
        if current == MemberRole::Owner {
            return Err(ImError::PermissionDenied(
                "the group owner's role cannot be changed".to_string(),
            ));
        }
        if current != role {
            transaction.execute(
                "UPDATE conversation_members SET role = ?3
                 WHERE conversation_id = ?1 AND principal_id = ?2 AND left_message_seq IS NULL",
                params![
                    conversation_id.as_str(),
                    principal_id.as_str(),
                    member_role_to_str(&role)
                ],
            )?;
            append_event_tx(
                &transaction,
                conversation_id,
                Some(actor_id),
                "member.role_changed",
                &json!({ "principal_id": principal_id.as_str(), "role": member_role_to_str(&role) }),
            )?;
        }
        transaction.commit()?;
        drop(connection);
        self.get_member(conversation_id, principal_id)
    }

    pub fn remove_member(
        &self,
        conversation_id: &ConversationId,
        actor_id: &PrincipalId,
        principal_id: &PrincipalId,
    ) -> Result<ConversationMember> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (kind, state, last_message_seq) = conversation_state_tx(&transaction, conversation_id)?;
        if kind != ConversationKind::Group {
            return Err(ImError::InvalidInput(
                "members can only be changed on group conversations".to_string(),
            ));
        }
        ensure_active_state(&state)?;
        let actor_role = member_role_tx(&transaction, conversation_id, actor_id)?;
        let target_role = member_role_tx(&transaction, conversation_id, principal_id)?;
        if target_role == MemberRole::Owner {
            return Err(ImError::PermissionDenied(
                "the group owner cannot leave or be removed; dissolve the group instead"
                    .to_string(),
            ));
        }
        if actor_id != principal_id && !matches!(actor_role, MemberRole::Owner | MemberRole::Admin)
        {
            return Err(ImError::PermissionDenied(
                "only an owner or admin can remove another member".to_string(),
            ));
        }

        transaction.execute(
            "UPDATE conversation_members
             SET left_message_seq = ?3
             WHERE conversation_id = ?1 AND principal_id = ?2
               AND left_message_seq IS NULL",
            params![
                conversation_id.as_str(),
                principal_id.as_str(),
                last_message_seq
            ],
        )?;
        if transaction.changes() == 0 {
            return Err(ImError::NotMember {
                principal: principal_id.clone(),
                conversation: conversation_id.clone(),
            });
        }
        append_event_tx(
            &transaction,
            conversation_id,
            Some(actor_id),
            "member.left",
            &json!({ "principal_id": principal_id.as_str() }),
        )?;
        transaction.commit()?;
        drop(connection);
        self.get_member(conversation_id, principal_id)
    }

    pub fn leave_conversation(
        &self,
        conversation_id: &ConversationId,
        principal_id: &PrincipalId,
    ) -> Result<ConversationMember> {
        self.remove_member(conversation_id, principal_id, principal_id)
    }

    pub fn get_member(
        &self,
        conversation_id: &ConversationId,
        principal_id: &PrincipalId,
    ) -> Result<ConversationMember> {
        let connection = self.lock()?;
        member_from_connection(&connection, conversation_id, principal_id)
    }

    pub fn send_message(&self, request: SendMessageRequest) -> Result<SendMessageOutcome> {
        validate_id(&request.sender_id, "sender id")?;
        if request.body.trim().is_empty() {
            return Err(ImError::InvalidInput(
                "message body cannot be empty".to_string(),
            ));
        }
        if request
            .client_message_id
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(ImError::InvalidInput(
                "client_message_id cannot be empty".to_string(),
            ));
        }
        if request
            .idempotency_key
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(ImError::InvalidInput(
                "idempotency_key cannot be empty".to_string(),
            ));
        }

        let conversation_id = request.conversation_id.clone();
        let request_hash = request_hash(&request)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (_, state, _) = conversation_state_tx(&transaction, &conversation_id)?;
        ensure_active_state(&state)?;
        ensure_principal_tx(&transaction, &request.sender_id)?;
        ensure_active_member_tx(&transaction, &conversation_id, &request.sender_id)?;

        let scope = format!("conversation:{conversation_id}:send");
        if let Some(idempotency_key) = request.idempotency_key.as_deref()
            && let Some((stored_hash, message_id)) = transaction
                .query_row(
                    "SELECT request_hash, response_message_id
                     FROM idempotency_records
                     WHERE actor_id = ?1 AND scope = ?2 AND request_key = ?3",
                    params![request.sender_id.as_str(), scope, idempotency_key],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?
        {
            if stored_hash != request_hash {
                return Err(ImError::IdempotencyConflict);
            }
            let message = get_message_tx(&transaction, &MessageId::new(message_id))?;
            return Ok(SendMessageOutcome {
                message,
                duplicate: true,
            });
        }

        let metadata_json = serde_json::to_string(&request.metadata)?;
        if let Some(client_message_id) = request.client_message_id.as_deref()
            && let Some(existing) = transaction
                .query_row(
                    "SELECT id, body, metadata_json, reply_to_id
                     FROM messages
                     WHERE conversation_id = ?1 AND sender_id = ?2
                       AND client_message_id = ?3",
                    params![
                        conversation_id.as_str(),
                        request.sender_id.as_str(),
                        client_message_id,
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    },
                )
                .optional()?
        {
            let (message_id, body, stored_metadata, reply_to_id) = existing;
            if body != request.body
                || stored_metadata != metadata_json
                || reply_to_id != request.reply_to_id.as_ref().map(ToString::to_string)
            {
                return Err(ImError::Conflict(format!(
                    "client_message_id {client_message_id} was reused with a different message"
                )));
            }
            let message = get_message_tx(&transaction, &MessageId::new(message_id))?;
            return Ok(SendMessageOutcome {
                message,
                duplicate: true,
            });
        }

        if let Some(reply_to_id) = &request.reply_to_id {
            let reply_conversation: Option<String> = transaction
                .query_row(
                    "SELECT conversation_id FROM messages WHERE id = ?1",
                    params![reply_to_id.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            let reply = get_message_tx(&transaction, reply_to_id)?;
            crate::sessions::ensure_message_visible(&transaction, &reply, &request.sender_id)?;
            match reply_conversation {
                Some(value) if value == conversation_id.as_str() => {}
                Some(_) => {
                    return Err(ImError::InvalidInput(
                        "reply_to_id belongs to another conversation".to_string(),
                    ));
                }
                None => return Err(ImError::MessageNotFound(reply_to_id.clone())),
            }
        }

        let now = now_millis();
        transaction.execute(
            "UPDATE conversations
             SET last_message_seq = last_message_seq + 1, updated_at = ?2
             WHERE id = ?1",
            params![conversation_id.as_str(), now],
        )?;
        let seq: i64 = transaction.query_row(
            "SELECT last_message_seq FROM conversations WHERE id = ?1",
            params![conversation_id.as_str()],
            |row| row.get(0),
        )?;
        let message = Message {
            id: MessageId::generate(),
            conversation_id: conversation_id.clone(),
            seq,
            sender_id: request.sender_id.clone(),
            body: request.body,
            client_message_id: request.client_message_id,
            reply_to_id: request.reply_to_id,
            metadata: request.metadata,
            created_at: now,
            deleted_at: None,
        };
        transaction.execute(
            "INSERT INTO messages
                (id, conversation_id, seq, sender_id, body, client_message_id,
                 reply_to_id, metadata_json, created_at, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)",
            params![
                message.id.as_str(),
                message.conversation_id.as_str(),
                message.seq,
                message.sender_id.as_str(),
                message.body,
                message.client_message_id,
                message.reply_to_id.as_ref().map(ToString::to_string),
                metadata_json,
                message.created_at,
            ],
        )?;
        append_event_tx(
            &transaction,
            &conversation_id,
            Some(&message.sender_id),
            "message.created",
            &serde_json::to_value(&message)?,
        )?;
        transaction.execute(
            "INSERT INTO message_deliveries (message_id,principal_id)
             SELECT ?1, principal_id FROM conversation_members
             WHERE conversation_id = ?2 AND left_message_seq IS NULL AND principal_id != ?3
               AND principal_id IN (SELECT id FROM principals WHERE deleted_at IS NULL)",
            params![
                message.id.as_str(),
                conversation_id.as_str(),
                message.sender_id.as_str()
            ],
        )?;

        if let Some(idempotency_key) = request.idempotency_key.as_deref() {
            transaction.execute(
                "INSERT INTO idempotency_records
                    (actor_id, scope, request_key, request_hash, response_message_id,
                     created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    request.sender_id.as_str(),
                    scope,
                    idempotency_key,
                    request_hash,
                    message.id.as_str(),
                    now,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(SendMessageOutcome {
            message,
            duplicate: false,
        })
    }

    pub fn list_messages(
        &self,
        conversation_id: &ConversationId,
        viewer_id: &PrincipalId,
        options: MessageListOptions,
    ) -> Result<MessagePage> {
        if options.before.is_some() && options.after.is_some() {
            return Err(ImError::InvalidInput(
                "before and after cursors cannot be used together".to_string(),
            ));
        }
        let limit = checked_limit(options.limit)?;
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT 1 FROM conversation_members WHERE conversation_id=?1 AND principal_id=?2",
                params![conversation_id.as_str(), viewer_id.as_str()],
                |_| Ok(()),
            )
            .optional()?
            .ok_or_else(|| ImError::NotMember {
                principal: viewer_id.clone(),
                conversation: conversation_id.clone(),
            })?;

        let before_seq = options
            .before
            .as_deref()
            .map(|cursor| decode_message_cursor(cursor, conversation_id))
            .transpose()?;
        let after_seq = options
            .after
            .as_deref()
            .map(|cursor| decode_message_cursor(cursor, conversation_id))
            .transpose()?;

        let (sql, cursor, descending) = if let Some(seq) = before_seq {
            (
                "SELECT id, conversation_id, seq, sender_id, body, client_message_id,
                        reply_to_id, metadata_json, created_at, deleted_at
                 FROM messages WHERE conversation_id = ?1 AND seq < ?2
                ORDER BY seq DESC LIMIT ?3",
                Some(seq),
                true,
            )
        } else if let Some(seq) = after_seq {
            (
                "SELECT id, conversation_id, seq, sender_id, body, client_message_id,
                        reply_to_id, metadata_json, created_at, deleted_at
                 FROM messages WHERE conversation_id = ?1 AND seq > ?2
                ORDER BY seq ASC LIMIT ?3",
                Some(seq),
                false,
            )
        } else {
            (
                "SELECT id, conversation_id, seq, sender_id, body, client_message_id,
                        reply_to_id, metadata_json, created_at, deleted_at
                 FROM messages WHERE conversation_id = ?1
                ORDER BY seq DESC LIMIT ?2",
                None,
                true,
            )
        };

        let visibility = "EXISTS(SELECT 1 FROM membership_history h WHERE h.conversation=messages.conversation_id AND h.principal=?4 AND messages.seq>=h.message_start AND (h.message_end IS NULL OR messages.seq<=h.message_end))";
        let sql = sql.replace("ORDER BY", &format!("AND {visibility} ORDER BY"));
        let mut statement = connection.prepare(&sql)?;
        statement.raw_bind_parameter(4, viewer_id.as_str())?;
        statement.raw_bind_parameter(1, conversation_id.as_str())?;
        if let Some(seq) = cursor {
            statement.raw_bind_parameter(2, seq)?;
            statement.raw_bind_parameter(3, (limit + 1) as i64)?;
        } else {
            statement.raw_bind_parameter(2, (limit + 1) as i64)?;
        }
        let mut messages = statement
            .raw_query()
            .mapped(message_from_row)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let has_more = messages.len() > limit;
        if has_more {
            messages.truncate(limit);
        }
        if descending {
            messages.reverse();
        }
        let next_cursor = if has_more && !messages.is_empty() {
            let continuation_seq = if after_seq.is_some() {
                messages[messages.len() - 1].seq
            } else {
                messages[0].seq
            };
            Some(encode_message_cursor(conversation_id, continuation_seq))
        } else {
            None
        };
        Ok(MessagePage {
            messages,
            next_cursor,
        })
    }

    pub fn list_events(
        &self,
        conversation_id: &ConversationId,
        viewer_id: &PrincipalId,
        after_seq: Option<i64>,
        limit: usize,
    ) -> Result<EventPage> {
        let limit = checked_limit(limit)?;
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT 1 FROM conversation_members WHERE conversation_id=?1 AND principal_id=?2",
                params![conversation_id.as_str(), viewer_id.as_str()],
                |_| Ok(()),
            )
            .optional()?
            .ok_or_else(|| ImError::NotMember {
                principal: viewer_id.clone(),
                conversation: conversation_id.clone(),
            })?;
        let after_seq = after_seq.unwrap_or(0);
        if after_seq < 0 {
            return Err(ImError::InvalidInput(
                "after_seq cannot be negative".to_string(),
            ));
        }
        let mut statement = connection.prepare(
            "SELECT id, conversation_id, seq, event_type, actor_id,
                    payload_json, created_at
             FROM im_events
             WHERE conversation_id = ?1 AND seq > ?2
             AND EXISTS(SELECT 1 FROM membership_history h WHERE h.conversation=im_events.conversation_id AND h.principal=?4 AND im_events.seq>=h.event_start AND (h.event_end IS NULL OR im_events.seq<=h.event_end))
             ORDER BY seq ASC LIMIT ?3",
        )?;
        let mut events = statement
            .query_map(
                params![
                    conversation_id.as_str(),
                    after_seq,
                    (limit + 1) as i64,
                    viewer_id.as_str()
                ],
                event_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let has_more = events.len() > limit;
        if has_more {
            events.truncate(limit);
        }
        Ok(EventPage {
            next_cursor: if has_more {
                events.last().map(|event| event.seq)
            } else {
                None
            },
            events,
        })
    }

    pub fn mark_read(
        &self,
        conversation_id: &ConversationId,
        principal_id: &PrincipalId,
        through_seq: i64,
    ) -> Result<ConversationMember> {
        if through_seq < 0 {
            return Err(ImError::InvalidInput(
                "through_seq cannot be negative".to_string(),
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (_, state, last_message_seq) = conversation_state_tx(&transaction, conversation_id)?;
        ensure_active_state(&state)?;
        let old_cursor = active_member_cursor_tx(&transaction, conversation_id, principal_id)?;
        let target = through_seq.min(last_message_seq);
        if target > old_cursor {
            transaction.execute(
                "UPDATE conversation_members SET last_read_seq = ?3
                 WHERE conversation_id = ?1 AND principal_id = ?2
                   AND left_message_seq IS NULL",
                params![conversation_id.as_str(), principal_id.as_str(), target],
            )?;
            append_event_tx(
                &transaction,
                conversation_id,
                Some(principal_id),
                "member.read",
                &json!({ "principal_id": principal_id.as_str(), "through_seq": target }),
            )?;
        }
        transaction.commit()?;
        drop(connection);
        self.get_member(conversation_id, principal_id)
    }

    pub fn archive_conversation(
        &self,
        conversation_id: &ConversationId,
        actor_id: &PrincipalId,
    ) -> Result<Conversation> {
        self.change_conversation_state(conversation_id, actor_id, ConversationState::Archived)
    }

    pub fn delete_conversation(
        &self,
        conversation_id: &ConversationId,
        actor_id: &PrincipalId,
    ) -> Result<Conversation> {
        self.change_conversation_state(conversation_id, actor_id, ConversationState::Deleted)
    }

    /// Restore an archived or soft-deleted conversation to the active state.
    ///
    /// Direct conversations remain canonical for a principal pair, so callers
    /// need an explicit restore operation after archiving or deleting one.
    pub fn reactivate_conversation(
        &self,
        conversation_id: &ConversationId,
        actor_id: &PrincipalId,
    ) -> Result<Conversation> {
        self.change_conversation_state(conversation_id, actor_id, ConversationState::Active)
    }

    pub fn delete_message(
        &self,
        message_id: &MessageId,
        actor_id: &PrincipalId,
    ) -> Result<Message> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let message = get_message_tx(&transaction, message_id)?;
        let (_, state, _) = conversation_state_tx(&transaction, &message.conversation_id)?;
        ensure_active_state(&state)?;
        ensure_active_member_tx(&transaction, &message.conversation_id, actor_id)?;
        crate::sessions::ensure_message_visible(&transaction, &message, actor_id)?;
        let can_delete = if &message.sender_id == actor_id {
            true
        } else {
            matches!(
                member_role_tx(&transaction, &message.conversation_id, actor_id)?,
                MemberRole::Owner | MemberRole::Admin
            )
        };
        if !can_delete {
            return Err(ImError::PermissionDenied(
                "only the sender or a group admin can delete a message".to_string(),
            ));
        }
        if message.deleted_at.is_some() {
            transaction.commit()?;
            return Ok(message);
        }
        let deleted_at = now_millis();
        transaction.execute(
            "UPDATE messages SET body = '', deleted_at = ?2 WHERE id = ?1",
            params![message_id.as_str(), deleted_at],
        )?;
        append_event_tx(
            &transaction,
            &message.conversation_id,
            Some(actor_id),
            "message.deleted",
            &json!({ "message_id": message_id.as_str() }),
        )?;
        transaction.commit()?;
        drop(connection);
        self.get_message(message_id)
    }

    pub fn get_message(&self, message_id: &MessageId) -> Result<Message> {
        let connection = self.lock()?;
        get_message_from_connection(&connection, message_id)
    }

    /// When the workspace last showed activity: the newest message anywhere, leaving out
    /// goal reviews (messages with `goal_review` metadata) and the replies to them, and,
    /// if given, messages from `except`.
    pub fn latest_activity(&self, except: Option<&PrincipalId>) -> Result<Option<i64>> {
        let connection = self.lock()?;
        Ok(connection.query_row(
            "SELECT MAX(m.created_at) FROM messages m
             WHERE m.deleted_at IS NULL
               AND json_extract(m.metadata_json, '$.goal_review') IS NULL
               AND (?1 IS NULL OR m.sender_id != ?1)
               AND NOT EXISTS (SELECT 1 FROM messages r WHERE r.id = m.reply_to_id
                               AND json_extract(r.metadata_json, '$.goal_review') IS NOT NULL)",
            params![except.map(PrincipalId::as_str)],
            |row| row.get(0),
        )?)
    }

    fn change_conversation_state(
        &self,
        conversation_id: &ConversationId,
        actor_id: &PrincipalId,
        new_state: ConversationState,
    ) -> Result<Conversation> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (kind, state, _) = conversation_state_tx(&transaction, conversation_id)?;
        ensure_active_member_tx(&transaction, conversation_id, actor_id)?;
        let dissolved: bool = transaction.query_row(
            "SELECT dissolved FROM conversations WHERE id=?1",
            [conversation_id.as_str()],
            |r| r.get(0),
        )?;
        if dissolved {
            return Err(ImError::Conflict(
                "a dissolved group cannot be restored".into(),
            ));
        }
        let transition_allowed = matches!(
            (&state, &new_state),
            (ConversationState::Active, ConversationState::Archived)
                | (ConversationState::Active, ConversationState::Deleted)
                | (ConversationState::Archived, ConversationState::Deleted)
                | (ConversationState::Archived, ConversationState::Active)
                | (ConversationState::Deleted, ConversationState::Active)
        );
        if !transition_allowed {
            return Err(ImError::Conflict(format!(
                "conversation cannot transition from {} to {}",
                conversation_state_to_str(&state),
                conversation_state_to_str(&new_state),
            )));
        }
        if kind == ConversationKind::Group
            && new_state == ConversationState::Deleted
            && member_role_tx(&transaction, conversation_id, actor_id)? != MemberRole::Owner
        {
            return Err(ImError::PermissionDenied(
                "only the group owner can delete the group".into(),
            ));
        }
        if kind == ConversationKind::Group
            && !matches!(
                member_role_tx(&transaction, conversation_id, actor_id)?,
                MemberRole::Owner | MemberRole::Admin
            )
        {
            return Err(ImError::PermissionDenied(
                "only the group owner or an admin can change conversation state".to_string(),
            ));
        }
        transaction.execute(
            "UPDATE conversations SET state = ?2, updated_at = ?3 WHERE id = ?1",
            params![
                conversation_id.as_str(),
                conversation_state_to_str(&new_state),
                now_millis(),
            ],
        )?;
        append_event_tx(
            &transaction,
            conversation_id,
            Some(actor_id),
            match new_state {
                ConversationState::Archived => "conversation.archived",
                ConversationState::Deleted => "conversation.deleted",
                ConversationState::Active => "conversation.reactivated",
            },
            &json!({ "state": conversation_state_to_str(&new_state) }),
        )?;
        transaction.commit()?;
        drop(connection);
        self.get_conversation(conversation_id)
    }

    pub(crate) fn lock(&self) -> Result<StoreGuard<'_>> {
        let guard = self.connection.lock().map_err(|_| ImError::LockPoisoned)?;
        let changes = guard.total_changes();
        Ok(StoreGuard {
            guard: Some(guard),
            changes,
            path: self.path.as_deref(),
        })
    }
}

fn configure_connection(connection: &Connection, enable_wal: bool) -> Result<()> {
    connection.busy_timeout(agt_storage::sqlite::BUSY_TIMEOUT)?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    if enable_wal {
        connection.pragma_update(None, "journal_mode", "WAL")?;
    }
    Ok(())
}

fn initialize_schema(connection: &Connection) -> Result<()> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let version: u32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 5 {
        return Ok(());
    }
    if version == 4 {
        migrate_events(&transaction)?;
        transaction.commit()?;
        return Ok(());
    }
    if version == 2 || version == 3 {
        if version == 2 {
            crate::sessions::migrate(&transaction)?;
        }
        crate::sessions::migrate_viewed(&transaction)?;
        migrate_events(&transaction)?;
        transaction.commit()?;
        return Ok(());
    }
    if version != 0 {
        return Err(ImError::InvalidStoredData(
            "unsupported IM schema version".into(),
        ));
    }
    transaction.execute_batch(
        "
         CREATE TABLE principals (
             id TEXT PRIMARY KEY NOT NULL,
             kind TEXT NOT NULL,
             display_name TEXT NOT NULL,
             metadata_json TEXT NOT NULL,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             deleted_at INTEGER
         );
         CREATE TABLE conversations (
             id TEXT PRIMARY KEY NOT NULL,
             kind TEXT NOT NULL CHECK (kind IN ('direct', 'group')),
             state TEXT NOT NULL CHECK (state IN ('active', 'archived', 'deleted')),
             title TEXT,
             owner_id TEXT REFERENCES principals(id),
             last_message_seq INTEGER NOT NULL DEFAULT 0,
             last_event_seq INTEGER NOT NULL DEFAULT 0,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL
         );
         CREATE TABLE direct_pairs (
             conversation_id TEXT PRIMARY KEY NOT NULL REFERENCES conversations(id),
             principal_a TEXT NOT NULL REFERENCES principals(id),
             principal_b TEXT NOT NULL REFERENCES principals(id),
             CHECK (principal_a < principal_b),
             UNIQUE (principal_a, principal_b)
         );
         CREATE TABLE conversation_members (
             conversation_id TEXT NOT NULL REFERENCES conversations(id),
             principal_id TEXT NOT NULL REFERENCES principals(id),
             role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
             joined_message_seq INTEGER NOT NULL DEFAULT 0,
             left_message_seq INTEGER,
             last_read_seq INTEGER NOT NULL DEFAULT 0,
             PRIMARY KEY (conversation_id, principal_id)
         );
         CREATE TABLE messages (
             id TEXT PRIMARY KEY NOT NULL,
             conversation_id TEXT NOT NULL REFERENCES conversations(id),
             seq INTEGER NOT NULL,
             sender_id TEXT NOT NULL REFERENCES principals(id),
             body TEXT NOT NULL,
             client_message_id TEXT,
             reply_to_id TEXT REFERENCES messages(id),
             metadata_json TEXT NOT NULL,
             created_at INTEGER NOT NULL,
             deleted_at INTEGER,
             UNIQUE (conversation_id, seq),
             UNIQUE (conversation_id, sender_id, client_message_id)
         );
         CREATE TABLE im_events (
             id TEXT PRIMARY KEY NOT NULL,
             conversation_id TEXT NOT NULL REFERENCES conversations(id),
             seq INTEGER NOT NULL,
             event_type TEXT NOT NULL,
             actor_id TEXT REFERENCES principals(id),
             payload_json TEXT NOT NULL,
             created_at INTEGER NOT NULL,
             UNIQUE (conversation_id, seq)
         );
         CREATE TABLE idempotency_records (
             actor_id TEXT NOT NULL REFERENCES principals(id),
             scope TEXT NOT NULL,
             request_key TEXT NOT NULL,
             request_hash TEXT NOT NULL,
             response_message_id TEXT NOT NULL REFERENCES messages(id),
             created_at INTEGER NOT NULL,
             PRIMARY KEY (actor_id, scope, request_key)
         );
         CREATE INDEX messages_page
             ON messages (conversation_id, seq DESC);
         CREATE INDEX events_page
             ON im_events (conversation_id, seq ASC);
         CREATE INDEX member_conversations
             ON conversation_members (principal_id, conversation_id);
         CREATE INDEX direct_pair_lookup
             ON direct_pairs (principal_a, principal_b);",
    )?;
    transaction.execute_batch(
        "CREATE TABLE message_deliveries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            message_id TEXT NOT NULL REFERENCES messages(id),
            principal_id TEXT NOT NULL REFERENCES principals(id),
            state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','processing','completed','skipped','failed')),
            run_id TEXT,
            error TEXT,
            UNIQUE(message_id, principal_id)
         );
         CREATE INDEX delivery_inbox ON message_deliveries(principal_id,state,id);
         PRAGMA user_version = 2;"
    )?;
    crate::sessions::migrate(&transaction)?;
    crate::sessions::migrate_viewed(&transaction)?;
    migrate_events(&transaction)?;
    transaction.commit()?;
    Ok(())
}

fn migrate_events(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch("CREATE TABLE IF NOT EXISTS change_outbox(seq INTEGER PRIMARY KEY AUTOINCREMENT, conversation TEXT, kind TEXT NOT NULL, at INTEGER NOT NULL);
        CREATE TRIGGER IF NOT EXISTS im_change AFTER INSERT ON im_events BEGIN
            INSERT INTO change_outbox(conversation,kind,at) VALUES(
              CASE WHEN NEW.event_type IN ('member.joined','member.left','group.dissolved','group.supervised') THEN NULL ELSE NEW.conversation_id END,
              'im', CAST(unixepoch('subsec')*1000 AS INTEGER));
        END;
        CREATE TRIGGER IF NOT EXISTS principal_added AFTER INSERT ON principals BEGIN
            INSERT INTO change_outbox(kind,at) VALUES('access',CAST(unixepoch('subsec')*1000 AS INTEGER));
        END;
        CREATE TRIGGER IF NOT EXISTS principal_changed AFTER UPDATE ON principals BEGIN
            INSERT INTO change_outbox(kind,at) VALUES('access',CAST(unixepoch('subsec')*1000 AS INTEGER));
        END;
        PRAGMA user_version=5;")?;
    Ok(())
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn validate_id<T: AsRef<str>>(id: &T, label: &str) -> Result<()> {
    if id.as_ref().trim().is_empty() {
        return Err(ImError::InvalidInput(format!("{label} cannot be empty")));
    }
    Ok(())
}

fn ensure_id<T: AsRef<str>>(id: &T, label: &str) -> Result<()> {
    validate_id(id, label)
}

fn validate_display_name(display_name: &str) -> Result<()> {
    if display_name.trim().is_empty() {
        return Err(ImError::InvalidInput(
            "principal display_name cannot be empty".to_string(),
        ));
    }
    Ok(())
}

fn ensure_non_empty_title(title: Option<&str>) -> Result<()> {
    if title.is_some_and(|value| value.trim().is_empty()) {
        return Err(ImError::InvalidInput(
            "group title cannot be empty".to_string(),
        ));
    }
    Ok(())
}

fn canonical_pair(first: &PrincipalId, second: &PrincipalId) -> (PrincipalId, PrincipalId) {
    if first <= second {
        (first.clone(), second.clone())
    } else {
        (second.clone(), first.clone())
    }
}

fn checked_limit(limit: usize) -> Result<usize> {
    if limit == 0 || limit > MAX_PAGE_SIZE {
        return Err(ImError::InvalidInput(format!(
            "page size must be between 1 and {MAX_PAGE_SIZE}"
        )));
    }
    Ok(limit)
}

fn principal_kind_to_str(kind: &PrincipalKind) -> &'static str {
    match kind {
        PrincipalKind::User => "user",
        PrincipalKind::Agent => "agent",
        PrincipalKind::Bot => "bot",
        PrincipalKind::Service => "service",
        PrincipalKind::Other => "other",
    }
}

fn principal_kind_from_str(value: &str) -> Result<PrincipalKind> {
    match value {
        "user" => Ok(PrincipalKind::User),
        "agent" => Ok(PrincipalKind::Agent),
        "bot" => Ok(PrincipalKind::Bot),
        "service" => Ok(PrincipalKind::Service),
        "other" => Ok(PrincipalKind::Other),
        _ => Err(ImError::InvalidStoredData(format!(
            "unknown principal kind: {value}"
        ))),
    }
}

fn conversation_kind_from_str(value: &str) -> Result<ConversationKind> {
    match value {
        "direct" => Ok(ConversationKind::Direct),
        "group" => Ok(ConversationKind::Group),
        _ => Err(ImError::InvalidStoredData(format!(
            "unknown conversation kind: {value}"
        ))),
    }
}

fn conversation_state_to_str(state: &ConversationState) -> &'static str {
    match state {
        ConversationState::Active => "active",
        ConversationState::Archived => "archived",
        ConversationState::Deleted => "deleted",
    }
}

fn conversation_state_from_str(value: &str) -> Result<ConversationState> {
    match value {
        "active" => Ok(ConversationState::Active),
        "archived" => Ok(ConversationState::Archived),
        "deleted" => Ok(ConversationState::Deleted),
        _ => Err(ImError::InvalidStoredData(format!(
            "unknown conversation state: {value}"
        ))),
    }
}

fn member_role_to_str(role: &MemberRole) -> &'static str {
    match role {
        MemberRole::Owner => "owner",
        MemberRole::Admin => "admin",
        MemberRole::Member => "member",
    }
}

fn member_role_from_str(value: &str) -> Result<MemberRole> {
    match value {
        "owner" => Ok(MemberRole::Owner),
        "admin" => Ok(MemberRole::Admin),
        "member" => Ok(MemberRole::Member),
        _ => Err(ImError::InvalidStoredData(format!(
            "unknown member role: {value}"
        ))),
    }
}

fn parse_json(value: String, field: &str) -> Result<Value> {
    serde_json::from_str(&value)
        .map_err(|error| ImError::InvalidStoredData(format!("{field}: {error}")))
}

fn principal_from_row(row: &Row<'_>) -> rusqlite::Result<Principal> {
    let kind: String = row.get(1)?;
    let metadata: String = row.get(3)?;
    Ok(Principal {
        id: PrincipalId::new(row.get::<_, String>(0)?),
        kind: principal_kind_from_str(&kind).map_err(to_sql_error)?,
        display_name: row.get(2)?,
        metadata: parse_json(metadata, "principal metadata").map_err(to_sql_error)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        deleted_at: row.get(6)?,
    })
}

fn conversation_from_row(row: &Row<'_>) -> rusqlite::Result<Conversation> {
    let kind: String = row.get(1)?;
    let state: String = row.get(2)?;
    Ok(Conversation {
        id: ConversationId::new(row.get::<_, String>(0)?),
        kind: conversation_kind_from_str(&kind).map_err(to_sql_error)?,
        state: conversation_state_from_str(&state).map_err(to_sql_error)?,
        title: row.get(3)?,
        owner_id: row.get::<_, Option<String>>(4)?.map(PrincipalId::new),
        last_message_seq: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn member_from_row(row: &Row<'_>) -> rusqlite::Result<ConversationMember> {
    let role: String = row.get(2)?;
    Ok(ConversationMember {
        conversation_id: ConversationId::new(row.get::<_, String>(0)?),
        principal_id: PrincipalId::new(row.get::<_, String>(1)?),
        role: member_role_from_str(&role).map_err(to_sql_error)?,
        joined_message_seq: row.get(3)?,
        left_message_seq: row.get(4)?,
        last_read_seq: row.get(5)?,
    })
}

fn message_from_row(row: &Row<'_>) -> rusqlite::Result<Message> {
    let metadata: String = row.get(7)?;
    Ok(Message {
        id: MessageId::new(row.get::<_, String>(0)?),
        conversation_id: ConversationId::new(row.get::<_, String>(1)?),
        seq: row.get(2)?,
        sender_id: PrincipalId::new(row.get::<_, String>(3)?),
        body: row.get(4)?,
        client_message_id: row.get(5)?,
        reply_to_id: row.get::<_, Option<String>>(6)?.map(MessageId::new),
        metadata: parse_json(metadata, "message metadata").map_err(to_sql_error)?,
        created_at: row.get(8)?,
        deleted_at: row.get(9)?,
    })
}

fn event_from_row(row: &Row<'_>) -> rusqlite::Result<ImEvent> {
    let payload: String = row.get(5)?;
    Ok(ImEvent {
        id: EventId::new(row.get::<_, String>(0)?),
        conversation_id: ConversationId::new(row.get::<_, String>(1)?),
        seq: row.get(2)?,
        event_type: row.get(3)?,
        actor_id: row.get::<_, Option<String>>(4)?.map(PrincipalId::new),
        payload: parse_json(payload, "event payload").map_err(to_sql_error)?,
        created_at: row.get(6)?,
    })
}

fn get_principal_from_connection(connection: &Connection, id: &PrincipalId) -> Result<Principal> {
    connection
        .query_row(
            "SELECT id, kind, display_name, metadata_json, created_at, updated_at, deleted_at
             FROM principals WHERE id = ?1 AND deleted_at IS NULL",
            params![id.as_str()],
            principal_from_row,
        )
        .optional()?
        .ok_or_else(|| ImError::PrincipalNotFound(id.clone()))
}

fn get_conversation_from_connection(
    connection: &Connection,
    id: &ConversationId,
) -> Result<Conversation> {
    connection
        .query_row(
            "SELECT id, kind, state, title, owner_id, last_message_seq,
                    created_at, updated_at
             FROM conversations WHERE id = ?1",
            params![id.as_str()],
            conversation_from_row,
        )
        .optional()?
        .ok_or_else(|| ImError::ConversationNotFound(id.clone()))
}

fn ensure_conversation_from_connection(connection: &Connection, id: &ConversationId) -> Result<()> {
    get_conversation_from_connection(connection, id).map(|_| ())
}

fn ensure_principal_tx(transaction: &Transaction<'_>, id: &PrincipalId) -> Result<()> {
    let exists: Option<i64> = transaction
        .query_row(
            "SELECT 1 FROM principals WHERE id = ?1 AND deleted_at IS NULL",
            params![id.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(ImError::PrincipalNotFound(id.clone()));
    }
    Ok(())
}

fn conversation_state_tx(
    transaction: &Transaction<'_>,
    id: &ConversationId,
) -> Result<(ConversationKind, ConversationState, i64)> {
    transaction
        .query_row(
            "SELECT kind, state, last_message_seq FROM conversations WHERE id = ?1",
            params![id.as_str()],
            |row| {
                let kind: String = row.get(0)?;
                let state: String = row.get(1)?;
                Ok((kind, state, row.get::<_, i64>(2)?))
            },
        )
        .optional()?
        .ok_or_else(|| ImError::ConversationNotFound(id.clone()))
        .and_then(|(kind, state, seq)| {
            Ok((
                conversation_kind_from_str(&kind)?,
                conversation_state_from_str(&state)?,
                seq,
            ))
        })
}

fn ensure_active_state(state: &ConversationState) -> Result<()> {
    if *state != ConversationState::Active {
        return Err(ImError::Conflict(format!(
            "conversation is {}",
            conversation_state_to_str(state)
        )));
    }
    Ok(())
}

fn ensure_active_member_tx(
    transaction: &Transaction<'_>,
    conversation_id: &ConversationId,
    principal_id: &PrincipalId,
) -> Result<()> {
    let active: Option<i64> = transaction
        .query_row(
            "SELECT 1 FROM conversation_members
             WHERE conversation_id = ?1 AND principal_id = ?2
               AND left_message_seq IS NULL",
            params![conversation_id.as_str(), principal_id.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    if active.is_none() {
        return Err(ImError::NotMember {
            principal: principal_id.clone(),
            conversation: conversation_id.clone(),
        });
    }
    Ok(())
}

fn member_role_tx(
    transaction: &Transaction<'_>,
    conversation_id: &ConversationId,
    principal_id: &PrincipalId,
) -> Result<MemberRole> {
    transaction
        .query_row(
            "SELECT role FROM conversation_members
             WHERE conversation_id = ?1 AND principal_id = ?2
               AND left_message_seq IS NULL",
            params![conversation_id.as_str(), principal_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| ImError::NotMember {
            principal: principal_id.clone(),
            conversation: conversation_id.clone(),
        })
        .and_then(|role| member_role_from_str(&role))
}

fn ensure_role_can_manage_members(
    transaction: &Transaction<'_>,
    conversation_id: &ConversationId,
    actor_id: &PrincipalId,
) -> Result<MemberRole> {
    let role = member_role_tx(transaction, conversation_id, actor_id)?;
    if !matches!(role, MemberRole::Owner | MemberRole::Admin) {
        return Err(ImError::PermissionDenied(
            "only the group owner or an admin can manage members".to_string(),
        ));
    }
    Ok(role)
}

fn active_member_cursor_tx(
    transaction: &Transaction<'_>,
    conversation_id: &ConversationId,
    principal_id: &PrincipalId,
) -> Result<i64> {
    transaction
        .query_row(
            "SELECT last_read_seq FROM conversation_members
             WHERE conversation_id = ?1 AND principal_id = ?2
               AND left_message_seq IS NULL",
            params![conversation_id.as_str(), principal_id.as_str()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| ImError::NotMember {
            principal: principal_id.clone(),
            conversation: conversation_id.clone(),
        })
}

fn member_from_connection(
    connection: &Connection,
    conversation_id: &ConversationId,
    principal_id: &PrincipalId,
) -> Result<ConversationMember> {
    connection
        .query_row(
            "SELECT conversation_id, principal_id, role, joined_message_seq,
                    left_message_seq, last_read_seq
             FROM conversation_members
             WHERE conversation_id = ?1 AND principal_id = ?2",
            params![conversation_id.as_str(), principal_id.as_str()],
            member_from_row,
        )
        .optional()?
        .ok_or_else(|| ImError::NotMember {
            principal: principal_id.clone(),
            conversation: conversation_id.clone(),
        })
}

fn get_message_from_connection(connection: &Connection, id: &MessageId) -> Result<Message> {
    connection
        .query_row(
            "SELECT id, conversation_id, seq, sender_id, body, client_message_id,
                    reply_to_id, metadata_json, created_at, deleted_at
             FROM messages WHERE id = ?1",
            params![id.as_str()],
            message_from_row,
        )
        .optional()?
        .ok_or_else(|| ImError::MessageNotFound(id.clone()))
}

fn get_message_tx(transaction: &Transaction<'_>, id: &MessageId) -> Result<Message> {
    transaction
        .query_row(
            "SELECT id, conversation_id, seq, sender_id, body, client_message_id,
                    reply_to_id, metadata_json, created_at, deleted_at
             FROM messages WHERE id = ?1",
            params![id.as_str()],
            message_from_row,
        )
        .optional()?
        .ok_or_else(|| ImError::MessageNotFound(id.clone()))
}

pub(crate) fn append_event_tx(
    transaction: &Transaction<'_>,
    conversation_id: &ConversationId,
    actor_id: Option<&PrincipalId>,
    event_type: &str,
    payload: &Value,
) -> Result<ImEvent> {
    let now = now_millis();
    transaction.execute(
        "UPDATE conversations
         SET last_event_seq = last_event_seq + 1, updated_at = ?2
         WHERE id = ?1",
        params![conversation_id.as_str(), now],
    )?;
    let seq: i64 = transaction.query_row(
        "SELECT last_event_seq FROM conversations WHERE id = ?1",
        params![conversation_id.as_str()],
        |row| row.get(0),
    )?;
    let event = ImEvent {
        id: EventId::generate(),
        conversation_id: conversation_id.clone(),
        seq,
        event_type: event_type.to_string(),
        actor_id: actor_id.cloned(),
        payload: payload.clone(),
        created_at: now,
    };
    transaction.execute(
        "INSERT INTO im_events
            (id, conversation_id, seq, event_type, actor_id, payload_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            event.id.as_str(),
            event.conversation_id.as_str(),
            event.seq,
            event.event_type,
            event.actor_id.as_ref().map(ToString::to_string),
            serde_json::to_string(&event.payload)?,
            event.created_at,
        ],
    )?;
    Ok(event)
}

fn request_hash(request: &SendMessageRequest) -> Result<String> {
    let canonical = json!({
        "conversation_id": request.conversation_id.as_str(),
        "sender_id": request.sender_id.as_str(),
        "body": request.body,
        "client_message_id": request.client_message_id,
        "reply_to_id": request.reply_to_id,
        "metadata": request.metadata,
    });
    let digest = Sha256::digest(serde_json::to_vec(&canonical)?);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn encode_message_cursor(conversation_id: &ConversationId, before_seq: i64) -> String {
    let raw = format!("v1\n{}\n{}", conversation_id.as_str(), before_seq);
    raw.as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn decode_message_cursor(cursor: &str, conversation_id: &ConversationId) -> Result<i64> {
    if cursor.is_empty() || !cursor.len().is_multiple_of(2) {
        return Err(ImError::InvalidCursor(
            "cursor is not valid hex".to_string(),
        ));
    }
    let mut bytes = Vec::with_capacity(cursor.len() / 2);
    for pair in cursor.as_bytes().as_chunks::<2>().0 {
        let value = std::str::from_utf8(pair)
            .ok()
            .and_then(|value| u8::from_str_radix(value, 16).ok())
            .ok_or_else(|| ImError::InvalidCursor("cursor is not valid hex".to_string()))?;
        bytes.push(value);
    }
    let raw = String::from_utf8(bytes)
        .map_err(|_| ImError::InvalidCursor("cursor is not UTF-8".to_string()))?;
    let mut parts = raw.split('\n');
    if parts.next() != Some("v1") || parts.next() != Some(conversation_id.as_str()) {
        return Err(ImError::InvalidCursor(
            "cursor belongs to another conversation".to_string(),
        ));
    }
    let seq = parts
        .next()
        .ok_or_else(|| ImError::InvalidCursor("cursor is missing sequence".to_string()))?
        .parse::<i64>()
        .map_err(|_| ImError::InvalidCursor("cursor sequence is invalid".to_string()))?;
    if parts.next().is_some() || seq < 0 {
        return Err(ImError::InvalidCursor(
            "cursor sequence is invalid".to_string(),
        ));
    }
    Ok(seq)
}

fn to_sql_error(error: ImError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

// Send only after the transaction and connection guard have been released.
// Rollbacks can cause harmless extra hints; consumers always re-read durable state.
pub(crate) struct StoreGuard<'a> {
    guard: Option<MutexGuard<'a, Connection>>,
    changes: u64,
    path: Option<&'a Path>,
}
impl std::ops::Deref for StoreGuard<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.guard.as_ref().unwrap()
    }
}
impl std::ops::DerefMut for StoreGuard<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.guard.as_mut().unwrap()
    }
}
impl Drop for StoreGuard<'_> {
    fn drop(&mut self) {
        let changed = self
            .guard
            .as_ref()
            .is_some_and(|c| c.is_autocommit() && c.total_changes() != self.changes);
        drop(self.guard.take());
        if changed
            && let Some(path) = self.path
            && path.file_name().is_some_and(|n| n == "im.db")
            && let Some(dir) = path.parent()
            && dir.file_name().is_some_and(|n| n == "im")
            && let Some(home) = dir.parent()
        {
            agt_storage::wake::notify(home);
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn committed_writes_wake_but_reads_do_not() {
        let home = tempfile::tempdir().unwrap();
        let store = ImStore::open(home.path().join("im/im.db")).unwrap();
        let listener = agt_storage::wake::Listener::open(home.path()).unwrap();
        principal(&store, "wake-user");
        assert!(listener.wait(Duration::from_millis(20)).unwrap());
        store.list_principals().unwrap();
        assert!(!listener.wait(Duration::from_millis(1)).unwrap());
    }
    use super::*;
    #[cfg(unix)]
    use std::time::Duration;

    fn principal(store: &ImStore, id: &str) -> PrincipalId {
        let id = PrincipalId::new(id);
        store
            .create_principal(PrincipalInput {
                id: id.clone(),
                kind: PrincipalKind::User,
                display_name: id.to_string(),
                metadata: Value::Null,
            })
            .expect("create principal");
        id
    }

    fn message(
        conversation_id: ConversationId,
        sender_id: PrincipalId,
        body: &str,
    ) -> SendMessageRequest {
        SendMessageRequest {
            conversation_id,
            sender_id,
            body: body.to_string(),
            client_message_id: None,
            reply_to_id: None,
            metadata: Value::Null,
            idempotency_key: None,
        }
    }

    #[test]
    fn direct_pair_is_canonical_and_message_round_trips() {
        let store = ImStore::open_in_memory().expect("open store");
        let alice = principal(&store, "alice");
        let bob = principal(&store, "bob");
        let conversation = store.create_direct(&bob, &alice).expect("direct");
        assert_eq!(
            store.create_direct(&alice, &bob).expect("same direct").id,
            conversation.id
        );

        let request = message(conversation.id.clone(), alice.clone(), "hello");
        let sent = store.send_message(request).expect("send");
        assert_eq!(sent.message.seq, 1);
        let page = store
            .list_messages(&conversation.id, &bob, MessageListOptions::default())
            .expect("list");
        assert_eq!(page.messages[0].body, "hello");
    }

    #[test]
    fn group_membership_and_idempotency_are_transactional() {
        let store = ImStore::open_in_memory().expect("open store");
        let owner = principal(&store, "owner");
        let member = principal(&store, "member");
        let guest = principal(&store, "guest");
        let conversation = store
            .create_group(&owner, Some("team".to_string()), vec![member.clone()])
            .expect("group");
        store
            .add_member(&conversation.id, &owner, &guest)
            .expect("add");

        let mut request = message(conversation.id.clone(), owner.clone(), "one");
        request.idempotency_key = Some("request-1".to_string());
        let first = store.send_message(request.clone()).expect("first");
        let second = store.send_message(request).expect("retry");
        assert!(!first.duplicate);
        assert!(second.duplicate);
        assert_eq!(first.message.id, second.message.id);

        let summaries = store.list_conversations(&member).expect("summaries");
        assert_eq!(summaries[0].unread_count, 1);
        let preview = summaries[0].last_message.as_ref().expect("preview");
        assert_eq!(
            (preview.sender_id.as_str(), preview.body.as_str()),
            (owner.as_str(), "one")
        );
        store.mark_read(&conversation.id, &member, 1).expect("read");
        let summaries = store.list_conversations(&member).expect("summaries");
        assert_eq!(summaries[0].unread_count, 0);
    }

    #[test]
    fn member_join_event_records_the_actor() {
        let store = ImStore::open_in_memory().expect("open store");
        let owner = principal(&store, "owner");
        let guest = principal(&store, "guest");
        let conversation = store
            .create_group(&owner, Some("team".to_string()), Vec::new())
            .expect("group");

        store
            .add_member(&conversation.id, &owner, &guest)
            .expect("add guest");
        let events = store
            .list_events(&conversation.id, &owner, None, 20)
            .expect("events")
            .events;
        let joined = events
            .iter()
            .find(|event| event.event_type == "member.joined")
            .expect("member.joined event");
        assert_eq!(joined.actor_id.as_ref(), Some(&owner));
        assert_eq!(joined.payload["principal_id"], "guest");
    }

    #[test]
    fn owner_can_promote_a_member_who_can_then_manage_members() {
        let store = ImStore::open_in_memory().expect("open store");
        let owner = principal(&store, "owner");
        let lead = principal(&store, "lead");
        let guest = principal(&store, "guest");
        let conversation = store
            .create_group(&owner, Some("team".to_string()), vec![lead.clone()])
            .expect("group");
        assert!(matches!(
            store.add_member(&conversation.id, &lead, &guest),
            Err(ImError::PermissionDenied(_))
        ));

        let promoted = store
            .set_member_role(&conversation.id, &owner, &lead, MemberRole::Admin)
            .expect("promote");
        assert_eq!(promoted.role, MemberRole::Admin);
        store
            .add_member(&conversation.id, &lead, &guest)
            .expect("admin adds a member");
        // Admins cannot change roles, and nobody can take or change ownership.
        assert!(matches!(
            store.set_member_role(&conversation.id, &lead, &guest, MemberRole::Admin),
            Err(ImError::PermissionDenied(_))
        ));
        assert!(matches!(
            store.set_member_role(&conversation.id, &owner, &guest, MemberRole::Owner),
            Err(ImError::InvalidInput(_))
        ));
        assert!(matches!(
            store.set_member_role(&conversation.id, &owner, &owner, MemberRole::Member),
            Err(ImError::PermissionDenied(_))
        ));

        let events = store
            .list_events(&conversation.id, &owner, None, 20)
            .expect("events")
            .events;
        let changed: Vec<_> = events
            .iter()
            .filter(|event| event.event_type == "member.role_changed")
            .collect();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].payload["role"], "admin");
        // Setting the same role again changes nothing.
        store
            .set_member_role(&conversation.id, &owner, &lead, MemberRole::Admin)
            .expect("same role");
        store
            .set_member_role(&conversation.id, &owner, &lead, MemberRole::Member)
            .expect("demote");
        assert!(matches!(
            store.add_member(&conversation.id, &lead, &principal(&store, "late")),
            Err(ImError::PermissionDenied(_))
        ));
    }

    #[test]
    fn after_cursor_supports_forward_continuation() {
        let store = ImStore::open_in_memory().expect("open store");
        let alice = principal(&store, "alice");
        let bob = principal(&store, "bob");
        let conversation = store.create_direct(&alice, &bob).expect("direct");
        for index in 1..=5 {
            store
                .send_message(message(
                    conversation.id.clone(),
                    alice.clone(),
                    &index.to_string(),
                ))
                .expect("send");
        }

        let first = store
            .list_messages(
                &conversation.id,
                &bob,
                MessageListOptions {
                    limit: 2,
                    after: Some(encode_message_cursor(&conversation.id, 0)),
                    ..Default::default()
                },
            )
            .expect("first page");
        assert_eq!(
            first
                .messages
                .iter()
                .map(|message| message.seq)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        let second = store
            .list_messages(
                &conversation.id,
                &bob,
                MessageListOptions {
                    limit: 2,
                    after: first.next_cursor,
                    ..Default::default()
                },
            )
            .expect("second page");
        assert_eq!(
            second
                .messages
                .iter()
                .map(|message| message.seq)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
        let third = store
            .list_messages(
                &conversation.id,
                &bob,
                MessageListOptions {
                    limit: 2,
                    after: second.next_cursor,
                    ..Default::default()
                },
            )
            .expect("third page");
        assert_eq!(
            third
                .messages
                .iter()
                .map(|message| message.seq)
                .collect::<Vec<_>>(),
            vec![5]
        );
        assert!(third.next_cursor.is_none());
    }

    #[test]
    fn archived_and_deleted_conversations_can_be_reactivated() {
        let store = ImStore::open_in_memory().expect("open store");
        let alice = principal(&store, "alice");
        let bob = principal(&store, "bob");
        let conversation = store.create_direct(&alice, &bob).expect("direct");

        assert_eq!(
            store
                .archive_conversation(&conversation.id, &alice)
                .expect("archive")
                .state,
            ConversationState::Archived
        );
        assert!(matches!(
            store.send_message(message(conversation.id.clone(), alice.clone(), "blocked")),
            Err(ImError::Conflict(_))
        ));
        assert_eq!(
            store
                .reactivate_conversation(&conversation.id, &alice)
                .expect("reactivate")
                .state,
            ConversationState::Active
        );
        assert_eq!(
            store
                .delete_conversation(&conversation.id, &alice)
                .expect("delete")
                .state,
            ConversationState::Deleted
        );
        assert_eq!(
            store
                .reactivate_conversation(&conversation.id, &alice)
                .expect("restore deleted")
                .state,
            ConversationState::Active
        );
    }

    #[test]
    fn file_store_survives_reopen() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("im.db");
        let store = ImStore::open(&path).expect("open");
        principal(&store, "alice");
        drop(store);
        let reopened = ImStore::open(&path).expect("reopen");
        assert_eq!(reopened.list_principals().expect("list").len(), 1);
    }

    #[test]
    fn message_cursor_walks_backwards_without_duplicates() {
        let store = ImStore::open_in_memory().expect("open store");
        let alice = principal(&store, "alice");
        let bob = principal(&store, "bob");
        let conversation = store.create_direct(&alice, &bob).expect("direct");
        for index in 1..=5 {
            store
                .send_message(message(
                    conversation.id.clone(),
                    alice.clone(),
                    &index.to_string(),
                ))
                .expect("send");
        }

        let first = store
            .list_messages(
                &conversation.id,
                &bob,
                MessageListOptions {
                    limit: 2,
                    ..Default::default()
                },
            )
            .expect("first page");
        assert_eq!(
            first
                .messages
                .iter()
                .map(|message| message.seq)
                .collect::<Vec<_>>(),
            vec![4, 5]
        );
        let second = store
            .list_messages(
                &conversation.id,
                &bob,
                MessageListOptions {
                    limit: 2,
                    before: first.next_cursor,
                    ..Default::default()
                },
            )
            .expect("second page");
        assert_eq!(
            second
                .messages
                .iter()
                .map(|message| message.seq)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        let after = store
            .list_messages(
                &conversation.id,
                &bob,
                MessageListOptions {
                    limit: 10,
                    after: Some(encode_message_cursor(&conversation.id, 2)),
                    ..Default::default()
                },
            )
            .expect("after page");
        assert_eq!(
            after
                .messages
                .iter()
                .map(|message| message.seq)
                .collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
    }

    #[test]
    fn member_can_leave_and_history_is_retained() {
        let store = ImStore::open_in_memory().expect("open store");
        let owner = principal(&store, "owner");
        let member = principal(&store, "member");
        let conversation = store
            .create_group(&owner, Some("team".to_string()), vec![member.clone()])
            .expect("group");
        let left = store
            .leave_conversation(&conversation.id, &member)
            .expect("leave");
        assert!(left.left_message_seq.is_some());
        assert!(
            store
                .list_messages(&conversation.id, &member, MessageListOptions::default())
                .unwrap()
                .messages
                .is_empty()
        );
        assert_eq!(store.list_conversations(&member).unwrap().len(), 1);
    }

    #[test]
    fn conversation_preview_stays_inside_membership_history() {
        let store = ImStore::open_in_memory().expect("open store");
        let owner = principal(&store, "owner");
        let member = principal(&store, "member");
        let conversation = store
            .create_group(&owner, Some("team".to_string()), vec![member.clone()])
            .expect("group");
        store
            .send_message(message(
                conversation.id.clone(),
                owner.clone(),
                "before leaving",
            ))
            .expect("visible message");
        store
            .leave_conversation(&conversation.id, &member)
            .expect("leave");
        store
            .send_message(message(
                conversation.id.clone(),
                owner.clone(),
                "after leaving",
            ))
            .expect("hidden message");
        let preview = |viewer: &PrincipalId| {
            store.list_conversations(viewer).unwrap()[0]
                .last_message
                .clone()
                .map(|m| m.body)
        };
        assert_eq!(preview(&member).as_deref(), Some("before leaving"));
        assert_eq!(preview(&owner).as_deref(), Some("after leaving"));
        let long = "长".repeat(200);
        store
            .send_message(message(conversation.id.clone(), owner.clone(), &long))
            .expect("long message");
        assert_eq!(preview(&owner).unwrap().chars().count(), 120);
    }

    #[test]
    fn members_added_later_do_not_get_old_unread_messages() {
        let store = ImStore::open_in_memory().expect("open store");
        let owner = principal(&store, "owner");
        let guest = principal(&store, "guest");
        let conversation = store
            .create_group(&owner, Some("team".to_string()), Vec::new())
            .expect("group");
        store
            .send_message(message(conversation.id.clone(), owner.clone(), "old"))
            .expect("old message");
        store
            .add_member(&conversation.id, &owner, &guest)
            .expect("add guest");
        let summaries = store.list_conversations(&guest).expect("summary");
        assert_eq!(summaries[0].unread_count, 0);
        store
            .send_message(message(conversation.id.clone(), owner, "new"))
            .expect("new message");
        let summaries = store.list_conversations(&guest).expect("summary");
        assert_eq!(summaries[0].unread_count, 1);
    }

    #[test]
    fn members_added_later_can_read_existing_history() {
        let store = ImStore::open_in_memory().expect("open store");
        let owner = principal(&store, "owner");
        let guest = principal(&store, "guest");
        let conversation = store
            .create_group(&owner, Some("team".to_string()), Vec::new())
            .expect("group");
        store
            .send_message(message(conversation.id.clone(), owner.clone(), "old"))
            .expect("old message");
        store
            .add_member(&conversation.id, &owner, &guest)
            .expect("add guest");

        let page = store
            .list_messages(&conversation.id, &guest, MessageListOptions::default())
            .expect("history");
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].body, "old");
    }
}
