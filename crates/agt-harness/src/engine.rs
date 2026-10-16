//! One interface over the external execution engines; `HarnessConfig::engine`
//! selects the implementation. Adding an engine means adding an arm here.
use crate::acp::{self, Acp};
use crate::claude::{self, ClaudeCode};
use crate::{ApprovalHandler, Codex, Event, Execution, HarnessConfig, ProcessDiagnostics};
use anyhow::{Result, bail};
use serde_json::{Value, json};

pub const CODEX: &str = "codex";
pub const CLAUDE_CODE: &str = "claude-code";
/// DeepSeek Harness (`dsh`), driven over the Agent Client Protocol.
pub const DEEPSEEK_HARNESS: &str = "deepseek-harness";
/// Any other Agent Client Protocol agent, started with the harness's own `args`.
pub const ACP: &str = "acp";
pub const ENGINES: [&str; 4] = [CODEX, CLAUDE_CODE, DEEPSEEK_HARNESS, ACP];

/// A connected engine that runs one turn with agt's tools and approvals.
pub trait Engine {
    fn execute(
        &mut self,
        harness: &HarnessConfig,
        execution: Execution<'_>,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
        tool: &mut dyn FnMut(&str, Value) -> Result<Value>,
        approvals: Option<&mut dyn ApprovalHandler>,
    ) -> Result<String>;
    fn diagnostics(&mut self) -> ProcessDiagnostics;
}

impl Engine for Codex {
    fn execute(
        &mut self,
        harness: &HarnessConfig,
        execution: Execution<'_>,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
        tool: &mut dyn FnMut(&str, Value) -> Result<Value>,
        approvals: Option<&mut dyn ApprovalHandler>,
    ) -> Result<String> {
        self.execute_with_approvals(harness, execution, event, tool, approvals)
    }
    fn diagnostics(&mut self) -> ProcessDiagnostics {
        Codex::diagnostics(self)
    }
}

impl Engine for ClaudeCode {
    fn execute(
        &mut self,
        harness: &HarnessConfig,
        execution: Execution<'_>,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
        tool: &mut dyn FnMut(&str, Value) -> Result<Value>,
        approvals: Option<&mut dyn ApprovalHandler>,
    ) -> Result<String> {
        ClaudeCode::execute(self, harness, execution, event, tool, approvals)
    }
    fn diagnostics(&mut self) -> ProcessDiagnostics {
        ClaudeCode::diagnostics(self)
    }
}

impl Engine for Acp {
    fn execute(
        &mut self,
        harness: &HarnessConfig,
        execution: Execution<'_>,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
        tool: &mut dyn FnMut(&str, Value) -> Result<Value>,
        approvals: Option<&mut dyn ApprovalHandler>,
    ) -> Result<String> {
        Acp::execute(self, harness, execution, event, tool, approvals)
    }
    fn diagnostics(&mut self) -> ProcessDiagnostics {
        Acp::diagnostics(self)
    }
}

/// Start the engine a harness names. Codex connects now; Claude Code starts its
/// process per turn.
pub fn connect(
    harness: &HarnessConfig,
    event: &mut dyn FnMut(Event<'_>) -> Result<()>,
) -> Result<Box<dyn Engine>> {
    Ok(match harness.engine.as_str() {
        CODEX => Box::new(Codex::connect(harness, event)?),
        CLAUDE_CODE => {
            harness.validate()?;
            Box::new(ClaudeCode::new())
        }
        DEEPSEEK_HARNESS | ACP => {
            harness.validate()?;
            Box::new(Acp::new(acp::Launch::for_harness(harness)))
        }
        other => bail!("unsupported execution engine: {other}"),
    })
}

/// Check that the engine starts, without running inference.
pub fn test(harness: &HarnessConfig) -> Result<Value> {
    match harness.engine.as_str() {
        CODEX => Codex::test(harness),
        CLAUDE_CODE => claude::test(harness),
        DEEPSEEK_HARNESS | ACP => acp::test(harness),
        other => bail!("unsupported execution engine: {other}"),
    }
}

/// What each engine supports, keyed by engine.
pub fn capabilities() -> Value {
    let mut codex = Codex::capabilities();
    let engines = json!({CODEX: codex.clone(), CLAUDE_CODE: claude::capabilities(),
        DEEPSEEK_HARNESS: acp::capabilities(DEEPSEEK_HARNESS), ACP: acp::capabilities(ACP)});
    codex["engines"] = engines;
    codex
}

pub fn models(harness: &HarnessConfig) -> Result<Value> {
    match harness.engine.as_str() {
        CODEX => Codex::models(harness),
        CLAUDE_CODE => Ok(claude::models()),
        // Models come from the engine's own provider setup, offered per session.
        DEEPSEEK_HARNESS | ACP => {
            bail!("models are offered per session by the agent; name one in the harness")
        }
        other => bail!("unsupported execution engine: {other}"),
    }
}

pub fn inspect(harness: &HarnessConfig, operation: &str) -> Result<Value> {
    match harness.engine.as_str() {
        CODEX => Codex::inspect(harness, operation),
        _ => bail!(
            "{operation} is not supported by the {} engine",
            harness.engine
        ),
    }
}

/// Rename the engine's session. Claude Code sessions keep agt's title only.
pub fn rename(harness: &HarnessConfig, thread: &str, name: &str) -> Result<()> {
    match harness.engine.as_str() {
        CODEX => Codex::rename(harness, thread, name),
        CLAUDE_CODE | DEEPSEEK_HARNESS | ACP => Ok(()),
        other => bail!("unsupported execution engine: {other}"),
    }
}

pub fn compact(
    harness: &HarnessConfig,
    thread: &str,
    event: &mut dyn FnMut(Event<'_>) -> Result<()>,
) -> Result<()> {
    match harness.engine.as_str() {
        CODEX => Codex::compact(harness, thread, event),
        _ => bail!(
            "compaction is not supported by the {} engine",
            harness.engine
        ),
    }
}
