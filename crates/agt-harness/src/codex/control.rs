//! Metadata operations and explicit compaction.
use super::*;
impl Codex {
    pub fn test(harness: &HarnessConfig) -> Result<Value> {
        let mut event = |_: Event<'_>| Ok(());
        let mut client = Self::connect(harness, &mut event)?;
        let account =
            client
                .transport
                .request("account/read", json!({"refreshToken":false}), &mut event)?;
        Ok(
            json!({"status":"connected","engine":"codex","authenticated":!account["account"].is_null(),"requires_openai_auth":account["requiresOpenaiAuth"],"inference":false}),
        )
    }
    /// Operations implemented against app-server, not terminal slash-command text.
    pub fn capabilities() -> Value {
        json!({"engine":"codex","operations":["models","configure","rename","compact","skills","mcp","status","interactive_approval"],"approval_requests":["commandExecution","fileChange","permissions"],"unsupported":["review","diff","fork","import"]})
    }
    pub fn models(harness: &HarnessConfig) -> Result<Value> {
        let mut event = |_: Event<'_>| Ok(());
        let mut client = Self::connect(harness, &mut event)?;
        // Metadata requests must finish before the Web CLI request timeout.
        client.transport.process.deadline = Instant::now() + Duration::from_secs(10);
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
            let page = self.transport.request(method, params.clone(), event)?;
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
        client.transport.process.deadline = Instant::now() + Duration::from_secs(10);
        match operation {
            "skills" => client.transport.request(
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
        client.transport.process.deadline = Instant::now() + Duration::from_secs(10);
        client.transport.request(
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
        client.transport.request("thread/resume", json!({"threadId":thread,"cwd":harness.working_directory,"model":harness.model,"approvalPolicy":"never","sandbox":harness.sandbox.protocol(),"config":{"model_reasoning_effort":harness.reasoning_effort}}), event)?;
        client.transport.next_id += 1;
        let id = client.transport.next_id;
        event(Event::Submitted)?;
        client
            .transport
            .send(json!({"id":id,"method":"thread/compact/start","params":{"threadId":thread}}))?;
        let mut turn = None;
        let mut terminal = false;
        let result = (|| {
            loop {
                let value = client.transport.receive(event)?;
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
            // Diagnostic persistence must not prevent protocol cleanup. Keep the original error.
            let _ = event(Event::Stopping(error));
        }
        if result.is_err()
            && !terminal
            && let Some(turn) = turn
        {
            client.interrupt_and_confirm(thread, &turn, event);
        }
        result
    }
}
