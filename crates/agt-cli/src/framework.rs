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
#[derive(Debug, Args)]
pub(crate) struct WorkerArgs {
    /// Keep polling; omit to process one finite batch
    #[arg(long)]
    watch: bool,
    /// Maximum deliveries per bound participant in each batch
    #[arg(long,default_value_t=50,value_parser=clap::value_parser!(u32).range(1..=200))]
    limit: u32,
    /// Permit MCP execution only for bindings that also explicitly allow it
    #[arg(long)]
    allow_mcp: bool,
}
pub(crate) fn worker(args: WorkerArgs, home: &Path, output: OutputMode) -> Result<()> {
    let mut dispatcher = agt_runtime::Dispatcher::new(home);
    dispatcher.allow_mcp = args.allow_mcp;
    let mut model = agt_runtime::ChatCompletions;
    loop {
        let outcomes = dispatcher.drain(
            &mut model,
            &agt_mcp::DefaultMcpClientFactory,
            args.limit,
            &agt_core::CancellationToken::new(),
        )?;
        if !outcomes.is_empty() || !args.watch {
            emit(&outcomes, output)?;
        }
        if !args.watch {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    Ok(())
}
