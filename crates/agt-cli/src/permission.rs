use crate::{OutputMode, emit};
use anyhow::Result;
use clap::Subcommand;
use std::path::Path;
#[derive(Debug, Subcommand)]
pub(crate) enum PermissionCommand {
    List {
        #[arg(long)]
        principal: Option<String>,
        #[arg(long)]
        before: Option<String>,
    },
    Info {
        id: String,
        #[arg(long)]
        principal: Option<String>,
    },
    Decide {
        id: String,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        decision: String,
        #[arg(long, default_value = "")]
        reason: String,
        #[arg(long)]
        principal: Option<String>,
    },
    /// List saved "always allow" rules
    Rules,
    /// Remove a saved "always allow" rule
    Forget {
        #[arg(long)]
        agent: String,
        #[arg(long)]
        tool: String,
    },
}
pub(crate) fn dispatch(command: PermissionCommand, home: &Path, output: OutputMode) -> Result<()> {
    let store = agt_runtime::approval::ApprovalStore::open(home)?;
    match command {
        PermissionCommand::List { principal, before } => emit(
            &store.list(principal.as_deref(), before.as_deref())?,
            output,
        ),
        PermissionCommand::Info { id, principal } => {
            emit(&store.get(&id, principal.as_deref())?, output)
        }
        PermissionCommand::Decide {
            id,
            expected_revision,
            decision,
            reason,
            principal,
        } => emit(
            &store.decide(
                &id,
                expected_revision,
                serde_json::from_str(&decision)?,
                &reason,
                principal.as_deref(),
            )?,
            output,
        ),
        PermissionCommand::Rules => emit(&store.rules()?, output),
        PermissionCommand::Forget { agent, tool } => emit(
            &serde_json::json!({"removed": store.forget_rule(&agent, &tool)?}),
            output,
        ),
    }
}
