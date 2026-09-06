use std::fmt;
use std::path::PathBuf;

use crate::SkillError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillDiagnosticCategory {
    InvalidIdentity,
    InvalidMetadata,
    MissingSkillFile,
    UnexpectedLayout,
    UnsupportedFileType,
    Io,
}

impl fmt::Display for SkillDiagnosticCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentity => f.write_str("invalid_identity"),
            Self::InvalidMetadata => f.write_str("invalid_metadata"),
            Self::MissingSkillFile => f.write_str("missing_skill_file"),
            Self::UnexpectedLayout => f.write_str("unexpected_layout"),
            Self::UnsupportedFileType => f.write_str("unsupported_file_type"),
            Self::Io => f.write_str("io"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillDiagnostic {
    pub path: PathBuf,
    pub package_hint: Option<String>,
    pub category: SkillDiagnosticCategory,
    pub message: String,
    pub remediation: String,
}

impl SkillDiagnostic {
    pub fn new(
        path: impl Into<PathBuf>,
        package_hint: Option<String>,
        category: SkillDiagnosticCategory,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            package_hint,
            category,
            message: message.into(),
            remediation: remediation.into(),
        }
    }

    pub(crate) fn from_error(
        path: impl Into<PathBuf>,
        package_hint: Option<String>,
        error: &SkillError,
    ) -> Self {
        let category = match error {
            SkillError::Category { .. } => SkillDiagnosticCategory::InvalidIdentity,
            SkillError::InvalidName { .. } | SkillError::InvalidPackageRef { .. } => {
                SkillDiagnosticCategory::InvalidIdentity
            }
            SkillError::NotFound { .. } => SkillDiagnosticCategory::MissingSkillFile,
            SkillError::UnsupportedFileType { .. } => SkillDiagnosticCategory::UnsupportedFileType,
            SkillError::Io { .. } => SkillDiagnosticCategory::Io,
            _ => SkillDiagnosticCategory::InvalidMetadata,
        };
        Self::new(
            path,
            package_hint,
            category,
            error.to_string(),
            match category {
                SkillDiagnosticCategory::InvalidIdentity => {
                    "使用 lowercase kebab-case，并让目录、frontmatter name 与 package leaf 一致"
                }
                SkillDiagnosticCategory::MissingSkillFile => "补充该目录下的 SKILL.md 或删除空目录",
                SkillDiagnosticCategory::UnexpectedLayout => {
                    "只使用 <category>/<name>/SKILL.md 或 <category>/@<scope>/<name>/SKILL.md"
                }
                SkillDiagnosticCategory::UnsupportedFileType => {
                    "移除 symlink 或特殊文件，只保留普通文件和目录"
                }
                SkillDiagnosticCategory::Io => "检查文件权限和路径是否可读",
                SkillDiagnosticCategory::InvalidMetadata => {
                    "修正 SKILL.md frontmatter 后重新运行 skill list"
                }
            },
        )
    }
}
