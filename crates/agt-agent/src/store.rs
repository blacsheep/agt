use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use agt_config::{AgtPaths, CATEGORY_README_FILE, DefinitionCategories};
use agt_core::{CategoryName, McpId, ModelId, PackageRef, PromptId, QualifiedRef, ResourceRef};
use agt_mcp::McpStore;
use agt_model::ModelStore;
use agt_prompt::PromptStore;
use agt_skill::SkillStore;
use agt_storage::{check_no_symlinks_under, ensure_dir_under, write_atomic_under};

use crate::config::{AGENT_FILE, AgentConfig, AgentConfigYaml};
use crate::diagnostic::{AgentDiagnostic, AgentDiagnosticCategory};
use crate::error::{AgentError, Result};
use crate::name::validate_agent_name;

#[derive(Clone, Debug)]
pub struct AgentStore {
    paths: AgtPaths,
    model_store: ModelStore,
    prompt_store: PromptStore,
    skill_store: SkillStore,
    mcp_store: McpStore,
}

impl AgentStore {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        Self {
            paths: AgtPaths::new(&home),
            model_store: ModelStore::new(&home),
            prompt_store: PromptStore::new(&home),
            skill_store: SkillStore::new(&home),
            mcp_store: McpStore::new(&home),
        }
    }

    pub fn from_default_home() -> agt_core::Result<Self> {
        let paths = AgtPaths::from_default_home()?;
        let home = paths.home().to_path_buf();
        Ok(Self {
            paths,
            model_store: ModelStore::new(&home),
            prompt_store: PromptStore::new(&home),
            skill_store: SkillStore::new(&home),
            mcp_store: McpStore::new(&home),
        })
    }

    pub fn agents_dir(&self) -> PathBuf {
        self.paths.data_dir("agents")
    }

    pub fn categories(&self) -> DefinitionCategories {
        DefinitionCategories::new(self.agents_dir())
    }

    pub fn category_dir(&self, category: &CategoryName) -> PathBuf {
        self.paths.category_dir("agents", category)
    }

    pub fn agent_dir(&self, name: &str) -> Result<PathBuf> {
        self.agent_dir_in(&CategoryName::default(), name)
    }

    pub fn agent_dir_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        validate_agent_name(name)?;
        Ok(self.category_dir(category).join(name))
    }

    pub fn agent_file(&self, name: &str) -> Result<PathBuf> {
        self.agent_file_in(&CategoryName::default(), name)
    }

    pub fn agent_file_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        Ok(self.agent_dir_in(category, name)?.join(AGENT_FILE))
    }

    pub fn create(&self, config: &AgentConfig) -> Result<StoredAgent> {
        self.create_in(&CategoryName::default(), config)
    }

    pub fn create_in(&self, category: &CategoryName, config: &AgentConfig) -> Result<StoredAgent> {
        self.require_write_category(category)?;
        config.validate()?;
        self.ensure_all_references_exist(config)?;

        let dir = self.agent_dir_in(category, config.name.as_str())?;
        let file = dir.join(AGENT_FILE);
        check_no_symlinks_under(self.paths.home(), &file).map_err(|error| match error {
            agt_core::AgtError::Io { path, source } => AgentError::io(path, source),
            other => AgentError::InvalidConfig {
                message: other.to_string(),
            },
        })?;
        if file.exists() {
            return Err(AgentError::AlreadyExists {
                name: config.name.to_string(),
            });
        }

        ensure_dir_under(self.paths.home(), &dir).map_err(map_core_error)?;
        self.write_config(&file, config)?;
        Ok(StoredAgent {
            category: category.clone(),
            path: file,
            config: config.clone(),
        })
    }

    pub fn read(&self, name: &str) -> Result<StoredAgent> {
        self.read_in(&CategoryName::default(), name)
    }

    pub fn read_in(&self, category: &CategoryName, name: &str) -> Result<StoredAgent> {
        if !category.is_default() {
            self.categories().require(category)?;
        }
        let file = self.agent_file_in(category, name)?;
        check_no_symlinks_under(self.paths.home(), &file).map_err(|error| match error {
            agt_core::AgtError::Io { path, source } => AgentError::io(path, source),
            other => AgentError::InvalidConfig {
                message: other.to_string(),
            },
        })?;
        if !file.exists() {
            return Err(AgentError::NotFound {
                name: name.to_string(),
            });
        }

        let text =
            fs::read_to_string(&file).map_err(|source| AgentError::io(file.clone(), source))?;
        let yaml: AgentConfigYaml =
            serde_yaml::from_str(&text).map_err(|source| AgentError::yaml(file.clone(), source))?;
        let config = AgentConfig::try_from(yaml)?;
        if config.name.as_str() != name {
            return Err(AgentError::InvalidConfig {
                message: format!("目录名 {name} 与 agent.yaml name {} 不一致", config.name),
            });
        }

        Ok(StoredAgent {
            category: category.clone(),
            path: file,
            config,
        })
    }

    pub fn list(&self) -> Result<Vec<StoredAgent>> {
        Ok(self.scan()?.items)
    }

    pub fn scan(&self) -> Result<AgentScan> {
        check_no_symlinks_under(self.paths.home(), self.paths.home()).map_err(
            |error| match error {
                agt_core::AgtError::Io { path, source } => AgentError::io(path, source),
                other => AgentError::InvalidConfig {
                    message: other.to_string(),
                },
            },
        )?;
        let agents_dir = self.agents_dir();
        let root_metadata = match fs::symlink_metadata(&agents_dir) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(AgentScan::default());
            }
            Err(source) => return Err(AgentError::io(&agents_dir, source)),
        };
        if !root_metadata.file_type().is_dir() {
            return Ok(AgentScan {
                items: Vec::new(),
                diagnostics: vec![AgentDiagnostic::new(
                    &agents_dir,
                    None,
                    AgentDiagnosticCategory::UnsupportedFileType,
                    "agents root 必须是普通目录",
                    "移除 symlink/特殊文件并创建普通 agents 目录",
                )],
            });
        }

        let mut scan = AgentScan::default();
        for entry in read_dir_sorted(&agents_dir)? {
            let path = entry.path();
            let category_text = entry.file_name().to_string_lossy().into_owned();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(source) => {
                    scan.diagnostics.push(AgentDiagnostic::new(
                        &path,
                        Some(category_text),
                        AgentDiagnosticCategory::Io,
                        AgentError::io(&path, source).to_string(),
                        "检查文件权限和路径是否可读",
                    ));
                    continue;
                }
            };
            if !file_type.is_dir() {
                scan.diagnostics.push(AgentDiagnostic::new(
                    &path,
                    Some(category_text.clone()),
                    if file_type.is_symlink() {
                        AgentDiagnosticCategory::UnsupportedFileType
                    } else {
                        AgentDiagnosticCategory::UnexpectedLayout
                    },
                    "agents root 只能包含 <category>/ 目录",
                    "移除该 entry 或改为规范 category 目录",
                ));
                continue;
            }
            let category = match CategoryName::new(&category_text) {
                Ok(category) => category,
                Err(error) => {
                    scan.diagnostics.push(AgentDiagnostic::new(
                        &path,
                        Some(category_text),
                        AgentDiagnosticCategory::InvalidIdentity,
                        error.to_string(),
                        "将目录名改为安全 category 名称",
                    ));
                    continue;
                }
            };
            self.scan_category(&category, &path, &mut scan)?;
        }
        scan.items.sort_by(|left, right| {
            (&left.category, left.config.name.as_str())
                .cmp(&(&right.category, right.config.name.as_str()))
        });
        scan.diagnostics
            .sort_by(|left, right| (&left.path, left.category).cmp(&(&right.path, right.category)));
        Ok(scan)
    }

    fn scan_category(
        &self,
        category: &CategoryName,
        category_dir: &Path,
        scan: &mut AgentScan,
    ) -> Result<()> {
        for entry in read_dir_sorted(category_dir)? {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == CATEGORY_README_FILE {
                continue;
            }
            let name_hint = Some(format!("{category}/{name}"));
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(source) => {
                    scan.diagnostics.push(AgentDiagnostic::new(
                        &path,
                        name_hint,
                        AgentDiagnosticCategory::Io,
                        AgentError::io(&path, source).to_string(),
                        "检查文件权限和路径是否可读",
                    ));
                    continue;
                }
            };
            if !file_type.is_dir() {
                scan.diagnostics.push(AgentDiagnostic::new(
                    &path,
                    name_hint,
                    if file_type.is_symlink() {
                        AgentDiagnosticCategory::UnsupportedFileType
                    } else {
                        AgentDiagnosticCategory::UnexpectedLayout
                    },
                    "agent category 只能包含 <name>/agent.yaml 目录",
                    "移除该 entry 或改为规范 agent 目录",
                ));
                continue;
            }

            let file = path.join(AGENT_FILE);
            match fs::symlink_metadata(&file) {
                Err(source) if source.kind() == io::ErrorKind::NotFound => {
                    scan.diagnostics.push(AgentDiagnostic::new(
                        &file,
                        name_hint,
                        AgentDiagnosticCategory::MissingConfigFile,
                        "agent 目录缺少 agent.yaml",
                        "补充 agent.yaml 或删除空目录",
                    ));
                    continue;
                }
                Err(source) => {
                    scan.diagnostics.push(AgentDiagnostic::new(
                        &file,
                        name_hint,
                        AgentDiagnosticCategory::Io,
                        AgentError::io(&file, source).to_string(),
                        "检查文件权限和路径是否可读",
                    ));
                    continue;
                }
                Ok(metadata) if !metadata.file_type().is_file() => {
                    scan.diagnostics.push(AgentDiagnostic::new(
                        &file,
                        name_hint,
                        AgentDiagnosticCategory::UnsupportedFileType,
                        "agent.yaml 必须是普通文件",
                        "移除 symlink/特殊文件并创建普通 agent.yaml",
                    ));
                    continue;
                }
                Ok(_) => {}
            }

            match self.read_in(category, &name) {
                Ok(agent) => scan.items.push(agent),
                Err(error) => scan
                    .diagnostics
                    .push(AgentDiagnostic::from_error(&file, name_hint, &error)),
            }
        }
        Ok(())
    }

    pub fn delete(&self, name: &str) -> Result<PathBuf> {
        self.delete_in(&CategoryName::default(), name)
    }

    pub fn delete_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        let dir = self.preflight_delete_in(category, name)?;
        ensure_child_path(&self.category_dir(category), &dir)?;
        fs::remove_dir_all(&dir).map_err(|source| AgentError::io(dir.clone(), source))?;
        Ok(dir)
    }

    pub fn preflight_delete_in(&self, category: &CategoryName, name: &str) -> Result<PathBuf> {
        let dir = self.agent_dir_in(category, name)?;
        let file = dir.join(AGENT_FILE);
        check_no_symlinks_under(self.paths.home(), &dir).map_err(|error| match error {
            agt_core::AgtError::Io { path, source } => AgentError::io(path, source),
            other => AgentError::InvalidConfig {
                message: other.to_string(),
            },
        })?;
        if !file.exists() {
            return Err(AgentError::NotFound {
                name: name.to_string(),
            });
        }
        self.read_in(category, name)?;
        Ok(dir)
    }

    pub fn set_model(&self, agent: &str, model: &str) -> Result<StoredAgent> {
        self.set_model_in(
            &CategoryName::default(),
            agent,
            QualifiedRef::in_default(ModelId::new(model)),
        )
    }

    pub fn set_model_in(
        &self,
        agent_category: &CategoryName,
        agent: &str,
        model: QualifiedRef<ModelId>,
    ) -> Result<StoredAgent> {
        self.ensure_model_exists(&model)?;
        self.update_in(agent_category, agent, |config| {
            config.model = model;
            Ok(())
        })
    }

    pub fn set_prompt(&self, agent: &str, prompt: QualifiedRef<PromptId>) -> Result<StoredAgent> {
        self.set_prompt_in(&CategoryName::default(), agent, prompt)
    }

    pub fn set_prompt_in(
        &self,
        agent_category: &CategoryName,
        agent: &str,
        prompt: QualifiedRef<PromptId>,
    ) -> Result<StoredAgent> {
        self.ensure_prompt_exists(&prompt)?;
        self.update_in(agent_category, agent, |config| {
            config.prompt = prompt;
            Ok(())
        })
    }

    pub fn add_skill(&self, agent: &str, skill: &str) -> Result<StoredAgent> {
        let package = parse_skill_ref(skill)?;
        self.add_skill_ref(agent, ResourceRef::persisted(package))
    }

    pub fn add_skill_ref(&self, agent: &str, reference: ResourceRef) -> Result<StoredAgent> {
        self.add_skill_ref_in(&CategoryName::default(), agent, reference)
    }

    pub fn add_skill_ref_in(
        &self,
        agent_category: &CategoryName,
        agent: &str,
        reference: ResourceRef,
    ) -> Result<StoredAgent> {
        self.ensure_skill_reference_available(&reference)?;
        self.update_in(agent_category, agent, |config| {
            push_unique_exact(&mut config.skills, reference);
            Ok(())
        })
    }

    pub fn remove_skill(&self, agent: &str, skill: &str) -> Result<StoredAgent> {
        let package = parse_skill_ref(skill)?;
        self.remove_skill_ref(agent, &ResourceRef::persisted(package))
    }

    pub fn remove_skill_ref(&self, agent: &str, reference: &ResourceRef) -> Result<StoredAgent> {
        self.remove_skill_ref_in(&CategoryName::default(), agent, reference)
    }

    pub fn remove_skill_ref_in(
        &self,
        agent_category: &CategoryName,
        agent: &str,
        reference: &ResourceRef,
    ) -> Result<StoredAgent> {
        self.update_in(agent_category, agent, |config| {
            remove_existing_exact(&mut config.skills, reference).map_err(|_| {
                AgentError::AttachmentNotFound {
                    agent: agent.to_string(),
                    kind: "skill",
                    name: reference.to_string(),
                }
            })
        })
    }

    pub fn add_mcp(&self, agent: &str, mcp: &str) -> Result<StoredAgent> {
        self.add_mcp_in(
            &CategoryName::default(),
            agent,
            QualifiedRef::in_default(McpId::new(mcp)),
        )
    }

    pub fn add_mcp_in(
        &self,
        agent_category: &CategoryName,
        agent: &str,
        mcp: QualifiedRef<McpId>,
    ) -> Result<StoredAgent> {
        self.ensure_mcp_exists(&mcp)?;
        self.update_in(agent_category, agent, |config| {
            push_unique_exact(&mut config.mcps, mcp);
            Ok(())
        })
    }

    pub fn remove_mcp(&self, agent: &str, mcp: &str) -> Result<StoredAgent> {
        self.remove_mcp_in(
            &CategoryName::default(),
            agent,
            &QualifiedRef::in_default(McpId::new(mcp)),
        )
    }

    pub fn remove_mcp_in(
        &self,
        agent_category: &CategoryName,
        agent: &str,
        mcp: &QualifiedRef<McpId>,
    ) -> Result<StoredAgent> {
        self.update_in(agent_category, agent, |config| {
            remove_existing_exact(&mut config.mcps, mcp).map_err(|_| {
                AgentError::AttachmentNotFound {
                    agent: agent.to_string(),
                    kind: "mcp",
                    name: mcp.to_string(),
                }
            })
        })
    }

    /// Apply a validated patch to an agent definition.
    ///
    /// The closure is intentionally kept at the store boundary so callers do
    /// not need to know the YAML representation.
    pub fn update_in(
        &self,
        agent_category: &CategoryName,
        agent: &str,
        mutate: impl FnOnce(&mut AgentConfig) -> Result<()>,
    ) -> Result<StoredAgent> {
        let mut stored = self.read_in(agent_category, agent)?;
        mutate(&mut stored.config)?;
        stored.config.validate()?;
        self.ensure_all_references_exist(&stored.config)?;
        self.write_config(&stored.path, &stored.config)?;
        Ok(stored)
    }

    fn ensure_all_references_exist(&self, config: &AgentConfig) -> Result<()> {
        self.ensure_model_exists(&config.model)?;
        self.ensure_prompt_exists(&config.prompt)?;
        for skill in &config.skills {
            self.ensure_skill_reference_available(skill)?;
        }
        for mcp in &config.mcps {
            self.ensure_mcp_exists(mcp)?;
        }
        Ok(())
    }

    fn ensure_model_exists(&self, model: &QualifiedRef<ModelId>) -> Result<()> {
        self.model_store
            .read_in(model.category(), model.name().as_str())
            .map(|_| ())
            .map_err(|source| AgentError::ModelUnavailable {
                name: model.to_string(),
                source,
            })
    }

    fn ensure_prompt_exists(&self, prompt: &QualifiedRef<PromptId>) -> Result<()> {
        self.prompt_store
            .read_in(prompt.category(), prompt.name().as_str())
            .map(|_| ())
            .map_err(|source| AgentError::PromptUnavailable {
                name: prompt.to_string(),
                source,
            })
    }

    fn ensure_skill_reference_available(&self, reference: &ResourceRef) -> Result<()> {
        self.skill_store
            .read_in(reference.category(), reference.package())
            .map(|_| ())
            .map_err(|source| AgentError::SkillUnavailable {
                name: reference.package().to_string(),
                source,
            })
    }

    fn ensure_mcp_exists(&self, mcp: &QualifiedRef<McpId>) -> Result<()> {
        self.mcp_store
            .read_in(mcp.category(), mcp.name().as_str())
            .map(|_| ())
            .map_err(|source| AgentError::McpUnavailable {
                name: mcp.to_string(),
                source: Box::new(source),
            })
    }

    fn write_config(&self, file: &Path, config: &AgentConfig) -> Result<()> {
        let yaml = serde_yaml::to_string(&AgentConfigYaml::from(config))
            .map_err(|source| AgentError::yaml(file, source))?;
        write_atomic_under(self.paths.home(), file, yaml.as_bytes())
            .map_err(|source| AgentError::io(file, source))
    }

    pub fn create_category(&self, category: &CategoryName) -> Result<PathBuf> {
        Ok(self.categories().create(category)?)
    }

    pub fn list_categories(&self) -> Result<Vec<CategoryName>> {
        Ok(self.categories().list()?)
    }

    pub fn delete_category(&self, category: &CategoryName) -> Result<PathBuf> {
        Ok(self.categories().delete_empty(category)?)
    }

    fn require_write_category(&self, category: &CategoryName) -> Result<PathBuf> {
        if category.is_default() {
            Ok(self.categories().ensure_default()?)
        } else {
            Ok(self.categories().require(category)?)
        }
    }
}

