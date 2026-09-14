use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, AgentError>;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error(transparent)]
    Category {
        #[from]
        source: agt_config::CategoryError,
    },

    #[error(
        "agent 名称无效：{name}。首字符必须是 ASCII 字母或数字，后续可包含 ASCII 字母、数字、'-'、'_'；'.' 只能分隔两个字母数字段"
    )]
    InvalidName { name: String },

    #[error("agent 已存在：{name}")]
    AlreadyExists { name: String },

    #[error("agent 不存在：{name}")]
    NotFound { name: String },

    #[error("agent {agent} 未挂载 {kind}：{name}")]
    AttachmentNotFound {
        agent: String,
        kind: &'static str,
        name: String,
    },

    #[error("agent 指向的 model 不存在或不可读：{name}")]
    ModelUnavailable {
        name: String,
        #[source]
        source: agt_model::ModelError,
    },

    #[error("agent 指向的 Prompt 不存在或不可读：{name}")]
    PromptUnavailable {
        name: String,
        #[source]
        source: agt_prompt::PromptError,
    },

    #[error("agent 指向的 skill 不存在或不可读：{name}")]
    SkillUnavailable {
        name: String,
        #[source]
        source: agt_skill::SkillError,
    },

    #[error("agent 指向的 MCP 不存在或不可读：{name}")]
    McpUnavailable {
        name: String,
        #[source]
        source: Box<agt_mcp::McpError>,
    },

    #[error("agent 配置无效：{message}")]
    InvalidConfig { message: String },

    #[error("agent 文件系统操作失败：{path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("agent YAML 解析失败：{path}")]
    Yaml {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
}

impl AgentError {
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
