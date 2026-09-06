use std::fmt;
use std::path::PathBuf;

use crate::McpError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpDiagnosticCategory {
    InvalidIdentity,
    MissingConfigFile,
    InvalidConfig,
    UnsupportedTransport,
    UnexpectedLayout,
    UnsupportedFileType,
    Io,
}

impl fmt::Display for McpDiagnosticCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentity => f.write_str("invalid_identity"),
            Self::MissingConfigFile => f.write_str("missing_config_file"),
            Self::InvalidConfig => f.write_str("invalid_config"),
            Self::UnsupportedTransport => f.write_str("unsupported_transport"),
            Self::UnexpectedLayout => f.write_str("unexpected_layout"),
            Self::UnsupportedFileType => f.write_str("unsupported_file_type"),
            Self::Io => f.write_str("io"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpDiagnostic {
    pub path: PathBuf,
    pub name_hint: Option<String>,
    pub category: McpDiagnosticCategory,
    pub message: String,
    pub remediation: String,
}

impl McpDiagnostic {
    pub fn new(
        path: impl Into<PathBuf>,
        name_hint: Option<String>,
        category: McpDiagnosticCategory,
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
        error: &McpError,
    ) -> Self {
        let category = match error {
            McpError::Category { .. } => McpDiagnosticCategory::InvalidIdentity,
            McpError::InvalidName { .. } => McpDiagnosticCategory::InvalidIdentity,
            McpError::NotFound { .. } => McpDiagnosticCategory::MissingConfigFile,
            McpError::UnsupportedTransport { .. } => McpDiagnosticCategory::UnsupportedTransport,
            McpError::UnsupportedFileType { .. } => McpDiagnosticCategory::UnsupportedFileType,
            McpError::Io { .. } => McpDiagnosticCategory::Io,
            _ => McpDiagnosticCategory::InvalidConfig,
        };
        let remediation = match category {
            McpDiagnosticCategory::InvalidIdentity => {
                "将目录名改为仅包含 ASCII 字母、数字、'-'、'_' 的安全名称"
            }
            McpDiagnosticCategory::MissingConfigFile => "补充该目录下的 mcp.json 或删除空目录",
            McpDiagnosticCategory::UnsupportedTransport => {
                "改用 stdio transport，或删除当前版本无法运行的配置"
            }
            McpDiagnosticCategory::UnexpectedLayout => {
                "只使用 mcps/<category>/<name>/mcp.json 布局"
            }
            McpDiagnosticCategory::UnsupportedFileType => {
                "移除 symlink 或特殊文件，只保留普通目录和普通 mcp.json 文件"
            }
            McpDiagnosticCategory::Io => "检查文件权限和路径是否可读",
            McpDiagnosticCategory::InvalidConfig => {
                "修正 mcp.json 后重新运行 mcp list；确保 name、transport、command、args、env 合法"
            }
        };
        Self::new(path, name_hint, category, error.to_string(), remediation)
    }
}
