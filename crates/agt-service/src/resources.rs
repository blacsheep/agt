use crate::deletion::{ResourceKey, ResourceKind};
use agt_agent::{AgentConfig, AgentStore};
use agt_core::{CategoryName, McpId, ModelId, PackageRef, PromptId, QualifiedRef, ResourceRef};
use agt_mcp::{McpDocument, McpStore, McpWritePlan};
use agt_model::{ModelConfig, ModelConfigUpdate, ModelReference, ModelStore, ModelSummary};
use agt_prompt::{PromptContentResult, PromptStore};
use agt_provider::{ProviderConfig, ProviderConfigUpdate, ProviderStore, ProviderSummary};
use agt_skill::{SkillCreateOptions, SkillDocument, SkillStore};
use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub struct Resources {
    home: PathBuf,
}
impl Resources {
    pub fn new(home: &Path) -> Self {
        Self {
            home: home.to_owned(),
        }
    }
    pub fn list(&self, kind: &str) -> Result<Value> {
        let h = &self.home;
        Ok(match kind {
            "provider" => serde_json::to_value(ProviderStore::new(h).scan_summary()?)?,
            "model" => serde_json::to_value(ModelStore::new(h).scan_summary()?)?,
            "prompt" => serde_json::to_value(PromptStore::new(h).scan_summary()?)?,
            "skill" => serde_json::to_value(SkillStore::new(h).scan_summary()?)?,
            "mcp" => serde_json::to_value(McpStore::new(h).scan_summary()?)?,
            "agent" => serde_json::to_value(AgentStore::new(h).scan_summary()?)?,
            _ => bail!("unknown resource kind"),
        })
    }
    pub fn read(&self, kind: &str, reference: &str) -> Result<Value> {
        let (category, name) = resource_ref(kind, reference)?;
        let h = &self.home;
        Ok(match kind {
            "provider" => serde_json::to_value(ProviderSummary::from(
                &ProviderStore::new(h).read_in(&category, &name)?,
            ))?,
            "model" => serde_json::to_value(ModelSummary::from(
                &ModelStore::new(h).read_in(&category, &name)?,
            ))?,
            "prompt" => serde_json::to_value(PromptContentResult::try_from(
                &PromptStore::new(h).read_in(&category, &name)?,
            )?)?,
            "skill" => serde_json::to_value(
                SkillStore::new(h).read_document_summary_in(&category, &name.parse()?)?,
            )?,
            "mcp" => serde_json::to_value(McpStore::new(h).info_summary_in(&category, &name)?)?,
            "agent" => serde_json::to_value(AgentStore::new(h).read_summary_in(&category, &name)?)?,
            _ => bail!("unknown resource kind"),
        })
    }
    /// Typed, allowlisted resource payloads. No arbitrary filesystem reads.
    pub fn write(&self, kind: &str, reference: &str, create: bool, data: Value) -> Result<Value> {
        let (category, name) = resource_ref(kind, reference)?;
        let h = &self.home;
        match kind {
            "provider" => {
                if create {
                    let d: ProviderCreate = serde_json::from_value(data)?;
                    let secret = std::env::var(d.api_key_env).map_err(|_| {
                        anyhow::anyhow!("provider credential environment variable is unavailable")
                    })?;
                    ProviderStore::new(h).create_in(
                        &category,
                        &ProviderConfig::openai_compatible(name.clone(), d.base_url, secret)?,
                    )?;
                } else {
                    let d: ProviderUpdate = serde_json::from_value(data)?;
                    let secret = d
                        .api_key_env
                        .map(|key| {
                            std::env::var(key).map_err(|_| {
                                anyhow::anyhow!(
                                    "provider credential environment variable is unavailable"
                                )
                            })
                        })
                        .transpose()?;
                    ProviderStore::new(h).update_in(
                        &category,
                        &name,
                        ProviderConfigUpdate {
                            base_url: d.base_url,
                            api_key: secret,
                        },
                    )?;
                }
            }
            "model" => {
                if create {
                    let d: ModelCreate = serde_json::from_value(data)?;
                    let (cat, provider) = crate::named_ref(&d.provider)?;
                    let mut config =
                        ModelConfig::new(&name, ModelReference::new_in(cat, provider, d.model)?)?;
                    config.apply_update(d.params)?;
                    ModelStore::new(h).create_in(&category, &config)?;
                } else {
                    ModelStore::new(h).update_in(
                        &category,
                        &name,
                        serde_json::from_value::<ModelConfigUpdate>(data)?,
                    )?;
                }
            }
            "prompt" => {
                let d: Text = serde_json::from_value(data)?;
                if create {
                    PromptStore::new(h).create_in(&category, &name, &d.text)?;
                } else {
                    PromptStore::new(h).update_in(&category, &name, &d.text)?;
                }
            }
            "skill" => {
                if create {
                    let d: SkillCreate = serde_json::from_value(data)?;
                    SkillStore::new(h)
                        .create_in(&category, SkillCreateOptions::new(&name, d.description)?)?;
                } else {
                    let d: Text = serde_json::from_value(data)?;
                    let package: PackageRef = name.parse()?;
                    let stored = SkillStore::new(h).read_in(&category, &package)?;
                    let document = SkillDocument::parse(&stored.path, &d.text)?;
                    if document.metadata.package_ref()? != package {
                        bail!("skill package identity mismatch");
                    }
                    agt_storage::write_atomic_under(
                        h,
                        &stored.path,
                        document.render()?.as_bytes(),
                    )?;
                }
            }
            "mcp" => {
                let document =
                    McpDocument::parse(Path::new("mcp.json"), &serde_json::to_string(&data)?)?;
                if document.name.as_str() != name {
                    bail!("MCP server identity mismatch");
                }
                let plan = if create {
                    McpWritePlan::create(document)
                } else {
                    McpWritePlan::replace(document)
                };
                McpStore::new(h).apply_batch_in(&category, vec![plan])?;
            }
            "agent" => {
                let d: AgentWrite = serde_json::from_value(data)?;
                let mutate = |config: &mut AgentConfig| -> agt_agent::Result<()> {
                    apply_agent(config, &d).map_err(|error| agt_agent::AgentError::InvalidConfig {
                        message: error.to_string(),
                    })
                };
                if create {
                    let model = d
                        .model
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("model required"))?;
                    let (_, leaf) = crate::named_ref(model)?;
                    let mut config = AgentConfig::new(&name, leaf)?;
                    mutate(&mut config)?;
                    AgentStore::new(h).create_in(&category, &config)?;
                } else {
                    AgentStore::new(h).update_in(&category, &name, mutate)?;
                }
            }
            _ => bail!("unknown resource kind"),
        }
        self.read(kind, reference)
    }
    pub fn delete(
        &self,
        kind: &str,
        reference: &str,
        recursive: bool,
        confirm: Option<Vec<String>>,
    ) -> Result<Value> {
        let (category, name) = resource_ref(kind, reference)?;
        let kind = match kind {
            "provider" => ResourceKind::Provider,
            "model" => ResourceKind::Model,
            "prompt" => ResourceKind::Prompt,
            "skill" => ResourceKind::Skill,
            "mcp" => ResourceKind::Mcp,
            "agent" => ResourceKind::Agent,
            _ => bail!("unknown resource kind"),
        };
        let target = ResourceKey {
            kind,
            category: category.to_string(),
            name,
        };
        let plan = crate::deletion::plan_delete(&self.home, &target, recursive)?;
        let items = plan.iter().map(ResourceKey::display).collect::<Vec<_>>();
        if let Some(expected) = confirm {
            if expected != items {
                return Err(crate::ToolError::new("confirmation_mismatch", "Fetch a fresh deletion plan by omitting confirm, then submit its exact ordered items.").into());
            }
            crate::deletion::execute_delete(&self.home, &target, recursive, &plan)?;
            Ok(json!({"deleted":items}))
        } else {
            Ok(json!({"items":items,"requires_confirmation":true}))
        }
    }
}

