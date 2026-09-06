use std::time::UNIX_EPOCH;

use agt_core::CategoryName;
use serde::Serialize;

use crate::{PromptDiagnostic, PromptError, PromptStore, Result, StoredPrompt};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PromptSummary {
    pub category: String,
    pub name: String,
    pub path: String,
    pub bytes: u64,
    pub modified_unix: u64,
    pub sha256: String,
}

impl TryFrom<&StoredPrompt> for PromptSummary {
    type Error = PromptError;

    fn try_from(prompt: &StoredPrompt) -> Result<Self> {
        let modified_unix = prompt
            .modified
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PromptError::InvalidModifiedTime {
                path: prompt.path.clone(),
            })?
            .as_secs();
        Ok(Self {
            category: prompt.category.to_string(),
            name: prompt.name.to_string(),
            path: prompt.path.display().to_string(),
            bytes: prompt.bytes,
            modified_unix,
            sha256: prompt.content_hash.clone(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PromptDiagnosticSummary {
    pub code: String,
    pub message: String,
    pub target: String,
    pub remediation: String,
    pub name_hint: Option<String>,
}

impl From<&PromptDiagnostic> for PromptDiagnosticSummary {
    fn from(diagnostic: &PromptDiagnostic) -> Self {
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
pub struct PromptScanResult {
    pub category: Option<String>,
    pub items: Vec<PromptSummary>,
    pub diagnostics: Vec<PromptDiagnosticSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PromptContentResult {
    pub prompt: PromptSummary,
    pub content: String,
}

impl TryFrom<&StoredPrompt> for PromptContentResult {
    type Error = PromptError;

    fn try_from(prompt: &StoredPrompt) -> Result<Self> {
        Ok(Self {
            prompt: PromptSummary::try_from(prompt)?,
            content: prompt.content.clone(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PromptDeleteResult {
    pub category: String,
    pub name: String,
    pub path: String,
}

impl PromptStore {
    pub fn scan_summary(&self) -> Result<PromptScanResult> {
        let scan = self.scan()?;
        Ok(PromptScanResult {
            category: None,
            items: scan
                .items
                .iter()
                .map(PromptSummary::try_from)
                .collect::<Result<Vec<_>>>()?,
            diagnostics: scan
                .diagnostics
                .iter()
                .map(PromptDiagnosticSummary::from)
                .collect(),
        })
    }

    pub fn scan_summary_in(&self, category: &CategoryName) -> Result<PromptScanResult> {
        let category_dir = self.category_dir(category);
        let mut scan = self.scan()?;
        scan.items.retain(|prompt| prompt.category == *category);
        scan.diagnostics
            .retain(|diagnostic| diagnostic.path.starts_with(&category_dir));
        Ok(PromptScanResult {
            category: Some(category.to_string()),
            items: scan
                .items
                .iter()
                .map(PromptSummary::try_from)
                .collect::<Result<Vec<_>>>()?,
            diagnostics: scan
                .diagnostics
                .iter()
                .map(PromptDiagnosticSummary::from)
                .collect(),
        })
    }

    pub fn read_content_summary_in(
        &self,
        category: &CategoryName,
        name: &str,
    ) -> Result<PromptContentResult> {
        let prompt = self.read_in(category, name)?;
        PromptContentResult::try_from(&prompt)
    }

    pub fn delete_summary_in(
        &self,
        category: &CategoryName,
        name: &str,
    ) -> Result<PromptDeleteResult> {
        let path = self.delete_in(category, name)?;
        Ok(PromptDeleteResult {
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

    fn test_home(name: &str) -> std::path::PathBuf {
        let home =
            std::env::temp_dir().join(format!("agt-prompt-safe-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        home
    }

    #[test]
    fn metadata_summary_excludes_body_and_content_read_is_explicit() {
        let home = test_home("separation");
        let store = PromptStore::new(&home);
        let stored = store.create("review", "private prompt body").unwrap();

        let summary = PromptSummary::try_from(&stored).unwrap();
        let summary_value = serde_json::to_value(&summary).unwrap();
        let content = store
            .read_content_summary_in(&CategoryName::default(), "review")
            .unwrap();

        assert_eq!(summary.name, "review");
        assert_eq!(summary.bytes, 19);
        assert_eq!(summary_value.get("content"), None);
        assert!(!summary_value.to_string().contains("private prompt body"));
        assert_eq!(content.prompt, summary);
        assert_eq!(content.content, "private prompt body");
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn scan_summary_keeps_valid_metadata_and_safe_diagnostics() {
        let home = test_home("scan");
        let store = PromptStore::new(&home);
        store.create("valid", "valid body").unwrap();
        let broken = store.prompt_dir("broken").unwrap();
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join("unexpected.txt"), "secret diagnostic body").unwrap();

        let result = store.scan_summary_in(&CategoryName::default()).unwrap();
        let value = serde_json::to_value(&result).unwrap();

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(value, json!(result));
        assert!(!value.to_string().contains("valid body"));
        assert!(!value.to_string().contains("secret diagnostic body"));
        let _ = fs::remove_dir_all(home);
    }
}
