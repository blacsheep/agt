use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, ModelError>;

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error(transparent)]
    Category {
        #[from]
        source: agt_config::CategoryError,
    },

    #[error(
        "model 名称无效：{name}。名称只能包含 ASCII 字母、数字、'-'、'_'、'.'，且不能是 '.' 或 '..'"
    )]
    InvalidName { name: String },

    #[error("model reference 无效：{reference}。格式必须是 <provider>/<model>")]
    InvalidReference { reference: String },

    #[error("provider-side model 名称无效：{name}")]
    InvalidProviderModelName { name: String },

    #[error("model 已存在：{name}")]
    AlreadyExists { name: String },

    #[error("model 不存在：{name}")]
    NotFound { name: String },

    #[error("model 指向的 provider 不存在或不可读：{name}")]
    ProviderUnavailable {
        name: String,
        #[source]
        source: agt_provider::ProviderError,
    },

    #[error("model 配置无效：{message}")]
    InvalidConfig { message: String },

    #[error("model 文件系统操作失败：{path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("model YAML 解析失败：{path}")]
    Yaml {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
}

impl ModelError {
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
}
