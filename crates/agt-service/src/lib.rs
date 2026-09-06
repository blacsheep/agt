mod bindings;
pub mod deletion;
pub mod mcp_server;
mod messaging;
mod resources;
mod tools;

pub use bindings::{Binding, BindingStore, ResponsePolicy};
pub use messaging::{Actor, MessageInput, Messaging, Role};
pub use resources::Resources;
pub use tools::ToolService;

use agt_core::CategoryName;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn named_ref(raw: &str) -> Result<(CategoryName, String)> {
    let (category, name) = raw.split_once('/').unwrap_or(("default", raw));
    agt_agent::validate_agent_name(name)?;
    Ok((CategoryName::new(category)?, name.to_owned()))
}

pub fn database_path(home: &Path, directory: &str, name: &str) -> Result<PathBuf> {
    let dir = home.join(directory);
    agt_storage::ensure_dir_under(home, &dir)?;
    for suffix in ["", "-wal", "-shm", "-journal"] {
        agt_storage::check_no_symlinks_under(home, dir.join(format!("{name}{suffix}")))?;
    }
    Ok(dir.join(name))
}
