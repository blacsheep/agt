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
    /// Host-owned causal-depth metadata for diagnostics, never an execution limit.
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

/// Timing within an active native conversation run; never grants response permission.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    Steering,
    FollowUp,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_mode: Option<InputMode>,
}

/// Refuse mentions that are not active members, naming them and the members there are.
pub fn check_mentions(
    im: &ImStore,
    conversation: &ConversationId,
    mentions: &[PrincipalId],
) -> Result<()> {
    if mentions.is_empty() {
        return Ok(());
    }
    let mut active = Vec::new();
    for member in im.list_members(conversation)? {
        if member.left_message_seq.is_some() {
            continue;
        }
        // A deleted identity keeps its membership rows but can no longer be mentioned.
        if let Ok(principal) = im.get_principal(&member.principal_id)
            && principal.deleted_at.is_none()
        {
            active.push((principal.id.to_string(), principal.display_name));
        }
    }
    let unknown: Vec<String> = mentions
        .iter()
        .filter(|m| !active.iter().any(|(id, _)| id == m.as_str()))
        .map(|m| m.to_string())
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(crate::UnknownMentions {
        unknown,
        members: active,
    }
    .into())
}

pub struct Messaging {
    pub im: ImStore,
    pub actor: Actor,
    discussion: Option<String>,
    run: Option<String>,
}
impl Messaging {
    pub fn open(home: &Path, actor: Actor) -> Result<Self> {
        let im = ImStore::open(crate::database_path(home, "im", "im.db")?)?;
        im.get_principal(&actor.principal)?;
        Ok(Self {
            im,
            actor,
            discussion: None,
            run: None,
        })
    }
    /// Runtime-only context, never accepted from model tool arguments.
    pub fn with_discussion(mut self, discussion: Option<String>) -> Self {
        self.discussion = discussion;
        self
    }
    /// Runtime-only provenance: the Run whose execution sends these messages.
    pub fn with_run(mut self, run: Option<String>) -> Self {
        self.run = run;
        self
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
        check_mentions(&self.im, &input.conversation, &input.mentions)?;
        let mut metadata = json!({"mentions":input.mentions,"automated_depth":self.actor.automated_depth.unwrap_or(0)});
        if let Some(mode) = input.input_mode {
            metadata["input_mode"] = json!(mode);
        }
        if let Some(id) = &self.discussion {
            metadata["discussion_id"] = json!(id);
        }
        if let Some(id) = &self.run {
            metadata["run_id"] = json!(id);
        }
        self.im
            .send_message(SendMessageRequest {
                conversation_id: input.conversation,
                sender_id: self.actor.principal.clone(),
                body: input.body,
                client_message_id: Some(format!("tool:{}", input.request_key)),
                reply_to_id: input.reply_to,
                metadata,
                idempotency_key: None,
            })
            .map_err(Into::into)
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
    ) -> Result<bool> {
        self.member(&message.conversation_id)?;
        let conversation = self.im.get_conversation(&message.conversation_id)?;
        if message.deleted_at.is_some()
            || conversation.state != ConversationState::Active
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
