use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use agt_agent::{AgentConfig, AgentError, AgentStore, AgentSummary, BuiltinTool};
use agt_config::{initialize_default_home, initialize_home};
use agt_core::CategoryName;
use agt_core::{McpId, ModelId, PackageRef, PromptId, QualifiedRef, ResourceRef};
use agt_im::{
    ConversationId, ImStore, MessageId, MessageListOptions, PrincipalId, PrincipalInput,
    PrincipalKind, SendMessageRequest,
};
use agt_mcp::{McpDocument, McpInfoResult, McpStore, McpSummary, McpWritePlan};
use agt_model::{
    ModelApiFormat, ModelConfig, ModelConfigUpdate, ModelReference, ModelStore, ModelSummary,
    ReasoningEffort,
};
use agt_prompt::{PromptContentResult, PromptStore, PromptSummary};
use agt_provider::{ProviderConfig, ProviderConfigUpdate, ProviderStore, ProviderSummary};
use agt_skill::{SkillCreateOptions, SkillDocument, SkillStore, SkillSummary};
use agt_storage::write_atomic;
use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::Value;

mod framework;
mod runtime;

#[derive(Debug, Parser)]
#[command(name = "agt", version, about = "CLI-first agent resource manager")]
struct Cli {
    /// agt data home (defaults to $HOME/.agt)
    #[arg(long, global = true, value_name = "DIR")]
    home: Option<PathBuf>,

    /// output format
    #[arg(long, global = true, value_enum, default_value_t = OutputMode::Human)]
    output: OutputMode,

    /// skip confirmation for destructive operations
    #[arg(long, global = true)]
    yes: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum OutputMode {
    #[default]
    Human,
    Json,
    Jsonl,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// tools for resource management and identity-scoped communication
    #[command(subcommand)]
    Tool(framework::ToolCommand),
    /// expose framework tools over MCP stdio (host-selected identity)
    Serve(framework::ServeArgs),
    /// persistent parallel discussion scheduling
    #[command(subcommand)]
    Schedule(framework::ScheduleCommand),
    /// initialize an explicitly selected agt home directory
    Init(InitArgs),
    /// manage provider definitions
    #[command(subcommand)]
    Provider(ProviderCommand),
    /// manage model definitions
    #[command(subcommand)]
    Model(ModelCommand),
    /// manage prompt definitions
    #[command(subcommand)]
    Prompt(PromptCommand),
    /// manage skill packages
    #[command(subcommand)]
    Skill(SkillCommand),
    /// manage MCP server definitions
    #[command(subcommand)]
    Mcp(McpCommand),
    /// manage agent definitions
    #[command(subcommand)]
    Agent(AgentCommand),
    /// inspect persisted agent executions
    #[command(subcommand)]
    Run(runtime::RunCommand),
    /// manage generic IM participants, conversations and messages
    #[command(subcommand, alias = "conversation")]
    Im(ImCommand),
}

#[derive(Debug, Args)]
struct InitArgs {
    /// target directory to initialize (for example: `agt init ~/my_agent_project`)
    #[arg(value_name = "DIR")]
    path: PathBuf,
}

#[derive(Debug, Subcommand)]
enum ProviderCommand {
    #[command(alias = "add")]
    Create(ProviderCreateArgs),
    #[command(alias = "ls")]
    List(ListArgs),
    #[command(alias = "show", alias = "get")]
    Info(RefArgs),
    #[command(alias = "edit")]
    Update(ProviderUpdateArgs),
    Test(RefArgs),
    ListModels(RefArgs),
    #[command(alias = "remove")]
    Delete(DeleteArgs),
    #[command(subcommand)]
    Category(CategoryCommand),
}

#[derive(Debug, Args)]
struct ProviderCreateArgs {
    name: String,
    #[arg(long, default_value = "openai-compatible")]
    r#type: String,
    #[arg(long)]
    base_url: String,
    /// pass the secret directly (prefer --api-key-stdin in shared shells)
    #[arg(long, conflicts_with_all = ["api_key_stdin", "api_key_env"])]
    api_key: Option<String>,
    #[arg(long, conflicts_with = "api_key_env")]
    api_key_stdin: bool,
    #[arg(long, conflicts_with = "api_key_stdin")]
    api_key_env: Option<String>,
    #[arg(long)]
    category: Option<String>,
}

#[derive(Debug, Args)]
struct ProviderUpdateArgs {
    name: String,
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long, conflicts_with_all = ["api_key_stdin", "api_key_env"])]
    api_key: Option<String>,
    #[arg(long, conflicts_with = "api_key_env")]
    api_key_stdin: bool,
    #[arg(long, conflicts_with = "api_key_stdin")]
    api_key_env: Option<String>,
    #[arg(long)]
    category: Option<String>,
}

#[derive(Debug, Subcommand)]
enum ModelCommand {
    #[command(alias = "add")]
    Create(ModelCreateArgs),
    #[command(alias = "ls")]
    List(ListArgs),
    #[command(alias = "show", alias = "get")]
    Info(RefArgs),
    #[command(alias = "edit", alias = "config")]
    Update(ModelUpdateArgs),
    #[command(alias = "remove")]
    Delete(DeleteArgs),
    #[command(subcommand)]
    Category(CategoryCommand),
}

#[derive(Debug, Args)]
struct ModelCreateArgs {
    name: String,
    #[arg(long)]
    provider: String,
    #[arg(long)]
    model: String,
    #[arg(long)]
    api_format: Option<String>,
    #[arg(long)]
    temperature: Option<f32>,
    #[arg(long)]
    max_tokens: Option<u32>,
    #[arg(long)]
    timeout: Option<u32>,
    #[arg(long)]
    reasoning_effort: Option<String>,
    #[arg(long)]
    tool_call: Option<bool>,
    /// Input context window used for automatic compaction; not an output token limit
    #[arg(long)]
    context_window: Option<u32>,
    #[arg(long)]
    category: Option<String>,
}

#[derive(Debug, Args)]
struct ModelUpdateArgs {
    name: String,
    #[arg(long)]
    category: Option<String>,
    #[arg(long)]
    api_format: Option<String>,
    #[arg(long)]
    temperature: Option<f32>,
    #[arg(long)]
    max_tokens: Option<u32>,
    #[arg(long)]
    clear_max_tokens: bool,
    #[arg(long)]
    timeout: Option<u32>,
    #[arg(long)]
    reasoning_effort: Option<String>,
    #[arg(long)]
    clear_reasoning_effort: bool,
    #[arg(long)]
    tool_call: Option<bool>,
    /// Input context window used for automatic compaction; not an output token limit
    #[arg(long)]
    context_window: Option<u32>,
}

