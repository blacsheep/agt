use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde_json::{Map, Value};

use crate::{McpDocument, McpError, Result};

const STDIO_FIELDS: &[&str] = &["type", "transport", "command", "args", "env", "cwd"];
const HTTP_FIELDS: &[&str] = &[
    "type",
    "transport",
    "url",
    "bearer_token_env_var",
    "http_headers",
    "env_http_headers",
];
const SAFE_IGNORED_FIELDS: &[&str] = &[
    "startup_timeout_sec",
    "startup_timeout_ms",
    "tool_timeout_sec",
    "tool_timeout_ms",
];

#[derive(Clone, PartialEq)]
pub struct ImportCandidate {
    pub source: String,
    pub name: String,
    pub transport: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub ignored_fields: Vec<String>,
    disabled: bool,
    issues: Vec<ImportCandidateIssue>,
    server: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportCandidateIssue {
    pub kind: ImportCandidateIssueKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ImportCandidateIssueKind {
    InvalidConfig,
    UnsupportedTransport,
    UnknownFields,
}

impl fmt::Debug for ImportCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportCandidate")
            .field("source", &self.source)
            .field("name", &self.name)
            .field("transport", &self.transport)
            .field("command", &self.command)
            .field("args_count", &self.args.len())
            .field("env_keys", &self.env.keys().collect::<Vec<_>>())
            .field("ignored_fields", &self.ignored_fields)
            .field("disabled", &self.disabled)
            .field("issues", &self.issues)
            .finish()
    }
}

impl ImportCandidate {
    pub fn is_supported(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn issues(&self) -> &[ImportCandidateIssue] {
        &self.issues
    }

    pub fn unsupported_reason(&self) -> Option<&str> {
        self.issues.first().map(|issue| issue.message.as_str())
    }

    pub fn env_keys(&self) -> Vec<String> {
        self.env.keys().cloned().collect()
    }

    pub fn credential_keys(&self) -> Vec<String> {
        let mut keys = self.env_keys();
        if let Some(server) = self.server.as_object() {
            if let Some(variable) = server.get("bearer_token_env_var").and_then(Value::as_str) {
                keys.push(format!("bearer_env:{variable}"));
            }
            for field in ["http_headers", "env_http_headers"] {
                if let Some(headers) = server.get(field).and_then(Value::as_object) {
                    keys.extend(headers.keys().map(|name| format!("header:{name}")));
                }
            }
        }
        keys.sort();
        keys.dedup();
        keys
    }

    pub fn into_document(self) -> Result<McpDocument> {
        if let Some(reason) = self.unsupported_reason() {
            return Err(McpError::InvalidConfig {
                message: format!("MCP `{}` 不可导入：{reason}", self.name),
            });
        }
        let mut servers = Map::new();
        servers.insert(self.name, self.server);
        McpDocument::from_value(Value::Object(Map::from_iter([(
            "mcpServers".to_string(),
            Value::Object(servers),
        )])))
    }
}

pub fn parse_codex_config(path: &Path, text: &str) -> Result<Vec<ImportCandidate>> {
    let value: toml::Value = text
        .parse()
        .map_err(|source: toml::de::Error| McpError::Toml {
            path: path.to_path_buf(),
            message: source.message().to_string(),
        })?;
    let root = value.as_table().ok_or_else(|| McpError::InvalidConfig {
        message: format!("Codex MCP 配置必须是 TOML table：{}", path.display()),
    })?;
    let servers = root
        .get("mcp_servers")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| McpError::InvalidConfig {
            message: format!("Codex 配置缺少 `[mcp_servers.<name>]`：{}", path.display()),
        })?;
    let mut candidates = Vec::with_capacity(servers.len());
    for (name, server) in servers {
        let server = serde_json::to_value(server).map_err(|source| McpError::InvalidConfig {
            message: format!("转换 Codex MCP `{name}` 失败：{source}"),
        })?;
        candidates.push(candidate_from_server(
            format!("codex:{}", path.display()),
            name.clone(),
            server,
        ));
    }
    candidates.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(candidates)
}

pub fn parse_claude_config(
    path: &Path,
    text: &str,
    project_root: Option<&Path>,
    source: &str,
) -> Result<Vec<ImportCandidate>> {
    let value: Value = serde_json::from_str(text).map_err(|source| McpError::json(path, source))?;
    let root = value.as_object().ok_or_else(|| McpError::InvalidConfig {
        message: format!("Claude MCP 配置必须是 JSON object：{}", path.display()),
    })?;
    let mut candidates = Vec::new();
    if let Some(servers) = root.get("mcpServers") {
        candidates.extend(candidates_from_servers(
            format!("{source}:{}", path.display()),
            servers,
        )?);
    }
    if let Some(project_root) = project_root
        && let Some(projects) = root.get("projects").and_then(Value::as_object)
    {
        let canonical = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.to_path_buf());
        let keys = [
            canonical.display().to_string(),
            project_root.display().to_string(),
        ];
        if let Some(project) = keys.iter().find_map(|key| projects.get(key))
            && let Some(servers) = project.get("mcpServers")
        {
            candidates.extend(candidates_from_servers(
                format!("{source}-local:{}", canonical.display()),
                servers,
            )?);
        }
    }
    candidates.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.source.cmp(&right.source))
    });
    Ok(candidates)
}

