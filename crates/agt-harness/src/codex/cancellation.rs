//! Stop recovery never dispatches tools or retries a submitted turn.
use super::*;
impl Codex {
    /// Cancellation is a request; only the matching terminal event confirms it.
    pub(super) fn interrupt_and_confirm(
        &mut self,
        thread: &str,
        turn: &str,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Option<String> {
        if event(Event::InterruptRequested).is_err() {
            return None;
        }
        self.transport.next_id += 1;
        if self.transport.send(json!({"id":self.transport.next_id,"method":"turn/interrupt","params":{"threadId":thread,"turnId":turn}})).is_err() { return None; }
        let mut output = None;
        self.transport.process.deadline = Instant::now() + Duration::from_secs(2);
        while let Ok(value) = self.transport.receive(&mut |_| Ok(())) {
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
    pub(super) fn finish_turn(
        &mut self,
        result: Result<String>,
        thread: &str,
        start_id: u64,
        mut turn: Option<String>,
        mut terminal: bool,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<String> {
        if let Err(error) = &result {
            // Diagnostic persistence must not prevent protocol cleanup. Keep the original error.
            let _ = event(Event::Stopping(error));
        }
        if result.is_err() && !terminal && turn.is_none() {
            // Submission may have reached the engine before cancellation was observed.
            // Read only protocol facts here: never dispatch tools after the stop request.
            self.transport.process.deadline = Instant::now() + Duration::from_secs(2);
            while let Ok(value) = self.transport.receive(&mut |_| Ok(())) {
                if value["id"] == start_id && value.get("method").is_none() {
                    if value.get("error").is_some() {
                        terminal = true;
                        event(Event::Settled("rejected"))?;
                        break;
                    }
                    turn = value["result"]["turn"]["id"].as_str().map(str::to_owned);
                } else if value["method"] == "turn/started" && value["params"]["threadId"] == thread
                {
                    turn = value["params"]["turn"]["id"].as_str().map(str::to_owned);
                }
                if turn.is_some() {
                    break;
                }
            }
        }
        if result.is_err()
            && !terminal
            && let Some(turn) = turn
            && let Some(output) = self.interrupt_and_confirm(thread, &turn, event)
        {
            return Ok(output);
        }
        result
    }
}
