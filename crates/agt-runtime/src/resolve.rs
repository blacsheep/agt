use std::path::Path;

use agt_agent::AgentStore;
use agt_core::CategoryName;
use agt_mcp::{McpDocument, McpStore};
use agt_model::{ModelConfig, ModelStore};
use agt_prompt::PromptStore;
use agt_provider::{ProviderConfig, ProviderStore};
use agt_skill::SkillStore;

use crate::{Result, RuntimeError, canonical_agent, fingerprint};

/// In-memory execution snapshot. Intentionally does not implement Debug or Serialize:
/// provider and MCP configuration can contain credentials.
#[derive(Clone)]
pub struct ResolvedAgent {
    pub name: String,
    pub model: ModelConfig,
    pub provider: ProviderConfig,
    pub system_prompt: String,
    pub mcps: Vec<McpDocument>,
    pub builtin_tools: Vec<agt_agent::BuiltinTool>,
    pub fingerprint: String,
    pub(crate) definition: Option<agt_agent::AgentConfig>,
    pub(crate) base_prompt: String,
}

impl ResolvedAgent {
    pub fn load(home: &Path, name: &str) -> Result<Self> {
        let name = canonical_agent(name)?;
        let (category, leaf) = name.split_once('/').expect("canonical reference");
        let category = CategoryName::new(category).expect("validated category");
        let agent = AgentStore::new(home)
            .read_in(&category, leaf)
            .map_err(|_| RuntimeError::Configuration("cannot load agent definition"))?
            .config;
        let reference = agent.model.as_ref().ok_or(RuntimeError::Configuration(
            "external agent requires harness execution",
        ))?;
        let model = ModelStore::new(home)
            .read_in(reference.category(), reference.name().as_str())
            .map_err(|_| RuntimeError::Configuration("cannot load model definition"))?
            .config;
        let provider = ProviderStore::new(home)
            .read_in(model.provider.category(), model.provider.name().as_str())
            .map_err(|_| RuntimeError::Configuration("cannot load provider definition"))?
            .config;
        let base_prompt = PromptStore::new(home)
            .read_in(agent.prompt.category(), agent.prompt.name().as_str())
            .map_err(|_| RuntimeError::Configuration("cannot load prompt"))?
            .content;
        let (system_prompt, mcps) = load_capabilities(home, &agent, &base_prompt)?;
        // Hash actual loaded values; never persist the credential-bearing material.
        let fingerprint = snapshot_fingerprint(&agent, &model, &provider, &system_prompt, &mcps)?;
        Ok(Self {
            name,
            model,
            provider,
            system_prompt,
            mcps,
            builtin_tools: agent.builtin_tools.clone(),
            definition: Some(agent),
            base_prompt,
            fingerprint,
        })
    }
}

fn load_capabilities(
    home: &Path,
    definition: &agt_agent::AgentConfig,
    base_prompt: &str,
) -> Result<(String, Vec<McpDocument>)> {
    let mut system_prompt = base_prompt.to_owned();
    for skill in &definition.skills {
        let document = SkillStore::new(home)
            .read_in(skill.category(), skill.package())
            .map_err(|_| RuntimeError::Configuration("cannot load skill"))?
            .document;
        if document.metadata.allowed_tools.is_some() {
            return Err(RuntimeError::Configuration(
                "skill allowed-tools enforcement is not supported yet",
            ));
        }
        system_prompt.push_str("\n\n");
        system_prompt.push_str(
            &document
                .render()
                .map_err(|_| RuntimeError::Configuration("cannot render skill"))?,
        );
    }
    let mcps = definition
        .mcps
        .iter()
        .map(|reference| {
            McpStore::new(home)
                .read_in(reference.category(), reference.name().as_str())
                .map(|stored| stored.document)
                .map_err(|_| RuntimeError::Configuration("cannot load MCP definition"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((system_prompt, mcps))
}

impl ResolvedAgent {
    /// Reload only mounts. The model, credentials, base prompt and built-in grants stay fixed.
    pub(crate) fn reload_capabilities(&self, home: &Path) -> Result<Self> {
        let (category, leaf) = self.name.split_once('/').expect("canonical reference");
        let current = AgentStore::new(home)
            .read_in(&CategoryName::new(category).expect("category"), leaf)
            .map_err(|_| RuntimeError::Configuration("cannot load agent definition"))?
            .config;
        let mut next = self.clone();
        let definition = next
            .definition
            .as_mut()
            .expect("Native execution has a definition");
        definition.mcps = current.mcps;
        definition.skills = current.skills;
        (next.system_prompt, next.mcps) = load_capabilities(home, definition, &self.base_prompt)?;
        next.fingerprint = snapshot_fingerprint(
            definition,
            &next.model,
            &next.provider,
            &next.system_prompt,
            &next.mcps,
        )?;
        Ok(next)
    }
}

fn snapshot_fingerprint(
    definition: &agt_agent::AgentConfig,
    model: &ModelConfig,
    provider: &ProviderConfig,
    system_prompt: &str,
    mcps: &[McpDocument],
) -> Result<String> {
    let material = format!(
        "{definition:?}{model:?}{}{}{system_prompt}{}",
        provider.base_url,
        fingerprint(provider.api_key.as_bytes()),
        serde_json::to_string(&mcps.iter().map(|m| &m.server).collect::<Vec<_>>())?
    );
    Ok(fingerprint(material.as_bytes()))
}