#[derive(Clone, Debug)]
pub struct StoredAgent {
    pub category: CategoryName,
    pub path: PathBuf,
    pub config: AgentConfig,
}

#[derive(Clone, Debug, Default)]
pub struct AgentScan {
    pub items: Vec<StoredAgent>,
    pub diagnostics: Vec<AgentDiagnostic>,
}

fn read_dir_sorted(path: &Path) -> Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)
        .map_err(|source| AgentError::io(path, source))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| AgentError::io(path, source))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn push_unique_exact<T>(items: &mut Vec<T>, value: T)
where
    T: Eq,
{
    if !items.contains(&value) {
        items.push(value);
    }
}

fn remove_existing_exact<T>(items: &mut Vec<T>, value: &T) -> std::result::Result<(), ()>
where
    T: Eq,
{
    let Some(index) = items.iter().position(|item| item == value) else {
        return Err(());
    };
    items.remove(index);
    Ok(())
}

fn parse_skill_ref(value: &str) -> Result<PackageRef> {
    value
        .parse::<PackageRef>()
        .map_err(|source| AgentError::InvalidConfig {
            message: format!("skill attachment ref 无效：{source}"),
        })
}

fn ensure_child_path(parent: &Path, child: &Path) -> Result<()> {
    let parent = parent
        .canonicalize()
        .map_err(|source| AgentError::io(parent, source))?;
    let child = child
        .canonicalize()
        .map_err(|source| AgentError::io(child, source))?;

    if child.starts_with(&parent) && child != parent {
        Ok(())
    } else {
        Err(AgentError::InvalidConfig {
            message: "agent 删除路径越界".to_string(),
        })
    }
}

