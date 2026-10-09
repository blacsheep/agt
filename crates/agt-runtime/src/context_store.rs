//! Private runtime summaries, separate from shared IM history and scheduler reset epochs.
use crate::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ConversationSummary {
    pub through_seq: i64,
    pub source_hash: String,
    pub agent_fingerprint: String,
    pub text: String,
}
pub(crate) const SCHEMA: agt_storage::sqlite::Schema = agt_storage::sqlite::Schema {
    module: "contexts",
    version: 1,
    ddl: "CREATE TABLE conversation_summaries(conversation TEXT NOT NULL,principal TEXT NOT NULL,record TEXT NOT NULL,PRIMARY KEY(conversation,principal));",
};
pub(crate) struct ContextStore {
    connection: agt_storage::sqlite::Db,
}
impl ContextStore {
    pub fn open(home: &Path) -> Result<Self> {
        let connection = crate::store::runtime_db(home).map_err(|e| {
            crate::RuntimeError::Configuration(crate::Reason::because(
                "runtime database unavailable",
                e,
            ))
        })?;
        Ok(Self { connection })
    }
    pub fn load(&self, conversation: &str, principal: &str) -> Result<Option<ConversationSummary>> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT record FROM conversation_summaries WHERE conversation=?1 AND principal=?2",
                params![conversation, principal],
                |r| r.get(0),
            )
            .optional()?;
        value.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    }
    /// Runs on the fence connection; the caller revalidates source visibility after inference.
    pub fn save(
        connection: &Connection,
        conversation: &str,
        principal: &str,
        summary: &ConversationSummary,
    ) -> Result<()> {
        connection.execute("INSERT INTO conversation_summaries VALUES(?1,?2,?3) ON CONFLICT(conversation,principal) DO UPDATE SET record=excluded.record", params![conversation,principal,serde_json::to_string(summary)?])?;
        Ok(())
    }
    /// Runs on the caller's connection so it commits with the reset epoch.
    pub fn clear(connection: &Connection, conversation: &str) -> Result<()> {
        connection.execute(
            "DELETE FROM conversation_summaries WHERE conversation=?1",
            [conversation],
        )?;
        Ok(())
    }
}
