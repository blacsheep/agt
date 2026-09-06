use std::fmt;
use std::path::PathBuf;

use crate::PromptError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptDiagnosticCategory {
    InvalidIdentity,
    MissingPromptFile,
    InvalidContent,
    UnexpectedLayout,
    UnsupportedFileType,
    Io,
}

impl fmt::Display for PromptDiagnosticCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentity => "invalid_identity",
            Self::MissingPromptFile => "missing_prompt_file",
            Self::InvalidContent => "invalid_content",
            Self::UnexpectedLayout => "unexpected_layout",
            Self::UnsupportedFileType => "unsupported_file_type",
            Self::Io => "io",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptDiagnostic {
    pub path: PathBuf,
    pub name_hint: Option<String>,
    pub category: PromptDiagnosticCategory,
    pub message: String,
    pub remediation: String,
}

impl PromptDiagnostic {
    pub fn new(
        path: impl Into<PathBuf>,
        name_hint: Option<String>,
        category: PromptDiagnosticCategory,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            name_hint,
            category,
            message: message.into(),
            remediation: remediation.into(),
        }
    }

    pub(crate) fn from_error(
        path: impl Into<PathBuf>,
        name_hint: Option<String>,
        error: &PromptError,
    ) -> Self {
        let category = match error {
            PromptError::Category { .. } | PromptError::InvalidName { .. } => {
                PromptDiagnosticCategory::InvalidIdentity
            }
            PromptError::NotFound { .. } => PromptDiagnosticCategory::MissingPromptFile,
            PromptError::UnsupportedFileType { .. } => {
                PromptDiagnosticCategory::UnsupportedFileType
            }
            PromptError::UnexpectedEntry { .. } => PromptDiagnosticCategory::UnexpectedLayout,
            PromptError::Io { .. } => PromptDiagnosticCategory::Io,
            PromptError::TooLarge { .. }
            | PromptError::InvalidUtf8 { .. }
            | PromptError::InvalidModifiedTime { .. } => PromptDiagnosticCategory::InvalidContent,
            PromptError::AlreadyExists { .. } | PromptError::ReservedDefault => {
                PromptDiagnosticCategory::InvalidContent
            }
        };
        let remediation = match category {
            PromptDiagnosticCategory::InvalidIdentity => "将目录名改为安全 Prompt name",
            PromptDiagnosticCategory::MissingPromptFile => "补充 PROMPT.md 或删除空目录",
            PromptDiagnosticCategory::InvalidContent => {
                "修正 PROMPT.md，使其为大小受限的 UTF-8 Markdown"
            }
            PromptDiagnosticCategory::UnexpectedLayout => {
                "只使用 prompts/<category>/<name>/PROMPT.md 布局"
            }
            PromptDiagnosticCategory::UnsupportedFileType => {
                "移除 symlink/特殊文件，只保留普通目录与普通 PROMPT.md"
            }
            PromptDiagnosticCategory::Io => "检查文件权限和路径是否可读",
        };
        Self::new(path, name_hint, category, error.to_string(), remediation)
    }
}