pub fn resource_ref(kind: &str, raw: &str) -> Result<(CategoryName, String)> {
    if kind == "skill" {
        let (category, name) = if raw.starts_with('@') {
            ("default", raw)
        } else {
            raw.split_once('/').unwrap_or(("default", raw))
        };
        let _: PackageRef = name.parse()?;
        Ok((CategoryName::new(category)?, name.into()))
    } else {
        crate::named_ref(raw)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderCreate {
    base_url: String,
    api_key_env: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderUpdate {
    base_url: Option<String>,
    api_key_env: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelCreate {
    provider: String,
    model: String,
    #[serde(default)]
    params: ModelConfigUpdate,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Text {
    text: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillCreate {
    description: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentWrite {
    model: Option<String>,
    prompt: Option<String>,
    skills: Option<Vec<String>>,
    mcps: Option<Vec<String>>,
    description: Option<String>,
}
fn apply_agent(config: &mut AgentConfig, d: &AgentWrite) -> Result<()> {
    if let Some(value) = &d.model {
        let (c, n) = crate::named_ref(value)?;
        config.model = Some(QualifiedRef::new(c, ModelId::new(n)));
    }
    if let Some(value) = &d.prompt {
        let (c, n) = crate::named_ref(value)?;
        config.prompt = QualifiedRef::new(c, PromptId::new(n));
    }
    if let Some(values) = &d.skills {
        config.skills = values
            .iter()
            .map(|v| {
                let (c, n) = resource_ref("skill", v)?;
                Ok(ResourceRef::persisted_in(c, n.parse()?))
            })
            .collect::<Result<_>>()?;
    }
    if let Some(values) = &d.mcps {
        config.mcps = values
            .iter()
            .map(|v| {
                let (c, n) = crate::named_ref(v)?;
                Ok(QualifiedRef::new(c, McpId::new(n)))
            })
            .collect::<Result<_>>()?;
    }
    if let Some(value) = &d.description {
        config.description = value.clone();
    }
    Ok(())
}
