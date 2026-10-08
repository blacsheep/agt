use serde::Serialize;
use serde_json::{Value, json};

/// Safe across the model, MCP and CLI boundaries. Never wraps raw parser/storage text.
#[derive(Clone, Debug, Serialize)]
pub struct ToolError {
    pub code: &'static str,
    pub message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_path: Option<String>,
}

impl ToolError {
    pub fn new(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message,
            field: None,
            argument_path: None,
            schema_path: None,
        }
    }

    pub(crate) fn invalid(field: Option<&str>, message: &'static str) -> Self {
        Self {
            code: "invalid_arguments",
            message,
            field: field.map(str::to_owned),
            argument_path: None,
            schema_path: None,
        }
    }

    pub fn from_contract(error: agt_core::tool_schema::ContractError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            field: error
                .field
                .split('/')
                .nth(1)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            argument_path: (!error.field.is_empty()).then_some(error.field),
            schema_path: (!error.schema_path.is_empty()).then_some(error.schema_path),
        }
    }

    pub fn as_value(&self) -> Value {
        json!({"error": self})
    }

    pub(crate) fn classify(error: anyhow::Error) -> Self {
        if let Some(error) = error.downcast_ref::<Self>() {
            return error.clone();
        }
        if let Some(agt_mcp::McpError::ToolSchema { tool, source }) =
            error.downcast_ref::<agt_mcp::McpError>()
        {
            let mut error = Self::from_contract(source.clone());
            error.field = Some(format!("tools/{tool}/inputSchema"));
            return error;
        }
        if error.downcast_ref::<serde_json::Error>().is_some() {
            return Self::invalid(
                None,
                "Arguments do not match the tool schema; check field types and required fields.",
            );
        }
        if let Some(error) = error.downcast_ref::<agt_im::ImError>() {
            use agt_im::ImError::*;
            return match error {
                PrincipalNotFound(_)
                | ConversationNotFound(_)
                | MessageNotFound(_)
                | EventNotFound(_) => Self::new(
                    "not_found",
                    "Referenced item was not found; obtain an ID from list/history tools.",
                ),
                NotMember { .. } | PermissionDenied(_) => Self::new(
                    "permission_denied",
                    "Your fixed identity lacks active membership or the required role.",
                ),
                AlreadyExists(_) => Self::new(
                    "already_exists",
                    "The item already exists; reuse it or choose a new identity.",
                ),
                Conflict(_) | IdempotencyConflict => Self::new(
                    "conflict",
                    "Check current state. Reuse request_key only for identical content; do not blindly retry mutations.",
                ),
                InvalidCursor(_) => Self::invalid(
                    Some("before"),
                    "Use next_cursor from this conversation, or omit before/null for the latest page.",
                ),
                InvalidInput(_) => Self::invalid(
                    None,
                    "Invalid operation arguments; check references and constraints in the tool schema.",
                ),
                _ => Self::new(
                    "internal_error",
                    "Storage operation failed. Inspect host diagnostics and current state before retrying a mutation.",
                ),
            };
        }
        macro_rules! resource_error {
            ($kind:path) => {
                if let Some(typed) = error.downcast_ref::<$kind>() {
                    use $kind as E;
                    match typed {
                        E::NotFound { .. } => {
                            return Self::new(
                                "not_found",
                                "Resource not found; use resource_list and check category/name.",
                            );
                        }
                        E::AlreadyExists { .. } => {
                            return Self::new(
                                "already_exists",
                                "Resource already exists; use create=false to update it.",
                            );
                        }
                        _ => {}
                    }
                }
            };
        }
        resource_error!(agt_agent::AgentError);
        resource_error!(agt_model::ModelError);
        resource_error!(agt_provider::ProviderError);
        resource_error!(agt_prompt::PromptError);
        resource_error!(agt_skill::SkillError);
        resource_error!(agt_mcp::McpError);
        Self::new(
            "operation_rejected",
            "Check resource references and configuration. Existing resources require create=false; deletion requires the exact current plan. Inspect current state before retrying.",
        )
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;
        if let Some(field) = &self.field {
            write!(f, " (field: {field})")?;
        }
        if let Some(path) = &self.schema_path {
            write!(f, " (schema: {path})")?;
        }
        Ok(())
    }
}
impl std::error::Error for ToolError {}
