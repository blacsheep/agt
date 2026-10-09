use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, PromptError>;

#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error(transparent)]
    Category(#[from] agt_config::CategoryError),

    #[error("Prompt 名称无效：{name}。名称只能包含 ASCII 字母、数字、'-'、'_'")]
    InvalidName { name: String },

    #[error("Prompt 已存在：{name}")]
    AlreadyExists { name: String },

    #[error("Prompt 不存在：{name}")]
    NotFound { name: String },

    #[error("default/default 是保留 Prompt，不能删除")]
    ReservedDefault,

    #[error("Prompt 超过大小限制：{path}；实际 {size} bytes，上限 {max} bytes")]
    TooLarge { path: PathBuf, size: u64, max: u64 },

    #[error("Prompt 不是 UTF-8 文本：{path}")]
    InvalidUtf8 {
        path: PathBuf,
        #[source]
        source: std::string::FromUtf8Error,
    },

    #[error("Prompt 修改时间早于 UNIX epoch：{path}")]
    InvalidModifiedTime { path: PathBuf },

    #[error("Prompt 文件类型不受支持：{path}；只允许普通目录和普通 PROMPT.md")]
    UnsupportedFileType { path: PathBuf },

    #[error("Prompt 目录包含不支持的 entry：{path}")]
    UnexpectedEntry { path: PathBuf },

    #[error("Prompt 文件系统操作失败：{path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl PromptError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
