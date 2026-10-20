use serde::Serialize;
use serde_json::{Value, json};

/// Safe across the model, MCP and CLI boundaries. Never wraps raw parser/storage text.
/// The hidden cause behind a classified failure stays in `diagnostic`, which is never
/// serialized; hosts may record it only in local, operator-only diagnostics.
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
    #[serde(skip)]
    diagnostic: Option<Box<str>>,
}

/// A refusal whose fixed text was written for the caller, such as a rule the request
/// broke. It reaches the caller as `operation_rejected` with this message.
#[derive(Clone, Copy, Debug)]
pub struct Refusal(pub &'static str);

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Refusal {}

impl ToolError {
    pub fn new(code: &'static str, message: &'static str) -> Self {
        Self {
            code,
            message,
            field: None,
            argument_path: None,
            schema_path: None,
            diagnostic: None,
        }
    }

    pub(crate) fn invalid(field: Option<&str>, message: &'static str) -> Self {
        Self {
            code: "invalid_arguments",
            message,
            field: field.map(str::to_owned),
            argument_path: None,
            schema_path: None,
            diagnostic: None,
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
            diagnostic: None,
        }
    }

    pub fn as_value(&self) -> Value {
        json!({"error": self})
    }

    /// Local cause chain for a classified failure; never part of the public error.
    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }

    pub(crate) fn classify(error: anyhow::Error) -> Self {
        if let Some(error) = error.downcast_ref::<Self>() {
            return error.clone();
        }
        let mut public = Self::public(&error);
        public.diagnostic = Some(format!("{error:#}").into());
        public
    }

    fn public(error: &anyhow::Error) -> Self {
        if let Some(Refusal(message)) = error.downcast_ref::<Refusal>() {
            return Self::new("operation_rejected", message);
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
        if let Some(error) = error.downcast_ref::<agt_mcp::McpError>()
            && let Some(message) = mcp_failure(error)
        {
            return Self::new("mcp_server_error", message);
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

/// Why connecting to or probing an MCP server failed, in fixed text. The server's own
/// output (stderr, paths, commands) stays in the local diagnostic.
fn mcp_failure(error: &agt_mcp::McpError) -> Option<&'static str> {
    use agt_mcp::McpError::*;
    Some(match error {
        Startup { .. } => {
            "The MCP server could not start. Check its command, arguments and working directory."
        }
        UnsupportedProtocolVersion { .. } => {
            "The MCP server answered with a protocol version agt does not support. agt asks for MCP 2025-11-25 and accepts servers that answer 2024-11-05 or later."
        }
        Protocol { .. } => {
            "The MCP server sent a reply that breaks the MCP protocol. Check its initialize and tools/list responses."
        }
        MissingCapability { .. } => {
            "The MCP server does not declare a capability agt needs, such as tools."
        }
        LimitExceeded { .. } => {
            "The MCP server exceeded an agt limit, such as the number or size of its tools."
        }
        Timeout { .. } => {
            "The MCP server did not answer in time. Check that it starts quickly and writes replies to stdout."
        }
        MissingCredential { .. } => {
            "The MCP server needs a credential from an environment variable that is not set."
        }
        AuthenticationRequired { .. } => {
            "The MCP server rejected the request as unauthenticated. Configure a bearer or header credential."
        }
        HttpTransport { .. } => "The MCP server could not be reached over HTTP.",
        InvalidConfig { .. } | ConflictingTransport { .. } | UnsupportedTransport { .. } => {
            "The MCP configuration is invalid. Check command, args, env and transport."
        }
        _ => return None,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classified_cause_is_kept_but_never_serialized() {
        let cause = anyhow::anyhow!("disk I/O at /home/x/.agt/runtime").context("save failed");
        let error = ToolError::classify(cause);
        let diagnostic = error.diagnostic().unwrap();
        assert!(diagnostic.contains("save failed") && diagnostic.contains("/home/x"));
        let public = error.as_value().to_string();
        assert!(!public.contains("/home/x") && !public.contains("diagnostic"));
        assert!(ToolError::new("conflict", "public").diagnostic().is_none());
        // An explicit ToolError passes through unchanged and carries no cause.
        let explicit = ToolError::classify(ToolError::new("conflict", "public").into());
        assert!(explicit.diagnostic().is_none());
    }

    #[test]
    fn mcp_failures_say_what_went_wrong_without_server_output() {
        let error = ToolError::classify(
            agt_mcp::McpError::UnsupportedProtocolVersion {
                name: "secret-server".into(),
                version: "1999-01-01".into(),
                stderr: agt_mcp::McpStderrSummary::default(),
            }
            .into(),
        );
        assert_eq!(error.code, "mcp_server_error");
        assert!(error.message.contains(agt_mcp::MCP_PROTOCOL_VERSION));
        assert!(!error.as_value().to_string().contains("secret-server"));
        assert!(error.diagnostic().unwrap().contains("1999-01-01"));
        // Resource errors keep their own codes.
        let missing = ToolError::classify(agt_mcp::McpError::NotFound { name: "x".into() }.into());
        assert_eq!(missing.code, "not_found");
    }

    #[test]
    fn a_refusal_reaches_the_caller_in_its_own_words() {
        let error = ToolError::classify(Refusal("timers cannot be set here").into());
        assert_eq!(error.code, "operation_rejected");
        assert_eq!(error.message, "timers cannot be set here");
    }
}
