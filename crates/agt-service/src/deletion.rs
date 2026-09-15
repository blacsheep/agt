use agt_agent::AgentStore;
use agt_core::{CategoryName, PackageRef};
use agt_mcp::McpStore;
use agt_model::ModelStore;
use agt_prompt::PromptStore;
use agt_provider::ProviderStore;
use agt_skill::SkillStore;
use agt_storage::check_no_symlinks_under;
use anyhow::{Context, Result, bail};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ResourceKind {
    Agent,
    Model,
    Provider,
    Prompt,
    Skill,
    Mcp,
    Harness,
}

impl ResourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Model => "model",
            Self::Provider => "provider",
            Self::Prompt => "prompt",
            Self::Skill => "skill",
            Self::Mcp => "mcp",
            Self::Harness => "harness",
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ResourceKey {
    pub kind: ResourceKind,
    pub category: String,
    pub name: String,
}

impl ResourceKey {
    pub fn display(&self) -> String {
        format!("{}/{}/{}", self.kind.as_str(), self.category, self.name)
    }
}

pub fn plan_delete(home: &Path, target: &ResourceKey, recursive: bool) -> Result<Vec<ResourceKey>> {
    let graph = load_graph(home)?;
    if !graph.nodes.contains(target) {
        bail!("{} 不存在", target.display());
    }
    let mut selected = vec![target.clone()];
    if recursive {
        let mut changed = true;
        while changed {
            changed = false;
            for (dependency, dependents) in &graph.reverse {
                if selected.contains(dependency) {
                    for dependent in dependents {
                        if !selected.contains(dependent) {
                            selected.push(dependent.clone());
                            changed = true;
                        }
                    }
                }
            }
        }
    } else if let Some(dependents) = graph.reverse.get(target)
        && !dependents.is_empty()
    {
        let names = dependents
            .iter()
            .map(ResourceKey::display)
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "{} 仍被引用：{}；如需级联删除请加 --recursive",
            target.display(),
            names
        );
    }

    selected.sort_by_key(|item| (delete_rank(item.kind), item.clone()));
    for item in &selected {
        ensure_delete_allowed(home, item)?;
    }
    if home.join("runtime/bindings.db").exists() {
        for status in crate::BindingStore::open(home)?.list_with_state()? {
            let binding = status.binding;
            if selected.iter().any(|item| {
                item.kind == ResourceKind::Agent
                    && format!("{}/{}", item.category, item.name) == binding.agent
            }) {
                bail!(
                    "agent {} is hosted by {}; unbind it before deleting resources",
                    binding.agent,
                    binding.principal
                );
            }
        }
    }
    Ok(selected)
}

pub fn execute_delete(
    home: &Path,
    target: &ResourceKey,
    recursive: bool,
    expected: &[ResourceKey],
) -> Result<Vec<ResourceKey>> {
    let selected = plan_delete(home, target, recursive)?;
    if selected != expected {
        bail!("delete plan changed; inspect it again");
    }
    delete_selected(home, &selected)
}
fn delete_rank(kind: ResourceKind) -> u8 {
    match kind {
        ResourceKind::Agent => 0,
        ResourceKind::Model => 1,
        ResourceKind::Harness
        | ResourceKind::Provider
        | ResourceKind::Prompt
        | ResourceKind::Skill
        | ResourceKind::Mcp => 2,
    }
}

fn ensure_delete_allowed(_home: &Path, key: &ResourceKey) -> Result<()> {
    match key.kind {
        ResourceKind::Prompt
            if key.category == CategoryName::default().to_string()
                && key.name == agt_prompt::DEFAULT_PROMPT_NAME =>
        {
            bail!("prompt/default/default 是保留资源，不能删除")
        }
        _ => Ok(()),
    }
}

#[derive(Default)]
struct ResourceGraph {
    nodes: std::collections::BTreeSet<ResourceKey>,
    reverse: std::collections::BTreeMap<ResourceKey, Vec<ResourceKey>>,
}

