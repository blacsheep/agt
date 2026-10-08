use std::fmt;
use std::path::{Component, Path, PathBuf};

use agt_core::McpId;
use serde_json::{Map, Value};

use crate::client::{DefaultMcpClientFactory, McpClientFactory};
use crate::error::{McpError, Result};
use crate::limits::McpLimits;
use crate::name::validate_mcp_name;

pub const MCP_FILE: &str = "mcp.json";

#[derive(Clone, PartialEq)]
pub struct McpDocument {
    pub name: McpId,
    pub server: Value,
}

impl fmt::Debug for McpDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpDocument")
            .field("name", &self.name)
            .field("summary", &self.summary())
            .finish()
    }
}

impl McpDocument {
    pub fn stdio(
        name: impl Into<String>,
        command: impl Into<String>,
        args: Vec<String>,
        env: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self> {
        let server = serde_json::json!({
            "type": "stdio",
            "command": command.into(),
            "args": args,
            "env": env
                .into_iter()
                .map(|(key, value)| (key, Value::String(value)))
                .collect::<Map<String, Value>>(),
        });
        let document = Self {
            name: McpId::new(name.into()),
            server,
        };
        document.validate()?;
        Ok(document)
    }

    pub fn parse(path: impl AsRef<Path>, text: &str) -> Result<Self> {
        let path = path.as_ref();
        let value: Value =
            serde_json::from_str(text).map_err(|source| McpError::json(path, source))?;
        Self::from_value(value)
    }

    pub fn from_value(value: Value) -> Result<Self> {
        let root = value.as_object().ok_or_else(|| McpError::InvalidConfig {
            message: "顶层 JSON 必须是对象，并包含 mcpServers 字段".to_string(),
        })?;
        let servers = root
            .get("mcpServers")
            .ok_or_else(|| McpError::InvalidConfig {
                message: "缺少 mcpServers 字段；当前只接受完整 MCP JSON".to_string(),
            })?
            .as_object()
            .ok_or_else(|| McpError::InvalidConfig {
                message: "mcpServers 必须是对象".to_string(),
            })?;

        if servers.is_empty() {
            return Err(McpError::InvalidConfig {
                message: "mcpServers 不能为空".to_string(),
            });
        }
        if servers.len() != 1 {
            return Err(McpError::InvalidConfig {
                message: "当前不支持批量导入多个 MCP server；请把每个 key 拆分为独立配置"
                    .to_string(),
            });
        }

        let Some((name, server)) = servers.iter().next() else {
            return Err(McpError::InvalidConfig {
                message: "mcpServers 不能为空".to_string(),
            });
        };
        let document = Self {
            name: McpId::new(name.clone()),
            server: server.clone(),
        };
        document.validate()?;
        Ok(document)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_document_shape()?;
        match self.transport_projection()?.as_str() {
            "stdio" => self.stdio_config_fields().map(|_| ()),
            "http" => self.http_config_fields().map(|_| ()),
            transport => Err(McpError::UnsupportedTransport {
                name: self.name.to_string(),
                transport: transport.to_string(),
            }),
        }
    }

    pub fn validate_document_shape(&self) -> Result<()> {
        validate_mcp_name(self.name.as_str())?;
        if !self.server.is_object() {
            return Err(McpError::InvalidConfig {
                message: format!("mcpServers.{} 必须是对象", self.name),
            });
        }
        Ok(())
    }

    pub fn validate_supported_transport(&self) -> Result<()> {
        let transport = self.transport_projection()?;
        if !matches!(transport.as_str(), "stdio" | "http") {
            return Err(McpError::UnsupportedTransport {
                name: self.name.to_string(),
                transport,
            });
        }
        Ok(())
    }

    pub fn test_config(&self) -> Result<McpTestResult> {
        self.validate()?;
        let summary = self.summary();
        Ok(McpTestResult {
            name: self.name.to_string(),
            transport: summary.transport.unwrap_or_else(|| "invalid".to_string()),
            target: summary.target.unwrap_or_else(|| "-".to_string()),
            tool_count: 0,
            tool_names: Vec::new(),
            smoke: "仅执行配置校验，未连接 MCP server".to_string(),
        })
    }

    pub fn smoke_test(&self, cwd: &Path) -> Result<McpTestResult> {
        self.validate()?;
        let summary = self.summary();
        let factory = DefaultMcpClientFactory;
        let mut client = factory.connect(self, cwd, McpLimits::default())?;
        let result: Result<Vec<String>> = (|| {
            let tools = client.list_tools()?;
            let mut names = Vec::with_capacity(tools.len());
            for tool in tools {
                agt_core::tool_schema::ToolSchema::compile(&tool.input_schema).map_err(
                    |source| McpError::ToolSchema {
                        tool: tool.name.clone(),
                        source,
                    },
                )?;
                names.push(tool.name);
            }
            names.sort();
            Ok(names)
        })();
        let closed = client.close();
        let tool_names: Vec<String> = result?;
        closed?;

        Ok(McpTestResult {
            name: self.name.to_string(),
            transport: summary.transport.unwrap_or_else(|| "invalid".to_string()),
            target: summary.target.unwrap_or_else(|| "-".to_string()),
            tool_count: tool_names.len(),
            tool_names,
            smoke: "initialize、tools/list 与 inputSchema 校验成功".to_string(),
        })
    }

    pub fn stdio_config(&self) -> Result<McpStdioServerConfig> {
        self.validate_document_shape()?;
        let transport = self.transport_projection()?;
        if transport != "stdio" {
            return Err(McpError::UnsupportedTransport {
                name: self.name.to_string(),
                transport,
            });
        }
        self.stdio_config_fields()
    }

    pub fn resolve_stdio_server(&self, cwd: &Path) -> Result<ResolvedMcpStdioServerConfig> {
        let config = self.stdio_config()?;
        let configured_cwd = config.cwd.as_deref().unwrap_or(".");
        let configured_cwd = resolve_workspace_placeholders(configured_cwd, cwd);
        if configured_cwd.trim().is_empty() {
            return Err(McpError::InvalidConfig {
                message: "stdio MCP cwd 不能为空".to_string(),
            });
        }
        let configured_cwd = PathBuf::from(configured_cwd);
        let server_cwd = if configured_cwd.is_absolute() {
            configured_cwd
        } else {
            cwd.join(configured_cwd)
        };
        let server_cwd = lexically_normalize(&server_cwd);
        Ok(ResolvedMcpStdioServerConfig {
            transport: config.transport,
            command: resolve_workspace_placeholders(&config.command, &server_cwd),
            args: config
                .args
                .into_iter()
                .map(|value| resolve_workspace_placeholders(&value, &server_cwd))
                .collect(),
            env: config
                .env
                .into_iter()
                .map(|(key, value)| (key, resolve_workspace_placeholders(&value, &server_cwd)))
                .collect(),
            cwd: server_cwd,
        })
    }

    pub fn http_config(&self) -> Result<McpHttpServerConfig> {
        self.validate_document_shape()?;
        let transport = self.transport_projection()?;
        if transport != "http" {
            return Err(McpError::UnsupportedTransport {
                name: self.name.to_string(),
                transport,
            });
        }
        self.http_config_fields()
    }

    pub fn summary(&self) -> McpServerSummary {
        let transport = self.transport_projection().ok();
        let target = match transport.as_deref() {
            Some("http") => string_field(&self.server, "url").map(|url| safe_http_target(&url)),
            _ => string_field(&self.server, "command"),
        };
        McpServerSummary {
            name: self.name.to_string(),
            transport,
            target,
        }
    }

    pub fn to_value(&self) -> Result<Value> {
        self.validate()?;
        let mut servers = Map::new();
        servers.insert(self.name.to_string(), self.server.clone());
        let mut root = Map::new();
        root.insert("mcpServers".to_string(), Value::Object(servers));
        Ok(Value::Object(root))
    }

    pub fn render_pretty_json(&self) -> Result<String> {
        let value = self.to_value()?;
        let json = serde_json::to_string_pretty(&value)
            .map_err(|source| McpError::json(MCP_FILE, source))?;
        Ok(format!("{json}\n"))
    }

    fn transport_projection(&self) -> Result<String> {
        let server = self
            .server
            .as_object()
            .ok_or_else(|| McpError::InvalidConfig {
                message: format!("mcpServers.{} 必须是对象", self.name),
            })?;
        let type_value = optional_string(server, "type")?;
        let transport_value = optional_string(server, "transport")?;

        match (type_value, transport_value) {
            (Some(type_value), Some(transport_value))
                if normalize_transport(&type_value) != normalize_transport(&transport_value) =>
            {
                Err(McpError::ConflictingTransport {
                    name: self.name.to_string(),
                    type_value,
                    transport_value,
                })
            }
            (Some(value), _) | (_, Some(value)) => Ok(normalize_transport(&value)),
            (None, None) if server.contains_key("url") => Ok("http".to_string()),
            (None, None) => Ok("stdio".to_string()),
        }
    }

    fn stdio_config_fields(&self) -> Result<McpStdioServerConfig> {
        let server = self
            .server
            .as_object()
            .ok_or_else(|| McpError::InvalidConfig {
                message: format!("mcpServers.{} 必须是对象", self.name),
            })?;
        let command = optional_string(server, "command")?
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| McpError::InvalidConfig {
                message: "stdio MCP 必须提供非空 command".to_string(),
            })?;
        let args = match server.get("args") {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| McpError::InvalidConfig {
                            message: "args 必须是字符串数组".to_string(),
                        })
                })
                .collect::<Result<Vec<_>>>()?,
            Some(_) => {
                return Err(McpError::InvalidConfig {
                    message: "args 必须是字符串数组".to_string(),
                });
            }
        };
        let env = match server.get("env") {
            None => Vec::new(),
            Some(Value::Object(entries)) => entries
                .iter()
                .map(|(key, value)| {
                    value
                        .as_str()
                        .map(|value| (key.clone(), value.to_string()))
                        .ok_or_else(|| McpError::InvalidConfig {
                            message: format!("env.{key} 必须是字符串"),
                        })
                })
                .collect::<Result<Vec<_>>>()?,
            Some(_) => {
                return Err(McpError::InvalidConfig {
                    message: "env 必须是对象".to_string(),
                });
            }
        };
        let cwd = optional_string(server, "cwd")?;
        if cwd.as_deref().is_some_and(|value| value.trim().is_empty()) {
            return Err(McpError::InvalidConfig {
                message: "stdio MCP cwd 不能为空".to_string(),
            });
        }

        Ok(McpStdioServerConfig {
            transport: "stdio".to_string(),
            command,
            args,
            env,
            cwd,
        })
    }

    fn http_config_fields(&self) -> Result<McpHttpServerConfig> {
        let server = self
            .server
            .as_object()
            .ok_or_else(|| McpError::InvalidConfig {
                message: format!("mcpServers.{} 必须是对象", self.name),
            })?;
        let url = optional_string(server, "url")?
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| McpError::InvalidConfig {
                message: "HTTP MCP 必须提供非空 url".to_string(),
            })?;
        let parsed = url::Url::parse(&url).map_err(|_| McpError::InvalidConfig {
            message: "HTTP MCP url 必须是合法的绝对 URL".to_string(),
        })?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(McpError::InvalidConfig {
                message: "HTTP MCP url 只支持 http 或 https scheme".to_string(),
            });
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(McpError::InvalidConfig {
                message: "HTTP MCP url 不允许内嵌 username/password；请使用 header credential"
                    .to_string(),
            });
        }
        let bearer_token_env_var = optional_string(server, "bearer_token_env_var")?;
        if bearer_token_env_var
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(McpError::InvalidConfig {
                message: "bearer_token_env_var 不能为空".to_string(),
            });
        }
        let http_headers = string_map(server, "http_headers")?;
        let env_http_headers = string_map(server, "env_http_headers")?;

        Ok(McpHttpServerConfig {
            transport: "http".to_string(),
            url,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpServerSummary {
    pub name: String,
    pub transport: Option<String>,
    pub target: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpTestResult {
    pub name: String,
    pub transport: String,
    pub target: String,
    pub tool_count: usize,
    pub tool_names: Vec<String>,
    pub smoke: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpStdioServerConfig {
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedMcpStdioServerConfig {
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
}

#[derive(Clone, Eq, PartialEq)]
pub struct McpHttpServerConfig {
    pub transport: String,
    pub url: String,
    pub bearer_token_env_var: Option<String>,
    pub http_headers: Vec<(String, String)>,
    pub env_http_headers: Vec<(String, String)>,
}

impl fmt::Debug for McpHttpServerConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpHttpServerConfig")
            .field("transport", &self.transport)
            .field("url", &safe_http_target(&self.url))
            .field("bearer_token_env_var", &self.bearer_token_env_var)
            .field(
                "http_header_names",
                &self
                    .http_headers
                    .iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>(),
            )
            .field(
                "env_http_headers",
                &self.env_http_headers.iter().collect::<Vec<_>>(),
            )
            .finish()
    }
}

fn optional_string(server: &Map<String, Value>, key: &str) -> Result<Option<String>> {
    match server.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(McpError::InvalidConfig {
            message: format!("{key} 必须是字符串"),
        }),
    }
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn string_map(server: &Map<String, Value>, key: &str) -> Result<Vec<(String, String)>> {
    match server.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Object(entries)) => entries
            .iter()
            .map(|(entry_key, value)| {
                value
                    .as_str()
                    .map(|value| (entry_key.clone(), value.to_string()))
                    .ok_or_else(|| McpError::InvalidConfig {
                        message: format!("{key}.{entry_key} 必须是字符串"),
                    })
            })
            .collect(),
        Some(_) => Err(McpError::InvalidConfig {
            message: format!("{key} 必须是对象"),
        }),
    }
}

fn normalize_transport(value: &str) -> String {
    match value {
        "streamable-http" => "http".to_string(),
        value => value.to_string(),
    }
}

fn safe_http_target(value: &str) -> String {
    let Ok(mut url) = url::Url::parse(value) else {
        return "<invalid-url>".to_string();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

fn resolve_workspace_placeholders(value: &str, cwd: &Path) -> String {
    let cwd = cwd.display().to_string();
    value.replace("${workspace}", &cwd).replace("${cwd}", &cwd)
}

fn lexically_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    normalized.components().next_back(),
                    Some(Component::RootDir | Component::Prefix(_)) | None
                ) {
                    normalized.pop();
                }
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn preserves_unknown_server_fields_during_round_trip() {
        let value = json!({
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "pkg"],
                    "vendorExtension": {"enabled": true}
                }
            }
        });
        let document = McpDocument::from_value(value).expect("parse mcp");
        assert_eq!(document.summary().transport.as_deref(), Some("stdio"));
        assert_eq!(
            document.to_value().expect("render")["mcpServers"]["filesystem"]["vendorExtension"],
            json!({"enabled": true})
        );
    }

    #[test]
    fn rejects_missing_or_multiple_mcp_servers() {
        assert!(McpDocument::from_value(json!({"filesystem": {}})).is_err());
        assert!(McpDocument::from_value(json!({"mcpServers": {}})).is_err());
        let error = McpDocument::from_value(json!({
            "mcpServers": {
                "one": {"command": "cmd"},
                "two": {"command": "cmd"}
            }
        }))
        .expect_err("multiple should fail");
        assert!(error.to_string().contains("拆分"));
    }

    #[test]
    fn rejects_invalid_stdio_fields_before_document_is_created() {
        for value in [
            json!({"mcpServers": {"bad": {"type": "stdio"}}}),
            json!({"mcpServers": {"bad": {"command": "cmd", "args": [1]}}}),
            json!({"mcpServers": {"bad": {"command": "cmd", "env": {"KEY": 1}}}}),
        ] {
            assert!(matches!(
                McpDocument::from_value(value),
                Err(McpError::InvalidConfig { .. })
            ));
        }
    }

    #[test]
    fn rejects_non_string_conflicting_and_unsupported_transports() {
        let non_string = McpDocument::from_value(json!({
            "mcpServers": {"bad": {"type": 1, "command": "cmd"}}
        }));
        assert!(matches!(non_string, Err(McpError::InvalidConfig { .. })));

        let conflict = McpDocument::from_value(json!({
            "mcpServers": {"bad": {"type": "stdio", "transport": "http", "command": "cmd"}}
        }));
        assert!(matches!(
            conflict,
            Err(McpError::ConflictingTransport { .. })
        ));

        let unsupported = McpDocument::from_value(json!({
            "mcpServers": {"bad": {"transport": "sse", "command": "cmd"}}
        }));
        assert!(matches!(
            unsupported,
            Err(McpError::UnsupportedTransport { .. })
        ));
    }

    #[test]
    fn resolves_workspace_placeholders_only_for_runtime_projection() {
        let document = McpDocument::from_value(json!({
            "mcpServers": {
                "filesystem": {
                    "command": "runner-${cwd}",
                    "args": ["${workspace}"],
                    "env": {"ROOT": "${workspace}"}
                }
            }
        }))
        .expect("document");
        let resolved = document
            .resolve_stdio_server(Path::new("/tmp/project"))
            .expect("resolve");
        assert_eq!(resolved.command, "runner-/tmp/project");
        assert_eq!(resolved.args, vec!["/tmp/project"]);
        assert_eq!(
            resolved.env,
            vec![("ROOT".to_string(), "/tmp/project".to_string())]
        );
        assert_eq!(resolved.cwd, Path::new("/tmp/project"));
        assert_eq!(document.server["command"], "runner-${cwd}");
    }

    #[test]
    fn resolves_relative_stdio_cwd_before_other_placeholders() {
        let document = McpDocument::from_value(json!({
            "mcpServers": {
                "filesystem": {
                    "command": "${cwd}/runner",
                    "args": ["${workspace}"],
                    "env": {"ROOT": "${cwd}"},
                    "cwd": "tools/server"
                }
            }
        }))
        .expect("document");

        let resolved = document
            .resolve_stdio_server(Path::new("/tmp/project"))
            .expect("resolve");

        assert_eq!(resolved.cwd, Path::new("/tmp/project/tools/server"));
        assert_eq!(resolved.command, "/tmp/project/tools/server/runner");
        assert_eq!(resolved.args, ["/tmp/project/tools/server"]);
        assert_eq!(
            resolved.env,
            [("ROOT".to_string(), "/tmp/project/tools/server".to_string())]
        );
    }

    #[test]
    fn accepts_streamable_http_and_redacts_header_values_from_debug() {
        let document = McpDocument::from_value(json!({
            "mcpServers": {
                "docs": {
                    "transport": "streamable-http",
                    "url": "https://example.com/mcp?token=query-secret",
                    "bearer_token_env_var": "MCP_TOKEN",
                    "http_headers": {"X-Api-Key": "literal-secret"},
                    "env_http_headers": {"X-Account": "MCP_ACCOUNT"}
                }
            }
        }))
        .expect("document");

        let config = document.http_config().expect("http config");
        assert_eq!(config.transport, "http");
        assert_eq!(config.url, "https://example.com/mcp?token=query-secret");
        assert_eq!(config.bearer_token_env_var.as_deref(), Some("MCP_TOKEN"));
        assert_eq!(document.summary().transport.as_deref(), Some("http"));
        assert!(!format!("{document:?}").contains("literal-secret"));
        assert!(!format!("{config:?}").contains("literal-secret"));
        assert!(!format!("{document:?}").contains("query-secret"));
    }

    #[test]
    fn rejects_invalid_http_url_and_header_maps() {
        for value in [
            json!({"mcpServers": {"bad": {"url": "file:///tmp/mcp"}}}),
            json!({"mcpServers": {"bad": {"url": "https://user:password@example.com/mcp"}}}),
            json!({"mcpServers": {"bad": {"url": "https://example.com", "http_headers": []}}}),
            json!({"mcpServers": {"bad": {"url": "https://example.com", "env_http_headers": {"X-Key": 1}}}}),
        ] {
            assert!(matches!(
                McpDocument::from_value(value),
                Err(McpError::InvalidConfig { .. })
            ));
        }
    }

    #[test]
    fn stdio_constructor_builds_a_runtime_readable_document() {
        let document = McpDocument::stdio(
            "filesystem",
            "npx",
            vec!["-y".to_string(), "pkg".to_string()],
            [("TOKEN".to_string(), "secret".to_string())],
        )
        .unwrap();

        let config = document.stdio_config().unwrap();
        assert_eq!(config.command, "npx");
        assert_eq!(config.args, ["-y", "pkg"]);
        assert_eq!(config.env, [("TOKEN".to_string(), "secret".to_string())]);
        assert_eq!(config.cwd, None);
    }
}
