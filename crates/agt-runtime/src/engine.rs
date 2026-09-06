use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use agt_core::CancellationToken;
use agt_im::{ImStore, PrincipalId, PrincipalInput, PrincipalKind, SendMessageRequest};
use agt_mcp::{McpClient, McpClientFactory, McpLimits};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::store::database_path;
use crate::{
    ModelClient, ResolvedAgent, Result, RunRecord, RunRequest, RunState, RunStore, RuntimeError,
    fingerprint,
};

pub struct Runtime<'a> {
    home: PathBuf,
    model: &'a mut dyn ModelClient,
    factory: &'a dyn McpClientFactory,
    store: RunStore,
}

impl<'a> Runtime<'a> {
    pub fn open(
        home: &Path,
        model: &'a mut dyn ModelClient,
        factory: &'a dyn McpClientFactory,
    ) -> Result<Self> {
        Ok(Self {
            home: home.to_owned(),
            model,
            factory,
            store: RunStore::open(home)?,
        })
    }

    pub fn runs(&self) -> &RunStore {
        &self.store
    }

    /// A replay returns the stored state, including failed or interrupted runs.
    pub fn run(
        &mut self,
        request: RunRequest,
        cancellation: &CancellationToken,
    ) -> Result<RunRecord> {
        let request = request.normalize()?;
        let (mut record, claimed) = self.store.claim(&request, None)?;
        if !claimed {
            return Ok(record);
        }
        let mut remaining = request.max_steps;
        let result = (|| {
            check_canceled(cancellation)?;
            let agents = std::iter::once(&request.agent)
                .chain(&request.delegates)
                .map(|name| Ok((name.clone(), ResolvedAgent::load(&self.home, name)?)))
                .collect::<Result<BTreeMap<_, _>>>()?;
            self.drive(&mut record, &agents, &mut remaining, cancellation)
        })();
        record.model_steps = request.max_steps - remaining;
        self.finish(record, result)
    }

    fn finish(&self, mut record: RunRecord, result: Result<String>) -> Result<RunRecord> {
        match result {
            Ok(output) => {
                record.state = RunState::Completed;
                record.output = Some(output);
            }
            Err(error) => {
                record.state = if matches!(error, RuntimeError::Canceled) {
                    RunState::Canceled
                } else {
                    RunState::Failed
                };
                record.error = Some(error.to_string());
            }
        }
        self.store.save(&record)?;
        Ok(record)
    }

    fn drive(
        &mut self,
        record: &mut RunRecord,
        agents: &BTreeMap<String, ResolvedAgent>,
        remaining: &mut u32,
        cancellation: &CancellationToken,
    ) -> Result<String> {
        check_canceled(cancellation)?;
        let agent = &agents[&record.request.agent];
        if (!agent.mcps.is_empty() || !record.request.delegates.is_empty())
            && !agent.model.capabilities.tool_call
        {
            return Err(RuntimeError::Configuration(
                "model tool_call capability is disabled",
            ));
        }
        let snapshot = if record.parent_run_id.is_none() {
            agents
                .iter()
                .map(|(name, agent)| (name, &agent.fingerprint))
                .collect::<Vec<_>>()
        } else {
            vec![(&agent.name, &agent.fingerprint)]
        };
        record.config_fingerprint = Some(fingerprint(&serde_json::to_vec(&snapshot)?));
        self.store.save(record)?;
        let mut tools = ToolSession::connect(agent, &record.request, self.factory, cancellation)?;
        // The discovered schemas are also part of the execution snapshot.
        record.config_fingerprint = Some(fingerprint(&serde_json::to_vec(&(
            &record.config_fingerprint,
            &tools.schemas,
        ))?));
        self.store.save(record)?;
        let mut messages = vec![
            json!({"role":"system", "content":agent.system_prompt}),
            json!({"role":"user", "content":record.request.input}),
        ];
        let mut seen_calls = BTreeSet::new();
        let initial_budget = *remaining;
        loop {
            check_canceled(cancellation)?;
            if *remaining == 0 {
                return Err(RuntimeError::Budget);
            }
            if serde_json::to_vec(&(&messages, &tools.schemas))?.len() > 1024 * 1024 {
                return Err(RuntimeError::Model(
                    "context exceeds 1 MiB; reduce input or tool output",
                ));
            }
            *remaining -= 1;
            record.model_steps = initial_budget - *remaining;
            self.store.save(record)?;
            let message = self.model.complete(agent, &messages, &tools.schemas)?;
            check_canceled(cancellation)?;
            let calls = validate_message(&message)?;
            if calls.is_empty() {
                return message["content"]
                    .as_str()
                    .filter(|text| !text.trim().is_empty())
                    .map(str::to_owned)
                    .ok_or(RuntimeError::Model("empty final response"));
            }
            // Validate the entire batch before allowing any side effect.
            for call in &calls {
                if !seen_calls.insert(call.id.clone()) {
                    return Err(RuntimeError::Model("duplicate tool call ID"));
                }
                if call.function.name == "delegate" && !record.request.delegates.is_empty() {
                    let args: Delegation = serde_json::from_str(&call.function.arguments)
                        .map_err(|_| RuntimeError::Tool("invalid delegation arguments"))?;
                    if !record.request.delegates.contains(&args.agent)
                        || args.input.trim().is_empty()
                        || args.input.len() > 256 * 1024
                    {
                        return Err(RuntimeError::Tool(
                            "delegation target or input is not allowed",
                        ));
                    }
                } else if !tools.targets.contains_key(&call.function.name) {
                    return Err(RuntimeError::Tool("unknown or unavailable tool"));
                }
            }
            messages.push(message);
            for call in calls {
                check_canceled(cancellation)?;
                // Leave at least one model step to consume the tool result.
                if *remaining == 0 {
                    return Err(RuntimeError::Budget);
                }
                let output = if call.function.name == "delegate" {
                    let args: Delegation = serde_json::from_str(&call.function.arguments)?;
                    self.delegate(record, args, &call.id, agents, remaining, cancellation)?
                } else {
                    tools.call(
                        &call.function.name,
                        serde_json::from_str(&call.function.arguments)?,
                        cancellation,
                    )?
                };
                messages.push(json!({"role":"tool", "tool_call_id":call.id, "content":output}));
            }
        }
    }