#[derive(Debug, Subcommand)]
enum PromptCommand {
    #[command(alias = "add")]
    Create(PromptWriteArgs),
    #[command(alias = "ls")]
    List(ListArgs),
    #[command(alias = "show", alias = "get")]
    Info(RefArgs),
    Read(RefArgs),
    #[command(alias = "edit")]
    Update(PromptWriteArgs),
    #[command(alias = "remove")]
    Delete(DeleteArgs),
    #[command(subcommand)]
    Category(CategoryCommand),
}

#[derive(Debug, Args)]
struct PromptWriteArgs {
    name: String,
    #[arg(long)]
    category: Option<String>,
    #[arg(long, conflicts_with_all = ["file", "stdin"])]
    text: Option<String>,
    #[arg(long, conflicts_with_all = ["text", "stdin"])]
    file: Option<PathBuf>,
    #[arg(long, conflicts_with_all = ["text", "file"])]
    stdin: bool,
}

#[derive(Debug, Subcommand)]
enum SkillCommand {
    #[command(alias = "add")]
    Create(SkillCreateArgs),
    #[command(alias = "ls")]
    List(ListArgs),
    #[command(alias = "show", alias = "get")]
    Info(SkillRefArgs),
    Read(SkillRefArgs),
    #[command(alias = "edit")]
    Update(SkillUpdateArgs),
    #[command(alias = "remove")]
    Delete(SkillDeleteArgs),
    #[command(subcommand)]
    Category(CategoryCommand),
}

#[derive(Debug, Args)]
struct SkillCreateArgs {
    package: String,
    #[arg(long)]
    description: String,
    #[arg(long)]
    category: Option<String>,
}

#[derive(Debug, Args)]
struct SkillUpdateArgs {
    package: String,
    /// a SKILL.md file or a directory containing it
    #[arg(long)]
    file: PathBuf,
    #[arg(long)]
    category: Option<String>,
}

#[derive(Debug, Args)]
struct SkillDeleteArgs {
    package: String,
    #[arg(long)]
    category: Option<String>,
    #[arg(long)]
    recursive: bool,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Args)]
struct SkillRefArgs {
    package: String,
    #[arg(long)]
    category: Option<String>,
}

#[derive(Debug, Subcommand)]
enum McpCommand {
    #[command(alias = "add")]
    Create(McpWriteArgs),
    #[command(alias = "ls")]
    List(ListArgs),
    #[command(alias = "show", alias = "get")]
    Info(RefArgs),
    #[command(alias = "edit")]
    Update(McpWriteArgs),
    Test(RefArgs),
    #[command(alias = "remove")]
    Delete(DeleteArgs),
    #[command(subcommand)]
    Category(CategoryCommand),
}

#[derive(Debug, Args)]
struct McpWriteArgs {
    name: String,
    #[arg(long)]
    file: PathBuf,
    #[arg(long)]
    category: Option<String>,
}

#[derive(Debug, Subcommand)]
enum AgentCommand {
    /// execute an agent with optional explicit delegation
    Run(runtime::AgentRunArgs),
    #[command(alias = "add")]
    Create(AgentCreateArgs),
    #[command(alias = "ls")]
    List(ListArgs),
    #[command(alias = "show", alias = "get")]
    Info(RefArgs),
    #[command(alias = "edit")]
    Update(AgentUpdateArgs),
    #[command(alias = "remove")]
    Delete(DeleteArgs),
    #[command(subcommand)]
    Category(CategoryCommand),
}

#[derive(Debug, Args)]
struct AgentCreateArgs {
    name: String,
    #[arg(long = "builtin-tool")]
    builtin_tools: Vec<BuiltinTool>,
    #[arg(long)]
    model: String,
    #[arg(long)]
    prompt: Option<String>,
    #[arg(long = "skill")]
    skills: Vec<String>,
    #[arg(long = "mcp")]
    mcps: Vec<String>,
    #[arg(long)]
    description: Option<String>,
    #[arg(long)]
    category: Option<String>,
}

#[derive(Debug, Args)]
struct AgentUpdateArgs {
    name: String,
    #[arg(long = "add-builtin-tool")]
    add_builtin_tools: Vec<BuiltinTool>,
    #[arg(long = "remove-builtin-tool")]
    remove_builtin_tools: Vec<BuiltinTool>,
    #[arg(long)]
    category: Option<String>,
    #[arg(long)]
    description: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    prompt: Option<String>,
    #[arg(long = "add-skill")]
    add_skills: Vec<String>,
    #[arg(long = "remove-skill")]
    remove_skills: Vec<String>,
    #[arg(long = "add-mcp")]
    add_mcps: Vec<String>,
    #[arg(long = "remove-mcp")]
    remove_mcps: Vec<String>,
}

#[derive(Debug, Subcommand)]
enum ImCommand {
    /// configure the persistent human owner for Agent-created groups
    #[command(subcommand)]
    Owner(ImOwnerCommand),
    /// manage participants (users, agents, bots or services)
    #[command(subcommand)]
    Principal(ImPrincipalCommand),
    /// create or resolve a canonical two-party conversation
    #[command(subcommand)]
    Direct(ImDirectCommand),
    /// create a group and manage its membership
    #[command(subcommand)]
    Group(ImGroupCommand),
    /// list, inspect or archive conversations
    #[command(subcommand)]
    Conversation(ImConversationCommand),
    /// send and inspect messages
    #[command(subcommand, alias = "msg")]
    Message(ImMessageCommand),
}

#[derive(Debug, Subcommand)]
enum ImOwnerCommand {
    Get,
    /// Set only if absent, then migrate existing Agent-owned groups
    Ensure(ImIdArgs),
}

#[derive(Debug, Subcommand)]
enum ImPrincipalCommand {
    #[command(alias = "add")]
    Create(ImPrincipalCreateArgs),
    #[command(alias = "ls")]
    List,
    #[command(alias = "show", alias = "get")]
    Info(ImIdArgs),
}

#[derive(Debug, Args)]
struct ImPrincipalCreateArgs {
    id: String,
    #[arg(long)]
    name: Option<String>,
    #[arg(long, default_value = "user")]
    kind: String,
}

#[derive(Debug, Subcommand)]
enum ImDirectCommand {
    #[command(alias = "add")]
    Create(ImDirectCreateArgs),
}

#[derive(Debug, Args)]
struct ImDirectCreateArgs {
    first: String,
    second: String,
}

