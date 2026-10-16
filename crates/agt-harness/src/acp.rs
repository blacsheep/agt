//! Agent Client Protocol (ACP v1) engines, such as DeepSeek Harness.
//!
//! One process per turn: `initialize`, then `session/new` or `session/resume`
//! with agt's tools mounted as a loopback HTTP MCP server, then one
//! `session/prompt`. Permission requests go to agt's approval flow.
use crate::mcp_http::Bridge;
use crate::process::Lines;
use crate::{
    ApprovalHandler, ApprovalRequest, DeliveryProgress, Event, Execution, Failure, FailureCode,
    HarnessConfig, ProcessDiagnostics,
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::process::Command;
use std::time::{Duration, Instant};

/// How to start one engine's ACP server.
pub(crate) struct Launch {
    /// Engine name for errors, such as "DeepSeek Harness".
    pub name: String,
    pub args: Vec<String>,
    /// ACP auth method to select after `initialize`, when the agent needs one.
    pub auth_method: Option<String>,
}

impl Launch {
    /// DeepSeek Harness has a fixed launch; a custom `acp` agent brings its own.
    pub(crate) fn for_harness(harness: &HarnessConfig) -> Self {
        if harness.engine == crate::DEEPSEEK_HARNESS {
            Self {
                name: "DeepSeek Harness".into(),
                args: vec!["--profile".into(), "acp".into()],
                auth_method: None,
            }
        } else {
            Self {
                name: "ACP agent".into(),
                args: harness.args.clone(),
                auth_method: harness.auth_method.clone(),
            }
        }
    }
}

pub(crate) struct Acp {
    launch: Launch,
    auth_methods: Vec<String>,
    process: Option<Lines>,
    next_id: u64,
}

struct Pending {
    request: ApprovalRequest,
    host_id: String,
    rpc_id: Value,
    allow: Option<String>,
    reject: Option<String>,
}

impl Acp {
    pub(crate) fn new(launch: Launch) -> Self {
        Self {
            launch,
            auth_methods: Vec::new(),
            process: None,
            next_id: 0,
        }
    }

    pub(crate) fn diagnostics(&mut self) -> ProcessDiagnostics {
        self.process
            .as_mut()
            .map(Lines::diagnostics)
            .unwrap_or_default()
    }

    fn process(&mut self) -> &mut Lines {
        self.process.as_mut().expect("process started")
    }

    fn send_request(&mut self, method: &str, params: Value) -> Result<u64> {
        self.next_id += 1;
        let id = self.next_id;
        self.process()
            .send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        Ok(id)
    }

    /// A request answered before the turn starts; nothing else may arrive meanwhile
    /// except notifications, which are dropped.
    fn call(
        &mut self,
        method: &str,
        params: Value,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Value> {
        let id = self.send_request(method, params)?;
        loop {
            let frame = self.process().receive(event)?;
            if frame["id"] == id && frame.get("method").is_none() {
                if let Some(error) = frame.get("error") {
                    bail!(
                        "{} rejected {method}: {}",
                        self.launch.name,
                        error["message"].as_str().unwrap_or("protocol error")
                    );
                }
                return Ok(frame["result"].clone());
            }
            if frame.get("id").is_some() && frame.get("method").is_some() {
                self.unsupported(&frame)?;
            }
        }
    }

    /// Start the server, `initialize`, and select the configured auth method.
    /// Returns the agent's `initialize` result.
    fn start(
        &mut self,
        harness: &HarnessConfig,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Value> {
        let mut command = Command::new(&harness.executable);
        command
            .args(&self.launch.args)
            .current_dir(&harness.working_directory);
        self.process = Some(Lines::spawn(command, self.launch.name.clone())?);
        let info = self.call(
            "initialize",
            json!({"protocolVersion":1,"clientInfo":{"name":"agt","version":env!("CARGO_PKG_VERSION")},
                "clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false}}),
            event,
        )?;
        self.auth_methods = info["authMethods"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m["id"].as_str().map(str::to_owned))
            .collect();
        if let Some(method) = self.launch.auth_method.clone() {
            if !self.auth_methods.contains(&method) {
                bail!(
                    "{} does not offer auth method {method:?}; available: {}",
                    self.launch.name,
                    self.auth_methods.join(", ")
                );
            }
            self.call("authenticate", json!({"methodId": method}), event)?;
        }
        Ok(info)
    }

    /// An agent that is not signed in refuses new sessions; say how to fix it.
    fn explain_auth(&self, error: anyhow::Error) -> anyhow::Error {
        if !error.to_string().contains("Authentication required") {
            return error;
        }
        anyhow::anyhow!(
            "{} needs authentication: sign in with the agent's own tool, or set auth_method to one of: {}",
            self.launch.name,
            if self.auth_methods.is_empty() {
                "(none offered)".to_owned()
            } else {
                self.auth_methods.join(", ")
            }
        )
    }

    fn unsupported(&mut self, frame: &Value) -> Result<()> {
        self.process()
            .send(&json!({"jsonrpc":"2.0","id":frame["id"],
            "error":{"code":-32601,"message":"AGT does not provide this client method"}}))
    }

    pub(crate) fn execute(
        &mut self,
        harness: &HarnessConfig,
        execution: Execution<'_>,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
        tool: &mut dyn FnMut(&str, Value) -> Result<Value>,
        mut approvals: Option<&mut dyn ApprovalHandler>,
    ) -> Result<String> {
        harness.validate()?;
        let validators = crate::tools::validators(execution.tools)?;
        let bridge = Bridge::start(
            crate::tools::descriptors(execution.tools),
            Duration::from_secs(harness.timeout_seconds),
        )?;
        let mut turn = Turn::default();
        let result = (|| -> Result<String> {
            self.start(harness, event)?;
            let cwd = harness.working_directory.to_string_lossy().to_string();
            let servers = json!([{"type":"http","name":"agt","url":bridge.url,
                "headers":[{"name":"authorization","value":format!("Bearer {}", bridge.token)}]}]);
            let session = match execution.thread {
                Some(id) => {
                    let response = self.call(
                        "session/resume",
                        json!({"sessionId":id,"cwd":cwd,"mcpServers":servers}),
                        event,
                    )?;
                    turn.options = response["configOptions"].clone();
                    id.to_owned()
                }
                None => {
                    let response = self
                        .call(
                            "session/new",
                            json!({"cwd":cwd,"mcpServers":servers}),
                            event,
                        )
                        .map_err(|e| self.explain_auth(e))?;
                    turn.options = response["configOptions"].clone();
                    response["sessionId"]
                        .as_str()
                        .with_context(|| format!("{} omitted the session id", self.launch.name))?
                        .to_owned()
                }
            };
            turn.session = session.clone();
            event(Event::Thread(&session))?;
            self.configure(harness, &mut turn, event)?;
            let prompt = self.send_request(
                "session/prompt",
                json!({"sessionId":session,"prompt":[{"type":"text","text":execution.input}]}),
            )?;
            turn.prompt = prompt;
            turn.submitted = true;
            event(Event::Submitted)?;
            event(Event::Turn(&session))?;
            event(Event::Accepted)?;
            loop {
                let Some(frame) = self.process().poll(event)? else {
                    self.tools(&bridge, &validators, event, tool)?;
                    if let Some(handler) = approvals.as_deref_mut() {
                        self.flush(&mut turn, handler, event)?;
                    }
                    continue;
                };
                if let Some(output) =
                    self.frame(&frame, &mut turn, event, approvals.as_deref_mut())?
                {
                    return Ok(output);
                }
                self.tools(&bridge, &validators, event, tool)?;
                if let Some(handler) = approvals.as_deref_mut() {
                    self.flush(&mut turn, handler, event)?;
                }
            }
        })();
        if let Err(error) = &result {
            // Diagnostic persistence must not prevent protocol cleanup. Keep the original error.
            let _ = event(Event::Stopping(error));
            if turn.submitted && !turn.terminal {
                self.cancel(&turn, event);
            }
        }
        // Closing makes the session resumable by the next process.
        if !turn.session.is_empty() {
            let session = turn.session.clone();
            let _ = self.send_request("session/close", json!({"sessionId":session}));
        }
        if let Some(process) = self.process.as_mut() {
            process.deadline = Instant::now() + Duration::from_secs(2);
            let _ = process.receive(&mut |_| Ok(()));
            process.close_input();
        }
        result
    }

    /// Apply the harness model and reasoning level through the session's options.
    fn configure(
        &mut self,
        harness: &HarnessConfig,
        turn: &mut Turn,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<()> {
        for (id, wanted) in [
            ("model", harness.model.as_deref()),
            ("reasoning_effort", harness.reasoning_effort.as_deref()),
        ] {
            let Some(option) = turn
                .options
                .as_array()
                .and_then(|o| o.iter().find(|o| o["id"] == id))
                .cloned()
            else {
                continue;
            };
            let choices = choices(&option["options"]);
            if let Some(wanted) = wanted {
                let (value, _) = choices
                    .iter()
                    .find(|(value, name)| {
                        value == wanted || name == wanted || inner(value) == wanted
                    })
                    .with_context(|| {
                        format!(
                            "{} does not offer {id} {wanted:?}; available: {}",
                            self.launch.name,
                            choices
                                .iter()
                                .map(|(_, n)| n.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })?
                    .clone();
                let session = turn.session.clone();
                let response = self.call(
                    "session/set_config_option",
                    json!({"sessionId":session,"configId":id,"value":value}),
                    event,
                )?;
                if let Some(options) = response.get("configOptions") {
                    turn.options = options.clone();
                }
            }
            let current = turn
                .options
                .as_array()
                .and_then(|o| o.iter().find(|o| o["id"] == id))
                .and_then(|o| o["currentValue"].as_str())
                .map(str::to_owned);
            if let Some(current) = current {
                let name = choices
                    .iter()
                    .find(|(value, _)| *value == current)
                    .map(|(_, name)| name.clone())
                    .unwrap_or(current);
                if id == "model" {
                    event(Event::Model(&name))?;
                } else {
                    event(Event::ReasoningEffort(&name))?;
                }
            }
        }
        Ok(())
    }

    /// Run agt tool calls the bridge has received.
    fn tools(
        &mut self,
        bridge: &Bridge,
        validators: &crate::tools::Validators,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
        tool: &mut dyn FnMut(&str, Value) -> Result<Value>,
    ) -> Result<()> {
        while let Ok(call) = bridge.calls.try_recv() {
            self.next_id += 1;
            let id = format!("agt-{}", self.next_id);
            let result =
                crate::tools::call(&id, &call.name, call.arguments, validators, event, tool)?;
            let _ = call.reply.send(result);
        }
        Ok(())
    }

    fn frame<'h>(
        &mut self,
        frame: &Value,
        turn: &mut Turn,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
        approvals: Option<&mut (dyn ApprovalHandler + 'h)>,
    ) -> Result<Option<String>> {
        if frame["id"] == turn.prompt && frame.get("method").is_none() {
            turn.terminal = true;
            if let Some(error) = frame.get("error") {
                event(Event::Settled("failed"))?;
                return Err(Failure::new(
                    FailureCode::EngineFailed,
                    format!(
                        "{} turn failed: {}",
                        self.launch.name,
                        error["message"].as_str().unwrap_or("protocol error")
                    ),
                )
                .into());
            }
            return match frame["result"]["stopReason"].as_str() {
                Some("end_turn") => {
                    event(Event::Settled("completed"))?;
                    event(Event::Completed)?;
                    if turn.text.trim().is_empty() {
                        return Err(Failure::new(
                            FailureCode::ResultMissing,
                            format!(
                                "{} completed without a final response; task was not replayed",
                                self.launch.name
                            ),
                        )
                        .into());
                    }
                    Ok(Some(std::mem::take(&mut turn.text)))
                }
                Some("cancelled") => {
                    event(Event::Settled("interrupted"))?;
                    Err(Failure::new(
                        FailureCode::Interrupted,
                        format!("{} turn canceled", self.launch.name),
                    )
                    .into())
                }
                reason => {
                    event(Event::Settled("failed"))?;
                    Err(Failure::new(
                        FailureCode::EngineFailed,
                        format!(
                            "{} turn stopped: {}",
                            self.launch.name,
                            reason.unwrap_or("unknown")
                        ),
                    )
                    .into())
                }
            };
        }
        match frame["method"].as_str() {
            Some("session/update") if frame["params"]["sessionId"] == turn.session.as_str() => {
                let update = &frame["params"]["update"];
                match update["sessionUpdate"].as_str() {
                    Some("agent_message_chunk") => {
                        if let Some(text) = update["content"]["text"].as_str() {
                            turn.text.push_str(text);
                            event(Event::Text(text))?;
                        }
                    }
                    Some("tool_call" | "tool_call_update") => {
                        let id = update["toolCallId"].as_str().unwrap_or("unknown");
                        let title = update["title"].as_str().unwrap_or("");
                        // agt's own tools (framework_*) are reported by the bridge itself.
                        if !title.contains("framework_") {
                            event(Event::ToolDetail { id, item: update })?;
                            event(Event::Activity {
                                tool_name: (!title.is_empty()).then_some(title),
                                id,
                                kind: kind(update["kind"].as_str()),
                                status: match update["status"].as_str() {
                                    Some("completed") => "completed",
                                    Some("failed") => "failed",
                                    _ => "running",
                                },
                            })?;
                        }
                    }
                    _ => {}
                }
            }
            Some("session/request_permission") => self.permission(frame, turn, approvals)?,
            Some(_) if frame.get("id").is_some() => self.unsupported(frame)?,
            _ => {}
        }
        Ok(None)
    }

    fn permission<'h>(
        &mut self,
        frame: &Value,
        turn: &mut Turn,
        approvals: Option<&mut (dyn ApprovalHandler + 'h)>,
    ) -> Result<()> {
        let params = &frame["params"];
        let options = params["options"].as_array().cloned().unwrap_or_default();
        let find = |kind: &str| {
            options
                .iter()
                .find(|o| o["kind"] == kind)
                .and_then(|o| o["optionId"].as_str())
                .map(str::to_owned)
        };
        let (allow, reject) = (find("allow_once"), find("reject_once"));
        let Some(handler) = approvals else {
            return self.answer(&frame["id"], reject.as_deref());
        };
        let mut offered = Vec::new();
        if allow.is_some() {
            offered.push(json!({"value":"accept","label":"批准本次"}));
        }
        if reject.is_some() {
            offered.push(json!({"value":"decline","label":"拒绝本次"}));
        }
        offered.push(json!({"value":"cancel","label":"取消执行"}));
        let request = ApprovalRequest {
            rpc_id: frame["id"].clone(),
            method: "acp/session/request_permission".into(),
            params: json!({"tool_call": params["toolCall"]}),
            options: offered,
            item_context: None,
        };
        let host_id = handler.request(&request)?;
        turn.pending.push(Pending {
            rpc_id: frame["id"].clone(),
            request,
            host_id,
            allow,
            reject,
        });
        Ok(())
    }

    /// Select an option, or report the request canceled when there is none.
    fn answer(&mut self, id: &Value, option: Option<&str>) -> Result<()> {
        let outcome = match option {
            Some(option) => json!({"outcome":"selected","optionId":option}),
            None => json!({"outcome":"cancelled"}),
        };
        self.process()
            .send(&json!({"jsonrpc":"2.0","id":id,"result":{"outcome":outcome}}))
    }

    fn flush(
        &mut self,
        turn: &mut Turn,
        handler: &mut dyn ApprovalHandler,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<()> {
        let mut index = 0;
        while index < turn.pending.len() {
            let Some(decision) = handler.poll(&turn.pending[index].host_id)? else {
                index += 1;
                continue;
            };
            event(Event::Tick)?;
            let entry = turn.pending.remove(index);
            if !entry.request.options.iter().any(|o| o["value"] == decision) {
                bail!("approval decision not offered");
            }
            let option = match decision.as_str() {
                Some("accept") => entry.allow.clone(),
                Some("decline") => entry.reject.clone(),
                _ => None,
            };
            handler.delivery(&entry.host_id, DeliveryProgress::Sending)?;
            self.answer(&entry.rpc_id, option.as_deref())?;
            handler.delivery(&entry.host_id, DeliveryProgress::Sent)?;
            if decision == "cancel" {
                return Err(
                    Failure::new(FailureCode::Interrupted, "permission request canceled").into(),
                );
            }
        }
        Ok(())
    }

    /// Ask the engine to stop the turn and wait briefly for it to confirm.
    fn cancel(&mut self, turn: &Turn, event: &mut dyn FnMut(Event<'_>) -> Result<()>) {
        if event(Event::InterruptRequested).is_err() {
            return;
        }
        let session = turn.session.clone();
        if self
            .process()
            .send(
                &json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}}),
            )
            .is_err()
        {
            return;
        }
        self.process().deadline = Instant::now() + Duration::from_secs(2);
        while let Ok(frame) = self.process().receive(&mut |_| Ok(())) {
            if frame["id"] == turn.prompt && frame.get("method").is_none() {
                let status = if frame["result"]["stopReason"] == "end_turn" {
                    "completed"
                } else {
                    "interrupted"
                };
                let _ = event(Event::Settled(status));
                return;
            }
        }
    }
}

#[derive(Default)]
struct Turn {
    session: String,
    options: Value,
    prompt: u64,
    text: String,
    pending: Vec<Pending>,
    submitted: bool,
    terminal: bool,
}

/// `(value, name)` pairs of a select option, flattening groups.
fn choices(options: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for option in options.as_array().into_iter().flatten() {
        if let Some(group) = option.get("options") {
            out.extend(choices(group));
        } else if let Some(value) = option["value"].as_str() {
            out.push((
                value.to_owned(),
                option["name"].as_str().unwrap_or(value).to_owned(),
            ));
        }
    }
    out
}

/// DeepSeek Harness encodes a model choice as `["provider","model"]`.
fn inner(value: &str) -> String {
    serde_json::from_str::<Vec<String>>(value)
        .ok()
        .and_then(|pair| pair.last().cloned())
        .unwrap_or_default()
}

fn kind(kind: Option<&str>) -> &'static str {
    match kind {
        Some("execute") => "command",
        Some("edit" | "delete" | "move") => "files",
        _ => "tool",
    }
}

pub(crate) fn test(harness: &HarnessConfig) -> Result<Value> {
    harness.validate()?;
    let mut engine = Acp::new(Launch::for_harness(harness));
    let info = engine.start(harness, &mut |_| Ok(()))?;
    if info["agentCapabilities"]["mcpCapabilities"]["http"] != true {
        bail!(
            "{} does not accept HTTP MCP servers, which agt needs for its tools",
            engine.launch.name
        );
    }
    Ok(
        json!({"status":"connected","engine":harness.engine,"agent":info["agentInfo"],
        "auth_methods":engine.auth_methods,"inference":false}),
    )
}

pub(crate) fn capabilities(engine: &str) -> Value {
    json!({"engine":engine,"operations":["status","interactive_approval"],
        "approval_requests":["session/request_permission"],
        "unsupported":["models","rename","compact","skills","mcp","review","diff","fork","import"]})
}