    fn delegate(
        &mut self,
        parent: &RunRecord,
        args: Delegation,
        call_id: &str,
        agents: &BTreeMap<String, ResolvedAgent>,
        remaining: &mut u32,
        cancellation: &CancellationToken,
    ) -> Result<String> {
        check_canceled(cancellation)?;
        if *remaining < 2 {
            return Err(RuntimeError::Budget);
        }
        let request = RunRequest {
            agent: args.agent,
            input: args.input,
            request_key: call_id.to_owned(),
            delegates: Vec::new(),
            max_steps: parent.request.max_steps,
            working_directory: parent.request.working_directory.clone(),
        };
        let im = ImStore::open(database_path(&self.home, "im", "im.db")?)
            .map_err(|_| RuntimeError::Im("cannot open message store"))?;
        let sender = bind_principal(&im, &parent.request.agent)?;
        let receiver = bind_principal(&im, &request.agent)?;
        let conversation = im
            .create_direct(&sender, &receiver)
            .map_err(|_| RuntimeError::Im("cannot create direct conversation"))?;
        let (mut child, claimed) = self.store.claim(&request, Some(&parent.id))?;
        let initial_budget = *remaining;
        let preparation = check_canceled(cancellation).and_then(|()| {
            im.send_message(SendMessageRequest {
                conversation_id: conversation.id.clone(),
                sender_id: sender,
                body: request.input.clone(),
                client_message_id: Some(format!("run:{}:input", child.id)),
                reply_to_id: None,
                metadata: json!({"run_id":child.id,"parent_run_id":parent.id}),
                idempotency_key: None,
            })
            .map_err(|_| RuntimeError::Im("cannot publish delegation input"))
        });
        if claimed {
            let result = match preparation.as_ref() {
                Ok(_) => self.drive(&mut child, agents, remaining, cancellation),
                Err(RuntimeError::Canceled) => Err(RuntimeError::Canceled),
                Err(_) => Err(RuntimeError::Im("cannot publish delegation input")),
            };
            child.model_steps = initial_budget - *remaining;
            child = self.finish(child, result)?;
        }
        let input_message = preparation?;
        match child.state {
            RunState::Running => return Err(RuntimeError::Busy),
            RunState::Canceled => return Err(RuntimeError::Canceled),
            RunState::Failed => {
                return Err(RuntimeError::Tool(
                    "delegated run failed; inspect child run",
                ));
            }
            RunState::Completed => {}
        }
        let output = child
            .output
            .ok_or(RuntimeError::Tool("completed child has no output"))?;
        check_canceled(cancellation)?;
        im.send_message(SendMessageRequest {
            conversation_id: conversation.id,
            sender_id: receiver,
            body: output.clone(),
            client_message_id: Some(format!("run:{}:output", child.id)),
            reply_to_id: Some(input_message.message.id),
            metadata: json!({"run_id":child.id,"parent_run_id":parent.id}),
            idempotency_key: None,
        })
        .map_err(|_| RuntimeError::Im("child completed but reply publication failed"))?;
        Ok(output)
    }
}

