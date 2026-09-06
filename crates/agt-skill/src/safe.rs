use std::fs;

use agt_core::CategoryName;
use serde::Serialize;

use crate::{
    SKILL_DOCUMENT_MAX_BYTES, SkillDiagnostic, SkillError, SkillStore, StoredSkill,
    StoredSkillMetadata,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkillSummary {
    pub category: String,
    pub skill: String,
    pub name: String,
    pub description: String,
    pub path: String,
    pub dir: String,
}

impl From<&StoredSkill> for SkillSummary {
    fn from(skill: &StoredSkill) -> Self {
        Self {
            category: stored_category(skill.category.as_ref()),
            skill: skill.package.to_string(),
            name: skill.document.metadata.name.to_string(),
            description: skill.document.metadata.description.clone(),
            path: skill.path.display().to_string(),
            dir: skill.dir.display().to_string(),
        }
    }
}

impl From<&StoredSkillMetadata> for SkillSummary {
    fn from(skill: &StoredSkillMetadata) -> Self {
        Self {
            category: stored_category(skill.category.as_ref()),
            skill: skill.package.to_string(),
            name: skill.metadata.name.to_string(),
            description: skill.metadata.description.clone(),
            path: skill.path.display().to_string(),
            dir: skill.dir.display().to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkillDiagnosticSummary {
    pub code: String,
    pub message: String,
    pub target: String,
    pub remediation: String,
    pub package_hint: Option<String>,
}

impl From<&SkillDiagnostic> for SkillDiagnosticSummary {
    fn from(diagnostic: &SkillDiagnostic) -> Self {
        Self {
            code: diagnostic.category.to_string(),
            message: diagnostic.message.clone(),
            target: diagnostic.path.display().to_string(),
            remediation: diagnostic.remediation.clone(),
            package_hint: diagnostic.package_hint.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkillScanResult {
    pub category: Option<String>,
    pub items: Vec<SkillSummary>,
    pub diagnostics: Vec<SkillDiagnosticSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkillDocumentResult {
    pub skill: SkillSummary,
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkillBatchResult {
    pub items: Vec<SkillSummary>,
}

impl SkillBatchResult {
    pub fn from_stored(skills: &[StoredSkill]) -> Self {
        Self {
            items: skills.iter().map(SkillSummary::from).collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkillImportResult {
    pub source: String,
    pub items: Vec<SkillSummary>,
}

impl SkillImportResult {
    pub fn new(source: impl Into<String>, skills: &[StoredSkill]) -> Self {
        Self {
            source: source.into(),
            items: skills.iter().map(SkillSummary::from).collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SkillDeleteResult {
    pub category: String,
    pub skill: String,
    pub path: String,
}

impl SkillStore {
    pub fn scan_summary(&self) -> crate::Result<SkillScanResult> {
        let scan = self.scan()?;
        Ok(SkillScanResult {
            category: None,
            items: scan.items.iter().map(SkillSummary::from).collect(),
            diagnostics: scan
                .diagnostics
                .iter()
                .map(SkillDiagnosticSummary::from)
                .collect(),
        })
    }

    pub fn scan_summary_in(&self, category: &CategoryName) -> crate::Result<SkillScanResult> {
        let category_dir = self.category_dir(category);
        let mut scan = self.scan()?;
        scan.items
            .retain(|skill| skill.category.as_ref() == Some(category));
        scan.diagnostics
            .retain(|diagnostic| diagnostic.path.starts_with(&category_dir));
        Ok(SkillScanResult {
            category: Some(category.to_string()),
            items: scan.items.iter().map(SkillSummary::from).collect(),
            diagnostics: scan
                .diagnostics
                .iter()
                .map(SkillDiagnosticSummary::from)
                .collect(),
        })
    }

    pub fn read_summary_in(
        &self,
        category: &CategoryName,
        package: &agt_core::PackageRef,
    ) -> crate::Result<SkillSummary> {
        Ok(SkillSummary::from(
            &self.read_metadata_in(category, package)?,
        ))
    }

    pub fn read_document_summary_in(
        &self,
        category: &CategoryName,
        package: &agt_core::PackageRef,
    ) -> crate::Result<SkillDocumentResult> {
        let skill = self.read_in(category, package)?;
        let bytes = fs::metadata(&skill.path)
            .map_err(|source| SkillError::io(&skill.path, source))?
            .len();
        if bytes > SKILL_DOCUMENT_MAX_BYTES {
            return Err(SkillError::LimitExceeded {
                path: skill.path.clone(),
                message: format!(
                    "Skill document {bytes} bytes，超过 {SKILL_DOCUMENT_MAX_BYTES} bytes"
                ),
            });
        }
        let content = fs::read_to_string(&skill.path)
            .map_err(|source| SkillError::io(&skill.path, source))?;
        Ok(SkillDocumentResult {
            skill: SkillSummary::from(&skill),
            content,
        })
    }

    pub fn delete_summary_in(
        &self,
        category: &CategoryName,
        package: &agt_core::PackageRef,
    ) -> crate::Result<SkillDeleteResult> {
        let path = self.delete_in(category, package)?;
        Ok(SkillDeleteResult {
            category: category.to_string(),
            skill: package.to_string(),
            path: path.display().to_string(),
        })
    }
}

fn stored_category(category: Option<&CategoryName>) -> String {
    category
        .map(ToString::to_string)
        .unwrap_or_else(|| CategoryName::default().to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::*;
    use crate::{SkillCreateOptions, SkillStore};

    fn test_home(name: &str) -> std::path::PathBuf {
        let home =
            std::env::temp_dir().join(format!("agt-skill-safe-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        home
    }

    #[test]
    fn metadata_summary_excludes_body_and_document_read_is_explicit() {
        let home = test_home("separation");
        let store = SkillStore::new(&home);
        let stored = store
            .create(SkillCreateOptions::new("review", "Review changes.").unwrap())
            .unwrap();
        fs::write(
            &stored.path,
            "---\nname: review\ndescription: Review changes.\n---\nprivate skill body\n",
        )
        .unwrap();

        let summary = store
            .read_summary_in(&CategoryName::default(), &"review".parse().unwrap())
            .unwrap();
        let summary_value = serde_json::to_value(&summary).unwrap();
        let document = store
            .read_document_summary_in(&CategoryName::default(), &"review".parse().unwrap())
            .unwrap();

        assert_eq!(summary.skill, "review");
        assert_eq!(summary_value.get("content"), None);
        assert!(!summary_value.to_string().contains("private skill body"));
        assert_eq!(document.skill, summary);
        assert!(document.content.ends_with("private skill body\n"));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn document_read_enforces_runtime_document_limit() {
        let home = test_home("document-limit");
        let store = SkillStore::new(&home);
        let stored = store
            .create(SkillCreateOptions::new("large", "Large skill.").unwrap())
            .unwrap();
        let body = "x".repeat(SKILL_DOCUMENT_MAX_BYTES as usize);
        fs::write(
            &stored.path,
            format!("---\nname: large\ndescription: Large skill.\n---\n{body}"),
        )
        .unwrap();

        let error = store
            .read_document_summary_in(&CategoryName::default(), &"large".parse().unwrap())
            .unwrap_err();

        assert!(matches!(error, SkillError::LimitExceeded { .. }));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn scan_summary_preserves_metadata_and_safe_diagnostics() {
        let home = test_home("scan");
        let store = SkillStore::new(&home);
        store
            .create(SkillCreateOptions::new("valid", "Valid skill.").unwrap())
            .unwrap();
        let broken = store.skill_dir(&"broken".parse().unwrap());
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join("SKILL.md"), "private broken body").unwrap();

        let result = store.scan_summary_in(&CategoryName::default()).unwrap();
        let value = serde_json::to_value(&result).unwrap();

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(value, json!(result));
        assert!(!value.to_string().contains("private broken body"));
        let _ = fs::remove_dir_all(home);
    }
}
