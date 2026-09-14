use agt_core::CategoryName;
use serde::Serialize;

use crate::{ModelDiagnostic, ModelStore, Result, StoredModel};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelProviderSummary {
    pub category: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelParamsSummary {
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    pub timeout: u32,
    pub reasoning_effort: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelCapabilitiesSummary {
    pub context_window: u32,
    pub tool_call: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelSummary {
    pub category: String,
    pub name: String,
    pub provider: ModelProviderSummary,
    pub model: String,
    pub api_format: String,
    pub params: ModelParamsSummary,
    pub capabilities: ModelCapabilitiesSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl From<&StoredModel> for ModelSummary {
    fn from(model: &StoredModel) -> Self {
        Self {
            category: model.category.to_string(),
            name: model.config.name.to_string(),
            provider: ModelProviderSummary {
                category: model.config.provider.category().to_string(),
                name: model.config.provider.name().to_string(),
            },
            model: model.config.model.clone(),
            api_format: model.config.api_format.as_str().to_string(),
            params: ModelParamsSummary {
                temperature: model.config.params.temperature,
                max_tokens: model.config.params.max_tokens,
                timeout: model.config.params.timeout,
                reasoning_effort: model
                    .config
                    .params
                    .reasoning_effort
                    .as_ref()
                    .map(|effort| effort.as_str().to_string()),
            },
            capabilities: ModelCapabilitiesSummary {
                tool_call: model.config.capabilities.tool_call,
                context_window: model.config.capabilities.context_window,
            },
            path: Some(model.path.display().to_string()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelDiagnosticSummary {
    pub code: String,
    pub message: String,
    pub target: String,
    pub remediation: String,
    pub name_hint: Option<String>,
}

impl From<&ModelDiagnostic> for ModelDiagnosticSummary {
    fn from(diagnostic: &ModelDiagnostic) -> Self {
        Self {
            code: diagnostic.category.to_string(),
            message: diagnostic.message.clone(),
            target: diagnostic.path.display().to_string(),
            remediation: diagnostic.remediation.clone(),
            name_hint: diagnostic.name_hint.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelScanResult {
    pub category: Option<String>,
    pub items: Vec<ModelSummary>,
    pub diagnostics: Vec<ModelDiagnosticSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelDeleteResult {
    pub category: String,
    pub name: String,
    pub path: String,
}

impl ModelStore {
    pub fn scan_summary(&self) -> Result<ModelScanResult> {
        let scan = self.scan()?;
        Ok(ModelScanResult {
            category: None,
            items: scan.items.iter().map(ModelSummary::from).collect(),
            diagnostics: scan
                .diagnostics
                .iter()
                .map(ModelDiagnosticSummary::from)
                .collect(),
        })
    }

    pub fn scan_summary_in(&self, category: &CategoryName) -> Result<ModelScanResult> {
        let category_dir = self.category_dir(category);
        let mut scan = self.scan()?;
        scan.items.retain(|model| model.category == *category);
        scan.diagnostics
            .retain(|diagnostic| diagnostic.path.starts_with(&category_dir));
        Ok(ModelScanResult {
            category: Some(category.to_string()),
            items: scan.items.iter().map(ModelSummary::from).collect(),
            diagnostics: scan
                .diagnostics
                .iter()
                .map(ModelDiagnosticSummary::from)
                .collect(),
        })
    }

    pub fn delete_summary_in(
        &self,
        category: &CategoryName,
        name: &str,
    ) -> Result<ModelDeleteResult> {
        let path = self.delete_in(category, name)?;
        Ok(ModelDeleteResult {
            category: category.to_string(),
            name: name.to_string(),
            path: path.display().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use agt_provider::{ProviderConfig, ProviderStore};

    use super::*;
    use crate::{ModelApiFormat, ModelConfig, ModelConfigUpdate, ModelReference, ReasoningEffort};

    fn test_home(name: &str) -> std::path::PathBuf {
        let home =
            std::env::temp_dir().join(format!("agt-model-safe-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        home
    }

    #[test]
    fn summary_preserves_reasoning_capabilities_and_exact_provider_reference() {
        let home = test_home("summary");
        ProviderStore::new(&home)
            .create(
                &ProviderConfig::openai_compatible("provider", "http://localhost/v1", "key")
                    .unwrap(),
            )
            .unwrap();
        let store = ModelStore::new(&home);
        store
            .create(
                &ModelConfig::new(
                    "model",
                    ModelReference::parse("provider/provider-model").unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
        let stored = store
            .update(
                "model",
                ModelConfigUpdate {
                    api_format: Some(ModelApiFormat::Responses),
                    reasoning_effort: Some(ReasoningEffort::XHigh),
                    tool_call: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();

        let summary = ModelSummary::from(&stored);

        assert_eq!(summary.provider.category, "default");
        assert_eq!(summary.provider.name, "provider");
        assert_eq!(summary.api_format, "responses");
        assert_eq!(summary.params.reasoning_effort.as_deref(), Some("xhigh"));
        assert!(summary.capabilities.tool_call);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn scan_summary_keeps_valid_models_and_safe_diagnostics() {
        let home = test_home("scan");
        ProviderStore::new(&home)
            .create(
                &ProviderConfig::openai_compatible("provider", "http://localhost/v1", "key")
                    .unwrap(),
            )
            .unwrap();
        let store = ModelStore::new(&home);
        store
            .create(
                &ModelConfig::new(
                    "valid",
                    ModelReference::parse("provider/provider-model").unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
        let broken = store.model_dir("broken").unwrap();
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join("model.yaml"), "provider: [invalid\n").unwrap();

        let result = store.scan_summary_in(&CategoryName::default()).unwrap();

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.diagnostics.len(), 1);
        let _ = fs::remove_dir_all(home);
    }
}