fn candidates_from_servers(source: String, value: &Value) -> Result<Vec<ImportCandidate>> {
    let servers = value.as_object().ok_or_else(|| McpError::InvalidConfig {
        message: format!("{source} 的 mcpServers 必须是 object"),
    })?;
    Ok(servers
        .iter()
        .map(|(name, server)| candidate_from_server(source.clone(), name.clone(), server.clone()))
        .collect())
}

fn candidate_from_server(source: String, name: String, server: Value) -> ImportCandidate {
    let object = server.as_object();
    let projection = object.map(transport_projection);
    let transport = projection
        .as_ref()
        .and_then(|projection| projection.as_ref().ok())
        .cloned()
        .unwrap_or_else(|| "invalid".to_string());
    let mut issues = Vec::new();
    if object.is_none() {
        push_issue(
            &mut issues,
            ImportCandidateIssueKind::InvalidConfig,
            "server config 必须是 object".to_string(),
        );
    } else if let Some(message) = projection.and_then(|result| result.err()) {
        push_issue(
            &mut issues,
            ImportCandidateIssueKind::InvalidConfig,
            message,
        );
    }
    if !matches!(transport.as_str(), "stdio" | "http") {
        let message = if transport == "sse" {
            "transport `sse` 是已废弃的 legacy HTTP+SSE；当前只支持 stdio 与 Streamable HTTP"
                .to_string()
        } else {
            format!("transport `{transport}` 不受支持；当前只支持 stdio 与 Streamable HTTP")
        };
        push_issue(
            &mut issues,
            ImportCandidateIssueKind::UnsupportedTransport,
            message,
        );
    }
    let mut ignored_fields = Vec::new();
    let mut disabled = false;
    if let Some(object) = object {
        if let Some(enabled) = object.get("enabled") {
            match enabled.as_bool() {
                Some(true) => ignored_fields.push("enabled".to_string()),
                Some(false) => {
                    disabled = true;
                    ignored_fields.push("enabled".to_string());
                }
                None => push_issue(
                    &mut issues,
                    ImportCandidateIssueKind::InvalidConfig,
                    "字段 `enabled` 必须是 boolean".to_string(),
                ),
            }
        }
        ignored_fields.extend(
            SAFE_IGNORED_FIELDS
                .iter()
                .filter(|field| object.contains_key(**field))
                .map(|field| (*field).to_string()),
        );
        let unknown = object
            .keys()
            .filter(|key| {
                !known_fields(&transport).contains(&key.as_str())
                    && !SAFE_IGNORED_FIELDS.contains(&key.as_str())
                    && key.as_str() != "enabled"
            })
            .cloned()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            push_issue(
                &mut issues,
                ImportCandidateIssueKind::UnknownFields,
                format!("包含无法安全转换的字段：{}", unknown.join(", ")),
            );
        }
    }
    let command = object
        .and_then(|value| value.get("command"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let args = object
        .and_then(|value| value.get("args"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();
    let env = object
        .and_then(|value| value.get("env"))
        .and_then(Value::as_object)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut sanitized_server = server;
    if let Some(object) = sanitized_server.as_object_mut() {
        object.remove("enabled");
        for field in SAFE_IGNORED_FIELDS {
            object.remove(*field);
        }
    }
    if issues.is_empty()
        && let Err(error) = candidate_document(&name, sanitized_server.clone())
    {
        push_issue(
            &mut issues,
            ImportCandidateIssueKind::InvalidConfig,
            error.to_string(),
        );
    }
    ImportCandidate {
        source,
        name,
        transport,
        command,
        args,
        env,
        ignored_fields,
        disabled,
        issues,
        server: sanitized_server,
    }
}

fn transport_projection(server: &Map<String, Value>) -> std::result::Result<String, String> {
    let type_value = string_field(server, "type")?;
    let transport_value = string_field(server, "transport")?;
    match (type_value, transport_value) {
        (Some(left), Some(right)) if normalize_transport(&left) != normalize_transport(&right) => {
            Err(format!("type `{left}` 与 transport `{right}` 冲突"))
        }
        (Some(value), _) | (_, Some(value)) => Ok(normalize_transport(&value)),
        (None, None) if server.contains_key("url") => Ok("http".to_string()),
        (None, None) => Ok("stdio".to_string()),
    }
}

fn known_fields(transport: &str) -> &'static [&'static str] {
    match transport {
        "http" => HTTP_FIELDS,
        _ => STDIO_FIELDS,
    }
}

fn normalize_transport(value: &str) -> String {
    match value {
        "streamable-http" => "http".to_string(),
        value => value.to_string(),
    }
}

fn push_issue(
    issues: &mut Vec<ImportCandidateIssue>,
    kind: ImportCandidateIssueKind,
    message: String,
) {
    issues.push(ImportCandidateIssue { kind, message });
    issues.sort_by_key(|issue| issue.kind);
}

fn candidate_document(name: &str, server: Value) -> Result<McpDocument> {
    let mut servers = Map::new();
    servers.insert(name.to_string(), server);
    McpDocument::from_value(Value::Object(Map::from_iter([(
        "mcpServers".to_string(),
        Value::Object(servers),
    )])))
}

fn string_field(
    server: &Map<String, Value>,
    key: &str,
) -> std::result::Result<Option<String>, String> {
    match server.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("字段 `{key}` 必须是 string")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_stdio_cwd_and_http_candidates() {
        let candidates = parse_codex_config(
            Path::new("/tmp/config.toml"),
            r#"
[mcp_servers.filesystem]
command = "npx"
args = ["-y", "pkg"]
env = { API_KEY = "secret" }

[mcp_servers.remote]
url = "https://example.com/mcp"

[mcp_servers.cwd-sensitive]
command = "runner"
cwd = "/tmp/project"

[mcp_servers.timeout]
command = "runner"
enabled = true
startup_timeout_sec = 10
"#,
        )
        .unwrap();

        assert_eq!(candidates.len(), 4);
        let filesystem = candidates
            .iter()
            .find(|item| item.name == "filesystem")
            .unwrap();
        assert!(filesystem.is_supported());
        assert_eq!(filesystem.env_keys(), ["API_KEY"]);
        assert!(!format!("{filesystem:?}").contains("secret"));
        assert!(
            candidates
                .iter()
                .find(|item| item.name == "remote")
                .unwrap()
                .is_supported()
        );
        assert!(
            candidates
                .iter()
                .find(|item| item.name == "cwd-sensitive")
                .unwrap()
                .is_supported()
        );
        let timeout = candidates
            .iter()
            .find(|item| item.name == "timeout")
            .unwrap();
        assert!(timeout.is_supported());
        assert_eq!(timeout.ignored_fields, ["enabled", "startup_timeout_sec"]);
    }

    #[test]
    fn keeps_disabled_state_separate_from_unknown_field_diagnostics() {
        let candidates = parse_codex_config(
            Path::new("/tmp/config.toml"),
            r#"
[mcp_servers.disabled]
command = "runner"
enabled = false
vendor_field = "unsupported"
"#,
        )
        .unwrap();
        let candidate = &candidates[0];

        assert!(candidate.is_disabled());
        assert!(!candidate.is_supported());
        assert!(
            candidate
                .unsupported_reason()
                .unwrap()
                .contains("vendor_field")
        );
    }

    #[test]
    fn sanitizes_source_only_fields_and_keeps_http_secrets_out_of_debug() {
        let candidates = parse_codex_config(
            Path::new("/tmp/config.toml"),
            r#"
[mcp_servers.remote]
url = "https://example.com/mcp"
enabled = false
http_headers = { X_Api_Key = "literal-secret" }
env_http_headers = { X_Account = "MCP_ACCOUNT" }
"#,
        )
        .unwrap();
        let candidate = candidates.into_iter().next().unwrap();

        assert!(candidate.is_supported());
        assert!(candidate.is_disabled());
        assert!(!format!("{candidate:?}").contains("literal-secret"));
        let value = candidate.into_document().unwrap().to_value().unwrap();
        assert!(value["mcpServers"]["remote"].get("enabled").is_none());
    }

    #[test]
    fn parses_claude_user_local_and_project_wrappers() {
        let root = Path::new("/tmp/project");
        let text = r#"{
          "mcpServers":{"user":{"command":"user-cmd"}},
          "projects":{"/tmp/project":{"mcpServers":{"local":{"type":"stdio","command":"local-cmd"}}}}
        }"#;
        let candidates = parse_claude_config(
            Path::new("/tmp/.claude.json"),
            text,
            Some(root),
            "claude-user",
        )
        .unwrap();
        assert_eq!(
            candidates
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            ["local", "user"]
        );

        let project = parse_claude_config(
            Path::new("/tmp/project/.mcp.json"),
            r#"{"mcpServers":{"project":{"command":"project-cmd"},"sse":{"type":"sse","url":"https://example.com"}}}"#,
            None,
            "claude-project",
        )
        .unwrap();
        assert_eq!(project.len(), 2);
        assert!(
            !project
                .iter()
                .find(|item| item.name == "sse")
                .unwrap()
                .is_supported()
        );
    }
}
