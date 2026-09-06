use agt_im::{
    ConversationId, ConversationKind, ConversationState, ImStore, MessageId, MessageListOptions,
    PrincipalId, SendMessageOutcome, SendMessageRequest,
};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Participant,
    Manager,
}

#[derive(Clone, Debug)]
pub struct Actor {
    pub principal: PrincipalId,
    pub role: Role,
    /// Set only by hosted execution. Tools cannot lower this depth.
    pub automated_depth: Option<u32>,
}

impl Actor {
    pub fn require_manager(&self) -> Result<()> {
        if self.role != Role::Manager {
            bail!("manager permission required");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageInput {
    pub conversation: ConversationId,
    pub body: String,
    pub request_key: String,
    #[serde(default)]
    pub mentions: Vec<PrincipalId>,
    #[serde(default)]
    pub reply_to: Option<MessageId>,
}

pub struct Messaging {
    pub im: ImStore,
    pub actor: Actor,
}
impl Messaging {
    pub fn open(home: &Path, actor: Actor) -> Result<Self> {
        let im = ImStore::open(crate::database_path(home, "im", "im.db")?)?;
        im.get_principal(&actor.principal)?;
        Ok(Self { im, actor })
    }
    pub fn member(&self, conversation: &ConversationId) -> Result<()> {
        if self
            .im
            .get_member(conversation, &self.actor.principal)?
            .left_message_seq
            .is_some()
        {
            return Err(crate::ToolError::new(
                "permission_denied",
                "Active conversation membership is required.",
            )
            .into());
        }
        Ok(())
    }
    pub fn send(&self, mut input: MessageInput) -> Result<SendMessageOutcome> {
        self.member(&input.conversation)?;
        if input.body.trim().is_empty() || input.body.len() > 256 * 1024 {
            return Err(crate::ToolError::invalid(
                Some("body"),
                "Body must be nonblank and at most 262144 UTF-8 bytes.",
            )
            .into());
        }
        if input.request_key.trim().is_empty() || input.request_key.len() > 256 {
            return Err(crate::ToolError::invalid(
                Some("request_key"),
                "Request key must be nonblank and at most 256 UTF-8 bytes.",
            )
            .into());
        }
        // Compatibility for clients that fill optional string fields with empty values.
        input.reply_to = input.reply_to.filter(|id| !id.as_str().trim().is_empty());
        if let Some(id) = &input.reply_to {
            let reply = self.im.get_message(id).map_err(|error| match error {
                agt_im::ImError::MessageNotFound(_) => anyhow::Error::from(crate::ToolError::invalid(Some("reply_to"), "Reply target must exist in this conversation; omit/null for a new message.")),
                other => other.into(),
            })?;
            if reply.conversation_id != input.conversation {
                return Err(crate::ToolError::invalid(
                    Some("reply_to"),
                    "Reply target must exist in this conversation; omit/null for a new message.",
                )
                .into());
            }
        }
        input.mentions.sort();
        input.mentions.dedup();
        for principal in &input.mentions {
            if self
                .im
                .get_member(&input.conversation, principal)?
                .left_message_seq
                .is_some()
            {
                return Err(crate::ToolError::invalid(
                    Some("mentions"),
                    "Mentioned identities must be active conversation members.",
                )
                .into());
            }
        }
        self.im.send_message(SendMessageRequest{
            conversation_id:input.conversation,sender_id:self.actor.principal.clone(),body:input.body,
            client_message_id:Some(format!("tool:{}",input.request_key)),reply_to_id:input.reply_to,
            metadata:json!({"mentions":input.mentions,"automated_depth":self.actor.automated_depth.unwrap_or(0)}),idempotency_key:None,
        }).map_err(Into::into)
    }
    pub fn history(
        &self,
        conversation: &ConversationId,
        before: Option<String>,
        limit: usize,
    ) -> Result<agt_im::MessagePage> {
        if !(1..=200).contains(&limit) {
            bail!("limit must be in 1..=200");
        }
        Ok(self.im.list_messages(
            conversation,
            &self.actor.principal,
            MessageListOptions {
                limit,
                before: before.filter(|cursor| !cursor.trim().is_empty()),
                after: None,
            },
        )?)
    }
    pub fn may_respond(
        &self,
        message: &agt_im::Message,
        policy: crate::ResponsePolicy,
        max_depth: u32,
    ) -> Result<bool> {
        self.member(&message.conversation_id)?;
        let conversation = self.im.get_conversation(&message.conversation_id)?;
        let depth = message
            .metadata
            .get("automated_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if message.deleted_at.is_some()
            || conversation.state != ConversationState::Active
            || depth >= u64::from(max_depth)
            || policy == crate::ResponsePolicy::Manual
        {
            return Ok(false);
        }
        Ok(policy == crate::ResponsePolicy::All
            || conversation.kind == ConversationKind::Direct
            || message
                .metadata
                .get("mentions")
                .and_then(|v| v.as_array())
                .is_some_and(|values| {
                    values
                        .iter()
                        .any(|v| v.as_str() == Some(self.actor.principal.as_str()))
                }))
    }
}
