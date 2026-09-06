use thiserror::Error;

use crate::model::{ConversationId, EventId, MessageId, PrincipalId};

pub type Result<T> = std::result::Result<T, ImError>;

#[derive(Debug, Error)]
pub enum ImError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("I/O error while accessing {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("IM store lock was poisoned")]
    LockPoisoned,

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("invalid stored IM data: {0}")]
    InvalidStoredData(String),

    #[error("invalid pagination cursor: {0}")]
    InvalidCursor(String),

    #[error("principal not found: {0}")]
    PrincipalNotFound(PrincipalId),

    #[error("conversation not found: {0}")]
    ConversationNotFound(ConversationId),

    #[error("message not found: {0}")]
    MessageNotFound(MessageId),

    #[error("event not found: {0}")]
    EventNotFound(EventId),

    #[error("principal {principal} is not an active member of conversation {conversation}")]
    NotMember {
        principal: PrincipalId,
        conversation: ConversationId,
    },

    #[error("operation is not allowed: {0}")]
    PermissionDenied(String),

    #[error("resource already exists: {0}")]
    AlreadyExists(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("idempotency key was reused with a different request")]
    IdempotencyConflict,
}

impl ImError {
    pub(crate) fn io(path: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
