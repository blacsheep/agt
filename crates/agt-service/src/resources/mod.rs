//! Shared configuration-resource operations. Adapters never mutate stored configs.
mod agent;
mod harness;
mod mcp;
mod model;
mod mutation;
mod prompt;
mod provider;
mod skill;
use crate::deletion::{ResourceKey, ResourceKind};
use agt_agent::{AgentConfig, AgentStore, BuiltinTool};
use agt_core::{
    CategoryName, HarnessId, McpId, ModelId, PackageRef, PromptId, QualifiedRef, ResourceRef,
};
use agt_mcp::{McpDocument, McpStore, McpWritePlan};
use agt_model::{ModelConfig, ModelConfigUpdate, ModelReference, ModelStore, ModelSummary};
use agt_prompt::{PromptContentResult, PromptStore};
use agt_provider::{ProviderConfig, ProviderConfigUpdate, ProviderStore, ProviderSummary};
use agt_skill::{SkillCreateOptions, SkillStore};
use anyhow::{Result, bail};
pub use mutation::WriteOptions;
pub(crate) use mutation::lock;
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
        self.list_in(kind, None)
    }
    pub fn list_in(&self, kind: &str, category: Option<&CategoryName>) -> Result<Value> {
        let _guard = lock(&self.home)?;
        let h = &self.home;
        Ok(match kind {
            "harness" => {
                let mut value = agt_harness::HarnessStore::new(h).scan_summary()?;
                if let Some(c) = category {
                    value["items"]
                        .as_array_mut()
                        .unwrap()
                        .retain(|item| item["category"] == c.to_string());
                }
                value
            }
            "provider" => serde_json::to_value(match category {
                Some(c) => ProviderStore::new(h).scan_summary_in(c)?,
                None => ProviderStore::new(h).scan_summary()?,
            })?,
            "model" => serde_json::to_value(match category {
                Some(c) => ModelStore::new(h).scan_summary_in(c)?,
                None => ModelStore::new(h).scan_summary()?,
            })?,
            "prompt" => serde_json::to_value(match category {
                Some(c) => PromptStore::new(h).scan_summary_in(c)?,
                None => PromptStore::new(h).scan_summary()?,
            })?,
            "skill" => serde_json::to_value(match category {
                Some(c) => SkillStore::new(h).scan_summary_in(c)?,
                None => SkillStore::new(h).scan_summary()?,
            })?,
            "mcp" => serde_json::to_value(match category {
                Some(c) => McpStore::new(h).scan_summary_in(c)?,
                None => McpStore::new(h).scan_summary()?,
            })?,
            "agent" => serde_json::to_value(match category {
                Some(c) => AgentStore::new(h).scan_summary_in(c)?,
                None => AgentStore::new(h).scan_summary()?,
            })?,
            _ => bail!("unknown resource kind"),
        })
    }
    pub fn read(&self, kind: &str, reference: &str) -> Result<Value> {
        let _guard = lock(&self.home)?;
        self.read_unlocked(kind, reference)
    }
    fn read_unlocked(&self, kind: &str, reference: &str) -> Result<Value> {
        let (category, name) = resource_ref(kind, reference)?;
        let h = &self.home;
        let mut value = match kind {
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
        };
        value["revision"] = json!(self.revision(kind, reference)?);
        Ok(value)
    }
    /// Typed, allowlisted resource payloads. No arbitrary filesystem reads.
    pub fn write(&self, kind: &str, reference: &str, create: bool, data: Value) -> Result<Value> {
        self.write_with(kind, reference, create, data, &WriteOptions::default())
    }
    pub fn write_with(
        &self,
        kind: &str,
        reference: &str,
        create: bool,
        mut data: Value,
        options: &WriteOptions,
    ) -> Result<Value> {
        let _guard = lock(&self.home)?;
        options.validate(kind, create, &mut data)?;
        if let Some(expected) = &options.expected_revision
            && self.revision(kind, reference)? != *expected
        {
            return Err(crate::ToolError::new(
                "conflict",
                "Resource changed; read the latest revision before updating.",
            )
            .into());
        }
        let (category, name) = resource_ref(kind, reference)?;
        let h = &self.home;
        match kind {
            "harness" => harness::write(h, &category, &name, create, data)?,
            "provider" => provider::write(h, &category, &name, create, data)?,
            "model" => model::write(h, &category, &name, create, data)?,
            "prompt" => prompt::write(h, &category, &name, create, data)?,
            "skill" => skill::write(h, &category, &name, create, data)?,
            "mcp" => mcp::write(h, &category, &name, create, data)?,
            "agent" => agent::write(h, &category, &name, create, data, options)?,
            _ => bail!("unknown resource kind"),
        }
        self.read_unlocked(kind, reference)
    }
    pub fn harness_test(&self, reference: &str) -> Result<Value> {
        let (category, name) = resource_ref("harness", reference)?;
        let config = {
            let _guard = lock(&self.home)?;
            agt_harness::HarnessStore::new(&self.home)
                .read_in(&category, &name)?
                .config
        };
        agt_harness::Codex::test(&config)
    }

    pub fn provider_models(&self, reference: &str) -> Result<Value> {
        let (category, name) = resource_ref("provider", reference)?;
        let provider = {
            let _guard = lock(&self.home)?;
            ProviderStore::new(&self.home)
                .read_in(&category, &name)?
                .config
        };
        let models = agt_provider::list_openai_compatible_models(&provider)?;
        Ok(serde_json::to_value(
            agt_provider::ProviderModelsResult::new(&category, &name, models),
        )?)
    }
    pub fn test(&self, kind: &str, reference: &str) -> Result<Value> {
        let _guard = lock(&self.home)?;
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
                    json!({"resource": self.read_unlocked(kind, reference)?, "status": "valid", "connected": false}),
                )
            }
            _ => bail!("configuration testing supports provider and mcp only"),
        }
    }
    /// Explicitly starts/connects the configured server; never calls its tools.
    pub fn mcp_test(&self, reference: &str) -> Result<agt_mcp::config::McpTestResult> {
        let _guard = lock(&self.home)?;
        let (category, name) = resource_ref("mcp", reference)?;
        Ok(McpStore::new(&self.home)
            .read_in(&category, &name)?
            .document
            .test_config()?)
    }
    pub fn mcp_probe(&self, reference: &str) -> Result<Value> {
        let (category, name) = resource_ref("mcp", reference)?;
        let document = {
            let _guard = lock(&self.home)?;
            McpStore::new(&self.home)
                .read_in(&category, &name)?
                .document
        };
        let result = document.smoke_test()?;
        let summary = agt_mcp::McpTestSummary {
            category: category.to_string(),
            name: result.name,
            transport: result.transport,
            target: result.target,
            cwd: document.stdio_cwd(),
            tool_count: result.tool_count,
            tool_names: result.tool_names,
            status: result.smoke,
        };
        Ok(json!({"connected":true,"schemas_valid":true,"probe":summary}))
    }

    pub fn agent_mcps(&self, agent: &str) -> Result<Value> {
        let _guard = lock(&self.home)?;
        let (category, name) = resource_ref("agent", agent)?;
        let config = AgentStore::new(&self.home)
            .read_in(&category, &name)?
            .config;
        Ok(
            json!({"agent":format!("{category}/{name}"),"mcps":config.mcps,
            "effective":"next_run", "native_self_update":"after_tool_batch_if_refresh_succeeds", "authorization":"Every MCP call goes through the workspace permission policy; there is no separate binding or discussion MCP switch."}),
        )
    }

    pub fn category_list(&self, kind: &str) -> Result<Vec<CategoryName>> {
        let _guard = lock(&self.home)?;
        Ok(self.categories(kind)?.list()?)
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Text {
    text: String,
}
