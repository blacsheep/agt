mod bindings;
pub mod catalog;
pub mod deletion;
mod error;
mod harness_tools;
pub mod mcp_server;
mod memory;
mod messaging;
mod resources;
mod tools;

pub use bindings::{Binding, BindingStore, ResponsePolicy, SCHEMA as BINDINGS_SCHEMA};
pub use error::{Refusal, ToolError, UnknownMentions, meant_for_caller};
pub use messaging::{Actor, InputMode, MessageInput, Messaging, Role, check_mentions};
pub use resources::{Resources, WriteOptions, resource_ref};
pub use tools::ToolService;

use agt_core::CategoryName;
use anyhow::Result;

pub fn named_ref(raw: &str) -> Result<(CategoryName, String)> {
    let (category, name) = raw.split_once('/').unwrap_or(("default", raw));
    agt_agent::validate_agent_name(name)?;
    Ok((CategoryName::new(category)?, name.to_owned()))
}

pub use agt_storage::database_path;

/// Runtime-owned task operations; implementations hold their fixed actor and execution context.
/// The service layer deliberately does not depend on the runtime crate.
pub trait WorkControl {
    fn supports_await(&self) -> bool {
        false
    }
    fn call(&self, name: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value>;
}

/// Runtime owns conversation session storage; service exposes the shared tool contract.
pub trait HarnessControl {
    fn call(&self, name: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value>;
}

pub trait ExecutionControl {
    fn call(&self, args: serde_json::Value) -> anyhow::Result<serde_json::Value>;
}
