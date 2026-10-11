use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use agt_core::CategoryName;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ImportCandidate, McpDiagnostic, McpError, McpStore, McpWritePlan, StoredMcp,
    mask_sensitive_json, parse_claude_config, parse_codex_config,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct McpSummary {
    pub category: String,
    pub name: String,
    pub transport: String,
    pub target: Option<String>,
    pub path: String,
    pub dir: String,
}

impl From<&StoredMcp> for McpSummary {
    fn from(mcp: &StoredMcp) -> Self {
        let summary = mcp.document.summary();
        Self {
            category: mcp.category.to_string(),
            name: summary.name,
            transport: summary.transport.unwrap_or_else(|| "invalid".to_string()),
            target: summary.target,
            path: mcp.path.display().to_string(),
            dir: mcp.dir.display().to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct McpDiagnosticSummary {
    pub code: String,
    pub message: String,
    pub target: String,
    pub remediation: String,
    pub name_hint: Option<String>,
}

impl From<&McpDiagnostic> for McpDiagnosticSummary {
    fn from(diagnostic: &McpDiagnostic) -> Self {
        Self {
            code: diagnostic.category.to_string(),
            message: diagnostic.message.clone(),
            target: diagnostic.path.display().to_string(),
            remediation: diagnostic.remediation.clone(),
            name_hint: diagnostic.name_hint.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct McpScanResult {
    pub category: Option<String>,
    pub items: Vec<McpSummary>,
    pub diagnostics: Vec<McpDiagnosticSummary>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct McpInfoResult {
    #[serde(flatten)]
    pub mcp: McpSummary,
    pub config: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct McpTestSummary {
    pub category: String,
    pub name: String,
    pub transport: String,
    pub target: String,
    pub cwd: String,
    pub tool_count: usize,
    pub tool_names: Vec<String>,
    pub status: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransportExpectation {
    Stdio,
    Http,
}

impl McpTransportExpectation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpImportSource {
    Codex,
    ClaudeCode,
}

impl McpImportSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
        }
    }

    pub fn load(self, path: &Path) -> crate::Result<Vec<ImportCandidate>> {
        let metadata = fs::symlink_metadata(path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                McpError::InvalidImportSource {
                    path: path.to_path_buf(),
                }
            } else {
                McpError::io(path, source)
            }
        })?;
        if !metadata.file_type().is_file() {
            return Err(McpError::UnsupportedFileType {
                path: path.to_path_buf(),
            });
        }
        let text = fs::read_to_string(path).map_err(|source| McpError::io(path, source))?;
        match self {
            Self::Codex => parse_codex_config(path, &text),
            Self::ClaudeCode => parse_claude_config(path, &text, None, "claude-code"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct McpImportRequest {
    pub selected_names: Vec<String>,
    #[serde(default)]
    pub include_disabled: bool,
    #[serde(default)]
    pub replace_existing: Vec<String>,
    #[serde(default)]
    pub skip_existing: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct McpImportItemSummary {
    pub category: String,
    pub name: String,
    pub transport: String,
    pub credential_keys: Vec<String>,
    pub source: String,
    pub path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct McpImportIssueSummary {
    pub name: String,
    pub transport: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct McpImportIgnoredSummary {
    pub name: String,
    pub fields: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct McpImportResult {
    pub items: Vec<McpImportItemSummary>,
    pub unsupported: Vec<McpImportIssueSummary>,
    pub ignored: Vec<McpImportIgnoredSummary>,
    pub skipped: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct McpDeleteResult {
    pub category: String,
    pub name: String,
    pub path: String,
}

impl McpStore {
    pub fn scan_summary(&self) -> crate::Result<McpScanResult> {
        let scan = self.scan()?;
        Ok(McpScanResult {
            category: None,
            items: scan.items.iter().map(McpSummary::from).collect(),
            diagnostics: scan
                .diagnostics
                .iter()
                .map(McpDiagnosticSummary::from)
                .collect(),
        })
    }

    pub fn scan_summary_in(&self, category: &CategoryName) -> crate::Result<McpScanResult> {
        let category_dir = self.category_dir(category);
        let mut scan = self.scan()?;
        scan.items.retain(|mcp| mcp.category == *category);
        scan.diagnostics
            .retain(|diagnostic| diagnostic.path.starts_with(&category_dir));
        Ok(McpScanResult {
            category: Some(category.to_string()),
            items: scan.items.iter().map(McpSummary::from).collect(),
            diagnostics: scan
                .diagnostics
                .iter()
                .map(McpDiagnosticSummary::from)
                .collect(),
        })
    }

    pub fn read_summary_in(
        &self,
        category: &CategoryName,
        name: &str,
    ) -> crate::Result<McpSummary> {
        Ok(McpSummary::from(&self.read_in(category, name)?))
    }

    pub fn info_summary_in(
        &self,
        category: &CategoryName,
        name: &str,
    ) -> crate::Result<McpInfoResult> {
        let stored = self.read_in(category, name)?;
        let summary = McpSummary::from(&stored);
        let mut config = mask_sensitive_json(&stored.document.to_value()?);
        if summary.transport == "http"
            && let Some(target) = &summary.target
            && let Some(server) = config
                .get_mut("mcpServers")
                .and_then(Value::as_object_mut)
                .and_then(|servers| servers.get_mut(name))
                .and_then(Value::as_object_mut)
        {
            server.insert("url".to_string(), Value::String(target.clone()));
        }
        Ok(McpInfoResult {
            mcp: summary,
            config,
        })
    }

    pub fn test_summary_in(
        &self,
        category: &CategoryName,
        name: &str,
        expected: Option<McpTransportExpectation>,
    ) -> crate::Result<McpTestSummary> {
        let stored = self.read_in(category, name)?;
        let transport = stored
            .document
            .summary()
            .transport
            .unwrap_or_else(|| "invalid".to_string());
        if let Some(expected) = expected
            && transport != expected.as_str()
        {
            return Err(McpError::InvalidConfig {
                message: format!(
                    "MCP transport 已变更：期望 {}，实际 {transport}",
                    expected.as_str()
                ),
            });
        }
        let result = stored.document.smoke_test()?;
        Ok(McpTestSummary {
            category: category.to_string(),
            name: result.name,
            transport: result.transport,
            target: result.target,
            cwd: stored.document.stdio_cwd(),
            tool_count: result.tool_count,
            tool_names: result.tool_names,
            status: result.smoke,
        })
    }

    pub fn import_candidates_in(
        &self,
        category: &CategoryName,
        candidates: Vec<ImportCandidate>,
        request: McpImportRequest,
    ) -> crate::Result<McpImportResult> {
        let candidates_by_name = unique_candidates(candidates)?;
        let selected = unique_names("selected_names", &request.selected_names)?;
        if selected.is_empty() {
            return Err(McpError::InvalidConfig {
                message: "MCP import selected_names 不能为空".to_string(),
            });
        }
        let replace = unique_names("replace_existing", &request.replace_existing)?;
        let skip = unique_names("skip_existing", &request.skip_existing)?;
        if !replace.is_disjoint(&skip) {
            return Err(McpError::InvalidConfig {
                message: "replace_existing 与 skip_existing 不能重叠".to_string(),
            });
        }
        if !replace.is_subset(&selected) || !skip.is_subset(&selected) {
            return Err(McpError::InvalidConfig {
                message: "replace/skip policy 只能引用 selected_names".to_string(),
            });
        }

        let unsupported = candidates_by_name
            .values()
            .filter_map(|candidate| {
                candidate
                    .unsupported_reason()
                    .map(|reason| McpImportIssueSummary {
                        name: candidate.name.clone(),
                        transport: candidate.transport.clone(),
                        reason: reason.to_string(),
                    })
            })
            .collect();
        let ignored = candidates_by_name
            .values()
            .filter(|candidate| candidate.is_supported() && !candidate.ignored_fields.is_empty())
            .map(|candidate| McpImportIgnoredSummary {
                name: candidate.name.clone(),
                fields: candidate.ignored_fields.clone(),
            })
            .collect();

        let mut plans = Vec::new();
        let mut metadata = BTreeMap::new();
        let mut skipped = Vec::new();
        for name in selected {
            let candidate =
                candidates_by_name
                    .get(&name)
                    .ok_or_else(|| McpError::InvalidConfig {
                        message: format!("MCP import candidate 不存在：{name}"),
                    })?;
            if let Some(reason) = candidate.unsupported_reason() {
                return Err(McpError::InvalidConfig {
                    message: format!("MCP `{name}` 不可导入：{reason}"),
                });
            }
            if candidate.is_disabled() && !request.include_disabled {
                return Err(McpError::InvalidConfig {
                    message: format!("MCP `{name}` enabled=false；需要 include_disabled=true"),
                });
            }
            let document = candidate.clone().into_document()?;
            match self.read_in(category, &name) {
                Ok(_) if replace.contains(&name) => plans.push(McpWritePlan::replace(document)),
                Ok(_) if skip.contains(&name) => skipped.push(name.clone()),
                Ok(_) => return Err(McpError::AlreadyExists { name }),
                Err(McpError::NotFound { .. }) => {
                    if replace.contains(&name) || skip.contains(&name) {
                        return Err(McpError::InvalidConfig {
                            message: format!("MCP `{name}` 不存在，不能 replace/skip"),
                        });
                    }
                    plans.push(McpWritePlan::create(document));
                }
                Err(error) => return Err(error),
            }
            metadata.insert(
                candidate.name.clone(),
                (
                    candidate.source.clone(),
                    candidate.transport.clone(),
                    candidate.credential_keys(),
                ),
            );
        }

        let stored = if plans.is_empty() {
            Vec::new()
        } else {
            self.apply_batch_in(category, plans)?
        };
        let items = stored
            .iter()
            .map(|item| {
                let name = item.document.name.to_string();
                let (source, transport, credential_keys) = &metadata[&name];
                McpImportItemSummary {
                    category: item.category.to_string(),
                    name,
                    transport: transport.clone(),
                    credential_keys: credential_keys.clone(),
                    source: source.clone(),
                    path: item.path.display().to_string(),
                }
            })
            .collect();
        Ok(McpImportResult {
            items,
            unsupported,
            ignored,
            skipped,
        })
    }

    pub fn delete_summary_in(
        &self,
        category: &CategoryName,
        name: &str,
    ) -> crate::Result<McpDeleteResult> {
        let path = self.delete_in(category, name)?;
        Ok(McpDeleteResult {
            category: category.to_string(),
            name: name.to_string(),
            path: path.display().to_string(),
        })
    }
}

fn unique_candidates(
    candidates: Vec<ImportCandidate>,
) -> crate::Result<BTreeMap<String, ImportCandidate>> {
    let mut by_name = BTreeMap::new();
    for candidate in candidates {
        let name = candidate.name.clone();
        if by_name.insert(name.clone(), candidate).is_some() {
            return Err(McpError::InvalidConfig {
                message: format!("MCP import candidate 重名：{name}"),
            });
        }
    }
    Ok(by_name)
}

fn unique_names(field: &str, names: &[String]) -> crate::Result<BTreeSet<String>> {
    let values = names.iter().cloned().collect::<BTreeSet<_>>();
    if values.len() != names.len() {
        return Err(McpError::InvalidConfig {
            message: format!("{field} 包含重复名称"),
        });
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::*;
    use crate::McpDocument;

    fn test_home(name: &str) -> std::path::PathBuf {
        let home = std::env::temp_dir().join(format!("agt-mcp-safe-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        home
    }

    #[test]
    fn summary_and_info_mask_credentials_and_http_query() {
        let home = test_home("mask");
        let store = McpStore::new(&home);
        store
            .add(
                McpDocument::from_value(json!({
                    "mcpServers": {"docs": {
                        "type": "http",
                        "url": "https://example.com/mcp?token=secret",
                        "http_headers": {"Authorization": "Bearer secret"}
                    }}
                }))
                .unwrap(),
            )
            .unwrap();

        let info = store
            .info_summary_in(&CategoryName::default(), "docs")
            .unwrap();
        let value = serde_json::to_value(info).unwrap().to_string();

        assert!(!value.contains("secret"));
        assert!(value.contains("<redacted>"));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn import_selection_is_atomic_and_requires_explicit_conflict_policy() {
        let home = test_home("import");
        let store = McpStore::new(&home);
        store
            .add(
                McpDocument::stdio("existing", "old", Vec::new(), [], &std::env::temp_dir())
                    .unwrap(),
            )
            .unwrap();
        let candidates = parse_codex_config(
            Path::new("config.toml"),
            "[mcp_servers.existing]\ncommand='new'\ncwd='/srv/new'\n[mcp_servers.fresh]\ncommand='fresh'\ncwd='/srv/fresh'\n",
        )
        .unwrap();

        let error = store
            .import_candidates_in(
                &CategoryName::default(),
                candidates.clone(),
                McpImportRequest {
                    selected_names: vec!["existing".to_string(), "fresh".to_string()],
                    include_disabled: false,
                    replace_existing: Vec::new(),
                    skip_existing: Vec::new(),
                },
            )
            .unwrap_err();
        assert!(matches!(error, McpError::AlreadyExists { .. }));
        assert!(store.read("fresh").is_err());

        let result = store
            .import_candidates_in(
                &CategoryName::default(),
                candidates,
                McpImportRequest {
                    selected_names: vec!["existing".to_string(), "fresh".to_string()],
                    include_disabled: false,
                    replace_existing: Vec::new(),
                    skip_existing: vec!["existing".to_string()],
                },
            )
            .unwrap();
        assert_eq!(result.skipped, ["existing"]);
        assert_eq!(result.items[0].name, "fresh");
        let _ = fs::remove_dir_all(home);
    }
}
