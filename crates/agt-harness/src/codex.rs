use crate::Environment;
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
    Text(&'a str),
    Tool,
    Thread(&'a str),
    Turn(&'a str),
    Model(&'a str),
}
/// A single local app-server connection. stdout is protocol-only; stderr is drained, not exposed.
pub struct Codex {
    child: Child,
    input: ChildStdin,
    messages: Receiver<Result<Value>>,
    next_id: u64,
    deadline: Instant,
}
impl Codex {
    pub fn connect(
        environment: &Environment,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Self> {
        environment.validate()?;
        let mut command = Command::new(&environment.executable);
        command
            .arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .current_dir(&environment.working_directory)
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
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut stderr, &mut std::io::sink());
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
            deadline: Instant::now() + Duration::from_secs(15),
        };
        client.request("initialize", json!({"clientInfo":{"name":"agt","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}), event)?;
        client.send(json!({"method":"initialized","params":{}}))?;
        client.deadline = Instant::now() + Duration::from_secs(environment.timeout_seconds);
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
                bail!("Codex execution timed out; inspect the run before retrying");
            }
            match self.messages.recv_timeout(Duration::from_millis(50)) {
                Ok(value) => return value,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => bail!("Codex app-server disconnected; task outcome may be incomplete"),
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
    pub fn test(environment: &Environment) -> Result<Value> {
        let mut event = |_: Event<'_>| Ok(());
        let mut client = Self::connect(environment, &mut event)?;
        let account = client.request("account/read", json!({"refreshToken":false}), &mut event)?;
        Ok(
            json!({"status":"connected","engine":"codex","authenticated":!account["account"].is_null(),"requires_openai_auth":account["requiresOpenaiAuth"],"inference":false}),
        )
    }
    /// Operations implemented against app-server, not terminal slash-command text.
    pub fn capabilities() -> Value {
        json!({"engine":"codex","operations":["models","configure","rename","compact","skills","mcp","status"],"unsupported":["review","diff","fork","import","interactive_approval"]})
    }
    pub fn models(environment: &Environment) -> Result<Value> {
        let mut event = |_: Event<'_>| Ok(());
        let mut client = Self::connect(environment, &mut event)?;
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
    pub fn inspect(environment: &Environment, operation: &str) -> Result<Value> {
        let mut event = |_: Event<'_>| Ok(());
        let mut client = Self::connect(environment, &mut event)?;
        match operation {
            "skills" => client.request(
                "skills/list",
                json!({"cwds":[environment.working_directory],"forceReload":true}),
                &mut event,
            ),
            "mcp" => client.catalog("mcpServerStatus/list", json!({"limit":100}), &mut event),
            _ => bail!("unsupported inspection operation"),
        }
    }
    pub fn rename(environment: &Environment, thread: &str, name: &str) -> Result<()> {
        let mut event = |_: Event<'_>| Ok(());
        let mut client = Self::connect(environment, &mut event)?;
        client.request(
            "thread/name/set",
            json!({"threadId":thread,"name":name}),
            &mut event,
        )?;
        Ok(())
    }
    /// The RPC acknowledgement is not completion. Keep the connection alive until the turn ends.
    pub fn compact(
        environment: &Environment,
        thread: &str,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<()> {
        let mut client = Self::connect(environment, event)?;
        client.request("thread/resume", json!({"threadId":thread,"cwd":environment.working_directory,"model":environment.model,"approvalPolicy":"never","sandbox":environment.sandbox.protocol(),"config":{"model_reasoning_effort":environment.reasoning_effort}}), event)?;
        client.next_id += 1;
        let id = client.next_id;
        client
            .send(json!({"id":id,"method":"thread/compact/start","params":{"threadId":thread}}))?;
        loop {
            let value = client.receive(event)?;
            if value["id"] == id && value.get("error").is_some() {
                bail!("Codex compaction rejected: {}", value["error"]["message"]);
            }
            if value["params"]["threadId"] == thread && value["method"] == "turn/completed" {
                if value["params"]["turn"]["status"] == "completed" {
                    return Ok(());
                }
                bail!("Codex compaction did not complete");
            }
            if value.get("method").is_some() && value.get("id").is_some() {
                bail!("Codex compaction requested unsupported interaction");
            }
        }
    }
    pub fn execute(
        &mut self,
        environment: &Environment,
        execution: Execution<'_>,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
        tool: &mut dyn FnMut(&str, Value) -> Result<Value>,
    ) -> Result<String> {
        let mut params = json!({"cwd":environment.working_directory,"approvalPolicy":"never","sandbox":environment.sandbox.protocol(),"developerInstructions":execution.instructions});
        if let Some(model) = &environment.model {
            params["model"] = json!(model);
        }
        if let Some(effort) = &environment.reasoning_effort {
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
        let thread = response["thread"]["id"]
            .as_str()
            .context("Codex omitted thread id")?
            .to_owned();
        event(Event::Thread(&thread))?;
        self.next_id += 1;
        let start_id = self.next_id;
        self.send(json!({"id":start_id,"method":"turn/start","params":{"threadId":thread,"input":[{"type":"text","text":execution.input}],"model":environment.model,"effort":environment.reasoning_effort}}))?;
        let mut turn = None;
        let mut final_text = String::new();
        let result = (|| {
            loop {
                let value = self.receive(event)?;
                if value["id"] == start_id && value.get("method").is_none() {
                    if value.get("error").is_some() {
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
                if method == "item/tool/call" {
                    event(Event::Tool)?;
                    let name = p["tool"].as_str().context("missing dynamic tool name")?;
                    let allowed = execution.tools.iter().any(|t| t.name == name);
                    let result = if allowed {
                        tool(name, p["arguments"].clone())
                    } else {
                        Err(anyhow::anyhow!("tool unavailable"))
                    };
                    let success = result.is_ok();
                    let text = match result {
                        Ok(v) => v.to_string(),
                        Err(e) => e.to_string(),
                    };
                    self.send(json!({"id":value["id"],"result":{"success":success,"contentItems":[{"type":"inputText","text":text}]}}))?;
                } else if value.get("id").is_some() && value.get("method").is_some() {
                    self.send(json!({"id":value["id"],"error":{"code":-32601,"message":"AGT does not support this interactive request; task stopped without approval"}}))?;
                    bail!(
                        "Codex requested {method}; interactive approvals/input are not supported by this environment"
                    );
                } else if method == "turn/started" {
                    if let Some(id) = p["turn"]["id"].as_str() {
                        turn = Some(id.to_owned());
                        event(Event::Turn(id))?;
                    }
                } else if method == "item/agentMessage/delta" {
                    event(Event::Text(p["delta"].as_str().unwrap_or("")))?;
                } else if method == "item/started"
                    && matches!(
                        p["item"]["type"].as_str(),
                        Some("commandExecution" | "fileChange" | "mcpToolCall" | "dynamicToolCall")
                    )
                {
                    event(Event::Tool)?;
                } else if method == "item/completed" && p["item"]["type"] == "agentMessage" {
                    if p["item"]["phase"] != "commentary" {
                        final_text = p["item"]["text"].as_str().unwrap_or("").to_owned();
                    }
                } else if method == "turn/completed" {
                    if turn.as_deref().is_some_and(|id| p["turn"]["id"] != id) {
                        continue;
                    }
                    match p["turn"]["status"].as_str() {
                        Some("completed") if !final_text.trim().is_empty() => {
                            return Ok(final_text);
                        }
                        Some("completed") => {
                            bail!("Codex completed without a final response; task was not replayed")
                        }
                        Some("interrupted") => bail!("Codex turn interrupted"),
                        _ => bail!(
                            "Codex turn failed: {}",
                            p["turn"]["error"]["message"]
                                .as_str()
                                .unwrap_or("unknown error")
                        ),
                    }
                }
            }
        })();
        if result.is_err()
            && let Some(turn) = turn
        {
            let _ = self.send(json!({"id":self.next_id+1,"method":"turn/interrupt","params":{"threadId":thread,"turnId":turn}}));
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
