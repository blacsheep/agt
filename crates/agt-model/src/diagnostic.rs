use std::fmt;
use std::path::PathBuf;

use crate::ModelError;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ModelDiagnosticCategory {
    InvalidIdentity,
    MissingConfigFile,
    InvalidConfig,
    UnexpectedLayout,
    UnsupportedFileType,
    Io,
}

impl fmt::Display for ModelDiagnosticCategory {
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
pub struct ModelDiagnostic {
    pub path: PathBuf,
    pub name_hint: Option<String>,
    pub category: ModelDiagnosticCategory,
    pub message: String,
    pub remediation: String,
}

impl ModelDiagnostic {
    pub fn new(
        path: impl Into<PathBuf>,
        name_hint: Option<String>,
        category: ModelDiagnosticCategory,
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
        error: &ModelError,
    ) -> Self {
        let category = match error {
            ModelError::Category { .. } => ModelDiagnosticCategory::InvalidIdentity,
            ModelError::InvalidName { .. } => ModelDiagnosticCategory::InvalidIdentity,
            ModelError::NotFound { .. } => ModelDiagnosticCategory::MissingConfigFile,
            ModelError::Io { .. } => ModelDiagnosticCategory::Io,
            _ => ModelDiagnosticCategory::InvalidConfig,
        };
        let remediation = match category {
            ModelDiagnosticCategory::InvalidIdentity => "将目录名改为安全 model alias",
            ModelDiagnosticCategory::MissingConfigFile => "补充该目录下的 model.yaml 或删除空目录",
            ModelDiagnosticCategory::UnexpectedLayout => {
                "只使用 models/<category>/<name>/model.yaml 布局"
            }
            ModelDiagnosticCategory::UnsupportedFileType => {
                "移除 symlink 或特殊文件，只保留普通目录和普通 model.yaml"
            }
            ModelDiagnosticCategory::Io => "检查文件权限和路径是否可读",
            ModelDiagnosticCategory::InvalidConfig => {
                "修正 model.yaml schema、identity 与 runtime 参数"
            }
        };
        Self::new(path, name_hint, category, error.to_string(), remediation)
    }
}
