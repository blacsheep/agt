mod bindings;
pub mod deletion;
mod error;
pub mod mcp_server;
mod messaging;
mod resources;
mod tools;

pub use bindings::{Binding, BindingStore, ResponsePolicy};
pub use error::ToolError;
pub use messaging::{Actor, MessageInput, Messaging, Role};
pub use resources::Resources;
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
