use crate::diagnostics::StderrTail;
use crate::{Failure, FailureCode, HarnessConfig, ProcessDiagnostics};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}
pub struct Execution<'a> {
    pub input: &'a str,
    pub instructions: &'a str,
    pub thread: Option<&'a str>,
    pub tools: &'a [Tool],
}
pub enum Event<'a> {
    Tick,
    Stopping(&'a anyhow::Error),
    InterruptRequested,
    Protocol,
    Submitted,
    Accepted,
    Settled(&'static str),
    Activity {
        tool_name: Option<&'a str>,
        id: &'a str,
        kind: &'static str,
        status: &'static str,
    },
    Completed,
    Text(&'a str),
    Tool,
    Thread(&'a str),
    Turn(&'a str),
    Model(&'a str),
    ReasoningEffort(&'a str),
}
/// A single local app-server connection. stdout is protocol-only; stderr is drained, not exposed.
pub struct Codex {
    child: Child,
    input: ChildStdin,
    messages: Receiver<Result<Value>>,
    next_id: u64,
    deadline: Instant,
    stderr: StderrTail,
}
impl Codex {
    pub fn diagnostics(&mut self) -> ProcessDiagnostics {
        let status = self.child.try_wait().ok().flatten();
        #[cfg(unix)]
        let signal = {
            use std::os::unix::process::ExitStatusExt;
            status.as_ref().and_then(|s| s.signal())
        };
        #[cfg(not(unix))]
        let signal = None;
        ProcessDiagnostics {
            exit_code: status.and_then(|s| s.code()),
            signal,
            stderr_tail: self.stderr.redacted(),
        }
    }

    pub fn connect(
        harness: &HarnessConfig,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Self> {
        harness.validate()?;
        let mut command = Command::new(&harness.executable);
        command
            .arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .current_dir(&harness.working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().context("cannot start Codex app-server")?;
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let stderr_tail = StderrTail::default();
        let tail = stderr_tail.clone();
        std::thread::spawn(move || {
            let mut bytes = [0; 2048];
            while let Ok(n) = stderr.read(&mut bytes) {
                if n == 0 {
                    break;
                }
                tail.append(&bytes[..n]);
            }
        });
        let (sender, messages) = mpsc::sync_channel(64);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                let result = match reader
                    .by_ref()
                    .take(4 * 1024 * 1024 + 1)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) => break,
                    Ok(_) if line.len() > 4 * 1024 * 1024 => {
                        Err(anyhow::anyhow!("Codex protocol frame exceeds 4 MiB"))
                    }
                    Ok(_) => serde_json::from_slice(&line).context("invalid Codex protocol frame"),
                    Err(e) => Err(e.into()),
                };
                let failed = result.is_err();
                if sender.send(result).is_err() || failed {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            input,
            messages,
            next_id: 0,
            stderr: stderr_tail,
            deadline: Instant::now() + Duration::from_secs(15),
        };
        client.request("initialize", json!({"clientInfo":{"name":"agt","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}), event)?;
        client.send(json!({"method":"initialized","params":{}}))?;
        client.deadline = Instant::now() + Duration::from_secs(harness.timeout_seconds);
        Ok(client)
    }
    fn send(&mut self, value: Value) -> Result<()> {
        serde_json::to_writer(&mut self.input, &value)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        Ok(())
    }
    fn receive(&mut self, event: &mut dyn FnMut(Event<'_>) -> Result<()>) -> Result<Value> {
        loop {
            event(Event::Tick)?;
            if Instant::now() >= self.deadline {
                return Err(Failure::new(FailureCode::Timeout, "Codex execution timed out").into());
            }
            match self.messages.recv_timeout(Duration::from_millis(50)) {
                Ok(value) => {
                    if value.is_ok() {
                        event(Event::Protocol)?;
                    }
                    return value;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => {
                    return Err(Failure::new(
                        FailureCode::Disconnected,
                        "Codex app-server disconnected",
                    )
                    .into());
                }
            }
        }
    }
    fn request(
        &mut self,
        method: &str,
        params: Value,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        loop {
            let value = self.receive(event)?;
            if value["id"] == id && value.get("method").is_none() {
                if value.get("error").is_some() {
                    bail!(
                        "Codex {method} rejected: {}",
                        value["error"]["message"]
                            .as_str()
                            .unwrap_or("protocol error")
                    );
                }
                return Ok(value["result"].clone());
            }
            if value.get("id").is_some() {
                bail!("Codex requested unsupported interaction during {method}");
            }
        }
    }
    pub fn test(harness: &HarnessConfig) -> Result<Value> {
        let mut event = |_: Event<'_>| Ok(());
        let mut client = Self::connect(harness, &mut event)?;
        let account = client.request("account/read", json!({"refreshToken":false}), &mut event)?;
        Ok(
            json!({"status":"connected","engine":"codex","authenticated":!account["account"].is_null(),"requires_openai_auth":account["requiresOpenaiAuth"],"inference":false}),
        )
    }
    /// Operations implemented against app-server, not terminal slash-command text.
    pub fn capabilities() -> Value {
        json!({"engine":"codex","operations":["models","configure","rename","compact","skills","mcp","status"],"unsupported":["review","diff","fork","import","interactive_approval"]})
    }
    pub fn models(harness: &HarnessConfig) -> Result<Value> {
        let mut event = |_: Event<'_>| Ok(());
        let mut client = Self::connect(harness, &mut event)?;
        // Metadata requests must finish before the Web CLI request timeout.
        client.deadline = Instant::now() + Duration::from_secs(10);
        client.catalog(
            "model/list",
            json!({"limit":100,"includeHidden":false}),
            &mut event,
        )
    }
    fn catalog(
        &mut self,
        method: &str,
        mut params: Value,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Value> {
        let mut data = Vec::new();
        let mut cursors = std::collections::HashSet::new();
        loop {
            let page = self.request(method, params.clone(), event)?;
            data.extend(
                page["data"]
                    .as_array()
                    .context("missing catalog data")?
                    .iter()
                    .cloned(),
            );
            let Some(cursor) = page["nextCursor"].as_str() else {
                break;
            };
            if !cursors.insert(cursor.to_owned()) || data.len() > 10000 {
                bail!("invalid catalog pagination");
            }
            params["cursor"] = json!(cursor);
        }
        Ok(json!({"data":data}))
    }
    pub fn inspect(harness: &HarnessConfig, operation: &str) -> Result<Value> {
        let mut event = |_: Event<'_>| Ok(());
        let mut client = Self::connect(harness, &mut event)?;
        // Metadata requests must finish before the Web CLI request timeout.
        client.deadline = Instant::now() + Duration::from_secs(10);
        match operation {
            "skills" => client.request(
                "skills/list",
                json!({"cwds":[harness.working_directory],"forceReload":true}),
                &mut event,
            ),
            "mcp" => client.catalog("mcpServerStatus/list", json!({"limit":100}), &mut event),
            _ => bail!("unsupported inspection operation"),
        }
    }
    pub fn rename(harness: &HarnessConfig, thread: &str, name: &str) -> Result<()> {
        let mut event = |_: Event<'_>| Ok(());
        let mut client = Self::connect(harness, &mut event)?;
        // Metadata requests must finish before the Web CLI request timeout.
        client.deadline = Instant::now() + Duration::from_secs(10);
        client.request(
            "thread/name/set",
            json!({"threadId":thread,"name":name}),
            &mut event,
        )?;
        Ok(())
    }
    /// The RPC acknowledgement is not completion. Keep the connection alive until the turn ends.
    pub fn compact(
        harness: &HarnessConfig,
        thread: &str,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<()> {
        let mut client = Self::connect(harness, event)?;
        client.request("thread/resume", json!({"threadId":thread,"cwd":harness.working_directory,"model":harness.model,"approvalPolicy":"never","sandbox":harness.sandbox.protocol(),"config":{"model_reasoning_effort":harness.reasoning_effort}}), event)?;
        client.next_id += 1;
        let id = client.next_id;
        event(Event::Submitted)?;
        client
            .send(json!({"id":id,"method":"thread/compact/start","params":{"threadId":thread}}))?;
        let mut turn = None;
        let mut terminal = false;
        let result = (|| {
            loop {
                let value = client.receive(event)?;
                if value["id"] == id && value.get("method").is_none() {
                    if value.get("error").is_some() {
                        terminal = true;
                        event(Event::Settled("rejected"))?;
                        bail!("Codex compaction rejected: {}", value["error"]["message"]);
                    }
                    event(Event::Accepted)?;
                }
                let p = &value["params"];
                if p["threadId"] == thread && value["method"] == "turn/started" {
                    turn = p["turn"]["id"].as_str().map(str::to_owned);
                }
                if p["threadId"] == thread && value["method"] == "turn/completed" {
                    if turn.as_deref().is_some_and(|id| p["turn"]["id"] != id) {
                        continue;
                    }
                    terminal = true;
                    let status = match p["turn"]["status"].as_str() {
                        Some("completed") => "completed",
                        Some("interrupted") => "interrupted",
                        Some("failed") => "failed",
                        _ => bail!("unknown compaction status"),
                    };
                    event(Event::Settled(status))?;
                    if status == "completed" {
                        return Ok(());
                    }
                    bail!("Codex compaction did not complete");
                }
                if value.get("method").is_some() && value.get("id").is_some() {
                    bail!("Codex compaction requested unsupported interaction");
                }
            }
        })();
        if let Err(error) = &result {
            event(Event::Stopping(error))?;
        }
        if result.is_err()
            && !terminal
            && let Some(turn) = turn
        {
            client.interrupt_and_confirm(thread, &turn, event);
        }
        result
    }
    /// Cancellation is a request; only the matching terminal event confirms it.
    fn interrupt_and_confirm(
        &mut self,
        thread: &str,
        turn: &str,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Option<String> {
        if event(Event::InterruptRequested).is_err() {
            return None;
        }
        self.next_id += 1;
        if self.send(json!({"id":self.next_id,"method":"turn/interrupt","params":{"threadId":thread,"turnId":turn}})).is_err() { return None; }
        let mut output = None;
        self.deadline = Instant::now() + Duration::from_secs(2);
        while let Ok(value) = self.receive(&mut |_| Ok(())) {
            let p = &value["params"];
            if p["threadId"] == thread
                && p["turnId"] == turn
                && value["method"] == "item/completed"
                && p["item"]["type"] == "agentMessage"
                && p["item"]["phase"] != "commentary"
            {
                output = p["item"]["text"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_owned);
            }
            if value["method"] == "turn/completed"
                && p["threadId"] == thread
                && p["turn"]["id"] == turn
            {
                let status = match p["turn"]["status"].as_str() {
                    Some("interrupted") => "interrupted",
                    Some("failed") => "failed",
                    Some("completed") => "completed",
                    _ => return None,
                };
                if event(Event::Settled(status)).is_err() {
                    return None;
                }
                return if status == "completed" { output } else { None };
            }
        }
        None
    }
    pub fn execute(
        &mut self,
        harness: &HarnessConfig,
        execution: Execution<'_>,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
        tool: &mut dyn FnMut(&str, Value) -> Result<Value>,
    ) -> Result<String> {
        let mut params = json!({"cwd":harness.working_directory,"approvalPolicy":"never","sandbox":harness.sandbox.protocol(),"developerInstructions":execution.instructions});
        if let Some(model) = &harness.model {
            params["model"] = json!(model);
        }
        if let Some(effort) = &harness.reasoning_effort {
            params["config"] = json!({"model_reasoning_effort":effort});
        }
        let method = if let Some(thread) = execution.thread {
            params["threadId"] = json!(thread);
            "thread/resume"
        } else {
            params["dynamicTools"] = json!(execution.tools.iter().map(|t| json!({"type":"function","name":t.name,"description":t.description,"inputSchema":t.input_schema})).collect::<Vec<_>>());
            "thread/start"
        };
        let response = self.request(method, params, event)?;
        if let Some(model) = response["model"].as_str() {
            event(Event::Model(model))?;
        }
        if let Some(effort) = response["reasoningEffort"].as_str() {
            event(Event::ReasoningEffort(effort))?;
        }
        let thread = response["thread"]["id"]
            .as_str()
            .context("Codex omitted thread id")?
            .to_owned();
        event(Event::Thread(&thread))?;
        self.next_id += 1;
        let start_id = self.next_id;
        event(Event::Submitted)?;
        self.send(json!({"id":start_id,"method":"turn/start","params":{"threadId":thread,"input":[{"type":"text","text":execution.input}],"model":harness.model,"effort":harness.reasoning_effort}}))?;
        let mut turn = None;
        let mut final_text = String::new();
        let mut terminal = false;
        let result = (|| {
            loop {
                let value = self.receive(event)?;
                if value["id"] == start_id && value.get("method").is_none() {
                    if value.get("error").is_some() {
                        terminal = true;
                        event(Event::Settled("rejected"))?;
                        bail!(
                            "Codex turn/start rejected: {}",
                            value["error"]["message"]
                                .as_str()
                                .unwrap_or("protocol error")
                        );
                    }
                    if let Some(id) = value["result"]["turn"]["id"].as_str() {
                        turn = Some(id.to_owned());
                        event(Event::Turn(id))?;
                    }
                    continue;
                }
                let method = value["method"].as_str().unwrap_or("");
                let p = &value["params"];
                if p.get("threadId").is_some_and(|id| id != &thread) {
                    continue;
                }
                if p.get("turnId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| turn.as_deref().is_some_and(|current| current != id))
                {
                    continue;
                }
                if method == "item/tool/call" {
                    event(Event::Activity {
                        tool_name: None,
                        id: p["callId"].as_str().unwrap_or("dynamic"),
                        kind: "framework",
                        status: "running",
                    })?;
                    let name = p["tool"].as_str().context("missing dynamic tool name")?;
                    let allowed = execution.tools.iter().any(|t| t.name == name);
                    let result = if allowed {
                        tool(name, p["arguments"].clone())
                    } else {
                        Err(anyhow::anyhow!("tool unavailable"))
                    };
                    let success = result.is_ok();
                    event(Event::Activity {
                        tool_name: None,
                        id: p["callId"].as_str().unwrap_or("dynamic"),
                        kind: "framework",
                        status: if success { "completed" } else { "failed" },
                    })?;
                    let text = match result {
                        Ok(v) => v.to_string(),
                        Err(e) => e.to_string(),
                    };
                    self.send(json!({"id":value["id"],"result":{"success":success,"contentItems":[{"type":"inputText","text":text}]}}))?;
                } else if value.get("id").is_some() && value.get("method").is_some() {
                    event(Event::Activity {
                        tool_name: None,
                        id: "input",
                        kind: "input",
                        status: "unsupported",
                    })?;
                    self.send(json!({"id":value["id"],"error":{"code":-32601,"message":"AGT does not support this interactive request; task stopped without approval"}}))?;
                    bail!(
                        "Codex requested {method}; interactive approvals/input are not supported by this harness"
                    );
                } else if method == "turn/started" {
                    if let Some(id) = p["turn"]["id"].as_str() {
                        turn = Some(id.to_owned());
                        event(Event::Turn(id))?;
                    }
                } else if method == "item/agentMessage/delta" {
                    event(Event::Text(p["delta"].as_str().unwrap_or("")))?;
                } else if matches!(method, "item/started" | "item/completed")
                    && matches!(
                        p["item"]["type"].as_str(),
                        Some("commandExecution" | "fileChange" | "mcpToolCall" | "dynamicToolCall")
                    )
                {
                    let kind = match p["item"]["type"].as_str() {
                        Some("commandExecution") => "command",
                        Some("fileChange") => "files",
                        Some("mcpToolCall") => "mcp",
                        _ => "framework",
                    };
                    let status = if method == "item/started" {
                        "running"
                    } else if matches!(p["item"]["status"].as_str(), Some("failed" | "declined")) {
                        "failed"
                    } else {
                        "completed"
                    };
                    event(Event::Activity {
                        tool_name: if kind == "mcp" {
                            p["item"]["tool"].as_str()
                        } else {
                            None
                        },
                        id: p["item"]["id"].as_str().unwrap_or("unknown"),
                        kind,
                        status,
                    })?;
                } else if method == "item/completed" && p["item"]["type"] == "agentMessage" {
                    if p["item"]["phase"] != "commentary" {
                        final_text = p["item"]["text"].as_str().unwrap_or("").to_owned();
                    }
                } else if method == "turn/completed" {
                    if turn.as_deref().is_some_and(|id| p["turn"]["id"] != id) {
                        continue;
                    }
                    let status = match p["turn"]["status"].as_str() {
                        Some("completed") => "completed",
                        Some("interrupted") => "interrupted",
                        Some("failed") => "failed",
                        _ => bail!("unknown Codex turn status"),
                    };
                    terminal = true;
                    event(Event::Settled(status))?;
                    match p["turn"]["status"].as_str() {
                        Some("completed") => {
                            event(Event::Completed)?;
                            if final_text.trim().is_empty() {
                                return Err(Failure::new(FailureCode::ResultMissing, "Codex completed without a final response; task was not replayed").into());
                            }
                            return Ok(final_text);
                        }
                        Some("interrupted") => {
                            return Err(Failure::new(
                                FailureCode::Interrupted,
                                "Codex turn interrupted",
                            )
                            .into());
                        }
                        _ => {
                            return Err(Failure::new(
                                FailureCode::EngineFailed,
                                format!(
                                    "Codex turn failed: {}",
                                    p["turn"]["error"]["message"]
                                        .as_str()
                                        .unwrap_or("unknown error")
                                ),
                            )
                            .into());
                        }
                    }
                }
            }
        })();
        if let Err(error) = &result {
            event(Event::Stopping(error))?;
        }
        if result.is_err()
            && !terminal
            && let Some(turn) = turn
            && let Some(output) = self.interrupt_and_confirm(&thread, &turn, event)
        {
            return Ok(output);
        }
        result
    }
}
impl Drop for Codex {
    fn drop(&mut self) {
        // Kill the local process group as well as the app-server; tools may own child processes.
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
