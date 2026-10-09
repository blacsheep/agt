use agt_core::CategoryName;
use serde::Serialize;

use crate::{ProviderDiagnostic, ProviderModel, ProviderStore, Result, StoredProvider};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProviderSummary {
    pub category: String,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub api_key_configured: bool,
    pub path: String,
}

impl From<&StoredProvider> for ProviderSummary {
    fn from(provider: &StoredProvider) -> Self {
        Self {
            category: provider.category.to_string(),
            name: provider.config.name.to_string(),
            provider_type: provider.config.provider_type.as_str().to_string(),
            base_url: provider.config.base_url.clone(),
            api_key_configured: !provider.config.api_key.trim().is_empty(),
            path: provider.path.display().to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProviderDiagnosticSummary {
    pub code: String,
    pub message: String,
    pub target: String,
    pub remediation: String,
    pub name_hint: Option<String>,
}

impl From<&ProviderDiagnostic> for ProviderDiagnosticSummary {
    fn from(diagnostic: &ProviderDiagnostic) -> Self {
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
pub struct ProviderScanResult {
    pub category: Option<String>,
    pub items: Vec<ProviderSummary>,
    pub diagnostics: Vec<ProviderDiagnosticSummary>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderReadiness {
    Ready,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProviderValidationResult {
    pub provider: ProviderSummary,
    pub status: ProviderReadiness,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProviderModelSummary {
    pub id: String,
    pub reference: String,
    pub owned_by: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProviderModelsResult {
    pub category: String,
    pub provider: String,
    pub items: Vec<ProviderModelSummary>,
}

impl ProviderModelsResult {
    pub fn new(category: &CategoryName, provider: &str, models: Vec<ProviderModel>) -> Self {
        Self {
            category: category.to_string(),
            provider: provider.to_string(),
            items: models
                .into_iter()
                .map(|model| ProviderModelSummary {
                    reference: format!("{category}/{provider}/{}", model.id),
                    id: model.id,
                    owned_by: model.owned_by,
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProviderDeleteResult {
    pub category: String,
    pub name: String,
    pub path: String,
}

impl ProviderStore {
    pub fn scan_summary(&self) -> Result<ProviderScanResult> {
        let scan = self.scan()?;
        Ok(ProviderScanResult {
            category: None,
            items: scan.items.iter().map(ProviderSummary::from).collect(),
            diagnostics: scan
                .diagnostics
                .iter()
                .map(ProviderDiagnosticSummary::from)
                .collect(),
        })
    }

    pub fn scan_summary_in(&self, category: &CategoryName) -> Result<ProviderScanResult> {
        let category_dir = self.category_dir(category);
        let mut scan = self.scan()?;
        scan.items.retain(|provider| provider.category == *category);
        scan.diagnostics
            .retain(|diagnostic| diagnostic.path.starts_with(&category_dir));
        Ok(ProviderScanResult {
            category: Some(category.to_string()),
            items: scan.items.iter().map(ProviderSummary::from).collect(),
            diagnostics: scan
                .diagnostics
                .iter()
                .map(ProviderDiagnosticSummary::from)
                .collect(),
        })
    }

    pub fn validate_summary_in(
        &self,
        category: &CategoryName,
        name: &str,
    ) -> Result<ProviderValidationResult> {
        let provider = self.read_in(category, name)?;
        provider.config.validate()?;
        Ok(ProviderValidationResult {
            provider: ProviderSummary::from(&provider),
            status: ProviderReadiness::Ready,
        })
    }

    pub fn list_models_summary_in(
        &self,
        category: &CategoryName,
        name: &str,
    ) -> Result<ProviderModelsResult> {
        Ok(ProviderModelsResult::new(
            category,
            name,
            self.list_models_in(category, name)?,
        ))
    }

    pub fn delete_summary_in(
        &self,
        category: &CategoryName,
        name: &str,
    ) -> Result<ProviderDeleteResult> {
        let path = self.delete_in(category, name)?;
        Ok(ProviderDeleteResult {
            category: category.to_string(),
            name: name.to_string(),
            path: path.display().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::*;
    use crate::ProviderConfig;

    fn test_home(name: &str) -> std::path::PathBuf {
        let home =
            std::env::temp_dir().join(format!("agt-provider-safe-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        home
    }

    #[test]
    fn summaries_expose_configuration_state_without_api_key() {
        let home = test_home("summary");
        let store = ProviderStore::new(&home);
        let config = ProviderConfig::openai_compatible(
            "provider",
            "https://api.example.com/v1",
            "sk-private-value",
        )
        .unwrap();
        let stored = store.create(&config).unwrap();

        let value = serde_json::to_value(ProviderSummary::from(&stored)).unwrap();

        assert_eq!(value["api_key_configured"], true);
        assert_eq!(value["name"], "provider");
        assert!(!value.to_string().contains("private"));
        assert_eq!(value.get("api_key"), None);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn scan_summary_preserves_partial_diagnostics_without_config_body() {
        let home = test_home("scan");
        let store = ProviderStore::new(&home);
        store
            .create(
                &ProviderConfig::openai_compatible("valid", "http://localhost/v1", "key").unwrap(),
            )
            .unwrap();
        let broken = store.provider_dir("broken").unwrap();
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join("provider.yaml"), "api_key: secret-in-broken\n").unwrap();

        let result = store.scan_summary_in(&CategoryName::default()).unwrap();
        let value = serde_json::to_value(&result).unwrap();

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.diagnostics.len(), 1);
        assert!(!value.to_string().contains("secret-in-broken"));
        assert_eq!(value, json!(result));
        let _ = fs::remove_dir_all(home);
    }
}
