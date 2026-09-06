use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpInitializeInfo {
    pub protocol_version: String,
    pub capabilities: McpServerCapabilities,
    pub server_info: McpImplementationInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpImplementationInfo {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub icons: Vec<McpIcon>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct McpServerCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<McpToolsCapability>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolsCapability {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpIcon {
    pub src: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sizes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(
        default,
        rename = "outputSchema",
        skip_serializing_if = "Option::is_none"
    )]
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<McpToolAnnotations>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub icons: Vec<McpIcon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<Value>,
    #[serde(default, rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolAnnotations {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_world_hint: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpContentBlock {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolCallResult {
    #[serde(default)]
    pub content: Vec<McpContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default, rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum McpClientEvent {
    ToolsListChanged,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conformance_fixtures_preserve_initialize_tool_and_result_metadata() {
        let initialize: McpInitializeInfo =
            serde_json::from_str(include_str!("../tests/fixtures/initialize_result.json"))
                .expect("initialize fixture");
        assert_eq!(initialize.protocol_version, MCP_PROTOCOL_VERSION);
        assert_eq!(
            initialize
                .capabilities
                .tools
                .and_then(|tools| tools.list_changed),
            Some(true)
        );
        assert_eq!(initialize.server_info.extra["vendorField"], "preserved");

        let tool: McpTool = serde_json::from_str(include_str!("../tests/fixtures/tool.json"))
            .expect("tool fixture");
        assert_eq!(
            tool.output_schema,
            Some(serde_json::json!({"type": "object"}))
        );
        assert_eq!(
            tool.annotations.as_ref().expect("annotations").extra["vendorHint"],
            "preserved"
        );

        let result: McpToolCallResult =
            serde_json::from_str(include_str!("../tests/fixtures/tool_call_result.json"))
                .expect("result fixture");
        assert_eq!(result.content[0].text.as_deref(), Some("fixture result"));
        assert_eq!(
            result.content[1].uri.as_deref(),
            Some("file:///tmp/result.txt")
        );
        assert_eq!(result.content[1].extra["vendorField"], "preserved");
    }
}
