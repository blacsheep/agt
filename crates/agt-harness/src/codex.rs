use crate::{Failure, FailureCode, HarnessConfig, ProcessDiagnostics};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

mod approval;
mod cancellation;
mod control;
mod transport;
mod turn;

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
    ToolDetail {
        id: &'a str,
        item: &'a Value,
    },
    Completed,
    Text(&'a str),
    Tool,
    Thread(&'a str),
    Turn(&'a str),
    Model(&'a str),
    ReasoningEffort(&'a str),
}
/// A single local app-server connection. The engine owns its model/tool loop.
pub struct Codex {
    transport: transport::Transport,
}
impl Codex {
    pub fn connect(
        harness: &HarnessConfig,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Self> {
        Ok(Self {
            transport: transport::Transport::connect(harness, event)?,
        })
    }
    pub fn diagnostics(&mut self) -> ProcessDiagnostics {
        self.transport.diagnostics()
    }
}
