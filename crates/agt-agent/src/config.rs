use agt_core::{
    AgentId, CategoryName, McpId, ModelId, PackageRef, PromptId, QualifiedRef, ResourceRef,
};
use serde::{Deserialize, Serialize};

use crate::error::{AgentError, Result};
use crate::name::validate_agent_name;

pub const AGENT_FILE: &str = "agent.yaml";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinTool {
    ResourceManagement,
}
impl std::fmt::Display for BuiltinTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("resource_management")
    }
}
impl std::str::FromStr for BuiltinTool {
    type Err = AgentError;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "resource_management" => Ok(Self::ResourceManagement),
            _ => Err(AgentError::InvalidConfig {
                message: "内置工具组无效，可选值为 resource_management".into(),
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentConfig {
    pub name: AgentId,
    pub description: String,
    pub model: QualifiedRef<ModelId>,
    pub prompt: QualifiedRef<PromptId>,
    pub skills: Vec<ResourceRef>,
    pub mcps: Vec<QualifiedRef<McpId>>,
    pub builtin_tools: Vec<BuiltinTool>,
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
            description: String::new(),
            model: QualifiedRef::in_default(ModelId::new(model)),
            prompt: QualifiedRef::in_default(PromptId::new(agt_prompt::DEFAULT_PROMPT_NAME)),
            skills: Vec::new(),
            mcps: Vec::new(),
            builtin_tools: Vec::new(),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn set_description(&mut self, description: impl Into<String>) {
        self.description = description.into();
    }

    pub fn set_prompt(&mut self, prompt: QualifiedRef<PromptId>) {
        self.prompt = prompt;
    }

    pub fn validate(&self) -> Result<()> {
        validate_agent_name(self.name.as_str())?;
        agt_model::validate_model_name(self.model.name().as_str()).map_err(|error| {
            AgentError::InvalidConfig {
                message: error.to_string(),
            }
        })?;
        agt_prompt::validate_prompt_name(self.prompt.name().as_str()).map_err(|error| {
            AgentError::InvalidConfig {
                message: error.to_string(),
            }
        })?;

        for mcp in &self.mcps {
            agt_mcp::validate_mcp_name(mcp.name().as_str()).map_err(|error| {
                AgentError::InvalidConfig {
                    message: error.to_string(),
                }
            })?;
        }
        ensure_unique_refs("skill", self.skills.iter())?;
        ensure_unique_refs("mcp", self.mcps.iter())?;
        ensure_unique_refs("builtin_tool", self.builtin_tools.iter())?;

        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct AgentConfigYaml {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub model: QualifiedRef<ModelId>,
    pub prompt: QualifiedRef<PromptId>,
    #[serde(default)]
    pub skills: Vec<SkillAttachmentYaml>,
    #[serde(default)]
    pub mcps: Vec<QualifiedRef<McpId>>,
    #[serde(default)]
    pub builtin_tools: Vec<BuiltinTool>,
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
    },
}

impl TryFrom<AgentConfigYaml> for AgentConfig {
    type Error = AgentError;

    fn try_from(value: AgentConfigYaml) -> Result<Self> {
        let config = Self {
            name: AgentId::new(value.name),
            description: value.description,
            model: value.model,
            prompt: value.prompt,
            skills: value
                .skills
                .into_iter()
                .map(ResourceRef::try_from)
                .collect::<Result<Vec<_>>>()?,
            mcps: value.mcps,
            builtin_tools: value.builtin_tools,
        };
        config.validate()?;
        Ok(config)
    }
}

impl From<&AgentConfig> for AgentConfigYaml {
    fn from(value: &AgentConfig) -> Self {
        Self {
            name: value.name.to_string(),
            description: value.description.clone(),
            model: value.model.clone(),
            prompt: value.prompt.clone(),
            skills: value.skills.iter().map(SkillAttachmentYaml::from).collect(),
            mcps: value.mcps.clone(),
            builtin_tools: value.builtin_tools.clone(),
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
            SkillAttachmentYaml::Detailed { category, package } => {
                Ok(ResourceRef::persisted_in(category, package))
            }
        }
    }
}

impl From<&ResourceRef> for SkillAttachmentYaml {
    fn from(value: &ResourceRef) -> Self {
        if value.category().is_default() {
            Self::Scalar(value.package().to_string())
        } else {
            Self::Detailed {
                category: value.category().clone(),
                package: value.package().clone(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_tools_default_empty_and_validate_round_trip() {
        let yaml = "name: coder\nmodel: {category: default, name: fast}\nprompt: {category: default, name: default}\n";
        let mut config =
            AgentConfig::try_from(serde_yaml::from_str::<AgentConfigYaml>(yaml).unwrap()).unwrap();
        assert!(config.builtin_tools.is_empty());
        config.builtin_tools.push(BuiltinTool::ResourceManagement);
        let encoded = serde_yaml::to_string(&AgentConfigYaml::from(&config)).unwrap();
        assert_eq!(
            AgentConfig::try_from(serde_yaml::from_str::<AgentConfigYaml>(&encoded).unwrap())
                .unwrap(),
            config
        );
        config.builtin_tools.push(BuiltinTool::ResourceManagement);
        assert!(config.validate().is_err());
        assert!(
            serde_yaml::from_str::<AgentConfigYaml>(&format!("{yaml}builtin_tools: [unknown]\n"))
                .is_err()
        );
    }

    #[test]
    fn config_round_trips_scoped_attachments() {
        let mut config = AgentConfig::new("coder", "fast-model").unwrap();
        config.skills.push(ResourceRef::persisted_in(
            CategoryName::new("team").unwrap(),
            "@acme/review".parse().unwrap(),
        ));
        let yaml = serde_yaml::to_string(&AgentConfigYaml::from(&config)).unwrap();
        let loaded =
            AgentConfig::try_from(serde_yaml::from_str::<AgentConfigYaml>(&yaml).unwrap()).unwrap();
        assert_eq!(loaded, config);
        assert!(!yaml.contains("interaction:"));
        assert!(!yaml.contains("model_policy:"));
    }

    #[test]
    fn rejects_duplicate_attachments() {
        let mut config = AgentConfig::new("coder", "fast-model").unwrap();
        let skill = ResourceRef::persisted("review".parse().unwrap());
        config.skills = vec![skill.clone(), skill];
        assert!(config.validate().is_err());
    }
}