#[derive(Debug, Subcommand)]
enum ImGroupCommand {
    Dissolve(ImConversationActorArgs),
    #[command(alias = "add")]
    Create(ImGroupCreateArgs),
    #[command(subcommand)]
    Members(ImMembersCommand),
}

#[derive(Debug, Args)]
struct ImGroupCreateArgs {
    #[arg(long)]
    owner: String,
    #[arg(long)]
    title: Option<String>,
    #[arg(long = "member")]
    members: Vec<String>,
}

#[derive(Debug, Subcommand)]
enum ImMembersCommand {
    List(ImConversationIdArgs),
    Add(ImMemberChangeArgs),
    Remove(ImMemberChangeArgs),
    Leave(ImMemberLeaveArgs),
}

#[derive(Debug, Args)]
struct ImConversationIdArgs {
    conversation: String,
}

#[derive(Debug, Args)]
struct ImMemberChangeArgs {
    conversation: String,
    principal: String,
    #[arg(long)]
    actor: String,
}

#[derive(Debug, Args)]
struct ImMemberLeaveArgs {
    conversation: String,
    principal: String,
}

#[derive(Debug, Subcommand)]
enum ImConversationCommand {
    #[command(alias = "ls")]
    List(ImConversationListArgs),
    #[command(alias = "show", alias = "get")]
    Info(ImIdArgs),
    Hide(ImConversationActorArgs),
    Unhide(ImConversationActorArgs),
    /// Record an explicit visit without changing messages or membership
    Visit(ImConversationActorArgs),
    Archive(ImConversationActorArgs),
    #[command(alias = "restore")]
    Reactivate(ImConversationActorArgs),
    #[command(alias = "remove")]
    Delete(ImConversationActorArgs),
}

#[derive(Debug, Args)]
struct ImConversationListArgs {
    #[arg(long)]
    include_hidden: bool,
    #[arg(long)]
    groups_only: bool,
    #[arg(long)]
    principal: String,
}

#[derive(Debug, Args)]
struct ImConversationActorArgs {
    conversation: String,
    #[arg(long)]
    actor: String,
}

#[derive(Debug, Args)]
struct ImIdArgs {
    id: String,
}

#[derive(Debug, Subcommand)]
enum ImMessageCommand {
    Send(ImMessageSendArgs),
    #[command(alias = "ls", alias = "history")]
    List(ImMessageListArgs),
    Delete(ImMessageDeleteArgs),
    Read(ImMessageReadArgs),
}

#[derive(Debug, Args)]
struct ImMessageSendArgs {
    conversation: String,
    #[arg(long)]
    sender: String,
    body: String,
    #[arg(long)]
    client_message_id: Option<String>,
    #[arg(long)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Args)]
struct ImMessageListArgs {
    conversation: String,
    #[arg(long)]
    viewer: String,
    #[arg(long, default_value_t = 50)]
    limit: usize,
    #[arg(long)]
    before: Option<String>,
    #[arg(long)]
    after: Option<String>,
}

#[derive(Debug, Args)]
struct ImMessageDeleteArgs {
    message: String,
    #[arg(long)]
    actor: String,
}

#[derive(Debug, Args)]
struct ImMessageReadArgs {
    conversation: String,
    #[arg(long)]
    principal: String,
    #[arg(long)]
    through_seq: i64,
}

#[derive(Debug, Args)]
struct ListArgs {
    #[arg(long)]
    category: Option<String>,
}

#[derive(Debug, Args)]
struct RefArgs {
    /// name or category/name
    name: String,
    #[arg(long)]
    category: Option<String>,
}

#[derive(Debug, Args)]
struct DeleteArgs {
    /// name or category/name
    name: String,
    #[arg(long)]
    category: Option<String>,
    #[arg(long)]
    recursive: bool,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Subcommand)]
enum CategoryCommand {
    Create(CategoryArgs),
    List,
    Delete(CategoryArgs),
}

#[derive(Debug, Args)]
struct CategoryArgs {
    name: String,
}

use agt_service::deletion::{ResourceKey, ResourceKind};

fn main() -> ExitCode {
    match run_cli() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::from(1)
        }
    }
}

fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    let output = cli.output;
    if let Command::Init(args) = &cli.command {
        if let Some(global_home) = &cli.home
            && global_home != &args.path
        {
            bail!("agt init 的位置参数目录与全局 --home 不一致，请只指定一个目标目录");
        }
        let summary = initialize_home(Some(args.path.clone())).context("初始化 agt home 失败")?;
        let prompt_store = PromptStore::new(summary.home.clone());
        prompt_store
            .ensure_default_prompt()
            .context("初始化默认 Prompt 失败")?;
        return emit_value(
            serde_json::json!({
                "home": summary.home,
                "config_file": summary.config_file,
                "data_dirs": summary.data_dirs,
            }),
            output,
        );
    }

    let home = match cli.home {
        Some(home) => home,
        None => ensure_default_home_ready()?,
    };
    dispatch(cli.command, &home, output, cli.yes)
}

fn ensure_default_home_ready() -> Result<PathBuf> {
    let summary = initialize_default_home().context("初始化默认 agt home 失败")?;
    let default_prompt = summary
        .home
        .join("prompts")
        .join("default")
        .join("default")
        .join(agt_prompt::PROMPT_FILE);
    let default_prompt_exists = fs::symlink_metadata(&default_prompt)
        .map(|metadata| metadata.file_type().is_file())
        .unwrap_or(false);
    if !default_prompt_exists {
        PromptStore::new(summary.home.clone())
            .ensure_default_prompt()
            .context("初始化默认 Prompt 失败")?;
    }
    Ok(summary.home)
}

fn dispatch(command: Command, home: &Path, output: OutputMode, yes: bool) -> Result<()> {
    match command {
        Command::Tool(command) => framework::tool(command, home, output),
        Command::Serve(args) => framework::serve(args, home),
        Command::Schedule(command) => framework::schedule(command, home, output),
        Command::Provider(command) => dispatch_provider(command, home, output, yes),
        Command::Model(command) => dispatch_model(command, home, output, yes),
        Command::Prompt(command) => dispatch_prompt(command, home, output, yes),
        Command::Skill(command) => dispatch_skill(command, home, output, yes),
        Command::Mcp(command) => dispatch_mcp(command, home, output, yes),
        Command::Agent(command) => dispatch_agent(command, home, output, yes),
        Command::Run(command) => runtime::inspect(command, home, output),
        Command::Im(command) => dispatch_im(command, home, output),
        Command::Init(_) => unreachable!(),
    }
}

