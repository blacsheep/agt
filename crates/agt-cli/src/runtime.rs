use std::io::{self, Read};
use std::path::{Path, PathBuf};

use agt_core::CancellationToken;
use agt_mcp::DefaultMcpClientFactory;
use agt_runtime::{ChatCompletions, RunRequest, RunState, RunStore, Runtime};
use anyhow::{Result, bail};
use clap::{Args, Subcommand};

use crate::{OutputMode, emit};

#[derive(Debug, Args)]
#[group(skip)]
#[command(group(clap::ArgGroup::new("run_input").required(true).args(["input", "stdin"])))]
pub(crate) struct AgentRunArgs {
    agent: String,
    #[arg(long)]
    input: Option<String>,
    #[arg(long)]
    stdin: bool,
    /// Allowed delegate agent references (repeatable); children cannot delegate
    #[arg(long = "delegate")]
    delegates: Vec<String>,
    /// Reuse this key only for the exact same request
    #[arg(long)]
    request_key: Option<String>,
    /// Legacy compatibility option; ignored (execution has no step limit)
    #[arg(long, hide = true, default_value_t = 0)]
    max_steps: u32,
    /// Working directory for attached MCP servers
    #[arg(long, default_value = ".")]
    cwd: PathBuf,
}

#[derive(Debug, Subcommand)]
pub(crate) enum RunCommand {
    List {
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=200))]
        limit: u32,
    },
    Info {
        id: String,
    },
    Children {
        id: String,
    },
    /// Inspect redacted model/tool invocation metadata (not private transcripts)
    Calls {
        id: String,
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(i64).range(0..))]
        after: i64,
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=200))]
        limit: u32,
    },
}

pub(crate) fn execute(args: AgentRunArgs, home: &Path, output: OutputMode) -> Result<()> {
    let input = if args.stdin {
        let mut text = String::new();
        io::stdin().take(256 * 1024 + 1).read_to_string(&mut text)?;
        text
    } else {
        args.input.expect("clap requires input or stdin")
    };
    let mut request = RunRequest::new(args.agent, input);
    request.delegates = args.delegates;
    request.max_steps = args.max_steps;
    request.working_directory = args.cwd;
    if let Some(key) = args.request_key {
        request.request_key = key;
    }
    let mut model = ChatCompletions;
    let mut runtime = Runtime::open(home, &mut model, &DefaultMcpClientFactory)?;
    let record = runtime.run(request, &CancellationToken::new())?;
    emit(&record, output)?;
    if record.state != RunState::Completed {
        bail!(
            "run {} is {:?}; inspect with `agt run info {}`",
            record.id,
            record.state,
            record.id
        );
    }
    Ok(())
}

pub(crate) fn inspect(command: RunCommand, home: &Path, output: OutputMode) -> Result<()> {
    let store = RunStore::open(home)?;
    match command {
        RunCommand::List { limit } => emit(&store.list(limit)?, output),
        RunCommand::Info { id } => emit(&store.get(&id)?, output),
        RunCommand::Calls { id, after, limit } => emit(&store.calls(&id, after, limit)?, output),
        RunCommand::Children { id } => {
            store.get(&id)?;
            emit(&store.children(&id)?, output)
        }
    }
}
