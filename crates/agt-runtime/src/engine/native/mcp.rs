use super::*;
use crate::RunRequest;
use crate::engine::check_canceled;
use agt_mcp::{McpClient, McpClientFactory, McpLimits};

pub(super) struct ToolSession {
    clients: Vec<Box<dyn McpClient>>,
    pub(super) targets: BTreeMap<String, (usize, String)>,
    pub(super) schemas: Vec<Value>,
    pub(super) validators: BTreeMap<String, agt_core::tool_schema::ToolSchema>,
}

impl ToolSession {
    pub(super) fn connect(
        agent: &ResolvedAgent,
        request: &RunRequest,
        factory: &dyn McpClientFactory,
        cancellation: &CancellationToken,
    ) -> Result<Self> {
        let mut session = Self {
            clients: Vec::new(),
            targets: BTreeMap::new(),
            schemas: Vec::new(),
            validators: BTreeMap::new(),
        };
        for (server_index, document) in agent.mcps.iter().enumerate() {
            check_canceled(cancellation)?;
            let client = factory
                .connect(document, &request.working_directory, McpLimits::default())
                .map_err(|e| {
                    RuntimeError::Tool(crate::Reason::because("MCP connection failed", e))
                })?;
            session.clients.push(client);
            let tools = session.clients[server_index].list_tools().map_err(|e| {
                RuntimeError::Tool(crate::Reason::because("MCP tools/list failed", e))
            })?;
            for tool in tools {
                let reference = agent
                    .definition
                    .as_ref()
                    .expect("MCPs require an agent definition")
                    .mcps[server_index]
                    .to_string();
                let key = serde_json::to_vec(&(&reference, &tool.name))?;
                let alias = format!("mcp_{}", &crate::fingerprint(&key)[..32]);
                if session.targets.contains_key(&alias) {
                    return Err(RuntimeError::Tool("duplicate MCP tool identity".into()));
                }
                session.schemas.push(json!({"type":"function","function":{
                    "name":alias,"description":format!("{} / {}: {}", document.name, tool.name, tool.description.unwrap_or_default()),
                    "parameters":tool.input_schema
                }}));
                session.targets.insert(alias, (server_index, tool.name));
            }
        }
        if !request.delegates.is_empty() {
            session.schemas.push(json!({"type":"function","function":{
                "name":"delegate", "description":"Assign an explicit task to an allowed agent and receive its final result. The child cannot delegate further.",
                "parameters":{"type":"object","properties":{
                    "agent":{"type":"string","enum":request.delegates},"input":{"type":"string"}
                },"required":["agent","input"],"additionalProperties":false},"strict":true
            }}));
        }
        if session.schemas.len() > 128 {
            return Err(RuntimeError::Tool("more than 128 tools attached".into()));
        }
        Ok(session)
    }

    pub(super) fn compile_schemas(&mut self) -> Result<()> {
        for schema in &self.schemas {
            let name = schema["function"]["name"].as_str().expect("tool name");
            let validator =
                agt_core::tool_schema::ToolSchema::compile(&schema["function"]["parameters"])
                    .map_err(|error| {
                        RuntimeError::Invalid(format!("invalid tool schema {name}: {error}"))
                    })?;
            self.validators.insert(name.to_owned(), validator);
        }
        Ok(())
    }

    pub(super) fn call(
        &mut self,
        alias: &str,
        arguments: Value,
        cancellation: &CancellationToken,
    ) -> Result<String> {
        let (index, name) = &self.targets[alias];
        let result =
            self.clients[*index].call_tool_with_cancellation(name, arguments, cancellation);
        check_canceled(cancellation)?;
        let result = result
            .map_err(|e| RuntimeError::Tool(crate::Reason::because("MCP tools/call failed", e)))?;
        if result.content.iter().any(|block| block.kind != "text") {
            return Err(RuntimeError::Tool(
                "only text and structured MCP results are supported".into(),
            ));
        }
        // MCP _meta is client-only; do not expose it to the model.
        Ok(serde_json::to_string(&json!({
            "is_error":result.is_error,"content":result.content.iter().filter_map(|block| block.text.as_deref()).collect::<Vec<_>>(),
            "structured_content":result.structured_content
        }))?)
    }
}

impl Drop for ToolSession {
    fn drop(&mut self) {
        for client in &mut self.clients {
            let _ = client.close();
        }
    }
}
