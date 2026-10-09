//! Thread preparation, turn event pump and dynamic-tool responses.
use super::*;
impl Codex {
    pub fn execute(
        &mut self,
        harness: &HarnessConfig,
        execution: Execution<'_>,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
        tool: &mut dyn FnMut(&str, Value) -> Result<Value>,
    ) -> Result<String> {
        self.execute_with_approvals(harness, execution, event, tool, None)
    }
    pub fn execute_with_approvals(
        &mut self,
        harness: &HarnessConfig,
        execution: Execution<'_>,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
        tool: &mut dyn FnMut(&str, Value) -> Result<Value>,
        mut approvals: Option<&mut dyn crate::ApprovalHandler>,
    ) -> Result<String> {
        let validators = execution
            .tools
            .iter()
            .map(|tool| {
                agt_core::tool_schema::ToolSchema::compile(&tool.input_schema)
                    .map(|schema| (tool.name.clone(), schema))
            })
            .collect::<std::result::Result<std::collections::BTreeMap<_, _>, _>>()?;
        let mut params = json!({"cwd":harness.working_directory,"approvalPolicy":if approvals.is_some(){"on-request"}else{"never"},"approvalsReviewer":"user","sandbox":harness.sandbox.protocol(),"developerInstructions":execution.instructions});
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
        let response = self.transport.request(method, params, event)?;
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
        self.transport.next_id += 1;
        let start_id = self.transport.next_id;
        event(Event::Submitted)?;
        self.transport.send(json!({"id":start_id,"method":"turn/start","params":{"threadId":thread,"input":[{"type":"text","text":execution.input}],"model":harness.model,"effort":harness.reasoning_effort}}))?;
        let mut turn = None;
        let mut final_text = String::new();
        let mut terminal = false;
        let mut item_contexts = std::collections::HashMap::new();
        let mut pending: Vec<super::approval::PendingApproval> = Vec::new();
        let result = (|| {
            let mut frames = 0;
            loop {
                let Some(value) = self.transport.poll(event)? else {
                    if let Some(handler) = approvals.as_deref_mut() {
                        self.flush_approvals(&mut pending, handler, event)?;
                    }
                    continue;
                };
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
                frames += 1;
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
                if matches!(method, "item/started" | "item/completed")
                    && let Some(id) = p["item"]["id"].as_str()
                    && matches!(
                        p["item"]["type"].as_str(),
                        Some("fileChange" | "commandExecution")
                    )
                {
                    item_contexts.insert(id.to_owned(), p["item"].clone());
                }
                if matches!(
                    method,
                    "item/commandExecution/requestApproval"
                        | "item/fileChange/requestApproval"
                        | "item/permissions/requestApproval"
                ) && approvals.is_some()
                {
                    let mut request = super::approval::parse(&value)?;
                    request.item_context = item_contexts
                        .get(request.params["itemId"].as_str().unwrap())
                        .cloned();
                    if let Some(previous) =
                        pending.iter().find(|e| e.request.rpc_id == request.rpc_id)
                    {
                        if previous.request.rpc_id != request.rpc_id
                            || previous.request.method != request.method
                            || previous.request.params != request.params
                            || previous.request.options != request.options
                        {
                            bail!("approval request identity reused")
                        }
                    } else {
                        let host_id = approvals.as_deref_mut().unwrap().request(&request)?;
                        pending.push(super::approval::PendingApproval {
                            request,
                            host_id,
                            sent: false,
                        });
                    }
                } else if method == "serverRequest/resolved" {
                    if let Some(entry) = pending
                        .iter_mut()
                        .find(|e| e.request.rpc_id == p["requestId"])
                    {
                        entry.sent = true;
                        if let Some(handler) = approvals.as_deref_mut() {
                            handler.delivery(&entry.host_id, crate::DeliveryProgress::Resolved)?;
                        }
                    }
                } else if method == "item/tool/call" {
                    self.call_tool(&value, &validators, event, tool)?;
                } else if value.get("id").is_some() && value.get("method").is_some() {
                    event(Event::Activity {
                        tool_name: None,
                        id: "input",
                        kind: "input",
                        status: "unsupported",
                    })?;
                    self.transport.send(json!({"id":value["id"],"error":{"code":-32601,"message":"AGT does not support this interactive request; task stopped without approval"}}))?;
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
                    event(Event::ToolDetail {
                        id: p["item"]["id"].as_str().unwrap_or("unknown"),
                        item: &p["item"],
                    })?;
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
                // Bound response latency even when the engine continuously emits progress.
                if frames >= 32 {
                    if let Some(handler) = approvals.as_deref_mut() {
                        self.flush_approvals(&mut pending, handler, event)?;
                    }
                    frames = 0;
                }
            }
        })();
        self.finish_turn(result, &thread, start_id, turn, terminal, event)
    }
    fn call_tool(
        &mut self,
        value: &Value,
        validators: &std::collections::BTreeMap<String, agt_core::tool_schema::ToolSchema>,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
        tool: &mut dyn FnMut(&str, Value) -> Result<Value>,
    ) -> Result<()> {
        let p = &value["params"];
        event(Event::ToolDetail {
            id: p["callId"].as_str().unwrap_or("dynamic"),
            item: &json!({"arguments":p["arguments"]}),
        })?;
        event(Event::Activity {
            tool_name: None,
            id: p["callId"].as_str().unwrap_or("dynamic"),
            kind: "framework",
            status: "running",
        })?;
        let name = p["tool"].as_str().context("missing dynamic tool name")?;
        let result = if let Some(validator) = validators.get(name) {
            validator
                .validate(&p["arguments"])
                .map_err(anyhow::Error::from)
                .and_then(|()| tool(name, p["arguments"].clone()))
        } else {
            Err(anyhow::anyhow!("tool unavailable"))
        };
        let success = result.is_ok();
        let payload = match &result {
            Ok(value) => json!({"result":value}),
            Err(error) => json!({"error":error.to_string()}),
        };
        event(Event::ToolDetail {
            id: p["callId"].as_str().unwrap_or("dynamic"),
            item: &payload,
        })?;
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
        self.transport.send(json!({"id":value["id"],"result":{"success":success,"contentItems":[{"type":"inputText","text":text}]}}))?;
        Ok(())
    }
}
