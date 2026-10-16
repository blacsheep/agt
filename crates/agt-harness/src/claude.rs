//! Claude Code in headless mode: one process per turn, speaking stream-json.
//!
//! agt's framework tools are served to Claude Code as an in-process ("sdk") MCP
//! server named `agt`, and Claude Code's permission prompts come to agt as
//! `can_use_tool` control requests, so approvals use agt's approval flow.
use crate::process::Lines;
use crate::{
    ApprovalHandler, ApprovalRequest, DeliveryProgress, Event, Execution, Failure, FailureCode,
    HarnessConfig, ProcessDiagnostics, Sandbox,
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::process::Command;
use std::time::{Duration, Instant};

const SERVER: &str = "agt";
/// Claude Code names tools of an MCP server `mcp__<server>__<tool>`.
const PREFIX: &str = "mcp__agt__";

pub(crate) struct ClaudeCode {
    process: Option<Lines>,
    next_id: u64,
}

struct Pending {
    request: ApprovalRequest,
    host_id: String,
    input: Value,
}

impl ClaudeCode {
    pub(crate) fn new() -> Self {
        Self {
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

    fn command(harness: &HarnessConfig, execution: &Execution<'_>, prompts: bool) -> Command {
        let mut command = Command::new(&harness.executable);
        command
            .current_dir(&harness.working_directory)
            .args(["-p", "--verbose", "--include-partial-messages"])
            .args([
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
            ])
            .arg("--mcp-config")
            .arg(json!({"mcpServers":{SERVER:{"type":"sdk","name":SERVER}}}).to_string())
            // agt authorizes its own tools; Claude Code need not ask about them.
            .arg("--allowed-tools")
            .arg(format!("{PREFIX}*"))
            .args([
                "--permission-prompts",
                if prompts { "host" } else { "none" },
            ])
            .arg("--append-system-prompt")
            .arg(execution.instructions);
        match harness.sandbox {
            // File edits inside the project are accepted; everything else is asked.
            Sandbox::WorkspaceWrite => command.args(["--permission-mode", "acceptEdits"]),
            Sandbox::ReadOnly => command
                .args(["--permission-mode", "manual"])
                .args(["--disallowed-tools", "Edit Write NotebookEdit MultiEdit"]),
        };
        if let Some(model) = &harness.model {
            command.args(["--model", model]);
        }
        if let Some(effort) = &harness.reasoning_effort {
            command.args(["--effort", effort]);
        }
        if let Some(session) = execution.thread {
            command.args(["--resume", session]);
        }
        command
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
        let validators = execution
            .tools
            .iter()
            .map(|tool| {
                agt_core::tool_schema::ToolSchema::compile(&tool.input_schema)
                    .map(|schema| (tool.name.clone(), schema))
            })
            .collect::<std::result::Result<BTreeMap<_, _>, _>>()?;
        let tools: Vec<Value> = execution
            .tools
            .iter()
            .map(
                |t| json!({"name":t.name,"description":t.description,"inputSchema":t.input_schema}),
            )
            .collect();
        let process = Lines::spawn(
            Self::command(harness, &execution, approvals.is_some()),
            "Claude Code",
        )?;
        self.process = Some(process);
        let init =
            self.request(json!({"subtype":"initialize","hooks":null,"sdkMcpServers":[SERVER]}))?;
        let mut state = Turn {
            tools: &tools,
            validators: &validators,
            pending: Vec::new(),
            initialized: false,
            init,
            session: None,
            text: String::new(),
            terminal: false,
            submitted: false,
        };
        let result = (|| -> Result<String> {
            // Serve MCP set-up until Claude Code acknowledges the handshake.
            while !state.initialized {
                let frame = self.process().receive(event)?;
                self.frame(&frame, &mut state, event, tool, approvals.as_deref_mut())?;
            }
            self.process().deadline = Instant::now() + Duration::from_secs(harness.timeout_seconds);
            self.process().send(
                &json!({"type":"user","session_id":"","parent_tool_use_id":null,
                "message":{"role":"user","content":[{"type":"text","text":execution.input}]}}),
            )?;
            state.submitted = true;
            event(Event::Submitted)?;
            loop {
                let Some(frame) = self.process().poll(event)? else {
                    if let Some(handler) = approvals.as_deref_mut() {
                        self.flush(&mut state, handler, event)?;
                    }
                    continue;
                };
                if let Some(output) =
                    self.frame(&frame, &mut state, event, tool, approvals.as_deref_mut())?
                {
                    return Ok(output);
                }
                if let Some(handler) = approvals.as_deref_mut() {
                    self.flush(&mut state, handler, event)?;
                }
            }
        })();
        if let Err(error) = &result {
            // Diagnostic persistence must not prevent protocol cleanup. Keep the original error.
            let _ = event(Event::Stopping(error));
            if state.submitted && !state.terminal {
                self.interrupt(event);
            }
        }
        if let Some(process) = self.process.as_mut() {
            process.close_input();
        }
        result
    }

    fn process(&mut self) -> &mut Lines {
        self.process.as_mut().expect("process started")
    }

    /// Send a control request and return its ID; its response arrives as a frame.
    fn request(&mut self, request: Value) -> Result<String> {
        self.next_id += 1;
        let id = format!("agt-{}", self.next_id);
        self.process()
            .send(&json!({"type":"control_request","request_id":id,"request":request}))?;
        Ok(id)
    }

    fn respond(&mut self, id: &Value, response: Value) -> Result<()> {
        self.process().send(&json!({"type":"control_response",
            "response":{"subtype":"success","request_id":id,"response":response}}))
    }

    /// Handle one frame. Returns the final text when the turn completes.
    fn frame<'h>(
        &mut self,
        frame: &Value,
        state: &mut Turn<'_>,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
        tool: &mut dyn FnMut(&str, Value) -> Result<Value>,
        approvals: Option<&mut (dyn ApprovalHandler + 'h)>,
    ) -> Result<Option<String>> {
        match frame["type"].as_str() {
            Some("control_response") => {
                let response = &frame["response"];
                if response["request_id"] == state.init.as_str() {
                    if response["subtype"] != "success" {
                        bail!(
                            "Claude Code rejected initialization: {}",
                            response["error"].as_str().unwrap_or("protocol error")
                        );
                    }
                    state.initialized = true;
                }
            }
            Some("control_request") => {
                let id = &frame["request_id"];
                let request = &frame["request"];
                match request["subtype"].as_str() {
                    Some("mcp_message") if request["server_name"] == SERVER => {
                        let reply = self.mcp(&request["message"], state, event, tool)?;
                        self.respond(id, json!({"mcp_response": reply}))?;
                    }
                    Some("can_use_tool") => self.permission(frame, state, approvals)?,
                    _ => {
                        self.process().send(&json!({"type":"control_response",
                            "response":{"subtype":"error","request_id":id,
                                "error":"AGT does not support this request"}}))?;
                    }
                }
            }
            Some("system") if frame["subtype"] == "init" => {
                if let Some(session) = frame["session_id"].as_str() {
                    state.session = Some(session.to_owned());
                    event(Event::Thread(session))?;
                    event(Event::Turn(session))?;
                }
                event(Event::Accepted)?;
                if let Some(model) = frame["model"].as_str() {
                    event(Event::Model(model))?;
                }
            }
            Some("stream_event") => {
                let e = &frame["event"];
                if e["type"] == "content_block_delta" {
                    match e["delta"]["type"].as_str() {
                        Some("text_delta") => {
                            event(Event::Text(e["delta"]["text"].as_str().unwrap_or("")))?
                        }
                        Some("input_json_delta") => event(Event::Tool)?,
                        _ => {}
                    }
                }
            }
            Some("assistant") => {
                for block in frame["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_use" {
                        let name = block["name"].as_str().unwrap_or("");
                        let id = block["id"].as_str().unwrap_or("unknown");
                        if !name.starts_with(PREFIX) {
                            event(Event::ToolDetail { id, item: block })?;
                            event(Event::Activity {
                                tool_name: Some(name),
                                id,
                                kind: kind(name),
                                status: "running",
                            })?;
                        }
                    } else if block["type"] == "text" {
                        state.text = block["text"].as_str().unwrap_or("").to_owned();
                    }
                }
            }
            Some("user") => {
                for block in frame["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_result"
                        && let Some(id) = block["tool_use_id"].as_str()
                        && !id.is_empty()
                    {
                        event(Event::Activity {
                            tool_name: None,
                            id,
                            kind: "tool",
                            status: if block["is_error"] == true {
                                "failed"
                            } else {
                                "completed"
                            },
                        })?;
                    }
                }
            }
            Some("result") => {
                state.terminal = true;
                if frame["subtype"] == "success" && frame["is_error"] != true {
                    event(Event::Settled("completed"))?;
                    event(Event::Completed)?;
                    let output = frame["result"].as_str().unwrap_or(&state.text).to_owned();
                    if output.trim().is_empty() {
                        return Err(Failure::new(
                            FailureCode::ResultMissing,
                            "Claude Code completed without a final response; task was not replayed",
                        )
                        .into());
                    }
                    return Ok(Some(output));
                }
                event(Event::Settled("failed"))?;
                return Err(Failure::new(
                    FailureCode::EngineFailed,
                    format!(
                        "Claude Code turn failed: {}",
                        frame["subtype"].as_str().unwrap_or("error")
                    ),
                )
                .into());
            }
            _ => {}
        }
        Ok(None)
    }

    /// The in-process MCP server that exposes agt's framework tools.
    fn mcp(
        &mut self,
        message: &Value,
        state: &mut Turn<'_>,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
        tool: &mut dyn FnMut(&str, Value) -> Result<Value>,
    ) -> Result<Value> {
        let id = message.get("id").cloned().unwrap_or(json!(0));
        let result = match message["method"].as_str() {
            Some("initialize") => json!({
                "protocolVersion": message["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "agt", "version": env!("CARGO_PKG_VERSION")},
            }),
            Some("tools/list") => json!({"tools": state.tools}),
            Some("tools/call") => {
                let name = message["params"]["name"].as_str().unwrap_or("");
                let arguments = message["params"]["arguments"].clone();
                let call = format!("agt-{}", uuid_like(&id));
                event(Event::ToolDetail {
                    id: &call,
                    item: &json!({"arguments": arguments}),
                })?;
                event(Event::Activity {
                    tool_name: None,
                    id: &call,
                    kind: "framework",
                    status: "running",
                })?;
                let outcome = match state.validators.get(name) {
                    Some(validator) => validator
                        .validate(&arguments)
                        .map_err(anyhow::Error::from)
                        .and_then(|()| tool(name, arguments)),
                    None => Err(anyhow::anyhow!("tool unavailable")),
                };
                let success = outcome.is_ok();
                let payload = match &outcome {
                    Ok(value) => json!({"result": value}),
                    Err(error) => json!({"error": error.to_string()}),
                };
                event(Event::ToolDetail {
                    id: &call,
                    item: &payload,
                })?;
                event(Event::Activity {
                    tool_name: None,
                    id: &call,
                    kind: "framework",
                    status: if success { "completed" } else { "failed" },
                })?;
                let text = match outcome {
                    Ok(value) => value.to_string(),
                    Err(error) => error.to_string(),
                };
                json!({"content":[{"type":"text","text":text}],"isError":!success})
            }
            // Notifications need no result; the control request still needs a reply.
            _ if message.get("id").is_none() => json!({}),
            _ => {
                return Ok(json!({"jsonrpc":"2.0","id":id,
                    "error":{"code":-32601,"message":"method not found"}}));
            }
        };
        Ok(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }

    fn permission<'h>(
        &mut self,
        frame: &Value,
        state: &mut Turn<'_>,
        approvals: Option<&mut (dyn ApprovalHandler + 'h)>,
    ) -> Result<()> {
        let id = frame["request_id"].clone();
        let request = &frame["request"];
        let input = request["input"].clone();
        if request["tool_name"]
            .as_str()
            .is_some_and(|name| name.starts_with(PREFIX))
        {
            return self.respond(&id, json!({"behavior":"allow","updatedInput":input}));
        }
        let Some(handler) = approvals else {
            return self.respond(
                &id,
                json!({"behavior":"deny","message":"Approvals are not available for this run"}),
            );
        };
        let approval = ApprovalRequest {
            rpc_id: id,
            method: "claude/can_use_tool".into(),
            params: json!({
                "tool_name": request["tool_name"],
                "input": input,
                "title": request["title"],
                "description": request["description"],
                "blocked_path": request["blocked_path"],
                "decision_reason": request["decision_reason"],
            }),
            options: [
                ("accept", "批准本次"),
                ("decline", "拒绝本次"),
                ("cancel", "取消执行"),
            ]
            .into_iter()
            .map(|(value, label)| json!({"value":value,"label":label}))
            .collect(),
            item_context: None,
        };
        let host_id = handler.request(&approval)?;
        state.pending.push(Pending {
            request: approval,
            host_id,
            input,
        });
        Ok(())
    }

    /// Answer permission prompts that agt has decided.
    fn flush(
        &mut self,
        state: &mut Turn<'_>,
        handler: &mut dyn ApprovalHandler,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<()> {
        let mut index = 0;
        while index < state.pending.len() {
            let entry = &state.pending[index];
            let Some(decision) = handler.poll(&entry.host_id)? else {
                index += 1;
                continue;
            };
            event(Event::Tick)?;
            let entry = state.pending.remove(index);
            let response = match decision.as_str() {
                Some("accept") => json!({"behavior":"allow","updatedInput":entry.input}),
                Some("decline") => json!({"behavior":"deny","message":"Denied by AGT approval"}),
                Some("cancel") => {
                    json!({"behavior":"deny","message":"Canceled by AGT approval","interrupt":true})
                }
                _ => bail!("approval decision not offered"),
            };
            handler.delivery(&entry.host_id, DeliveryProgress::Sending)?;
            self.respond(&entry.request.rpc_id, response)?;
            handler.delivery(&entry.host_id, DeliveryProgress::Sent)?;
            if decision == "cancel" {
                return Err(
                    Failure::new(FailureCode::Interrupted, "permission request canceled").into(),
                );
            }
        }
        Ok(())
    }

    /// Ask Claude Code to stop and wait briefly for it to confirm.
    fn interrupt(&mut self, event: &mut dyn FnMut(Event<'_>) -> Result<()>) {
        if event(Event::InterruptRequested).is_err() {
            return;
        }
        let Ok(_) = self.request(json!({"subtype":"interrupt"})) else {
            return;
        };
        self.process().deadline = Instant::now() + Duration::from_secs(2);
        while let Ok(frame) = self.process().receive(&mut |_| Ok(())) {
            if frame["type"] == "result" {
                let _ = event(Event::Settled("interrupted"));
                return;
            }
        }
    }
}

struct Turn<'a> {
    tools: &'a [Value],
    validators: &'a BTreeMap<String, agt_core::tool_schema::ToolSchema>,
    pending: Vec<Pending>,
    initialized: bool,
    init: String,
    session: Option<String>,
    /// The latest assistant text, in case the result omits it.
    text: String,
    terminal: bool,
    submitted: bool,
}

/// Activity kind for Claude Code's built-in tools.
fn kind(name: &str) -> &'static str {
    match name {
        "Bash" | "BashOutput" | "KillShell" | "PowerShell" => "command",
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => "files",
        name if name.starts_with("mcp__") => "mcp",
        _ => "tool",
    }
}

fn uuid_like(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Operations without a turn.
pub(crate) fn test(harness: &HarnessConfig) -> Result<Value> {
    harness.validate()?;
    let output = Command::new(&harness.executable)
        .arg("--version")
        .current_dir(&harness.working_directory)
        .output()
        .context("cannot start Claude Code")?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() || !version.contains("Claude Code") {
        bail!("executable did not report a Claude Code version");
    }
    Ok(json!({"status":"connected","engine":"claude-code","version":version,"inference":false}))
}

pub(crate) fn capabilities() -> Value {
    json!({"engine":"claude-code","operations":["models","configure","status","interactive_approval"],
        "approval_requests":["can_use_tool"],"unsupported":["rename","compact","skills","mcp","review","diff","fork","import"]})
}

/// Claude Code accepts model aliases or full names; these are the aliases it documents.
pub(crate) fn models() -> Value {
    let efforts: Vec<Value> = ["low", "medium", "high", "xhigh", "max"]
        .iter()
        .map(|e| json!({"reasoningEffort": e}))
        .collect();
    let data: Vec<Value> = [
        ("fable", "Fable"),
        ("opus", "Opus"),
        ("sonnet", "Sonnet"),
        ("haiku", "Haiku"),
    ]
    .iter()
    .map(|(model, name)| {
        json!({"model":model,"displayName":name,
            "defaultReasoningEffort":"high","supportedReasoningEfforts":efforts})
    })
    .collect();
    json!({"data": data})
}