fn check_canceled(token: &CancellationToken) -> Result<()> {
    if token.is_canceled() {
        Err(RuntimeError::Canceled)
    } else {
        Ok(())
    }
}

fn bind_principal(im: &ImStore, agent: &str) -> Result<PrincipalId> {
    let id = PrincipalId::new(format!("runtime:{agent}"));
    let metadata = json!({"agent_ref":agent,"binding":"agt-runtime-v1"});
    match im.create_principal(PrincipalInput {
        id: id.clone(),
        kind: PrincipalKind::Agent,
        display_name: agent.to_owned(),
        metadata: metadata.clone(),
    }) {
        Ok(_) => {}
        Err(agt_im::ImError::AlreadyExists(_)) => {
            let principal = im
                .get_principal(&id)
                .map_err(|_| RuntimeError::Im("cannot load participant"))?;
            if principal.kind != PrincipalKind::Agent || principal.metadata != metadata {
                return Err(RuntimeError::Im(
                    "runtime participant identity is already bound differently",
                ));
            }
        }
        Err(_) => return Err(RuntimeError::Im("cannot bind participant")),
    }
    Ok(id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Delegation {
    agent: String,
    input: String,
}

#[derive(Deserialize)]
struct ToolCall {
    id: String,
    function: FunctionCall,
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Deserialize)]
struct FunctionCall {
    name: String,
    arguments: String,
}

fn validate_message(message: &Value) -> Result<Vec<ToolCall>> {
    if message["role"] != "assistant"
        || (!message["content"].is_null() && !message["content"].is_string())
    {
        return Err(RuntimeError::Model("expected a text assistant message"));
    }
    let calls: Vec<ToolCall> = if message["tool_calls"].is_null() {
        Vec::new()
    } else {
        serde_json::from_value(message["tool_calls"].clone())
            .map_err(|_| RuntimeError::Model("invalid tool calls"))?
    };
    if calls.len() > 32 {
        return Err(RuntimeError::Model("too many tool calls in one response"));
    }
    for call in &calls {
        if call.kind != "function" || call.id.trim().is_empty() || call.id.len() > 256 {
            return Err(RuntimeError::Model("invalid function call identity"));
        }
        if !serde_json::from_str::<Value>(&call.function.arguments)
            .map_err(|_| RuntimeError::Model("tool arguments must be JSON"))?
            .is_object()
        {
            return Err(RuntimeError::Model("tool arguments must be an object"));
        }
    }
    Ok(calls)
}

struct ToolSession {
    clients: Vec<Box<dyn McpClient>>,
    targets: BTreeMap<String, (usize, String)>,
    schemas: Vec<Value>,
}

impl ToolSession {
    fn connect(
        agent: &ResolvedAgent,
        request: &RunRequest,
        factory: &dyn McpClientFactory,
        cancellation: &CancellationToken,
    ) -> Result<Self> {
        let mut session = Self {
            clients: Vec::new(),
            targets: BTreeMap::new(),
            schemas: Vec::new(),
        };
        for (server_index, document) in agent.mcps.iter().enumerate() {
            check_canceled(cancellation)?;
            let client = factory
                .connect(document, &request.working_directory, McpLimits::default())
                .map_err(|_| RuntimeError::Tool("MCP connection failed"))?;
            session.clients.push(client);
            let tools = session.clients[server_index]
                .list_tools()
                .map_err(|_| RuntimeError::Tool("MCP tools/list failed"))?;
            for (tool_index, tool) in tools.into_iter().enumerate() {
                let alias = format!("mcp_{server_index}_{tool_index}");
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
            return Err(RuntimeError::Tool("more than 128 tools attached"));
        }
        Ok(session)
    }

    fn call(
        &mut self,
        alias: &str,
        arguments: Value,
        cancellation: &CancellationToken,
    ) -> Result<String> {
        let (index, name) = &self.targets[alias];
        let result =
            self.clients[*index].call_tool_with_cancellation(name, arguments, cancellation);
        check_canceled(cancellation)?;
        let result = result.map_err(|_| RuntimeError::Tool("MCP tools/call failed"))?;
        if result.content.iter().any(|block| block.kind != "text") {
            return Err(RuntimeError::Tool(
                "only text and structured MCP results are supported",
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
