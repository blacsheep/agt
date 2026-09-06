use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, AgtError>;

#[derive(Debug, thiserror::Error)]
pub enum AgtError {
    #[error("无法确定用户主目录，不能解析默认 .agt 路径")]
    HomeDirUnavailable,

    #[error("无效输入：{message}")]
    InvalidInput { message: String },

    #[error("文件系统操作失败：{path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl AgtError {
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::InvalidInput {
            message: message.into(),
        }
    }

    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
