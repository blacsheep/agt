use std::fmt;
use std::path::PathBuf;

use crate::ProviderError;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ProviderDiagnosticCategory {
    InvalidIdentity,
    MissingConfigFile,
    InvalidConfig,
    UnexpectedLayout,
    UnsupportedFileType,
    Io,
}

impl fmt::Display for ProviderDiagnosticCategory {
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
pub struct ProviderDiagnostic {
    pub path: PathBuf,
    pub name_hint: Option<String>,
    pub category: ProviderDiagnosticCategory,
    pub message: String,
    pub remediation: String,
}

impl ProviderDiagnostic {
    pub fn new(
        path: impl Into<PathBuf>,
        name_hint: Option<String>,
        category: ProviderDiagnosticCategory,
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
        error: &ProviderError,
    ) -> Self {
        let category = match error {
            ProviderError::Category { .. } => ProviderDiagnosticCategory::InvalidIdentity,
            ProviderError::InvalidName { .. } => ProviderDiagnosticCategory::InvalidIdentity,
            ProviderError::NotFound { .. } => ProviderDiagnosticCategory::MissingConfigFile,
            ProviderError::Io { .. } => ProviderDiagnosticCategory::Io,
            _ => ProviderDiagnosticCategory::InvalidConfig,
        };
        let remediation = match category {
            ProviderDiagnosticCategory::InvalidIdentity => {
                "将目录名改为仅包含 ASCII 字母、数字、'-'、'_' 的安全名称"
            }
            ProviderDiagnosticCategory::MissingConfigFile => {
                "补充该目录下的 provider.yaml 或删除空目录"
            }
            ProviderDiagnosticCategory::UnexpectedLayout => {
                "只使用 providers/<category>/<name>/provider.yaml 布局"
            }
            ProviderDiagnosticCategory::UnsupportedFileType => {
                "移除 symlink 或特殊文件，只保留普通目录和普通 provider.yaml"
            }
            ProviderDiagnosticCategory::Io => "检查文件权限和路径是否可读",
            ProviderDiagnosticCategory::InvalidConfig => {
                "修正 provider.yaml schema、identity、base_url 与 credential"
            }
        };
        Self::new(path, name_hint, category, error.to_string(), remediation)
    }
}
