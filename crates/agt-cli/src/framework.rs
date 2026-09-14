use crate::{OutputMode, emit};
use agt_im::PrincipalId;
use agt_service::{Actor, Role, ToolService};
use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::Value;
use std::io;
use std::path::Path;

#[derive(Debug, Args)]
pub(crate) struct ServeArgs {
    /// Existing principal identity fixed by the host, never supplied by model tool arguments
    #[arg(long)]
    principal: String,
    /// Grant resource provisioning and binding management to this trusted local process
    #[arg(long)]
    manager: bool,
}
#[derive(Debug, Subcommand)]
pub(crate) enum ToolCommand {
    List {
        #[command(flatten)]
        identity: ServeArgs,
    },
    Call {
        #[command(flatten)]
        identity: ServeArgs,
        name: String,
        #[arg(long, default_value = "{}")]
        args: String,
    },
}
fn service(home: &Path, identity: ServeArgs) -> Result<ToolService> {
    ToolService::open(
        home,
        Actor {
            principal: PrincipalId::new(identity.principal),
            role: if identity.manager {
                Role::Manager
            } else {
                Role::Participant
            },
            automated_depth: None,
        },
    )
}
pub(crate) fn serve(args: ServeArgs, home: &Path) -> Result<()> {
    let service = service(home, args)?;
    agt_service::mcp_server::serve(&service, io::stdin().lock(), io::stdout().lock())
}
pub(crate) fn tool(command: ToolCommand, home: &Path, output: OutputMode) -> Result<()> {
    match command {
        ToolCommand::List { identity } => emit(&service(home, identity)?.schemas(), output),
        ToolCommand::Call {
            identity,
            name,
            args,
        } => {
            let args: Value = serde_json::from_str(&args).map_err(|_| {
                agt_service::ToolError::new("invalid_arguments", "Arguments must be valid JSON.")
            })?;
            match service(home, identity)?.call(&name, args) {
                Ok(result) => emit(&result, output),
                Err(error) => {
                    emit(&error.as_value(), output)?;
                    Err(error.into())
                }
            }
        }
    }
}
#[derive(Debug, Subcommand)]
pub(crate) enum ScheduleCommand {
    /// Run the coordinator; without --watch, drain currently ready work only
    Run {
        #[arg(long)]
        watch: bool,
        #[arg(long, default_value_t = 3)]
        concurrency: usize,
    },
    /// Authorize a human message, or continue pending discussion work
    Start {
        conversation: String,
        #[arg(long)]
        message: Option<String>,
        #[arg(long)]
        allow_mcp: bool,
    },
    /// Stop claiming work; current replies may finish
    Pause {
        conversation: String,
    },
    /// Cancel discussion replies and reminders
    End {
        conversation: String,
    },
    /// Permanently dissolve a group, retaining read-only history
    Dissolve {
        conversation: String,
        #[arg(long)]
        actor: String,
    },
    /// Soft-delete with IM permissions and cancel discussion work
    Delete {
        conversation: String,
        #[arg(long)]
        actor: String,
    },
    /// Explicitly renew the finite discussion allowance
    Resume {
        conversation: String,
        #[arg(long)]
        allow_mcp: bool,
    },
    /// Stop related work and reset model context; preserve IM history
    Reset {
        conversation: String,
        #[arg(long)]
        actor: String,
    },
    Status,
}

pub(crate) fn schedule(command: ScheduleCommand, home: &Path, output: OutputMode) -> Result<()> {
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    };
    match command {
        ScheduleCommand::Run { watch, concurrency } => {
            let cancellation = crate::signals::cancellation();
            let mut scheduler = loop {
                if cancellation.is_canceled() {
                    return Ok(());
                }
                match agt_runtime::Scheduler::open(home, concurrency) {
                    Ok(scheduler) => break scheduler,
                    Err(error) if watch && error.to_string().contains("owns this home") => {
                        std::thread::sleep(std::time::Duration::from_secs(1))
                    }
                    Err(error) => return Err(error),
                }
            };
            scheduler.run(watch, &cancellation)?;
        }
        ScheduleCommand::Start {
            conversation,
            message,
            allow_mcp,
        } => agt_runtime::start_discussion(home, &conversation, message.as_deref(), allow_mcp)?,
        ScheduleCommand::Pause { conversation } => {
            agt_runtime::ScheduleStore::open(home)?.control(&conversation, "pause", false, now())?
        }
        ScheduleCommand::End { conversation } => {
            agt_runtime::ScheduleStore::open(home)?.control(&conversation, "end", false, now())?
        }
        ScheduleCommand::Dissolve {
            conversation,
            actor,
        } => {
            agt_runtime::dissolve_scheduled_group(home, &conversation, &actor)?;
        }
        ScheduleCommand::Delete {
            conversation,
            actor,
        } => {
            return emit(
                &agt_runtime::delete_scheduled_conversation(home, &conversation, &actor)?,
                output,
            );
        }
        ScheduleCommand::Resume {
            conversation,
            allow_mcp,
        } => agt_runtime::ScheduleStore::open(home)?.control(
            &conversation,
            "resume",
            allow_mcp,
            now(),
        )?,
        ScheduleCommand::Reset {
            conversation,
            actor,
        } => {
            return emit(
                &agt_runtime::reset_conversation_context(home, &conversation, &actor)?,
                output,
            );
        }
        ScheduleCommand::Status => {}
    }
    emit(&agt_runtime::ScheduleStore::open(home)?.snapshot()?, output)
}
