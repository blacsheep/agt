use std::fmt;
use std::path::PathBuf;

use crate::AgentError;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AgentDiagnosticCategory {
    InvalidIdentity,
    MissingConfigFile,
    InvalidConfig,
    UnexpectedLayout,
    UnsupportedFileType,
    Io,
}

impl fmt::Display for AgentDiagnosticCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentity => "invalid_identity",
            Self::MissingConfigFile => "missing_config_file",
            Self::InvalidConfig => "invalid_config",
            Self::UnexpectedLayout => "unexpected_layout",
            Self::UnsupportedFileType => "unsupported_file_type",
            Self::Io => "io",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentDiagnostic {
    pub path: PathBuf,
    pub name_hint: Option<String>,
    pub category: AgentDiagnosticCategory,
    pub message: String,
    pub remediation: String,
}

impl AgentDiagnostic {
    pub fn new(
        path: impl Into<PathBuf>,
        name_hint: Option<String>,
        category: AgentDiagnosticCategory,
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
        error: &AgentError,
    ) -> Self {
        let category = match error {
            AgentError::Category { .. } => AgentDiagnosticCategory::InvalidIdentity,
            AgentError::InvalidName { .. } => AgentDiagnosticCategory::InvalidIdentity,
            AgentError::NotFound { .. } => AgentDiagnosticCategory::MissingConfigFile,
            AgentError::Io { .. } => AgentDiagnosticCategory::Io,
            _ => AgentDiagnosticCategory::InvalidConfig,
        };
        let remediation = match category {
            AgentDiagnosticCategory::InvalidIdentity => "将目录名改为安全 agent name",
            AgentDiagnosticCategory::MissingConfigFile => "补充该目录下的 agent.yaml 或删除空目录",
            AgentDiagnosticCategory::UnexpectedLayout => {
                "只使用 agents/<category>/<name>/agent.yaml 布局"
            }
            AgentDiagnosticCategory::UnsupportedFileType => {
                "移除 symlink 或特殊文件，只保留普通目录和普通 agent.yaml"
            }
            AgentDiagnosticCategory::Io => "检查文件权限和路径是否可读",
            AgentDiagnosticCategory::InvalidConfig => "修正 agent.yaml schema 与 identity",
        };
        Self::new(path, name_hint, category, error.to_string(), remediation)
    }
}