fn dispatch_im(command: ImCommand, home: &Path, output: OutputMode) -> Result<()> {
    let store = ImStore::open(home.join("im").join("im.db"))?;
    match command {
        ImCommand::Principal(command) => dispatch_im_principal(command, &store, output),
        ImCommand::Direct(command) => dispatch_im_direct(command, &store, output),
        ImCommand::Owner(command) => match command {
            ImOwnerCommand::Get => emit(&store.workspace_owner()?, output),
            ImOwnerCommand::Ensure(args) => emit(
                &store.ensure_workspace_owner(&PrincipalId::new(args.id))?,
                output,
            ),
        },
        ImCommand::Group(command) => dispatch_im_group(command, &store, output),
        ImCommand::Conversation(command) => dispatch_im_conversation(command, &store, output),
        ImCommand::Message(command) => dispatch_im_message(command, &store, output),
    }
}

fn dispatch_im_principal(
    command: ImPrincipalCommand,
    store: &ImStore,
    output: OutputMode,
) -> Result<()> {
    match command {
        ImPrincipalCommand::Create(args) => {
            let id = PrincipalId::new(args.id);
            let kind = parse_principal_kind(&args.kind)?;
            let principal = store.create_principal(PrincipalInput {
                display_name: args.name.unwrap_or_else(|| id.to_string()),
                id,
                kind,
                metadata: Value::Object(Default::default()),
            })?;
            emit(&principal, output)
        }
        ImPrincipalCommand::List => emit(&store.list_principals()?, output),
        ImPrincipalCommand::Info(args) => {
            emit(&store.get_principal(&PrincipalId::new(args.id))?, output)
        }
    }
}

fn dispatch_im_direct(command: ImDirectCommand, store: &ImStore, output: OutputMode) -> Result<()> {
    match command {
        ImDirectCommand::Create(args) => emit(
            &store.create_direct(
                &PrincipalId::new(args.first),
                &PrincipalId::new(args.second),
            )?,
            output,
        ),
    }
}

fn dispatch_im_group(command: ImGroupCommand, store: &ImStore, output: OutputMode) -> Result<()> {
    match command {
        ImGroupCommand::Create(args) => {
            let members = args.members.into_iter().map(PrincipalId::new).collect();
            emit(
                &store.create_group(&PrincipalId::new(args.owner), args.title, members)?,
                output,
            )
        }
        ImGroupCommand::Dissolve(args) => {
            store.dissolve_group(
                &ConversationId::new(args.conversation),
                &PrincipalId::new(args.actor),
            )?;
            emit(&serde_json::json!({"dissolved":true}), output)
        }
        ImGroupCommand::Members(command) => dispatch_im_members(command, store, output),
    }
}

fn dispatch_im_members(
    command: ImMembersCommand,
    store: &ImStore,
    output: OutputMode,
) -> Result<()> {
    match command {
        ImMembersCommand::List(args) => emit(
            &store.list_members(&ConversationId::new(args.conversation))?,
            output,
        ),
        ImMembersCommand::Add(args) => emit(
            &store.add_member(
                &ConversationId::new(args.conversation),
                &PrincipalId::new(args.actor),
                &PrincipalId::new(args.principal),
            )?,
            output,
        ),
        ImMembersCommand::Remove(args) => emit(
            &store.remove_member(
                &ConversationId::new(args.conversation),
                &PrincipalId::new(args.actor),
                &PrincipalId::new(args.principal),
            )?,
            output,
        ),
        ImMembersCommand::Leave(args) => emit(
            &store.leave_conversation(
                &ConversationId::new(args.conversation),
                &PrincipalId::new(args.principal),
            )?,
            output,
        ),
    }
}

fn dispatch_im_conversation(
    command: ImConversationCommand,
    store: &ImStore,
    output: OutputMode,
) -> Result<()> {
    match command {
        ImConversationCommand::List(args) => emit(
            &store.list_conversation_entries(
                &PrincipalId::new(args.principal),
                args.include_hidden,
                args.groups_only,
            )?,
            output,
        ),
        ImConversationCommand::Info(args) => emit(
            &store.get_conversation(&ConversationId::new(args.id))?,
            output,
        ),
        ImConversationCommand::Hide(args) => {
            store.set_conversation_hidden(
                &ConversationId::new(args.conversation),
                &PrincipalId::new(args.actor),
                true,
            )?;
            emit(&serde_json::json!({"hidden":true}), output)
        }
        ImConversationCommand::Unhide(args) => {
            store.set_conversation_hidden(
                &ConversationId::new(args.conversation),
                &PrincipalId::new(args.actor),
                false,
            )?;
            emit(&serde_json::json!({"hidden":false}), output)
        }
        ImConversationCommand::Visit(args) => {
            let viewed = store.mark_conversation_viewed(
                &ConversationId::new(args.conversation),
                &PrincipalId::new(args.actor),
            )?;
            emit(&serde_json::json!({"last_viewed_at":viewed}), output)
        }
        ImConversationCommand::Archive(args) => emit(
            &store.archive_conversation(
                &ConversationId::new(args.conversation),
                &PrincipalId::new(args.actor),
            )?,
            output,
        ),
        ImConversationCommand::Reactivate(args) => emit(
            &store.reactivate_conversation(
                &ConversationId::new(args.conversation),
                &PrincipalId::new(args.actor),
            )?,
            output,
        ),
        ImConversationCommand::Delete(args) => emit(
            &store.delete_conversation(
                &ConversationId::new(args.conversation),
                &PrincipalId::new(args.actor),
            )?,
            output,
        ),
    }
}

fn dispatch_im_message(
    command: ImMessageCommand,
    store: &ImStore,
    output: OutputMode,
) -> Result<()> {
    match command {
        ImMessageCommand::Send(args) => emit(
            &store.send_message(SendMessageRequest {
                conversation_id: ConversationId::new(args.conversation),
                sender_id: PrincipalId::new(args.sender),
                body: args.body,
                client_message_id: args.client_message_id,
                reply_to_id: None,
                metadata: Value::Object(Default::default()),
                idempotency_key: args.idempotency_key,
            })?,
            output,
        ),
        ImMessageCommand::List(args) => emit(
            &store.list_messages(
                &ConversationId::new(args.conversation),
                &PrincipalId::new(args.viewer),
                MessageListOptions {
                    limit: args.limit,
                    before: args.before,
                    after: args.after,
                },
            )?,
            output,
        ),
        ImMessageCommand::Delete(args) => emit(
            &store.delete_message(&MessageId::new(args.message), &PrincipalId::new(args.actor))?,
            output,
        ),
        ImMessageCommand::Read(args) => emit(
            &store.mark_read(
                &ConversationId::new(args.conversation),
                &PrincipalId::new(args.principal),
                args.through_seq,
            )?,
            output,
        ),
    }
}

