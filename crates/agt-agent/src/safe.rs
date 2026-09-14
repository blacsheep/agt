use agt_core::{CategoryName, QualifiedRef};
use serde::Serialize;

use crate::{AgentDiagnostic, AgentStore, StoredAgent};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentRefSummary {
    pub category: String,
    pub name: String,
}

impl AgentRefSummary {
    pub fn from_ref<T: ToString>(reference: &QualifiedRef<T>) -> Self {
        Self {
            category: reference.category().to_string(),
            name: reference.name().to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentSummary {
    pub category: String,
    pub name: String,
    pub description: String,
    pub model: AgentRefSummary,
    pub prompt: AgentRefSummary,
    pub skills: Vec<String>,
    pub mcps: Vec<AgentRefSummary>,
    pub builtin_tools: Vec<crate::BuiltinTool>,
    pub path: String,
}

impl From<&StoredAgent> for AgentSummary {
    fn from(agent: &StoredAgent) -> Self {
        Self {
            category: agent.category.to_string(),
            name: agent.config.name.to_string(),
            description: agent.config.description.clone(),
            model: AgentRefSummary::from_ref(&agent.config.model),
            prompt: AgentRefSummary::from_ref(&agent.config.prompt),
            skills: agent
                .config
                .skills
                .iter()
                .map(ToString::to_string)
                .collect(),
            mcps: agent
                .config
                .mcps
                .iter()
                .map(AgentRefSummary::from_ref)
                .collect(),
            builtin_tools: agent.config.builtin_tools.clone(),
            path: agent.path.display().to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentDiagnosticSummary {
    pub code: String,
    pub message: String,
    pub target: String,
    pub remediation: String,
    pub name_hint: Option<String>,
}

impl From<&AgentDiagnostic> for AgentDiagnosticSummary {
    fn from(diagnostic: &AgentDiagnostic) -> Self {
        Self {
            code: diagnostic.category.to_string(),
            message: diagnostic.message.clone(),
            target: diagnostic.path.display().to_string(),
            remediation: diagnostic.remediation.clone(),
            name_hint: diagnostic.name_hint.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentScanResult {
    pub category: Option<String>,
    pub items: Vec<AgentSummary>,
    pub diagnostics: Vec<AgentDiagnosticSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentDeleteResult {
    pub category: String,
    pub name: String,
    pub path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentQueryResult {
    pub category: String,
    pub agent: String,
    pub model_alias: String,
    pub provider: String,
    pub provider_model: String,
    pub content: String,
}

impl AgentQueryResult {
    pub fn new(
        category: &CategoryName,
        agent: impl Into<String>,
        model_alias: impl Into<String>,
        provider: impl Into<String>,
        provider_model: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            category: category.to_string(),
            agent: agent.into(),
            model_alias: model_alias.into(),
            provider: provider.into(),
            provider_model: provider_model.into(),
            content: content.into(),
        }
    }
}

impl AgentStore {
    pub fn scan_summary(&self) -> crate::Result<AgentScanResult> {
        let scan = self.scan()?;
        Ok(AgentScanResult {
            category: None,
            items: scan.items.iter().map(AgentSummary::from).collect(),
            diagnostics: scan
                .diagnostics
                .iter()
                .map(AgentDiagnosticSummary::from)
                .collect(),
        })
    }

    pub fn scan_summary_in(&self, category: &CategoryName) -> crate::Result<AgentScanResult> {
        let category_dir = self.category_dir(category);
        let mut scan = self.scan()?;
        scan.items.retain(|agent| agent.category == *category);
        scan.diagnostics
            .retain(|diagnostic| diagnostic.path.starts_with(&category_dir));
        Ok(AgentScanResult {
            category: Some(category.to_string()),
            items: scan.items.iter().map(AgentSummary::from).collect(),
            diagnostics: scan
                .diagnostics
                .iter()
                .map(AgentDiagnosticSummary::from)
                .collect(),
        })
    }

    pub fn read_summary_in(
        &self,
        category: &CategoryName,
        name: &str,
    ) -> crate::Result<AgentSummary> {
        Ok(AgentSummary::from(&self.read_in(category, name)?))
    }

    pub fn delete_summary_in(
        &self,
        category: &CategoryName,
        name: &str,
    ) -> crate::Result<AgentDeleteResult> {
        let path = self.delete_in(category, name)?;
        Ok(AgentDeleteResult {
            category: category.to_string(),
            name: name.to_string(),
            path: path.display().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use agt_model::{ModelConfig, ModelReference, ModelStore};
    use agt_prompt::PromptStore;
    use agt_provider::{ProviderConfig, ProviderStore};

    use super::*;
    use crate::AgentConfig;

    fn test_home(name: &str) -> std::path::PathBuf {
        let home =
            std::env::temp_dir().join(format!("agt-agent-safe-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        home
    }

    fn create_dependencies(home: &std::path::Path) {
        ProviderStore::new(home)
            .create(
                &ProviderConfig::openai_compatible(
                    "provider",
                    "http://127.0.0.1:9/v1",
                    "private-key",
                )
                .unwrap(),
            )
            .unwrap();
        ModelStore::new(home)
            .create(
                &ModelConfig::new(
                    "model",
                    ModelReference::parse("provider/provider-model").unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
        PromptStore::new(home).ensure_default_prompt().unwrap();
    }

    #[test]
    fn summary_preserves_exact_refs_without_dependency_contents() {
        let home = test_home("summary");
        create_dependencies(&home);
        let store = AgentStore::new(&home);
        let mut config = AgentConfig::new("coder", "model").unwrap();
        config.set_description("Code assistant");
        let stored = store.create(&config).unwrap();

        let summary = AgentSummary::from(&stored);

        let model = summary.model;
        assert_eq!(model.category, "default");
        assert_eq!(model.name, "model");
        assert_eq!(summary.prompt.name, "default");
        assert_eq!(summary.description, "Code assistant");
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn scan_summary_keeps_valid_items_and_diagnostics() {
        let home = test_home("scan");
        create_dependencies(&home);
        let store = AgentStore::new(&home);
        store
            .create(&AgentConfig::new("coder", "model").unwrap())
            .unwrap();
        fs::create_dir(home.join("agents/default/broken")).unwrap();

        let scan = store.scan_summary_in(&CategoryName::default()).unwrap();

        assert_eq!(scan.items.len(), 1);
        assert_eq!(scan.diagnostics.len(), 1);
        assert_eq!(scan.diagnostics[0].code, "missing_config_file");
        let _ = fs::remove_dir_all(home);
    }
}
