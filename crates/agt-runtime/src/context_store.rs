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
pub(crate) struct ContextStore {
    connection: Connection,
}
impl ContextStore {
    pub fn open(home: &Path) -> Result<Self> {
        let connection =
            Connection::open(crate::store::database_path(home, "runtime", "contexts.db")?)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("CREATE TABLE IF NOT EXISTS conversation_summaries(conversation TEXT NOT NULL,principal TEXT NOT NULL,record TEXT NOT NULL,PRIMARY KEY(conversation,principal));")?;
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
    /// Caller holds the scheduler fence and revalidates source visibility after inference.
    pub fn save(
        &self,
        conversation: &str,
        principal: &str,
        summary: &ConversationSummary,
    ) -> Result<()> {
        self.connection.execute("INSERT INTO conversation_summaries VALUES(?1,?2,?3) ON CONFLICT(conversation,principal) DO UPDATE SET record=excluded.record", params![conversation,principal,serde_json::to_string(summary)?])?;
        Ok(())
    }
    pub fn clear(&self, conversation: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM conversation_summaries WHERE conversation=?1",
            [conversation],
        )?;
        Ok(())
    }
}
