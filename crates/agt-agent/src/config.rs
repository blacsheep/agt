use agt_core::{
    AgentId, CategoryName, GatewayDomain, McpId, ModelId, PackageRef, PromptId, QualifiedRef,
    ResourceRef, ResourceSourceSelector,
};
use serde::{Deserialize, Serialize};

use crate::error::{AgentError, Result};
use crate::name::validate_agent_name;

pub const AGENT_FILE: &str = "agent.yaml";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentScope {
    Global,
}

impl AgentScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
        }
    }
}

impl std::fmt::Display for AgentScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    #[default]
    Standard,
    Gateway,
}

impl AgentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Gateway => "gateway",
        }
    }
}

impl std::fmt::Display for AgentKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelPolicy {
    #[default]
    Exact,
    InheritLead,
}

impl AgentModelPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::InheritLead => "inherit_lead",
        }
    }
}

impl std::fmt::Display for AgentModelPolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentMemory {
    pub enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentInteraction {
    pub role: String,
    pub speak_policy: String,
}

impl Default for AgentInteraction {
    fn default() -> Self {
        Self {
            role: "assistant".to_string(),
            speak_policy: "when_mentioned_or_assigned".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentConfig {
    pub name: AgentId,
    pub scope: AgentScope,
    pub description: String,
    pub kind: AgentKind,
    pub gateway_domain: Option<GatewayDomain>,
    pub model_policy: AgentModelPolicy,
    pub model: Option<QualifiedRef<ModelId>>,
    pub prompt: QualifiedRef<PromptId>,
    pub skills: Vec<ResourceRef>,
    pub mcps: Vec<QualifiedRef<McpId>>,
    pub memory: AgentMemory,
    pub interaction: AgentInteraction,
}

impl AgentConfig {
    pub fn new(name: impl Into<String>, model: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let model = model.into();
        validate_agent_name(&name)?;
        agt_model::validate_model_name(&model).map_err(|error| AgentError::InvalidConfig {
            message: error.to_string(),
        })?;

        let config = Self {
            name: AgentId::new(name),
            scope: AgentScope::Global,
            description: String::new(),
            kind: AgentKind::Standard,
            gateway_domain: None,
            model_policy: AgentModelPolicy::Exact,
            model: Some(QualifiedRef::in_default(ModelId::new(model))),
            prompt: QualifiedRef::in_default(PromptId::new(agt_prompt::DEFAULT_PROMPT_NAME)),
            skills: Vec::new(),
            mcps: Vec::new(),
            memory: AgentMemory::default(),
            interaction: AgentInteraction::default(),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn gateway(
        name: impl Into<String>,
        domain: GatewayDomain,
        prompt: QualifiedRef<PromptId>,
    ) -> Result<Self> {
        let config = Self {
            name: AgentId::new(name.into()),
            scope: AgentScope::Global,
            description: String::new(),
            kind: AgentKind::Gateway,
            gateway_domain: Some(domain),
            model_policy: AgentModelPolicy::InheritLead,
            model: None,
            prompt,
            skills: Vec::new(),
            mcps: Vec::new(),
            memory: AgentMemory::default(),
            interaction: AgentInteraction::default(),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn exact_model(&self) -> Option<&QualifiedRef<ModelId>> {
        self.model.as_ref()
    }

    pub fn is_gateway(&self) -> bool {
        self.kind == AgentKind::Gateway
    }

    pub fn set_description(&mut self, description: impl Into<String>) {
        self.description = description.into();
    }

    pub fn set_prompt(&mut self, prompt: QualifiedRef<PromptId>) {
        self.prompt = prompt;
    }

    pub fn validate(&self) -> Result<()> {
        validate_agent_name(self.name.as_str())?;
        match (
            self.kind,
            self.gateway_domain,
            self.model_policy,
            self.model.as_ref(),
        ) {
            (AgentKind::Standard, None, AgentModelPolicy::Exact, Some(model)) => {
                agt_model::validate_model_name(model.name().as_str()).map_err(|error| {
                    AgentError::InvalidConfig {
                        message: error.to_string(),
                    }
                })?;
            }
            (AgentKind::Gateway, Some(_), AgentModelPolicy::InheritLead, None) => {}
            _ => {
                return Err(AgentError::InvalidConfig {
                    message: "agent kind、gateway_domain、model_policy 与 model 组合无效"
                        .to_string(),
                });
            }
        }
        agt_prompt::validate_prompt_name(self.prompt.name().as_str()).map_err(|error| {
            AgentError::InvalidConfig {
                message: error.to_string(),
            }
        })?;

        for skill in &self.skills {
            if matches!(skill.source(), ResourceSourceSelector::Registry { .. }) {
                return Err(AgentError::InvalidConfig {
                    message: format!("skill registry source 尚未激活：{skill}"),
                });
            }
        }
        for mcp in &self.mcps {
            agt_mcp::validate_mcp_name(mcp.name().as_str()).map_err(|error| {
                AgentError::InvalidConfig {
                    message: error.to_string(),
                }
            })?;
        }
        ensure_unique_refs("skill", self.skills.iter())?;
        ensure_unique_refs("mcp", self.mcps.iter())?;

        if self.is_gateway()
            && (!self.skills.is_empty() || !self.mcps.is_empty() || self.memory.enabled)
        {
            return Err(AgentError::InvalidConfig {
                message: "gateway agent 不允许普通资源挂载或 memory 配置".to_string(),
            });
        }

        if self.interaction.role.trim().is_empty() {
            return Err(AgentError::InvalidConfig {
                message: "interaction.role 不能为空".to_string(),
            });
        }
        if self.interaction.speak_policy.trim().is_empty() {
            return Err(AgentError::InvalidConfig {
                message: "interaction.speak_policy 不能为空".to_string(),
            });
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct AgentConfigYaml {
    pub name: String,
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "is_standard_kind")]
    pub kind: AgentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_domain: Option<GatewayDomain>,
    #[serde(default, skip_serializing_if = "is_exact_model_policy")]
    pub model_policy: AgentModelPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<QualifiedRef<ModelId>>,
    pub prompt: QualifiedRef<PromptId>,
    #[serde(default)]
    pub skills: Vec<SkillAttachmentYaml>,
    #[serde(default)]
    pub mcps: Vec<QualifiedRef<McpId>>,
    #[serde(default)]
    pub memory: AgentMemoryYaml,
    #[serde(default)]
    pub interaction: AgentInteractionYaml,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct AgentMemoryYaml {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct AgentInteractionYaml {
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default = "default_speak_policy")]
    pub speak_policy: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum SkillAttachmentYaml {
    Scalar(String),
    Detailed {
        #[serde(default)]
        category: CategoryName,
        #[serde(rename = "ref")]
        package: PackageRef,
        source: ResourceSourceSelector,
    },
}

impl Default for AgentInteractionYaml {
    fn default() -> Self {
        Self {
            role: default_role(),
            speak_policy: default_speak_policy(),
        }
    }
}

impl TryFrom<AgentConfigYaml> for AgentConfig {
    type Error = AgentError;

    fn try_from(value: AgentConfigYaml) -> Result<Self> {
        let scope = match value.scope.as_str() {
            "global" => AgentScope::Global,
            _ => {
                return Err(AgentError::InvalidConfig {
                    message: format!("scope 无效：{}，MVP 仅支持 global", value.scope),
                });
            }
        };
        let config = Self {
            name: AgentId::new(value.name),
            scope,
            description: value.description,
            kind: value.kind,
            gateway_domain: value.gateway_domain,
            model_policy: value.model_policy,
            model: value.model,
            prompt: value.prompt,
            skills: value
                .skills
                .into_iter()
                .map(ResourceRef::try_from)
                .collect::<Result<Vec<_>>>()?,
            mcps: value.mcps,
            memory: AgentMemory {
                enabled: value.memory.enabled,
            },
            interaction: AgentInteraction {
                role: value.interaction.role,
                speak_policy: value.interaction.speak_policy,
            },
        };
        config.validate()?;
        Ok(config)
    }
}

impl From<&AgentConfig> for AgentConfigYaml {
    fn from(value: &AgentConfig) -> Self {
        Self {
            name: value.name.to_string(),
            scope: value.scope.to_string(),
            description: value.description.clone(),
            kind: value.kind,
            gateway_domain: value.gateway_domain,
            model_policy: value.model_policy,
            model: value.model.clone(),
            prompt: value.prompt.clone(),
            skills: value.skills.iter().map(SkillAttachmentYaml::from).collect(),
            mcps: value.mcps.clone(),
            memory: AgentMemoryYaml {
                enabled: value.memory.enabled,
            },
            interaction: AgentInteractionYaml {
                role: value.interaction.role.clone(),
                speak_policy: value.interaction.speak_policy.clone(),
            },
        }
    }
}

fn ensure_unique_refs<'a, T>(kind: &'static str, values: impl Iterator<Item = &'a T>) -> Result<()>
where
    T: Eq + std::fmt::Display + 'a,
{
    let mut seen = Vec::new();
    for value in values {
        if seen.contains(&value) {
            return Err(AgentError::InvalidConfig {
                message: format!("{kind} 重复挂载：{value}"),
            });
        }
        seen.push(value);
    }
    Ok(())
}

impl TryFrom<SkillAttachmentYaml> for ResourceRef {
    type Error = AgentError;

    fn try_from(value: SkillAttachmentYaml) -> Result<Self> {
        match value {
            SkillAttachmentYaml::Scalar(raw) => raw
                .parse::<PackageRef>()
                .map(ResourceRef::persisted)
                .map_err(|source| AgentError::InvalidConfig {
                    message: format!("skill attachment ref 无效：{source}"),
                }),
            SkillAttachmentYaml::Detailed {
                category,
                package,
                source,
            } => Ok(ResourceRef::new_in(category, package, source)),
        }
    }
}

impl From<&ResourceRef> for SkillAttachmentYaml {
    fn from(value: &ResourceRef) -> Self {
        if value.category().is_default() && value.source() == &ResourceSourceSelector::Persisted {
            Self::Scalar(value.package().to_string())
        } else {
            Self::Detailed {
                category: value.category().clone(),
                package: value.package().clone(),
                source: value.source().clone(),
            }
        }
    }
}

fn default_scope() -> String {
    "global".to_string()
}

fn is_standard_kind(kind: &AgentKind) -> bool {
    *kind == AgentKind::Standard
}

fn is_exact_model_policy(policy: &AgentModelPolicy) -> bool {
    *policy == AgentModelPolicy::Exact
}

fn default_role() -> String {
    "assistant".to_string()
}

fn default_speak_policy() -> String {
    "when_mentioned_or_assigned".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_agent_config_with_defaults() {
        let config = AgentConfig::new("coder", "fast-model").expect("config");

        assert_eq!(config.scope, AgentScope::Global);
        assert_eq!(config.description, "");
        let model = config.exact_model().expect("exact model");
        assert_eq!(model.name().as_str(), "fast-model");
        assert!(model.category().is_default());
        assert_eq!(config.kind, AgentKind::Standard);
        assert_eq!(config.model_policy, AgentModelPolicy::Exact);
        assert_eq!(config.gateway_domain, None);
        assert_eq!(
            config.prompt,
            QualifiedRef::in_default(PromptId::new(agt_prompt::DEFAULT_PROMPT_NAME))
        );
        assert!(config.skills.is_empty());
        assert!(config.mcps.is_empty());
        assert!(!config.memory.enabled);
        assert_eq!(config.interaction.role, "assistant");
        assert_eq!(
            config.interaction.speak_policy,
            "when_mentioned_or_assigned"
        );
    }

    #[test]
    fn serializes_yaml_with_stable_fields() {
        let mut config = AgentConfig::new("coder", "fast-model").expect("config");
        config.set_description("Backend coding agent");
        config.set_prompt(QualifiedRef::new(
            CategoryName::new("team").unwrap(),
            PromptId::new("coding"),
        ));
        config
            .skills
            .push(ResourceRef::persisted("code-review".parse().unwrap()));
        config
            .mcps
            .push(QualifiedRef::in_default(McpId::new("filesystem")));
        let yaml = serde_yaml::to_string(&AgentConfigYaml::from(&config)).expect("serialize");

        assert!(yaml.contains("name: coder"));
        assert!(yaml.contains("scope: global"));
        assert!(yaml.contains("description: Backend coding agent"));
        assert!(yaml.contains("model:\n  category: default\n  name: fast-model"));
        assert!(yaml.contains("prompt:\n  category: team\n  name: coding"));
        assert!(yaml.contains("- code-review"));
        assert!(yaml.contains("category: default"));
        assert!(yaml.contains("name: filesystem"));
        assert!(yaml.contains("enabled: false"));
        assert!(yaml.contains("role: assistant"));
        assert!(yaml.contains("speak_policy: when_mentioned_or_assigned"));
    }

    #[test]
    fn rejects_duplicate_attachments() {
        let mut config = AgentConfig::new("coder", "fast-model").expect("config");
        let skill = ResourceRef::persisted("code-review".parse().unwrap());
        config.skills.push(skill.clone());
        config.skills.push(skill);

        assert!(matches!(
            config.validate(),
            Err(AgentError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn gateway_config_has_explicit_inherit_lead_policy() {
        let config = AgentConfig::gateway(
            "agent-gateway",
            GatewayDomain::Agent,
            QualifiedRef::new(
                CategoryName::new("internal").unwrap(),
                PromptId::new("agent-gateway"),
            ),
        )
        .expect("gateway config");

        assert!(config.is_gateway());
        assert_eq!(config.gateway_domain, Some(GatewayDomain::Agent));
        assert_eq!(config.model_policy, AgentModelPolicy::InheritLead);
        assert_eq!(config.exact_model(), None);

        let yaml = serde_yaml::to_string(&AgentConfigYaml::from(&config)).unwrap();
        assert!(yaml.contains("kind: gateway"));
        assert!(yaml.contains("gateway_domain: agent"));
        assert!(yaml.contains("model_policy: inherit_lead"));
        assert!(!yaml.contains("\nmodel:"));

        let loaded =
            AgentConfig::try_from(serde_yaml::from_str::<AgentConfigYaml>(&yaml).unwrap()).unwrap();
        assert_eq!(loaded, config);
    }

    #[test]
    fn rejects_mixed_gateway_and_standard_model_fields() {
        let yaml = r#"
name: agent-gateway
kind: gateway
gateway_domain: agent
model_policy: inherit_lead
model:
  category: default
  name: fast-model
prompt:
  category: internal
  name: agent-gateway
"#;
        let raw: AgentConfigYaml = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(
            AgentConfig::try_from(raw),
            Err(AgentError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn rejects_invalid_gateway_field_combinations_and_resources() {
        let gateway_prompt = QualifiedRef::new(
            CategoryName::new("internal").unwrap(),
            PromptId::new("agent-gateway"),
        );
        let mut cases = Vec::new();

        let mut standard_inherit = AgentConfig::new("coder", "fast-model").unwrap();
        standard_inherit.model_policy = AgentModelPolicy::InheritLead;
        cases.push(standard_inherit);

        let mut standard_domain = AgentConfig::new("coder", "fast-model").unwrap();
        standard_domain.gateway_domain = Some(GatewayDomain::Agent);
        cases.push(standard_domain);

        let mut gateway_exact = AgentConfig::gateway(
            "agent-gateway",
            GatewayDomain::Agent,
            gateway_prompt.clone(),
        )
        .unwrap();
        gateway_exact.model_policy = AgentModelPolicy::Exact;
        cases.push(gateway_exact);

        let mut gateway_resource =
            AgentConfig::gateway("agent-gateway", GatewayDomain::Agent, gateway_prompt).unwrap();
        gateway_resource.memory.enabled = true;
        cases.push(gateway_resource);

        for config in cases {
            assert!(matches!(
                config.validate(),
                Err(AgentError::InvalidConfig { .. })
            ));
        }
    }

    #[test]
    fn skill_attachments_read_scalar_and_render_source_qualified_objects() {
        let yaml = r#"
name: coder
model:
  category: default
  name: fast-model
prompt:
  category: default
  name: default
skills:
  - code-review
  - ref: "@acme/debug-db"
    source: project
"#;
        let raw: AgentConfigYaml = serde_yaml::from_str(yaml).unwrap();
        let config = AgentConfig::try_from(raw).unwrap();
        assert_eq!(
            config.skills[0].source(),
            &ResourceSourceSelector::Persisted
        );
        assert_eq!(config.skills[1].package().to_string(), "@acme/debug-db");
        assert_eq!(config.skills[1].source(), &ResourceSourceSelector::Project);

        let rendered = serde_yaml::to_string(&AgentConfigYaml::from(&config)).unwrap();
        assert!(rendered.contains("- code-review"));
        assert!(
            rendered.contains("ref: '@acme/debug-db'") || rendered.contains("ref: @acme/debug-db")
        );
        assert!(rendered.contains("source: project"));
    }
}
