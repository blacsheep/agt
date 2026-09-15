use crate::{OutputMode, emit, parse_named_ref};
use agt_harness::{Codex, HarnessStore};
use agt_runtime::HarnessSessions;
use anyhow::Result;
use clap::Subcommand;
use std::path::Path;

#[derive(Debug, Subcommand)]
pub(crate) enum HarnessCommand {
    /// Supported engine operations, independent of terminal slash commands
    Capabilities,
    /// Create, edit and delete reusable Harness resources
    #[command(subcommand)]
    Resource(crate::HarnessResourceCommand),
    /// Models and reasoning levels reported by this resource's connection
    Models { reference: String },
    /// Inspect engine-managed skills for the configured project
    Skills { reference: String },
    /// Inspect engine-managed MCP servers
    Mcp { reference: String },
    #[command(subcommand)]
    Session(SessionCommand),
}
#[derive(Debug, Subcommand)]
pub(crate) enum SessionCommand {
    List,
    /// Cancel the active session operation; inspect/reset before resuming
    Cancel {
        id: String,
    },
    /// Get or prepare a snapshot for one Agent in one chat (does not start Codex)
    Current {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        conversation: String,
    },
    Info {
        id: String,
    },
    Models {
        id: String,
    },
    Skills {
        id: String,
    },
    Mcp {
        id: String,
    },
    /// Change only this session; applied to the next turn
    Configure {
        id: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        effort: Option<String>,
    },
    /// Set the user-facing thread name (engine session ID stays unchanged)
    Rename {
        id: String,
        title: String,
    },
    /// Compact via the engine and wait for completion; may invoke the model
    Compact {
        id: String,
    },
}
pub(crate) fn dispatch(
    command: HarnessCommand,
    home: &Path,
    output: OutputMode,
    yes: bool,
) -> Result<()> {
    match command {
        HarnessCommand::Resource(command) => {
            crate::dispatch_harness_resource(command, home, output, yes)
        }
        HarnessCommand::Capabilities => emit(&Codex::capabilities(), output),
        HarnessCommand::Models { reference } => {
            resource_operation(&reference, home, output, "models")
        }
        HarnessCommand::Skills { reference } => {
            resource_operation(&reference, home, output, "skills")
        }
        HarnessCommand::Mcp { reference } => resource_operation(&reference, home, output, "mcp"),
        HarnessCommand::Session(command) => {
            let store = HarnessSessions::open(home)?;
            match command {
                SessionCommand::Cancel { id } => {
                    store.cancel(&id)?;
                    emit(&serde_json::json!({"status":"cancel_requested"}), output)
                }
                SessionCommand::List => emit(&store.list()?, output),
                SessionCommand::Current {
                    principal,
                    conversation,
                } => emit(&store.current(home, &principal, &conversation)?, output),
                SessionCommand::Info { id } => emit(&store.get(&id)?, output),
                SessionCommand::Models { id } => {
                    emit(&Codex::models(&store.get(&id)?.config)?, output)
                }
                SessionCommand::Skills { id } => {
                    emit(&Codex::inspect(&store.get(&id)?.config, "skills")?, output)
                }
                SessionCommand::Mcp { id } => {
                    emit(&Codex::inspect(&store.get(&id)?.config, "mcp")?, output)
                }
                SessionCommand::Configure { id, model, effort } => {
                    emit(&store.configure(&id, &model, effort.as_deref())?, output)
                }
                SessionCommand::Rename { id, title } => emit(&store.rename(&id, &title)?, output),
                SessionCommand::Compact { id } => {
                    store.compact(&id, &crate::signals::cancellation())?;
                    emit(&serde_json::json!({"status":"completed"}), output)
                }
            }
        }
    }
}
fn resource_operation(
    reference: &str,
    home: &Path,
    output: OutputMode,
    operation: &str,
) -> Result<()> {
    let (c, n) = parse_named_ref(reference, None)?;
    let config = HarnessStore::new(home).read_in(&c, &n)?.config;
    emit(
        &if operation == "models" {
            Codex::models(&config)?
        } else {
            Codex::inspect(&config, operation)?
        },
        output,
    )
}