fn map_core_error(error: agt_core::AgtError) -> AgentError {
    match error {
        agt_core::AgtError::Io { path, source } => AgentError::io(path, source),
        other => AgentError::InvalidConfig {
            message: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use agt_mcp::McpStore;
    use agt_model::{ModelConfig, ModelReference};
    use agt_prompt::PromptStore;
    use agt_provider::{ProviderConfig, ProviderStore};
    use agt_skill::{SkillCreateOptions, SkillStore};

    use super::*;

    fn test_home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("agt-agent-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        home
    }

    fn create_provider(home: &Path) {
        let provider_store = ProviderStore::new(home);
        let provider = ProviderConfig::openai_compatible(
            "my-provider",
            "https://api.example.com/v1",
            "sk-test1234",
        )
        .expect("provider config");
        match provider_store.create(&provider) {
            Ok(_) | Err(agt_provider::ProviderError::AlreadyExists { .. }) => {}
            Err(error) => panic!("create provider: {error}"),
        }
    }

    fn create_model(home: &Path, alias: &str) {
        PromptStore::new(home)
            .ensure_default_prompt()
            .expect("create default prompt");
        create_provider(home);
        let store = ModelStore::new(home);
        let reference = ModelReference::parse("my-provider/deepseek-chat").expect("reference");
        let config = ModelConfig::new(alias, reference).expect("model config");
        store.create(&config).expect("create model");
    }

    fn create_skill(home: &Path) {
        let store = SkillStore::new(home);
        let options =
            SkillCreateOptions::new("code-review", "Review code changes").expect("skill options");
        store.create(options).expect("create skill");
    }

    fn create_mcp(home: &Path) {
        let store = McpStore::new(home);
        store
            .add_from_str(
                r#"{"mcpServers":{"filesystem":{"type":"stdio","command":"npx","args":["-y","pkg"]}}}"#,
            )
            .expect("create mcp");
    }

    fn agent_config() -> AgentConfig {
        AgentConfig::new("coder", "fast-model").expect("agent config")
    }

    #[test]
    fn create_read_list_update_and_delete_agent() {
        let home = test_home("roundtrip");
        create_model(&home, "fast-model");
        let store = AgentStore::new(&home);
        let mut config = agent_config();
        config.set_description("Backend coding agent");

        let stored = store.create(&config).expect("create agent");
        assert_eq!(stored.path, home.join("agents/default/coder/agent.yaml"));
        assert!(stored.path.exists());

        let loaded = store.read("coder").expect("read agent");
        assert_eq!(loaded.config.name.as_str(), "coder");
        assert_eq!(loaded.config.model.name().as_str(), "fast-model");

        let agents = store.list().expect("list agents");
        assert_eq!(agents.len(), 1);

        create_model(&home, "reasoning-model");
        let updated = store
            .set_model("coder", "reasoning-model")
            .expect("set model");
        assert_eq!(updated.config.model.name().as_str(), "reasoning-model");

        let team = CategoryName::new("team").unwrap();
        let prompt_store = PromptStore::new(&home);
        prompt_store.create_category(&team).unwrap();
        prompt_store
            .create_in(&team, "review", "prompt from file")
            .unwrap();
        let prompt = QualifiedRef::new(team, PromptId::new("review"));
        let prompted = store
            .set_prompt("coder", prompt.clone())
            .expect("set prompt");
        assert_eq!(prompted.config.prompt, prompt);

        let deleted = store.delete("coder").expect("delete agent");
        assert_eq!(deleted, home.join("agents/default/coder"));
        assert!(!deleted.exists());

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn create_read_and_delete_agent_with_segmented_dot_name() {
        let home = test_home("dotted-name");
        create_model(&home, "fast-model");
        let store = AgentStore::new(&home);
        let config = AgentConfig::new("example-model-1.5", "fast-model").expect("agent config");

        let stored = store.create(&config).expect("create dotted agent");
        assert_eq!(
            stored.path,
            home.join("agents/default/example-model-1.5/agent.yaml")
        );
        assert_eq!(
            store
                .read("example-model-1.5")
                .expect("read dotted agent")
                .config
                .name
                .as_str(),
            "example-model-1.5"
        );

        let deleted = store.delete("example-model-1.5").expect("delete dotted agent");
        assert_eq!(deleted, home.join("agents/default/example-model-1.5"));
        assert!(!deleted.exists());

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn rejects_missing_model_duplicate_and_unsafe_names() {
        let home = test_home("validation");
        let store = AgentStore::new(&home);

        assert!(matches!(
            store.create(&agent_config()),
            Err(AgentError::ModelUnavailable { .. })
        ));

        create_model(&home, "fast-model");
        store
            .create(&agent_config())
            .expect("first create succeeds");
        assert!(matches!(
            store.create(&agent_config()),
            Err(AgentError::AlreadyExists { .. })
        ));
        assert!(store.agent_dir("../bad").is_err());

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn attaches_and_removes_resources() {
        let home = test_home("attachments");
        create_model(&home, "fast-model");
        create_skill(&home);
        create_mcp(&home);
        let store = AgentStore::new(&home);
        store.create(&agent_config()).expect("create agent");

        let with_skill = store.add_skill("coder", "code-review").expect("add skill");
        assert_eq!(with_skill.config.skills.len(), 1);
        let with_skill_again = store
            .add_skill("coder", "code-review")
            .expect("add skill idempotent");
        assert_eq!(with_skill_again.config.skills.len(), 1);

        let with_mcp = store.add_mcp("coder", "filesystem").expect("add mcp");
        assert_eq!(with_mcp.config.mcps.len(), 1);

        let removed_skill = store
            .remove_skill("coder", "code-review")
            .expect("remove skill");
        assert!(removed_skill.config.skills.is_empty());
        let removed_mcp = store.remove_mcp("coder", "filesystem").expect("remove mcp");
        assert!(removed_mcp.config.mcps.is_empty());
        assert!(matches!(
            store.remove_skill("coder", "code-review"),
            Err(AgentError::AttachmentNotFound { .. })
        ));

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn rejects_missing_attachment_targets() {
        let home = test_home("missing-attachments");
        create_model(&home, "fast-model");
        let store = AgentStore::new(&home);
        store.create(&agent_config()).expect("create agent");

        assert!(matches!(
            store.add_skill("coder", "missing-skill"),
            Err(AgentError::SkillUnavailable { .. })
        ));
        assert!(matches!(
            store.add_mcp("coder", "missing-mcp"),
            Err(AgentError::McpUnavailable { .. })
        ));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn scan_keeps_valid_items_and_reports_malformed_siblings() {
        let home = test_home("scan-diagnostics");
        create_model(&home, "fast-model");
        let store = AgentStore::new(&home);
        store
            .create(&AgentConfig::new("valid", "fast-model").unwrap())
            .unwrap();
        fs::create_dir_all(home.join("agents/default/broken")).unwrap();
        fs::write(
            home.join("agents/default/broken/agent.yaml"),
            "name: [broken",
        )
        .unwrap();
        fs::write(home.join("agents/default/stray"), "not a directory").unwrap();

        let scan = store.scan().unwrap();
        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.items[0].config.name.as_str(), "valid");
        assert_eq!(scan.diagnostics.len(), 2);
        assert_eq!(store.list().unwrap().len(), 1);
        let _ = fs::remove_dir_all(&home);
    }
}
