use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn generate() -> Self {
                Self(Uuid::new_v4().to_string())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

define_id!(PrincipalId);
define_id!(ConversationId);
define_id!(MessageId);
define_id!(EventId);

/// A participant in the IM system.  The core deliberately does not depend on
/// the agent resource model; an agent can be represented by the `Agent` kind
/// later without changing the storage or conversation APIs.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PrincipalKind {
    #[default]
    User,
    Agent,
    Bot,
    Service,
    Other,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Principal {
    pub id: PrincipalId,
    pub kind: PrincipalKind,
    pub display_name: String,
    pub metadata: Value,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrincipalInput {
    pub id: PrincipalId,
    #[serde(default)]
    pub kind: PrincipalKind,
    pub display_name: String,
    #[serde(default = "empty_object")]
    pub metadata: Value,
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationKind {
    Direct,
    Group,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationState {
    Active,
    Archived,
    Deleted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MemberRole {
    Owner,
    Admin,
    Member,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Conversation {
    pub id: ConversationId,
    pub kind: ConversationKind,
    pub state: ConversationState,
    pub title: Option<String>,
    pub owner_id: Option<PrincipalId>,
    pub last_message_seq: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConversationMember {
    pub conversation_id: ConversationId,
    pub principal_id: PrincipalId,
    pub role: MemberRole,
    pub joined_message_seq: i64,
    pub left_message_seq: Option<i64>,
    pub last_read_seq: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConversationSummary {
    pub conversation: Conversation,
    pub member_count: i64,
    pub unread_count: i64,
    #[serde(default)]
    pub left_message_seq: Option<i64>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub dissolved: bool,
    #[serde(default)]
    pub last_viewed_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Message {
    pub id: MessageId,
    pub conversation_id: ConversationId,
    pub seq: i64,
    pub sender_id: PrincipalId,
    pub body: String,
    pub client_message_id: Option<String>,
    pub reply_to_id: Option<MessageId>,
    pub metadata: Value,
    pub created_at: i64,
    pub deleted_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SendMessageRequest {
    pub conversation_id: ConversationId,
    pub sender_id: PrincipalId,
    pub body: String,
    #[serde(default)]
    pub client_message_id: Option<String>,
    #[serde(default)]
    pub reply_to_id: Option<MessageId>,
    #[serde(default = "empty_object")]
    pub metadata: Value,
    /// A request-level idempotency key.  This is intentionally separate from
    /// `client_message_id`, which identifies the business message.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SendMessageOutcome {
    pub message: Message,
    pub duplicate: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MessageListOptions {
    pub limit: usize,
    pub before: Option<String>,
    pub after: Option<String>,
}

impl Default for MessageListOptions {
    fn default() -> Self {
        Self {
            limit: 50,
            before: None,
            after: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MessagePage {
    /// Messages are returned oldest-to-newest within the page. The default
    /// page contains the newest messages, and `next_cursor` is passed as
    /// `before`; for an `after` request it is passed as `after` to continue
    /// walking toward newer messages.
    pub messages: Vec<Message>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImEvent {
    pub id: EventId,
    pub conversation_id: ConversationId,
    pub seq: i64,
    pub event_type: String,
    pub actor_id: Option<PrincipalId>,
    pub payload: Value,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EventPage {
    pub events: Vec<ImEvent>,
    pub next_cursor: Option<i64>,
}
