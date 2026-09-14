use std::fmt::Debug;
use std::path::Path;

use agt_core::CancellationToken;
use serde_json::Value;

use crate::config::McpDocument;
use crate::error::Result;
use crate::http::HttpMcpClient;
use crate::limits::McpLimits;
use crate::protocol::{McpClientEvent, McpInitializeInfo, McpTool, McpToolCallResult};
use crate::stdio::MinimalStdioMcpClient;

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
