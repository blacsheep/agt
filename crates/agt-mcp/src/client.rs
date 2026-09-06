use std::fmt::Debug;
use std::path::{Path, PathBuf};

use agt_core::CancellationToken;
use serde_json::Value;

use crate::config::McpDocument;
use crate::error::Result;
use crate::http::HttpMcpClient;
use crate::limits::McpLimits;
use crate::protocol::{McpClientEvent, McpInitializeInfo, McpTool, McpToolCallResult};
use crate::stdio::MinimalStdioMcpClient;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpConnectionContext {
    cwd: PathBuf,
    principal: Option<String>,
    lead_agent: Option<String>,
    lead_session_id: Option<String>,
}

impl McpConnectionContext {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            principal: None,
            lead_agent: None,
            lead_session_id: None,
        }
    }

    pub fn with_trusted_lead(
        mut self,
        principal: impl Into<String>,
        lead_agent: impl Into<String>,
        lead_session_id: impl Into<String>,
    ) -> Self {
        self.principal = Some(principal.into());
        self.lead_agent = Some(lead_agent.into());
        self.lead_session_id = Some(lead_session_id.into());
        self
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn principal(&self) -> Option<&str> {
        self.principal.as_deref()
    }

    pub fn lead_agent(&self) -> Option<&str> {
        self.lead_agent.as_deref()
    }

    pub fn lead_session_id(&self) -> Option<&str> {
        self.lead_session_id.as_deref()
    }
}

pub trait McpClient: Debug + Send {
    fn initialize_info(&self) -> &McpInitializeInfo;
    fn list_tools(&mut self) -> Result<Vec<McpTool>>;
    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<McpToolCallResult>;
    fn call_tool_with_cancellation(
        &mut self,
        name: &str,
        arguments: Value,
        cancellation: &CancellationToken,
    ) -> Result<McpToolCallResult> {
        if cancellation.is_canceled() {
            return Err(crate::error::McpError::Canceled {
                operation: "tools/call".to_string(),
            });
        }
        self.call_tool(name, arguments)
    }
    fn poll_event(&mut self) -> Result<Option<McpClientEvent>>;
    fn close(&mut self) -> Result<()>;
}

pub trait McpClientFactory: Send + Sync {
    fn connect(
        &self,
        document: &McpDocument,
        cwd: &Path,
        limits: McpLimits,
    ) -> Result<Box<dyn McpClient>>;

    fn connect_with_context(
        &self,
        document: &McpDocument,
        context: &McpConnectionContext,
        limits: McpLimits,
    ) -> Result<Box<dyn McpClient>> {
        self.connect(document, context.cwd(), limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_context_carries_trusted_identity_without_model_fields() {
        let context = McpConnectionContext::new("/tmp/project").with_trusted_lead(
            "local-user",
            "default/lead",
            "session_lead",
        );

        assert_eq!(context.cwd(), Path::new("/tmp/project"));
        assert_eq!(context.principal(), Some("local-user"));
        assert_eq!(context.lead_agent(), Some("default/lead"));
        assert_eq!(context.lead_session_id(), Some("session_lead"));
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StdioMcpClientFactory;

impl McpClientFactory for StdioMcpClientFactory {
    fn connect(
        &self,
        document: &McpDocument,
        cwd: &Path,
        limits: McpLimits,
    ) -> Result<Box<dyn McpClient>> {
        Ok(Box::new(MinimalStdioMcpClient::start_with_limits(
            document, cwd, limits,
        )?))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultMcpClientFactory;

impl McpClientFactory for DefaultMcpClientFactory {
    fn connect(
        &self,
        document: &McpDocument,
        cwd: &Path,
        limits: McpLimits,
    ) -> Result<Box<dyn McpClient>> {
        document.validate()?;
        match document.summary().transport.as_deref() {
            Some("stdio") => StdioMcpClientFactory.connect(document, cwd, limits),
            Some("http") => Ok(Box::new(HttpMcpClient::start_with_limits(
                document, limits,
            )?)),
            _ => Err(crate::error::McpError::InvalidConfig {
                message: "MCP transport 无法解析".to_string(),
            }),
        }
    }
}