fn parse_principal_kind(raw: &str) -> Result<PrincipalKind> {
    match raw {
        "user" => Ok(PrincipalKind::User),
        "agent" => Ok(PrincipalKind::Agent),
        "bot" => Ok(PrincipalKind::Bot),
        "service" => Ok(PrincipalKind::Service),
        "other" => Ok(PrincipalKind::Other),
        _ => bail!("不支持的 principal kind：{raw}（可选 user、agent、bot、service、other）"),
    }
}

fn dispatch_provider(
    command: ProviderCommand,
    home: &Path,
    output: OutputMode,
    yes: bool,
) -> Result<()> {
    let store = ProviderStore::new(home);
    match command {
        ProviderCommand::Create(args) => {
            let category = parse_category(args.category.as_deref())?;
            let api_key = read_secret(args.api_key, args.api_key_stdin, args.api_key_env)?;
            if args.r#type != "openai-compatible" {
                bail!("当前只支持 provider type=openai-compatible");
            }
            let config = ProviderConfig::openai_compatible(args.name, args.base_url, api_key)?;
            let stored = store.create_in(&category, &config)?;
            emit(&ProviderSummary::from(&stored), output)
        }
        ProviderCommand::List(args) => {
            let result = match args.category.as_deref() {
                Some(category) => store.scan_summary_in(&parse_category(Some(category))?)?,
                None => store.scan_summary()?,
            };
            emit(&result, output)
        }
        ProviderCommand::Info(args) => {
            let (category, name) = parse_named_ref(&args.name, args.category.as_deref())?;
            let stored = store.read_in(&category, &name)?;
            emit(&ProviderSummary::from(&stored), output)
        }
        ProviderCommand::Update(args) => {
            let (category, name) = parse_named_ref(&args.name, args.category.as_deref())?;
            let api_key =
                if args.api_key.is_some() || args.api_key_stdin || args.api_key_env.is_some() {
                    read_secret(args.api_key, args.api_key_stdin, args.api_key_env)?
                } else {
                    String::new()
                };
            let update = ProviderConfigUpdate {
                base_url: args.base_url,
                api_key: if api_key.is_empty() {
                    None
                } else {
                    Some(api_key)
                },
            };
            let stored = store.update_in(&category, &name, update)?;
            emit(&ProviderSummary::from(&stored), output)
        }
        ProviderCommand::Test(args) => {
            let (category, name) = parse_named_ref(&args.name, args.category.as_deref())?;
            let provider = store.read_in(&category, &name)?;
            provider.config.validate()?;
            emit(&ProviderSummary::from(&provider), output)
        }
        ProviderCommand::ListModels(args) => {
            let (category, name) = parse_named_ref(&args.name, args.category.as_deref())?;
            let result = store.list_models_summary_in(&category, &name)?;
            emit(&result, output)
        }
        ProviderCommand::Delete(args) => delete_resource(
            ResourceKind::Provider,
            &args.name,
            args.category.as_deref(),
            args.recursive,
            args.dry_run,
            home,
            output,
            yes,
        ),
        ProviderCommand::Category(command) => {
            category_command(ResourceKind::Provider, command, home, output)
        }
    }
}

fn dispatch_model(command: ModelCommand, home: &Path, output: OutputMode, yes: bool) -> Result<()> {
    let store = ModelStore::new(home);
    match command {
        ModelCommand::Create(args) => {
            let category = parse_category(args.category.as_deref())?;
            let (provider_category, provider_name) = parse_named_ref(&args.provider, None)?;
            let reference = ModelReference::new_in(provider_category, provider_name, args.model)?;
            let mut config = ModelConfig::new(args.name, reference)?;
            let update = model_update_from(
                args.api_format,
                args.temperature,
                args.max_tokens,
                false,
                args.timeout,
                args.reasoning_effort,
                false,
                args.tool_call,
                args.context_window,
            )?;
            if !update.is_empty() {
                config.apply_update(update)?;
            }
            let stored = store.create_in(&category, &config)?;
            emit(&ModelSummary::from(&stored), output)
        }
        ModelCommand::List(args) => {
            let result = match args.category.as_deref() {
                Some(category) => store.scan_summary_in(&parse_category(Some(category))?)?,
                None => store.scan_summary()?,
            };
            emit(&result, output)
        }
        ModelCommand::Info(args) => {
            let (category, name) = parse_named_ref(&args.name, args.category.as_deref())?;
            let stored = store.read_in(&category, &name)?;
            emit(&ModelSummary::from(&stored), output)
        }
        ModelCommand::Update(args) => {
            let (category, name) = parse_named_ref(&args.name, args.category.as_deref())?;
            let update = model_update_from(
                args.api_format,
                args.temperature,
                args.max_tokens,
                args.clear_max_tokens,
                args.timeout,
                args.reasoning_effort,
                args.clear_reasoning_effort,
                args.tool_call,
                args.context_window,
            )?;
            let stored = store.update_in(&category, &name, update)?;
            emit(&ModelSummary::from(&stored), output)
        }
        ModelCommand::Delete(args) => delete_resource(
            ResourceKind::Model,
            &args.name,
            args.category.as_deref(),
            args.recursive,
            args.dry_run,
            home,
            output,
            yes,
        ),
        ModelCommand::Category(command) => {
            category_command(ResourceKind::Model, command, home, output)
        }
    }
}

fn dispatch_prompt(
    command: PromptCommand,
    home: &Path,
    output: OutputMode,
    yes: bool,
) -> Result<()> {
    let store = PromptStore::new(home);
    match command {
        PromptCommand::Create(args) => {
            let category = parse_category(args.category.as_deref())?;
            let content = read_content(args.text, args.file, args.stdin)?;
            let stored = store.create_in(&category, &args.name, &content)?;
            emit(&PromptSummary::try_from(&stored)?, output)
        }
        PromptCommand::List(args) => {
            let result = match args.category.as_deref() {
                Some(category) => store.scan_summary_in(&parse_category(Some(category))?)?,
                None => store.scan_summary()?,
            };
            emit(&result, output)
        }
        PromptCommand::Info(args) => {
            let (category, name) = parse_named_ref(&args.name, args.category.as_deref())?;
            let stored = store.read_in(&category, &name)?;
            emit(&PromptSummary::try_from(&stored)?, output)
        }
        PromptCommand::Read(args) => {
            let (category, name) = parse_named_ref(&args.name, args.category.as_deref())?;
            let stored = store.read_in(&category, &name)?;
            emit(&PromptContentResult::try_from(&stored)?, output)
        }
        PromptCommand::Update(args) => {
            let (category, name) = parse_named_ref(&args.name, args.category.as_deref())?;
            let content = read_content(args.text, args.file, args.stdin)?;
            let stored = store.update_in(&category, &name, &content)?;
            emit(&PromptSummary::try_from(&stored)?, output)
        }
        PromptCommand::Delete(args) => delete_resource(
            ResourceKind::Prompt,
            &args.name,
            args.category.as_deref(),
            args.recursive,
            args.dry_run,
            home,
            output,
            yes,
        ),
        PromptCommand::Category(command) => {
            category_command(ResourceKind::Prompt, command, home, output)
        }
    }
}

