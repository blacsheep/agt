use std::fmt;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, SkillError>;

#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error(transparent)]
    Category(#[from] agt_config::CategoryError),

    #[error("skill 名称无效：{name}。名称必须是 1-64 字符的 lowercase kebab-case")]
    InvalidName {
        name: String,
        #[source]
        source: Box<agt_core::PackageRefParseError>,
    },

    #[error("skill package ref 无效：{reference}")]
    InvalidPackageRef {
        reference: String,
        #[source]
        source: Box<agt_core::PackageRefParseError>,
    },

    #[error("skill 已存在：{name}")]
    AlreadyExists { name: String },

    #[error("skill 不存在：{name}")]
    NotFound { name: String },

    #[error("skill 配置无效：{message}")]
    InvalidConfig { message: String },

    #[error("skill 导入源无效：{path}")]
    InvalidImportSource { path: PathBuf },

    #[error("skill 压缩包无效：{path}：{message}")]
    InvalidArchive { path: PathBuf, message: String },

    #[error("skill 资源超过限制：{path}：{message}")]
    LimitExceeded { path: PathBuf, message: String },

    #[error("skill 包含不支持的文件类型：{path}")]
    UnsupportedFileType { path: PathBuf },

    #[error("skill 导入事务失败：{primary}{cleanup}")]
    ImportTransaction {
        #[source]
        primary: Box<SkillError>,
        cleanup: ImportCleanupReport,
    },

    #[error("skill 文件系统操作失败：{path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("skill frontmatter 解析失败：{path}")]
    Yaml {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
}

#[derive(Debug)]
pub struct ImportCleanupFailure {
    pub path: PathBuf,
    pub source: std::io::Error,
}

#[derive(Debug, Default)]
pub struct ImportCleanupReport {
    pub failures: Vec<ImportCleanupFailure>,
}

impl ImportCleanupReport {
    pub fn is_empty(&self) -> bool {
        self.failures.is_empty()
    }
}

impl fmt::Display for ImportCleanupReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.failures.is_empty() {
            return Ok(());
        }
        formatter.write_str("；回滚失败：")?;
        for (index, failure) in self.failures.iter().enumerate() {
            if index > 0 {
                formatter.write_str("；")?;
            }
            write!(formatter, "{}: {}", failure.path.display(), failure.source)?;
        }
        Ok(())
    }
}

impl SkillError {
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
