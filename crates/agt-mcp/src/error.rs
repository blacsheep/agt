use std::fmt;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, McpError>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct McpStderrSummary {
    pub bytes_seen: usize,
    pub had_output: bool,
    pub truncated: bool,
}

impl fmt::Display for McpStderrSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "stderr(had_output={}, bytes_seen={}, truncated={})",
            self.had_output, self.bytes_seen, self.truncated
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error(transparent)]
    Category {
        #[from]
        source: agt_config::CategoryError,
    },

    #[error("mcp 名称无效：{name}。名称只能包含 ASCII 字母、数字、'-'、'_'")]
    InvalidName { name: String },

    #[error("mcp 已存在：{name}")]
    AlreadyExists { name: String },

    #[error("mcp 不存在：{name}")]
    NotFound { name: String },

    #[error("mcp 配置无效：{message}")]
    InvalidConfig { message: String },

    #[error("mcp store 包含 {count} 个损坏项；请运行 `agt mcp list` 查看诊断")]
    StoreDiagnostics { count: usize },

    #[error("mcp batch 提交失败：{message}；rollback_failures={rollback_failures:?}")]
    BatchTransaction {
        message: String,
        rollback_failures: Vec<String>,
    },

    #[error("mcp transport 冲突：{name} 的 type={type_value}，transport={transport_value}")]
    ConflictingTransport {
        name: String,
        type_value: String,
        transport_value: String,
    },

    #[error("mcp transport 不受支持：{name} 使用 {transport}；当前支持 stdio 与 Streamable HTTP")]
    UnsupportedTransport { name: String, transport: String },

    #[error("mcp 导入源无效：{path}")]
    InvalidImportSource { path: PathBuf },

    #[error("mcp 路径类型不受支持：{path}；只允许普通目录和普通文件")]
    UnsupportedFileType { path: PathBuf },

    #[error("mcp 文件系统操作失败：{path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("mcp 启动失败：{name}（command: {command}）")]
    Startup {
        name: String,
        command: String,
        #[source]
        source: std::io::Error,
    },

    #[error("mcp HTTP credential 缺失：{name} 需要环境变量 {variable}")]
    MissingCredential { name: String, variable: String },

    #[error(
        "mcp HTTP 鉴权失败：{name} 返回 {status}；请配置 bearer/header credential，OAuth 登录暂不支持"
    )]
    AuthenticationRequired { name: String, status: u16 },

    #[error("mcp HTTP 传输失败：{name}：{operation}：{message}")]
    HttpTransport {
        name: String,
        operation: String,
        message: String,
    },

    #[error("mcp JSON 解析失败：{path}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("mcp TOML 解析失败：{path}：{message}")]
    Toml { path: PathBuf, message: String },

    #[error("mcp 协议错误：{name}：{message}；{stderr}")]
    Protocol {
        name: String,
        message: String,
        stderr: McpStderrSummary,
    },

    #[error(
        "mcp 协议版本不受支持：{name} 返回 {version:?}，仅支持 {supported}；{stderr}",
        supported = crate::protocol::MCP_PROTOCOL_VERSION
    )]
    UnsupportedProtocolVersion {
        name: String,
        version: String,
        stderr: McpStderrSummary,
    },

    #[error("mcp capability 缺失：{name} 未声明 {capability}；{stderr}")]
    MissingCapability {
        name: String,
        capability: &'static str,
        stderr: McpStderrSummary,
    },

    #[error("mcp 资源限制触发：{name} 的 {limit} 为 {actual}，上限 {max}；{stderr}")]
    LimitExceeded {
        name: String,
        limit: &'static str,
        max: usize,
        actual: usize,
        stderr: McpStderrSummary,
    },

    #[error("mcp 请求超时：{name}：{operation}；{stderr}")]
    Timeout {
        name: String,
        operation: String,
        stderr: McpStderrSummary,
    },

    #[error("mcp 请求已取消：{operation}")]
    Canceled { operation: String },

    #[error("mcp 进程清理失败：{name}")]
    ProcessCleanup {
        name: String,
        #[source]
        source: std::io::Error,
    },
}

impl McpError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn json(path: impl Into<PathBuf>, source: serde_json::Error) -> Self {
        Self::Json {
            path: path.into(),
            source,
        }
    }
}