fn dispatch_skill(command: SkillCommand, home: &Path, output: OutputMode, yes: bool) -> Result<()> {
    let store = SkillStore::new(home);
    match command {
        SkillCommand::Create(args) => {
            let category = parse_category(args.category.as_deref())?;
            let options = SkillCreateOptions::new(&args.package, args.description)?;
            let stored = store.create_in(&category, options)?;
            emit(&SkillSummary::from(&stored), output)
        }
        SkillCommand::List(args) => {
            let result = match args.category.as_deref() {
                Some(category) => store.scan_summary_in(&parse_category(Some(category))?)?,
                None => store.scan_summary()?,
            };
            emit(&result, output)
        }
        SkillCommand::Info(args) => {
            let (category, package) = parse_skill_ref(&args.package, args.category.as_deref())?;
            let result = store.read_summary_in(&category, &package)?;
            emit(&result, output)
        }
        SkillCommand::Read(args) => {
            let (category, package) = parse_skill_ref(&args.package, args.category.as_deref())?;
            let result = store.read_document_summary_in(&category, &package)?;
            emit(&result, output)
        }
        SkillCommand::Update(args) => {
            let (category, package) = parse_skill_ref(&args.package, args.category.as_deref())?;
            let stored = store.read_in(&category, &package)?;
            let file = if args.file.is_dir() {
                args.file.join(agt_skill::config::SKILL_FILE)
            } else {
                args.file
            };
            let text = fs::read_to_string(&file)
                .with_context(|| format!("读取 skill 文件失败：{}", file.display()))?;
            let document = SkillDocument::parse(&file, &text)?;
            if document.metadata.package_ref()? != package {
                bail!(
                    "skill 文件中的 package {} 与目标 {} 不一致",
                    document.metadata.package_ref()?,
                    package
                );
            }
            let rendered = document.render()?;
            atomic_replace_text(&stored.path, &rendered)?;
            let result = store.read_document_summary_in(&category, &package)?;
            emit(&result, output)
        }
        SkillCommand::Delete(args) => delete_resource(
            ResourceKind::Skill,
            &args.package,
            args.category.as_deref(),
            args.recursive,
            args.dry_run,
            home,
            output,
            yes,
        ),
        SkillCommand::Category(command) => {
            category_command(ResourceKind::Skill, command, home, output)
        }
    }
}

fn dispatch_mcp(command: McpCommand, home: &Path, output: OutputMode, yes: bool) -> Result<()> {
    let store = McpStore::new(home);
    match command {
        McpCommand::Create(args) => {
            let category = parse_category(args.category.as_deref())?;
            let text = fs::read_to_string(&args.file)
                .with_context(|| format!("读取 MCP 文件失败：{}", args.file.display()))?;
            let document = McpDocument::parse(&args.file, &text)?;
            if document.name.as_str() != args.name {
                bail!(
                    "MCP JSON 中的 server 名称 {} 与目标 {} 不一致",
                    document.name,
                    args.name
                );
            }
            let stored = store.add_in(&category, document)?;
            emit(&McpSummary::from(&stored), output)
        }
        McpCommand::List(args) => {
            let result = match args.category.as_deref() {
                Some(category) => store.scan_summary_in(&parse_category(Some(category))?)?,
                None => store.scan_summary()?,
            };
            emit(&result, output)
        }
        McpCommand::Info(args) => {
            let (category, name) = parse_named_ref(&args.name, args.category.as_deref())?;
            let result: McpInfoResult = store.info_summary_in(&category, &name)?;
            emit(&result, output)
        }
        McpCommand::Update(args) => {
            let (category, name) = parse_named_ref(&args.name, args.category.as_deref())?;
            let text = fs::read_to_string(&args.file)
                .with_context(|| format!("读取 MCP 文件失败：{}", args.file.display()))?;
            let document = McpDocument::parse(&args.file, &text)?;
            if document.name.as_str() != name {
                bail!(
                    "MCP JSON 中的 server 名称 {} 与目标 {} 不一致",
                    document.name,
                    name
                );
            }
            let stored = store
                .apply_batch_in(&category, vec![McpWritePlan::replace(document)])?
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("MCP 更新没有返回结果"))?;
            emit(&McpSummary::from(&stored), output)
        }
        McpCommand::Test(args) => {
            let (category, name) = parse_named_ref(&args.name, args.category.as_deref())?;
            let result = store.read_in(&category, &name)?.document.test_config()?;
            emit_value(
                serde_json::json!({
                    "name": result.name,
                    "transport": result.transport,
                    "target": result.target,
                    "tool_count": result.tool_count,
                    "tool_names": result.tool_names,
                    "smoke": result.smoke,
                }),
                output,
            )
        }
        McpCommand::Delete(args) => delete_resource(
            ResourceKind::Mcp,
            &args.name,
            args.category.as_deref(),
            args.recursive,
            args.dry_run,
            home,
            output,
            yes,
        ),
        McpCommand::Category(command) => category_command(ResourceKind::Mcp, command, home, output),
    }
}

