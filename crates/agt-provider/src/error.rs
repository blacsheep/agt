use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, ProviderError>;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error(transparent)]
    Category {
        #[from]
        source: agt_config::CategoryError,
    },

    #[error("provider 名称无效：{name}。名称只能包含 ASCII 字母、数字、'-'、'_'")]
    InvalidName { name: String },

    #[error("provider 已存在：{name}")]
    AlreadyExists { name: String },

    #[error("provider 不存在：{name}")]
    NotFound { name: String },

    #[error("provider 配置不完整：{message}")]
    InvalidConfig { message: String },

    #[error("provider 远端请求失败：{message}")]
    Remote { message: String },

    #[error("provider HTTP 请求失败")]
    Http {
        #[source]
        source: reqwest::Error,
    },

    #[error("provider 文件系统操作失败：{path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("provider YAML 解析失败：{path}")]
    Yaml {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
}

impl ProviderError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn yaml(path: impl Into<PathBuf>, source: serde_yaml::Error) -> Self {
        Self::Yaml {
            path: path.into(),
            source,
        }
    }

    pub fn http(source: reqwest::Error) -> Self {
        Self::Http { source }
    }
}
