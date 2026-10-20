use agt_config::{AgtPaths, DefinitionCategories};
use agt_core::CategoryName;
use agt_storage::{check_no_symlinks_under, ensure_dir_under, write_atomic_under};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Sandbox {
    ReadOnly,
    WorkspaceWrite,
}
impl Sandbox {
    pub(crate) fn protocol(&self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HarnessConfig {
    pub engine: String,
    pub executable: PathBuf,
    pub working_directory: PathBuf,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    pub sandbox: Sandbox,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    /// `acp` engine only: arguments that start the agent's ACP server.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// `acp` engine only: the ACP auth method to use when the agent needs one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    /// `codex` engine only: false starts Codex without the MCP servers in its own config
    /// files, so it offers only agt's tools and starts no extra server processes.
    #[serde(default = "inherit", skip_serializing_if = "is_inherited")]
    pub inherit_mcp_servers: bool,
}
fn default_timeout() -> u64 {
    1800
}
fn inherit() -> bool {
    true
}
fn is_inherited(value: &bool) -> bool {
    *value
}
impl HarnessConfig {
    pub fn validate(&self) -> Result<()> {
        if !crate::ENGINES.contains(&self.engine.as_str()) {
            bail!("supported execution engines: codex, claude-code, deepseek-harness, acp");
        }
        // Only a custom ACP agent takes launch arguments, so a Codex harness can never be
        // started any way other than `codex app-server`.
        if self.engine == crate::ACP {
            if self.args.len() > 32 || self.args.iter().any(|a| a.len() > 4096) {
                bail!("args must be at most 32 arguments of up to 4096 bytes");
            }
            if self
                .auth_method
                .as_ref()
                .is_some_and(|m| m.trim().is_empty())
            {
                bail!("auth_method must be non-empty or omitted");
            }
        } else if !self.args.is_empty() || self.auth_method.is_some() {
            bail!("args and auth_method apply only to the acp engine");
        }
        if !self.inherit_mcp_servers && self.engine != crate::CODEX {
            bail!("inherit_mcp_servers applies only to the codex engine");
        }
        if !self.executable.is_absolute() || !self.executable.is_file() {
            bail!("executable must be an existing absolute file path on the server");
        }
        if !self.working_directory.is_absolute() || !self.working_directory.is_dir() {
            bail!("working_directory must be an existing absolute directory on the server");
        }
        if !(10..=86400).contains(&self.timeout_seconds) {
            bail!("timeout_seconds must be 10–86400");
        }
        if self.model.as_ref().is_some_and(|m| m.trim().is_empty()) {
            bail!("model must be non-empty or omitted");
        }
        if self.description.len() > 4096 {
            bail!("description exceeds 4096 bytes");
        }
        if self
            .reasoning_effort
            .as_ref()
            .is_some_and(|e| e.trim().is_empty())
        {
            bail!("reasoning_effort must be non-empty or omitted");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct StoredHarness {
    pub category: String,
    pub name: String,
    #[serde(flatten)]
    pub config: HarnessConfig,
}
pub struct HarnessStore {
    paths: AgtPaths,
}
impl HarnessStore {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self {
            paths: AgtPaths::new(home),
        }
    }
    pub fn categories(&self) -> DefinitionCategories {
        DefinitionCategories::new(self.paths.data_dir("harnesses"))
    }
    pub fn file(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        CategoryName::new(name)?;
        let file = self
            .paths
            .category_dir("harnesses", category)
            .join(name)
            .join("harness.yaml");
        check_no_symlinks_under(self.paths.home(), &file)?;
        Ok(file)
    }
    pub fn read_in(&self, category: &CategoryName, name: &str) -> Result<StoredHarness> {
        let config: HarnessConfig =
            serde_yaml::from_str(&std::fs::read_to_string(self.file(category, name)?)?)?;
        // Machine paths can become unavailable; listing must still allow users to repair them.
        Ok(StoredHarness {
            category: category.to_string(),
            name: name.into(),
            config,
        })
    }
    pub fn write_in(
        &self,
        category: &CategoryName,
        name: &str,
        config: &HarnessConfig,
        create: bool,
    ) -> Result<()> {
        config.validate()?;
        let file = self.file(category, name)?;
        if create == file.exists() {
            bail!(if create {
                "harness already exists"
            } else {
                "harness not found"
            });
        }
        if category.is_default() {
            ensure_dir_under(
                self.paths.home(),
                self.paths.category_dir("harnesses", category),
            )?;
        } else {
            self.categories().require(category)?;
        }
        ensure_dir_under(self.paths.home(), file.parent().unwrap())?;
        write_atomic_under(
            self.paths.home(),
            &file,
            serde_yaml::to_string(config)?.as_bytes(),
        )?;
        Ok(())
    }
    pub fn scan(&self) -> Result<Vec<StoredHarness>> {
        let root = self.paths.data_dir("harnesses");
        check_no_symlinks_under(self.paths.home(), &root)?;
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut items = Vec::new();
        for cat in std::fs::read_dir(root)? {
            let cat = cat?;
            if cat.file_name() == "README.md" {
                continue;
            }
            let category = CategoryName::new(cat.file_name().to_string_lossy())?;
            check_no_symlinks_under(self.paths.home(), cat.path())?;
            for entry in std::fs::read_dir(cat.path())? {
                let entry = entry?;
                if entry.file_name() == "README.md" {
                    continue;
                }
                items.push(self.read_in(&category, &entry.file_name().to_string_lossy())?);
            }
        }
        items.sort_by(|a, b| (&a.category, &a.name).cmp(&(&b.category, &b.name)));
        Ok(items)
    }
    pub fn scan_summary(&self) -> Result<Value> {
        Ok(json!({"items":self.scan()?,"diagnostics":[]}))
    }
}