fn dispatch_agent(command: AgentCommand, home: &Path, output: OutputMode, yes: bool) -> Result<()> {
    let store = AgentStore::new(home);
    match command {
        AgentCommand::Run(args) => runtime::execute(args, home, output),
        AgentCommand::Create(args) => {
            let category = parse_category(args.category.as_deref())?;
            PromptStore::new(home).ensure_default_prompt()?;
            let (model_category, model_name) = parse_named_ref(&args.model, None)?;
            let mut config = AgentConfig::new(&args.name, &model_name)?;
            config.model = QualifiedRef::new(model_category, ModelId::new(model_name));
            config.builtin_tools = args.builtin_tools;
            if let Some(prompt) = args.prompt {
                let (prompt_category, prompt_name) = parse_named_ref(&prompt, None)?;
                config.prompt = QualifiedRef::new(prompt_category, PromptId::new(prompt_name));
            }
            if let Some(description) = args.description {
                config.set_description(description);
            }
            for skill in args.skills {
                let (skill_category, package) = parse_skill_ref(&skill, None)?;
                config
                    .skills
                    .push(ResourceRef::persisted_in(skill_category, package));
            }
            for mcp in args.mcps {
                let (mcp_category, mcp_name) = parse_named_ref(&mcp, None)?;
                config
                    .mcps
                    .push(QualifiedRef::new(mcp_category, McpId::new(mcp_name)));
            }
            let stored = store.create_in(&category, &config)?;
            emit(&AgentSummary::from(&stored), output)
        }
        AgentCommand::List(args) => {
            let result = match args.category.as_deref() {
                Some(category) => store.scan_summary_in(&parse_category(Some(category))?)?,
                None => store.scan_summary()?,
            };
            emit(&result, output)
        }
        AgentCommand::Info(args) => {
            let (category, name) = parse_named_ref(&args.name, args.category.as_deref())?;
            let result = store.read_summary_in(&category, &name)?;
            emit(&result, output)
        }
        AgentCommand::Update(args) => {
            let (category, name) = parse_named_ref(&args.name, args.category.as_deref())?;
            if args.description.is_none()
                && args.model.is_none()
                && args.prompt.is_none()
                && args.add_skills.is_empty()
                && args.remove_skills.is_empty()
                && args.add_mcps.is_empty()
                && args.remove_mcps.is_empty()
                && args.add_builtin_tools.is_empty()
                && args.remove_builtin_tools.is_empty()
            {
                bail!("至少提供一个 Agent 更新字段");
            }
            let model = args
                .model
                .map(|value| parse_named_ref(&value, None))
                .transpose()?;
            let prompt = args
                .prompt
                .map(|value| parse_named_ref(&value, None))
                .transpose()?;
            let add_skills = args
                .add_skills
                .iter()
                .map(|value| parse_skill_ref(value, None))
                .collect::<Result<Vec<_>>>()?;
            let remove_skills = args
                .remove_skills
                .iter()
                .map(|value| parse_skill_ref(value, None))
                .collect::<Result<Vec<_>>>()?;
            let add_mcps = args
                .add_mcps
                .iter()
                .map(|value| parse_named_ref(value, None))
                .collect::<Result<Vec<_>>>()?;
            let remove_mcps = args
                .remove_mcps
                .iter()
                .map(|value| parse_named_ref(value, None))
                .collect::<Result<Vec<_>>>()?;
            let result = store.update_in(&category, &name, |config| {
                for tool in args.add_builtin_tools {
                    if !config.builtin_tools.contains(&tool) {
                        config.builtin_tools.push(tool);
                    }
                }
                for tool in args.remove_builtin_tools {
                    if let Some(index) = config.builtin_tools.iter().position(|item| item == &tool)
                    {
                        config.builtin_tools.remove(index);
                    } else {
                        return Err(AgentError::AttachmentNotFound {
                            agent: name.clone(),
                            kind: "builtin_tool",
                            name: tool.to_string(),
                        });
                    }
                }
                if let Some(description) = args.description {
                    config.set_description(description);
                }
                if let Some((model_category, model_name)) = model {
                    config.model = QualifiedRef::new(model_category, ModelId::new(model_name));
                }
                if let Some((prompt_category, prompt_name)) = prompt {
                    config.prompt = QualifiedRef::new(prompt_category, PromptId::new(prompt_name));
                }
                for (skill_category, package) in add_skills {
                    let reference = ResourceRef::persisted_in(skill_category, package);
                    if !config.skills.contains(&reference) {
                        config.skills.push(reference);
                    }
                }
                for (skill_category, package) in remove_skills {
                    let reference = ResourceRef::persisted_in(skill_category, package);
                    if let Some(index) = config.skills.iter().position(|item| item == &reference) {
                        config.skills.remove(index);
                    } else {
                        return Err(AgentError::AttachmentNotFound {
                            agent: name.clone(),
                            kind: "skill",
                            name: reference.to_string(),
                        });
                    }
                }
                for (mcp_category, mcp_name) in add_mcps {
                    let reference = QualifiedRef::new(mcp_category, McpId::new(mcp_name));
                    if !config.mcps.contains(&reference) {
                        config.mcps.push(reference);
                    }
                }
                for (mcp_category, mcp_name) in remove_mcps {
                    let reference = QualifiedRef::new(mcp_category, McpId::new(mcp_name));
                    if let Some(index) = config.mcps.iter().position(|item| item == &reference) {
                        config.mcps.remove(index);
                    } else {
                        return Err(AgentError::AttachmentNotFound {
                            agent: name.clone(),
                            kind: "mcp",
                            name: reference.to_string(),
                        });
                    }
                }
                Ok(())
            })?;
            emit(&AgentSummary::from(&result), output)
        }
        AgentCommand::Delete(args) => delete_resource(
            ResourceKind::Agent,
            &args.name,
            args.category.as_deref(),
            args.recursive,
            args.dry_run,
            home,
            output,
            yes,
        ),
        AgentCommand::Category(command) => {
            category_command(ResourceKind::Agent, command, home, output)
        }
    }
}

fn category_command(
    kind: ResourceKind,
    command: CategoryCommand,
    home: &Path,
    output: OutputMode,
) -> Result<()> {
    let category = match command {
        CategoryCommand::Create(args) => {
            let category = CategoryName::new(args.name)?;
            let path = match kind {
                ResourceKind::Provider => {
                    ProviderStore::new(home).categories().create(&category)?
                }
                ResourceKind::Model => ModelStore::new(home).categories().create(&category)?,
                ResourceKind::Prompt => PromptStore::new(home).categories().create(&category)?,
                ResourceKind::Skill => SkillStore::new(home).categories().create(&category)?,
                ResourceKind::Mcp => McpStore::new(home).categories().create(&category)?,
                ResourceKind::Agent => AgentStore::new(home).categories().create(&category)?,
            };
            return emit(
                &serde_json::json!({"category": category, "path": path}),
                output,
            );
        }
        CategoryCommand::List => {
            let categories = match kind {
                ResourceKind::Provider => ProviderStore::new(home).categories().list()?,
                ResourceKind::Model => ModelStore::new(home).categories().list()?,
                ResourceKind::Prompt => PromptStore::new(home).categories().list()?,
                ResourceKind::Skill => SkillStore::new(home).categories().list()?,
                ResourceKind::Mcp => McpStore::new(home).categories().list()?,
                ResourceKind::Agent => AgentStore::new(home).categories().list()?,
            };
            return emit(&categories, output);
        }
        CategoryCommand::Delete(args) => CategoryName::new(args.name)?,
    };
    let path = match kind {
        ResourceKind::Provider => ProviderStore::new(home)
            .categories()
            .delete_empty(&category)?,
        ResourceKind::Model => ModelStore::new(home).categories().delete_empty(&category)?,
        ResourceKind::Prompt => PromptStore::new(home)
            .categories()
            .delete_empty(&category)?,
        ResourceKind::Skill => SkillStore::new(home).categories().delete_empty(&category)?,
        ResourceKind::Mcp => McpStore::new(home).categories().delete_empty(&category)?,
        ResourceKind::Agent => AgentStore::new(home).categories().delete_empty(&category)?,
    };
    emit(
        &serde_json::json!({"category": category, "path": path}),
        output,
    )
}

