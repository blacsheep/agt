use agt_agent::AgentStore;
use agt_im::{ConversationId, ImStore, PrincipalId, PrincipalKind};
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponsePolicy {
    #[default]
    Mentioned,
    All,
    Manual,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub principal: PrincipalId,
    pub agent: String,
    pub working_directory: PathBuf,
    #[serde(default)]
    pub policy: ResponsePolicy,
    /// Legacy field retained for saved bindings; no longer limits execution.
    #[serde(default)]
    pub max_steps: u32,
    #[serde(default)]
    pub allow_mcp: bool,
}

#[derive(Serialize)]
pub struct BindingStatus {
    #[serde(flatten)]
    pub binding: Binding,
    pub enabled: bool,
}

pub struct BindingStore {
    connection: Connection,
    home: PathBuf,
}

impl BindingStore {
    pub fn open(home: &Path) -> Result<Self> {
        let mut connection =
            Connection::open(crate::database_path(home, "runtime", "bindings.db")?)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: u32 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => tx.execute_batch("CREATE TABLE bindings(principal TEXT PRIMARY KEY, agent TEXT NOT NULL, definition TEXT NOT NULL);
                CREATE TABLE response_policies(conversation TEXT NOT NULL,principal TEXT NOT NULL,policy TEXT NOT NULL,PRIMARY KEY(conversation,principal)); PRAGMA user_version=1;")?,
            1..=4 => {},
            _ => bail!("unsupported bindings database version"),
        }
        if version < 2 {
            tx.execute_batch("ALTER TABLE bindings ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1; PRAGMA user_version=2;")?;
        }
        if version < 3 {
            tx.execute_batch("CREATE TABLE IF NOT EXISTS binding_origins(principal TEXT PRIMARY KEY, agent TEXT NOT NULL);
                INSERT OR REPLACE INTO binding_origins SELECT principal,agent FROM bindings;
                PRAGMA user_version=3;")?;
        }
        if version < 4 {
            tx.execute_batch("CREATE TABLE IF NOT EXISTS change_outbox(seq INTEGER PRIMARY KEY AUTOINCREMENT,conversation TEXT,kind TEXT NOT NULL,at INTEGER NOT NULL);
                CREATE TRIGGER IF NOT EXISTS binding_added AFTER INSERT ON bindings BEGIN INSERT INTO change_outbox(kind,at) VALUES('access',CAST(unixepoch('subsec')*1000 AS INTEGER)); END;
                CREATE TRIGGER IF NOT EXISTS binding_changed AFTER UPDATE ON bindings BEGIN INSERT INTO change_outbox(kind,at) VALUES('access',CAST(unixepoch('subsec')*1000 AS INTEGER)); END;
                CREATE TRIGGER IF NOT EXISTS binding_removed AFTER DELETE ON bindings BEGIN INSERT INTO change_outbox(kind,at) VALUES('access',CAST(unixepoch('subsec')*1000 AS INTEGER)); END;
                CREATE TRIGGER IF NOT EXISTS policy_added AFTER INSERT ON response_policies BEGIN INSERT INTO change_outbox(conversation,kind,at) VALUES(NEW.conversation,'binding',CAST(unixepoch('subsec')*1000 AS INTEGER)); END;
                CREATE TRIGGER IF NOT EXISTS policy_changed AFTER UPDATE ON response_policies BEGIN INSERT INTO change_outbox(conversation,kind,at) VALUES(NEW.conversation,'binding',CAST(unixepoch('subsec')*1000 AS INTEGER)); END;
                PRAGMA user_version=4;")?;
        }
        tx.commit()?;
        Ok(Self {
            connection,
            home: home.to_owned(),
        })
    }

    pub fn bind(&self, mut binding: Binding) -> Result<Binding> {
        let (category, name) = crate::named_ref(&binding.agent)?;
        let agent = AgentStore::new(&self.home)
            .read_in(&category, &name)?
            .config;
        if let Some(harness) = agent.harness {
            let harness = agt_harness::HarnessStore::new(&self.home)
                .read_in(harness.category(), harness.name().as_str())?
                .config;
            harness.validate()?;
            binding.working_directory = harness.working_directory;
            if binding.policy == ResponsePolicy::All {
                binding.policy = ResponsePolicy::Mentioned;
            }
            binding.allow_mcp = false;
        }
        binding.agent = format!("{category}/{name}");
        let im = ImStore::open(crate::database_path(&self.home, "im", "im.db")?)?;
        if im.get_principal(&binding.principal)?.kind != PrincipalKind::Agent {
            bail!("only Agent principals can be hosted");
        }
        binding.working_directory = std::fs::canonicalize(&binding.working_directory)?;
        if !binding.working_directory.is_dir() {
            bail!("invalid binding working directory");
        }
        let tx = self.connection.unchecked_transaction()?;
        tx.execute("INSERT INTO bindings(principal,agent,definition) VALUES(?1,?2,?3)
            ON CONFLICT(principal) DO UPDATE SET agent=excluded.agent,definition=excluded.definition",
            params![binding.principal.as_str(),binding.agent,serde_json::to_string(&binding)?])?;
        tx.execute(
            "INSERT INTO binding_origins(principal,agent) VALUES(?1,?2)
            ON CONFLICT(principal) DO UPDATE SET agent=excluded.agent",
            params![binding.principal.as_str(), binding.agent],
        )?;
        tx.commit()?;
        agt_storage::wake::notify(&self.home);
        Ok(binding)
    }

    /// Last binding survives unbind so deleting the resource can retire its identity.
    pub fn origins(&self) -> Result<std::collections::BTreeMap<String, String>> {
        let mut statement = self
            .connection
            .prepare("SELECT principal,agent FROM binding_origins")?;
        Ok(statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?)
    }

    pub fn get(&self, principal: &PrincipalId) -> Result<Option<Binding>> {
        let definition: Option<String> = self
            .connection
            .query_row(
                "SELECT definition FROM bindings WHERE principal=?1 AND enabled=1",
                [principal.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        definition
            .map(|s| Ok(serde_json::from_str(&s)?))
            .transpose()
    }
    pub fn list(&self) -> Result<Vec<Binding>> {
        let mut statement = self
            .connection
            .prepare("SELECT definition FROM bindings WHERE enabled=1 ORDER BY principal")?;
        statement
            .query_map([], |r| r.get::<_, String>(0))?
            .map(|r| Ok(serde_json::from_str(&r?)?))
            .collect()
    }
    pub fn list_with_state(&self) -> Result<Vec<BindingStatus>> {
        let mut statement = self
            .connection
            .prepare("SELECT definition,enabled FROM bindings ORDER BY principal")?;
        statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))?
            .map(|row| {
                let (definition, enabled) = row?;
                Ok(BindingStatus {
                    binding: serde_json::from_str(&definition)?,
                    enabled,
                })
            })
            .collect()
    }
    pub fn set_enabled(&self, principal: &PrincipalId, enabled: bool) -> Result<()> {
        if enabled {
            let binding = self
                .list_with_state()?
                .into_iter()
                .find(|b| &b.binding.principal == principal)
                .ok_or_else(|| anyhow::anyhow!("Agent has no saved running settings"))?
                .binding;
            self.bind(binding)?;
        }
        if self.connection.execute(
            "UPDATE bindings SET enabled=?2 WHERE principal=?1",
            params![principal.as_str(), enabled],
        )? == 0
        {
            bail!("Agent has no saved running settings");
        }
        agt_storage::wake::notify(&self.home);
        Ok(())
    }
    pub fn unbind(&self, principal: &PrincipalId) -> Result<()> {
        self.connection.execute(
            "DELETE FROM bindings WHERE principal=?1",
            [principal.as_str()],
        )?;
        agt_storage::wake::notify(&self.home);
        Ok(())
    }
    pub fn set_policy(
        &self,
        conversation: &ConversationId,
        principal: &PrincipalId,
        policy: ResponsePolicy,
    ) -> Result<()> {
        let im = ImStore::open(crate::database_path(&self.home, "im", "im.db")?)?;
        if im
            .get_member(conversation, principal)?
            .left_message_seq
            .is_some()
        {
            bail!("not an active member");
        }
        self.connection.execute("INSERT INTO response_policies VALUES(?1,?2,?3) ON CONFLICT(conversation,principal) DO UPDATE SET policy=excluded.policy",
            params![conversation.as_str(),principal.as_str(),serde_json::to_string(&policy)?])?;
        agt_storage::wake::notify(&self.home);
        Ok(())
    }
    pub fn policy(
        &self,
        conversation: &ConversationId,
        binding: &Binding,
    ) -> Result<ResponsePolicy> {
        let policy: Option<String> = self
            .connection
            .query_row(
                "SELECT policy FROM response_policies WHERE conversation=?1 AND principal=?2",
                params![conversation.as_str(), binding.principal.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match policy {
            Some(value) => serde_json::from_str(&value)?,
            None => binding.policy,
        })
    }
}
