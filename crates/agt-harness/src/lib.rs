//! Execution engine adapters and reusable harness definitions. No IM or scheduling policy.
mod acp;
mod approval;
mod claude;
mod codex;
mod engine;
pub use approval::{ApprovalHandler, ApprovalRequest, DeliveryProgress};
pub use engine::{
    CLAUDE_CODE, CODEX, DEEPSEEK_HARNESS, ENGINES, Engine, capabilities, compact, connect, inspect,
    models, rename, test,
};
mod diagnostics;
pub use diagnostics::{Failure, FailureCode, ProcessDiagnostics};
mod mcp_http;
mod process;
mod resource;
mod tools;
pub use codex::{Codex, Event, Execution, Tool};
pub use resource::{HarnessConfig, HarnessStore, Sandbox, StoredHarness};