#[allow(clippy::too_many_arguments)]
fn model_update_from(
    api_format: Option<String>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    clear_max_tokens: bool,
    timeout: Option<u32>,
    reasoning_effort: Option<String>,
    clear_reasoning_effort: bool,
    tool_call: Option<bool>,
    context_window: Option<u32>,
) -> Result<ModelConfigUpdate> {
    Ok(ModelConfigUpdate {
        temperature,
        max_tokens,
        clear_max_tokens,
        timeout,
        api_format: api_format
            .map(|value| value.parse::<ModelApiFormat>())
            .transpose()?,
        reasoning_effort: reasoning_effort
            .map(|value| value.parse::<ReasoningEffort>())
            .transpose()?,
        clear_reasoning_effort,
        tool_call,
        context_window,
    })
}

fn parse_category(raw: Option<&str>) -> Result<CategoryName> {
    raw.map(CategoryName::new)
        .transpose()
        .map(|category| category.unwrap_or_default())
        .map_err(Into::into)
}

fn parse_named_ref(raw: &str, category_override: Option<&str>) -> Result<(CategoryName, String)> {
    if let Some(category) = category_override {
        return Ok((parse_category(Some(category))?, raw.to_string()));
    }
    if let Some((category, name)) = raw.split_once('/')
        && !name.is_empty()
        && let Ok(category) = CategoryName::new(category)
    {
        return Ok((category, name.to_string()));
    }
    Ok((CategoryName::default(), raw.to_string()))
}

fn parse_skill_ref(
    raw: &str,
    category_override: Option<&str>,
) -> Result<(CategoryName, PackageRef)> {
    if let Some(category) = category_override {
        return Ok((parse_category(Some(category))?, raw.parse()?));
    }
    if raw.starts_with('@') {
        return Ok((CategoryName::default(), raw.parse()?));
    }
    if let Some((category, package)) = raw.split_once('/')
        && let Ok(category) = CategoryName::new(category)
        && !package.is_empty()
        && let Ok(package) = package.parse::<PackageRef>()
    {
        return Ok((category, package));
    }
    Ok((CategoryName::default(), raw.parse()?))
}

fn read_secret(
    direct: Option<String>,
    from_stdin: bool,
    env_name: Option<String>,
) -> Result<String> {
    if from_stdin {
        let mut value = String::new();
        io::stdin().read_to_string(&mut value)?;
        return Ok(value.trim_end_matches(['\r', '\n']).to_string());
    }
    if let Some(name) = env_name {
        return std::env::var(&name).with_context(|| format!("读取 API key 环境变量失败：{name}"));
    }
    direct.ok_or_else(|| anyhow!("必须提供 --api-key、--api-key-stdin 或 --api-key-env"))
}

fn read_content(text: Option<String>, file: Option<PathBuf>, stdin: bool) -> Result<String> {
    match (text, file, stdin) {
        (Some(text), None, false) => Ok(text),
        (None, Some(file), false) => fs::read_to_string(&file)
            .with_context(|| format!("读取文本文件失败：{}", file.display())),
        (None, None, true) => {
            let mut content = String::new();
            io::stdin().read_to_string(&mut content)?;
            Ok(content)
        }
        _ => bail!("必须且只能提供 --text、--file 或 --stdin 之一"),
    }
}

fn atomic_replace_text(path: &Path, content: &str) -> Result<()> {
    write_atomic(path, content.as_bytes())
        .with_context(|| format!("原子替换文件失败：{}", path.display()))
}

fn emit<T: Serialize>(value: &T, mode: OutputMode) -> Result<()> {
    let value = serde_json::to_value(value)?;
    emit_value(value, mode)
}

fn emit_value(value: Value, mode: OutputMode) -> Result<()> {
    match mode {
        OutputMode::Human | OutputMode::Json => {
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        OutputMode::Jsonl => {
            if let Value::Array(items) = value {
                for item in items {
                    println!("{}", serde_json::to_string(&item)?);
                }
            } else {
                println!("{}", serde_json::to_string(&value)?);
            }
        }
    }
    Ok(())
}

// The dependency-aware delete implementation is kept below the CRUD
// dispatchers so it can be reused by all six resource commands.

#[allow(clippy::too_many_arguments)]
fn delete_resource(
    kind: ResourceKind,
    raw_name: &str,
    category_override: Option<&str>,
    recursive: bool,
    dry_run: bool,
    home: &Path,
    output: OutputMode,
    yes: bool,
) -> Result<()> {
    let target = if kind == ResourceKind::Skill {
        let (category, package) = parse_skill_ref(raw_name, category_override)?;
        ResourceKey {
            kind,
            category: category.to_string(),
            name: package.to_string(),
        }
    } else {
        let (category, name) = parse_named_ref(raw_name, category_override)?;
        ResourceKey {
            kind,
            category: category.to_string(),
            name,
        }
    };

    let selected = agt_service::deletion::plan_delete(home, &target, recursive)?;
    let plan = serde_json::json!({
        "recursive": recursive,
        "dry_run": dry_run,
        "items": selected.iter().map(ResourceKey::display).collect::<Vec<_>>(),
    });
    if dry_run {
        return emit(&plan, output);
    }
    if !yes && !io::stdin().is_terminal() {
        emit(&plan, output)?;
        bail!("删除操作需要交互确认，请传入 --yes");
    }
    if !yes {
        emit(&plan, OutputMode::Human)?;
        eprint!("确认删除以上 {} 个资源？[y/N] ", selected.len());
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            bail!("已取消删除");
        }
    }

    let deleted = agt_service::deletion::execute_delete(home, &target, recursive, &selected)?;
    emit(
        &serde_json::json!({"deleted": deleted.iter().map(ResourceKey::display).collect::<Vec<_>>() }),
        output,
    )
}
