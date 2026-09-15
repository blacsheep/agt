use crate::deletion::{ResourceKey, ResourceKind};
use agt_agent::{AgentConfig, AgentStore, BuiltinTool};
use agt_core::{
    CategoryName, HarnessId, McpId, ModelId, PackageRef, PromptId, QualifiedRef, ResourceRef,
};
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
            "harness" => agt_harness::HarnessStore::new(h).scan_summary()?,
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
            "harness" => {
                serde_json::to_value(agt_harness::HarnessStore::new(h).read_in(&category, &name)?)?
            }
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
            "harness" => {
                let config = if create {
                    data
                } else {
                    let mut old = serde_json::to_value(
                        agt_harness::HarnessStore::new(h)
                            .read_in(&category, &name)?
                            .config,
                    )?;
                    for (key, value) in data
                        .as_object()
                        .ok_or_else(|| anyhow::anyhow!("data must be an object"))?
                    {
                        old[key] = value.clone();
                    }
                    old
                };
                agt_harness::HarnessStore::new(h).write_in(
                    &category,
                    &name,
                    &serde_json::from_value(config)?,
                    create,
                )?;
            }
            "provider" => {
                if create {
                    let d: ProviderCreate = serde_json::from_value(data)?;
                    let secret = provider_secret(d.api_key, d.api_key_env)?.ok_or_else(|| {
                        crate::ToolError::invalid(
                            Some("data.api_key"),
                            "Provider creation requires api_key or api_key_env.",
                        )
                    })?;
                    ProviderStore::new(h).create_in(
                        &category,
                        &ProviderConfig::openai_compatible(name.clone(), d.base_url, secret)?,
                    )?;
                } else {
                    let d: ProviderUpdate = serde_json::from_value(data)?;
                    let secret = provider_secret(d.api_key, d.api_key_env)?;
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
                    return Err(crate::ToolError::invalid(
                        Some("data.mcpServers"),
                        "MCP server name must match the resource name.",
                    )
                    .into());
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
                    let mut config = if let Some(harness) = &d.harness {
                        let (c, n) = crate::named_ref(harness)?;
                        AgentConfig::external(&name, QualifiedRef::new(c, HarnessId::new(n)))?
                    } else {
                        let model = d
                            .model
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("model or harness required"))?;
                        let (_, leaf) = crate::named_ref(model)?;
                        AgentConfig::new(&name, leaf)?
                    };
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
    pub fn provider_models(&self, reference: &str) -> Result<Value> {
        let (category, name) = resource_ref("provider", reference)?;
        Ok(serde_json::to_value(
            ProviderStore::new(&self.home).list_models_summary_in(&category, &name)?,
        )?)
    }
    pub fn test(&self, kind: &str, reference: &str) -> Result<Value> {
        let (category, name) = resource_ref(kind, reference)?;
        match kind {
            "provider" => Ok(serde_json::to_value(
                ProviderStore::new(&self.home).validate_summary_in(&category, &name)?,
            )?),
            "mcp" => {
                McpStore::new(&self.home)
                    .read_in(&category, &name)?
                    .document
                    .validate()?;
                Ok(
                    json!({"resource": self.read(kind, reference)?, "status": "valid", "connected": false}),
                )
            }
            _ => bail!("configuration testing supports provider and mcp only"),
        }
    }
    pub fn categories(&self, kind: &str) -> Result<agt_config::DefinitionCategories> {
        let h = &self.home;
        Ok(match kind {
            "harness" => agt_harness::HarnessStore::new(h).categories(),
            "provider" => ProviderStore::new(h).categories(),
            "model" => ModelStore::new(h).categories(),
            "prompt" => PromptStore::new(h).categories(),
            "skill" => SkillStore::new(h).categories(),
            "mcp" => McpStore::new(h).categories(),
            "agent" => AgentStore::new(h).categories(),
            _ => bail!("unknown resource kind"),
        })
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
            "harness" => ResourceKind::Harness,
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
        let _: PackageRef = name.parse().map_err(|_| crate::ToolError::invalid(
            Some("reference"),
            "Skill names require lowercase letters, digits and single hyphens; use package or @scope/package (each component at most 64 characters).",
        ))?;
        Ok((CategoryName::new(category)?, name.into()))
    } else {
        crate::named_ref(raw)
    }
}

fn provider_secret(key: Option<String>, env: Option<String>) -> Result<Option<String>> {
    match (key, env) {
        (Some(_), Some(_)) => Err(crate::ToolError::invalid(
            Some("data.api_key"),
            "Use api_key or api_key_env, not both.",
        )
        .into()),
        (Some(key), None) if key.trim().is_empty() => Err(crate::ToolError::invalid(
            Some("data.api_key"),
            "API key must not be empty.",
        )
        .into()),
        (Some(key), None) => Ok(Some(key)),
        (None, Some(env)) => provider_credential(&env).map(Some),
        (None, None) => Ok(None),
    }
}

fn provider_credential(key: &str) -> Result<String> {
    let secret = std::env::var(key.trim()).map_err(|_| crate::ToolError::invalid(
        Some("data.api_key_env"),
        "Provider credential environment variable is unavailable. Set it before starting the Web server, then restart the server.",
    ))?;
    if secret.trim().is_empty() {
        return Err(crate::ToolError::invalid(
            Some("data.api_key_env"),
            "Provider credential environment variable is empty. Set a non-empty API key and restart the Web server.",
        ).into());
    }
    Ok(secret)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderCreate {
    base_url: String,
    api_key: Option<String>,
    api_key_env: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderUpdate {
    base_url: Option<String>,
    api_key: Option<String>,
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
    harness: Option<String>,
    model: Option<String>,
    prompt: Option<String>,
    skills: Option<Vec<String>>,
    mcps: Option<Vec<String>>,
    builtin_tools: Option<Vec<BuiltinTool>>,
    description: Option<String>,
}
fn apply_agent(config: &mut AgentConfig, d: &AgentWrite) -> Result<()> {
    if d.model.is_some() && d.harness.is_some() {
        bail!("choose model or harness, not both");
    }
    if let Some(value) = &d.harness {
        let (c, n) = crate::named_ref(value)?;
        config.harness = Some(QualifiedRef::new(c, HarnessId::new(n)));
        config.model = None;
    }
    if let Some(value) = &d.model {
        let (c, n) = crate::named_ref(value)?;
        config.model = Some(QualifiedRef::new(c, ModelId::new(n)));
        config.harness = None;
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
    if let Some(values) = &d.builtin_tools {
        config.builtin_tools = values.clone();
    }
    if let Some(value) = &d.description {
        config.description = value.clone();
    }
    Ok(())
}