fn load_graph(home: &Path) -> Result<ResourceGraph> {
    let providers = ProviderStore::new(home);
    let models = ModelStore::new(home);
    let prompts = PromptStore::new(home);
    let skills = SkillStore::new(home);
    let mcps = McpStore::new(home);
    let agents = AgentStore::new(home);
    let provider_scan = providers.scan()?;
    let model_scan = models.scan()?;
    let prompt_scan = prompts.scan()?;
    let skill_scan = skills.scan()?;
    let mcp_scan = mcps.scan()?;
    let agent_scan = agents.scan()?;
    let diagnostics = provider_scan.diagnostics.len()
        + model_scan.diagnostics.len()
        + prompt_scan.diagnostics.len()
        + skill_scan.diagnostics.len()
        + mcp_scan.diagnostics.len()
        + agent_scan.diagnostics.len();
    if diagnostics > 0 {
        bail!("资源扫描发现 {diagnostics} 个诊断，递归删除已停止；请先修复损坏资源");
    }

    let mut graph = ResourceGraph::default();
    for item in agt_harness::HarnessStore::new(home).scan()? {
        graph.nodes.insert(ResourceKey {
            kind: ResourceKind::Harness,
            category: item.category,
            name: item.name,
        });
    }
    for item in provider_scan.items {
        graph.nodes.insert(ResourceKey {
            kind: ResourceKind::Provider,
            category: item.category.to_string(),
            name: item.config.name.to_string(),
        });
    }
    for item in model_scan.items {
        let model = ResourceKey {
            kind: ResourceKind::Model,
            category: item.category.to_string(),
            name: item.config.name.to_string(),
        };
        let provider = ResourceKey {
            kind: ResourceKind::Provider,
            category: item.config.provider.category().to_string(),
            name: item.config.provider.name().to_string(),
        };
        graph.nodes.insert(model.clone());
        add_edge(&mut graph, provider, model);
    }
    for item in prompt_scan.items {
        graph.nodes.insert(ResourceKey {
            kind: ResourceKind::Prompt,
            category: item.category.to_string(),
            name: item.name.to_string(),
        });
    }
    for item in skill_scan.items {
        graph.nodes.insert(ResourceKey {
            kind: ResourceKind::Skill,
            category: item.category.clone().unwrap_or_default().to_string(),
            name: item.package.to_string(),
        });
    }
    for item in mcp_scan.items {
        graph.nodes.insert(ResourceKey {
            kind: ResourceKind::Mcp,
            category: item.category.to_string(),
            name: item.document.name.to_string(),
        });
    }
    for item in agent_scan.items {
        let agent = ResourceKey {
            kind: ResourceKind::Agent,
            category: item.category.to_string(),
            name: item.config.name.to_string(),
        };
        graph.nodes.insert(agent.clone());
        if let Some(model) = item.config.model {
            add_edge(
                &mut graph,
                ResourceKey {
                    kind: ResourceKind::Model,
                    category: model.category().to_string(),
                    name: model.name().to_string(),
                },
                agent.clone(),
            );
        }
        if let Some(harness) = item.config.harness {
            add_edge(
                &mut graph,
                ResourceKey {
                    kind: ResourceKind::Harness,
                    category: harness.category().to_string(),
                    name: harness.name().to_string(),
                },
                agent.clone(),
            );
        }
        add_edge(
            &mut graph,
            ResourceKey {
                kind: ResourceKind::Prompt,
                category: item.config.prompt.category().to_string(),
                name: item.config.prompt.name().to_string(),
            },
            agent.clone(),
        );
        for skill in item.config.skills {
            add_edge(
                &mut graph,
                ResourceKey {
                    kind: ResourceKind::Skill,
                    category: skill.category().to_string(),
                    name: skill.package().to_string(),
                },
                agent.clone(),
            );
        }
        for mcp in item.config.mcps {
            add_edge(
                &mut graph,
                ResourceKey {
                    kind: ResourceKind::Mcp,
                    category: mcp.category().to_string(),
                    name: mcp.name().to_string(),
                },
                agent.clone(),
            );
        }
    }
    for dependents in graph.reverse.values_mut() {
        dependents.sort();
    }
    let dangling = graph
        .reverse
        .keys()
        .filter(|dependency| !graph.nodes.contains(*dependency))
        .map(ResourceKey::display)
        .collect::<Vec<_>>();
    if !dangling.is_empty() {
        bail!("发现悬空资源引用：{}；递归删除已停止", dangling.join(", "));
    }
    Ok(graph)
}

fn add_edge(graph: &mut ResourceGraph, dependency: ResourceKey, dependent: ResourceKey) {
    graph.reverse.entry(dependency).or_default().push(dependent);
}

