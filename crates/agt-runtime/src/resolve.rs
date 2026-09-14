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
        let reference = &agent.model;
        let model = ModelStore::new(home)
            .read_in(reference.category(), reference.name().as_str())
            .map_err(|_| RuntimeError::Configuration("cannot load model definition"))?
            .config;
        let provider = ProviderStore::new(home)
            .read_in(model.provider.category(), model.provider.name().as_str())
            .map_err(|_| RuntimeError::Configuration("cannot load provider definition"))?
            .config;
        let mut system_prompt = PromptStore::new(home)
            .read_in(agent.prompt.category(), agent.prompt.name().as_str())
            .map_err(|_| RuntimeError::Configuration("cannot load prompt"))?
            .content;
        for skill in &agent.skills {
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
        let mcps = agent
            .mcps
            .iter()
            .map(|reference| {
                McpStore::new(home)
                    .read_in(reference.category(), reference.name().as_str())
                    .map(|stored| stored.document)
                    .map_err(|_| RuntimeError::Configuration("cannot load MCP definition"))
            })
            .collect::<Result<Vec<_>>>()?;
        // Hash actual loaded values; never persist the credential-bearing material.
        let material = format!(
            "{agent:?}{model:?}{}{system_prompt}{}",
            provider.base_url,
            serde_json::to_string(&mcps.iter().map(|mcp| &mcp.server).collect::<Vec<_>>())?
        );
        Ok(Self {
            name,
            model,
            provider,
            system_prompt,
            mcps,
            builtin_tools: agent.builtin_tools,
            fingerprint: fingerprint(material.as_bytes()),
        })
    }
}