fn delete_selected(home: &Path, selected: &[ResourceKey]) -> Result<Vec<ResourceKey>> {
    let trash = create_trash_dir(home)?;
    let mut moved = Vec::new();
    let mut scope_parents = std::collections::BTreeSet::new();
    for (index, key) in selected.iter().enumerate() {
        let source = resource_dir(home, key)?;
        if let Err(error) = check_no_symlinks_under(home, &source) {
            rollback_moves(&moved)?;
            bail!("资源路径不能包含 symlink：{}：{error}", source.display());
        }
        match fs::symlink_metadata(&source) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(metadata) if metadata.file_type().is_symlink() => {
                rollback_moves(&moved)?;
                bail!("资源目录不能是 symlink：{}", source.display());
            }
            Ok(_) | Err(_) => {
                rollback_moves(&moved)?;
                bail!("资源目录不存在：{}", source.display());
            }
        }
        if key.kind == ResourceKind::Skill
            && let Some(parent) = source.parent()
        {
            scope_parents.insert(parent.to_path_buf());
        }
        let destination = trash.join(format!("{index}-{}", key.kind.as_str()));
        if let Err(error) = fs::rename(&source, &destination) {
            rollback_moves(&moved)?;
            return Err(error).with_context(|| format!("移动待删除资源失败：{}", source.display()));
        }
        moved.push((source, destination));
    }
    if let Err(error) = fs::remove_dir_all(&trash) {
        let rollback = rollback_moves(&moved);
        return match rollback {
            Ok(()) => {
                Err(error).with_context(|| format!("清理删除暂存目录失败：{}", trash.display()))
            }
            Err(rollback_error) => Err(error).with_context(|| {
                format!(
                    "清理删除暂存目录失败：{}；回滚也失败：{}",
                    trash.display(),
                    rollback_error
                )
            }),
        };
    }
    for parent in scope_parents {
        remove_empty_scope_dir(&parent);
    }
    Ok(selected.to_vec())
}

fn create_trash_dir(home: &Path) -> Result<PathBuf> {
    let root = home.join(".trash");
    if let Err(error) = check_no_symlinks_under(home, &root) {
        bail!("删除暂存路径不能包含 symlink：{}：{error}", root.display());
    }
    match fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => bail!("删除暂存目录不是普通目录：{}", root.display()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&root)
                .with_context(|| format!("创建删除暂存根目录失败：{}", root.display()))?;
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("检查删除暂存根目录失败：{}", root.display()));
        }
    }

    for attempt in 0..1000u32 {
        let candidate = root.join(format!(
            "{}-{}-{attempt}",
            chrono_like_timestamp(),
            std::process::id()
        ));
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&candidate)
                    .with_context(|| format!("创建删除暂存目录失败：{}", candidate.display()))?;
                return Ok(candidate);
            }
            Ok(metadata) if metadata.file_type().is_dir() => continue,
            Ok(_) => bail!("删除暂存路径不是普通目录：{}", candidate.display()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("检查删除暂存目录失败：{}", candidate.display()));
            }
        }
    }
    bail!("无法创建唯一的删除暂存目录：{}", root.display())
}

fn remove_empty_scope_dir(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if !metadata.file_type().is_dir() {
        return;
    }
    let Ok(mut entries) = fs::read_dir(path) else {
        return;
    };
    if entries.next().is_none() {
        let _ = fs::remove_dir(path);
    }
}

fn rollback_moves(moved: &[(PathBuf, PathBuf)]) -> Result<()> {
    for (source, destination) in moved.iter().rev() {
        if destination.exists() {
            fs::rename(destination, source)?;
        }
    }
    Ok(())
}

fn resource_dir(home: &Path, key: &ResourceKey) -> Result<PathBuf> {
    let category = CategoryName::new(&key.category)?;
    match key.kind {
        ResourceKind::Harness => Ok(agt_harness::HarnessStore::new(home)
            .file(&category, &key.name)?
            .parent()
            .unwrap()
            .to_owned()),
        ResourceKind::Provider => ProviderStore::new(home)
            .provider_dir_in(&category, &key.name)
            .map_err(Into::into),
        ResourceKind::Model => ModelStore::new(home)
            .model_dir_in(&category, &key.name)
            .map_err(Into::into),
        ResourceKind::Prompt => PromptStore::new(home)
            .prompt_dir_in(&category, &key.name)
            .map_err(Into::into),
        ResourceKind::Mcp => McpStore::new(home)
            .mcp_dir_in(&category, &key.name)
            .map_err(Into::into),
        ResourceKind::Agent => AgentStore::new(home)
            .agent_dir_in(&category, &key.name)
            .map_err(Into::into),
        ResourceKind::Skill => {
            let package = key.name.parse::<PackageRef>()?;
            Ok(SkillStore::new(home).skill_dir_in(&category, &package))
        }
    }
}

fn chrono_like_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}
